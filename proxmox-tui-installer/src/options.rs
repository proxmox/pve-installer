use crate::SummaryOption;

use proxmox_installer_common::{
    EMAIL_DEFAULT_PLACEHOLDER,
    options::{
        BootdiskOptions, BtrfsRaidLevel, FsType, NetworkOptions, TimezoneOptions, ZfsRaidLevel,
    },
    setup::LocaleInfo,
};

pub const FS_TYPES: &[FsType] = {
    use FsType::*;
    &[
        Ext4,
        Xfs,
        Zfs(ZfsRaidLevel::Raid0),
        Zfs(ZfsRaidLevel::Raid1),
        Zfs(ZfsRaidLevel::Raid10),
        Zfs(ZfsRaidLevel::RaidZ),
        Zfs(ZfsRaidLevel::RaidZ2),
        Zfs(ZfsRaidLevel::RaidZ3),
        Btrfs(BtrfsRaidLevel::Raid0),
        Btrfs(BtrfsRaidLevel::Raid1),
        Btrfs(BtrfsRaidLevel::Raid10),
    ]
};

#[derive(Clone)]
pub struct PasswordOptions {
    pub email: String,
    pub root_password: String,
}

impl Default for PasswordOptions {
    fn default() -> Self {
        Self {
            email: EMAIL_DEFAULT_PLACEHOLDER.to_string(),
            root_password: String::new(),
        }
    }
}

#[derive(Clone)]
pub struct InstallerOptions {
    pub bootdisk: BootdiskOptions,
    pub timezone: TimezoneOptions,
    pub password: PasswordOptions,
    pub network: NetworkOptions,
    pub autoreboot: bool,
}

impl InstallerOptions {
    pub fn to_summary(&self, locales: &LocaleInfo) -> Vec<SummaryOption> {
        let kb_layout = locales
            .kmap
            .get(&self.timezone.kb_layout)
            .map(|l| &l.name)
            .unwrap_or(&self.timezone.kb_layout);

        vec![
            SummaryOption::new("Bootdisk filesystem", self.bootdisk.fstype.to_string()),
            SummaryOption::new(
                "Bootdisk(s)",
                self.bootdisk
                    .disks
                    .iter()
                    .map(|d| d.path.as_str())
                    .collect::<Vec<&str>>()
                    .join(", "),
            ),
            SummaryOption::new("Timezone", &self.timezone.timezone),
            SummaryOption::new("Keyboard layout", kb_layout),
            SummaryOption::new("Administrator email", &self.password.email),
            SummaryOption::new("Management interface", &self.network.ifname),
            SummaryOption::new("Hostname", self.network.fqdn.to_string()),
            SummaryOption::new("Host IP (CIDR)", self.network.address.to_string()),
            SummaryOption::new("Gateway", self.network.gateway.to_string()),
            SummaryOption::new("DNS", self.network.dns_server.to_string()),
        ]
    }
}
