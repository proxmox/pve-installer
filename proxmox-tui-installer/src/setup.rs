use std::{
    collections::HashMap,
    fmt,
    net::IpAddr,
};

use serde::{Serialize, Serializer};

use crate::options::InstallerOptions;
use proxmox_installer_common::{
        options::{AdvancedBootdiskOptions, BtrfsRaidLevel, Disk, FsType, ZfsRaidLevel},
        setup::InstallZfsOption,
        utils::CidrAddress,
    };

/// See Proxmox::Install::Config
#[derive(Serialize)]
pub struct InstallConfig {
    autoreboot: usize,

    #[serde(serialize_with = "serialize_fstype")]
    filesys: FsType,
    hdsize: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    swapsize: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    maxroot: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    minfree: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    maxvz: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    zfs_opts: Option<InstallZfsOption>,

    #[serde(
        serialize_with = "serialize_disk_opt",
        skip_serializing_if = "Option::is_none"
    )]
    target_hd: Option<Disk>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    disk_selection: HashMap<String, String>,

    country: String,
    timezone: String,
    keymap: String,

    password: String,
    mailto: String,

    mngmt_nic: String,

    hostname: String,
    domain: String,
    #[serde(serialize_with = "serialize_as_display")]
    cidr: CidrAddress,
    gateway: IpAddr,
    dns: IpAddr,
}

impl From<InstallerOptions> for InstallConfig {
    fn from(options: InstallerOptions) -> Self {
        let mut config = Self {
            autoreboot: options.autoreboot as usize,

            filesys: options.bootdisk.fstype,
            hdsize: 0.,
            swapsize: None,
            maxroot: None,
            minfree: None,
            maxvz: None,
            zfs_opts: None,
            target_hd: None,
            disk_selection: HashMap::new(),

            country: options.timezone.country,
            timezone: options.timezone.timezone,
            keymap: options.timezone.kb_layout,

            password: options.password.root_password,
            mailto: options.password.email,

            mngmt_nic: options.network.ifname,

            // Safety: At this point, it is know that we have a valid FQDN, as
            // this is set by the TUI network panel, which only lets the user
            // continue if a valid FQDN is provided.
            hostname: options.network.fqdn.host().expect("valid FQDN").to_owned(),
            domain: options.network.fqdn.domain(),
            cidr: options.network.address,
            gateway: options.network.gateway,
            dns: options.network.dns_server,
        };

        match &options.bootdisk.advanced {
            AdvancedBootdiskOptions::Lvm(lvm) => {
                config.hdsize = lvm.total_size;
                config.target_hd = Some(options.bootdisk.disks[0].clone());
                config.swapsize = lvm.swap_size;
                config.maxroot = lvm.max_root_size;
                config.minfree = lvm.min_lvm_free;
                config.maxvz = lvm.max_data_size;
            }
            AdvancedBootdiskOptions::Zfs(zfs) => {
                config.hdsize = zfs.disk_size;
                config.zfs_opts = Some(zfs.clone().into());

                for (i, disk) in options.bootdisk.disks.iter().enumerate() {
                    config
                        .disk_selection
                        .insert(i.to_string(), disk.index.clone());
                }
            }
            AdvancedBootdiskOptions::Btrfs(btrfs) => {
                config.hdsize = btrfs.disk_size;

                for (i, disk) in options.bootdisk.disks.iter().enumerate() {
                    config
                        .disk_selection
                        .insert(i.to_string(), disk.index.clone());
                }
            }
        }

        config
    }
}

fn serialize_disk_opt<S>(value: &Option<Disk>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(disk) = value {
        serializer.serialize_str(&disk.path)
    } else {
        serializer.serialize_none()
    }
}

fn serialize_fstype<S>(value: &FsType, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    use FsType::*;
    let value = match value {
        // proxinstall::$fssetup
        Ext4 => "ext4",
        Xfs => "xfs",
        // proxinstall::get_zfs_raid_setup()
        Zfs(ZfsRaidLevel::Raid0) => "zfs (RAID0)",
        Zfs(ZfsRaidLevel::Raid1) => "zfs (RAID1)",
        Zfs(ZfsRaidLevel::Raid10) => "zfs (RAID10)",
        Zfs(ZfsRaidLevel::RaidZ) => "zfs (RAIDZ-1)",
        Zfs(ZfsRaidLevel::RaidZ2) => "zfs (RAIDZ-2)",
        Zfs(ZfsRaidLevel::RaidZ3) => "zfs (RAIDZ-3)",
        // proxinstall::get_btrfs_raid_setup()
        Btrfs(BtrfsRaidLevel::Raid0) => "btrfs (RAID0)",
        Btrfs(BtrfsRaidLevel::Raid1) => "btrfs (RAID1)",
        Btrfs(BtrfsRaidLevel::Raid10) => "btrfs (RAID10)",
    };

    serializer.collect_str(value)
}

fn serialize_as_display<S, T>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: fmt::Display,
{
    serializer.collect_str(value)
}
