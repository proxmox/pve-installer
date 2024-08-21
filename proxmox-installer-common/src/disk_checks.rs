use std::collections::HashSet;

use crate::options::{BtrfsRaidLevel, Disk, ZfsRaidLevel};
use crate::setup::BootType;

/// Checks a list of disks for duplicate entries, using their index as key.
///
/// # Arguments
///
/// * `disks` - A list of disks to check for duplicates.
pub fn check_for_duplicate_disks(disks: &[Disk]) -> Result<(), &Disk> {
    let mut set = HashSet::new();

    for disk in disks {
        if !set.insert(&disk.index) {
            return Err(disk);
        }
    }

    Ok(())
}

/// Simple wrapper which returns an descriptive error if the list of disks is too short.
///
/// # Arguments
///
/// * `disks` - A list of disks to check the length of.
/// * `min` - Minimum number of disks
pub fn check_raid_min_disks(disks: &[Disk], min: usize) -> Result<(), String> {
    if disks.len() < min {
        Err(format!("Need at least {min} disks"))
    } else {
        Ok(())
    }
}

/// Checks all disks for legacy BIOS boot compatibility and reports an error as appropriate. 4Kn
/// disks are generally broken with legacy BIOS and cannot be booted from.
///
/// # Arguments
///
/// * `runinfo` - `RuntimeInfo` instance of currently running system
/// * `disks` - List of disks designated as bootdisk targets.
pub fn check_disks_4kn_legacy_boot(boot_type: BootType, disks: &[Disk]) -> Result<(), &str> {
    if boot_type == BootType::Bios && disks.iter().any(|disk| disk.block_size == Some(4096)) {
        return Err("Booting from 4Kn drive in legacy BIOS mode is not supported.");
    }

    Ok(())
}

/// Checks whether a user-supplied ZFS RAID setup is valid or not, such as disk sizes andminimum
/// number of disks.
///
/// # Arguments
///
/// * `level` - The targeted ZFS RAID level by the user.
/// * `disks` - List of disks designated as RAID targets.
pub fn check_zfs_raid_config(level: ZfsRaidLevel, disks: &[Disk]) -> Result<(), String> {
    // See also Proxmox/Install.pm:get_zfs_raid_setup()

    let check_mirror_size = |disk1: &Disk, disk2: &Disk| {
        if (disk1.size - disk2.size).abs() > disk1.size / 10. {
            Err(format!(
                "Mirrored disks must have same size:\n\n  * {disk1}\n  * {disk2}"
            ))
        } else {
            Ok(())
        }
    };

    match level {
        ZfsRaidLevel::Raid0 => check_raid_min_disks(disks, 1)?,
        ZfsRaidLevel::Raid1 => {
            check_raid_min_disks(disks, 2)?;
            for disk in disks {
                check_mirror_size(&disks[0], disk)?;
            }
        }
        ZfsRaidLevel::Raid10 => {
            check_raid_min_disks(disks, 4)?;

            if disks.len() % 2 != 0 {
                return Err(format!(
                    "Needs an even number of disks, currently selected: {}",
                    disks.len(),
                ));
            }

            // Pairs need to have the same size
            for i in (0..disks.len()).step_by(2) {
                check_mirror_size(&disks[i], &disks[i + 1])?;
            }
        }
        // For RAID-Z: minimum disks number is level + 2
        ZfsRaidLevel::RaidZ => {
            check_raid_min_disks(disks, 3)?;
            for disk in disks {
                check_mirror_size(&disks[0], disk)?;
            }
        }
        ZfsRaidLevel::RaidZ2 => {
            check_raid_min_disks(disks, 4)?;
            for disk in disks {
                check_mirror_size(&disks[0], disk)?;
            }
        }
        ZfsRaidLevel::RaidZ3 => {
            check_raid_min_disks(disks, 5)?;
            for disk in disks {
                check_mirror_size(&disks[0], disk)?;
            }
        }
    }

    Ok(())
}

/// Checks whether a user-supplied Btrfs RAID setup is valid or not, such as minimum
/// number of disks.
///
/// # Arguments
///
/// * `level` - The targeted Btrfs RAID level by the user.
/// * `disks` - List of disks designated as RAID targets.
pub fn check_btrfs_raid_config(level: BtrfsRaidLevel, disks: &[Disk]) -> Result<(), String> {
    // See also Proxmox/Install.pm:get_btrfs_raid_setup()

    match level {
        BtrfsRaidLevel::Raid0 => check_raid_min_disks(disks, 1)?,
        BtrfsRaidLevel::Raid1 => check_raid_min_disks(disks, 2)?,
        BtrfsRaidLevel::Raid10 => check_raid_min_disks(disks, 4)?,
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_disk(index: usize) -> Disk {
        Disk {
            index: index.to_string(),
            path: format!("/dev/dummy{index}"),
            model: Some("Dummy disk".to_owned()),
            size: 1024. * 1024. * 1024. * 8.,
            block_size: Some(512),
        }
    }

    fn dummy_disks(num: usize) -> Vec<Disk> {
        (0..num).map(dummy_disk).collect()
    }

    #[test]
    fn duplicate_disks() {
        assert!(check_for_duplicate_disks(&dummy_disks(2)).is_ok());
        assert_eq!(
            check_for_duplicate_disks(&[
                dummy_disk(0),
                dummy_disk(1),
                dummy_disk(2),
                dummy_disk(2),
                dummy_disk(3),
            ]),
            Err(&dummy_disk(2)),
        );
    }

    #[test]
    fn raid_min_disks() {
        let disks = dummy_disks(10);

        assert!(check_raid_min_disks(&disks[..1], 2).is_err());
        assert!(check_raid_min_disks(&disks[..1], 1).is_ok());
        assert!(check_raid_min_disks(&disks, 1).is_ok());
    }

    #[test]
    fn bios_boot_compat_4kn() {
        for i in 0..10 {
            let mut disks = dummy_disks(10);
            disks[i].block_size = Some(4096);

            // Must fail if /any/ of the disks are 4Kn
            assert!(check_disks_4kn_legacy_boot(BootType::Bios, &disks).is_err());
            // For UEFI, we allow it for every configuration
            assert!(check_disks_4kn_legacy_boot(BootType::Efi, &disks).is_ok());
        }
    }

    #[test]
    fn btrfs_raid() {
        let disks = dummy_disks(10);

        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid0, &[]).is_err());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid0, &disks[..1]).is_ok());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid0, &disks).is_ok());

        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid1, &[]).is_err());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid1, &disks[..1]).is_err());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid1, &disks[..2]).is_ok());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid1, &disks).is_ok());

        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid10, &[]).is_err());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid10, &disks[..3]).is_err());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid10, &disks[..4]).is_ok());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid10, &disks).is_ok());
    }

    #[test]
    fn zfs_raid() {
        let disks = dummy_disks(10);

        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid0, &[]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid0, &disks[..1]).is_ok());
        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid0, &disks).is_ok());

        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid1, &[]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid1, &disks[..2]).is_ok());
        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid1, &disks).is_ok());

        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid10, &[]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid10, &dummy_disks(4)).is_ok());
        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid10, &disks).is_ok());

        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ, &[]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ, &disks[..2]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ, &disks[..3]).is_ok());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ, &disks).is_ok());

        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ2, &[]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ2, &disks[..3]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ2, &disks[..4]).is_ok());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ2, &disks).is_ok());

        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ3, &[]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ3, &disks[..4]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ3, &disks[..5]).is_ok());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ3, &disks).is_ok());
    }
}
