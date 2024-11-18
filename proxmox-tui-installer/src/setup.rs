use std::collections::BTreeMap;

use crate::options::InstallerOptions;
use proxmox_installer_common::{
    options::AdvancedBootdiskOptions,
    setup::{InstallConfig, InstallFirstBootSetup, InstallRootPassword},
};

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
            btrfs_opts: None,
            target_hd: None,
            disk_selection: BTreeMap::new(),
            existing_storage_auto_rename: 0,

            country: options.timezone.country,
            timezone: options.timezone.timezone,
            keymap: options.timezone.kb_layout,

            root_password: InstallRootPassword {
                plain: Some(options.password.root_password),
                hashed: None,
            },
            mailto: options.password.email,
            root_ssh_keys: vec![],

            mngmt_nic: options.network.ifname,

            // Safety: At this point, it is know that we have a valid FQDN, as
            // this is set by the TUI network panel, which only lets the user
            // continue if a valid FQDN is provided.
            hostname: options.network.fqdn.host().expect("valid FQDN").to_owned(),
            domain: options.network.fqdn.domain(),
            cidr: options.network.address,
            gateway: options.network.gateway,
            dns: options.network.dns_server,

            first_boot: InstallFirstBootSetup::default(),
        };

        match &options.bootdisk.advanced {
            AdvancedBootdiskOptions::Lvm(lvm) => {
                config.hdsize = lvm.total_size;
                config.target_hd = Some(options.bootdisk.disks[0].path.clone());
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
                config.btrfs_opts = Some(btrfs.clone().into());

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
