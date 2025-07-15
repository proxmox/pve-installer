use std::collections::HashSet;

use anyhow::ensure;

use crate::options::{Disk, LvmBootdiskOptions};
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

/// Checks whether the configured swap size exceeds the allowed threshold.
///
/// # Arguments
///
/// * `swapsize` - The size of the swap in GiB
/// * `hdsize` - The total size of the hard disk in GiB
pub fn check_swapsize(swapsize: f64, hdsize: f64) -> anyhow::Result<()> {
    let threshold = hdsize / 2.0;
    ensure!(
        swapsize <= threshold,
        "Swap size {swapsize} GiB cannot be greater than {threshold} GiB (hard disk size / 2)"
    );
    Ok(())
}

/// Checks whether a user-supplied LVM setup is valid or not, such as the swapsize not
/// exceeding a certain threshold.
///
/// # Arguments
///
/// * `bootdisk_opts` - The LVM options set by the user.
pub fn check_lvm_bootdisk_opts(bootdisk_opts: &LvmBootdiskOptions) -> anyhow::Result<()> {
    if let Some(swap_size) = bootdisk_opts.swap_size {
        check_swapsize(swap_size, bootdisk_opts.total_size)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_disks(num: usize) -> Vec<Disk> {
        (0..num).map(Disk::dummy).collect()
    }

    #[test]
    fn duplicate_disks() {
        assert!(check_for_duplicate_disks(&dummy_disks(2)).is_ok());
        assert_eq!(
            check_for_duplicate_disks(&[
                Disk::dummy(0),
                Disk::dummy(1),
                Disk::dummy(2),
                Disk::dummy(2),
                Disk::dummy(3),
            ]),
            Err(&Disk::dummy(2)),
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
}
