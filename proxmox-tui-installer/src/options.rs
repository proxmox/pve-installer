use std::net::{IpAddr, Ipv4Addr};
use std::{cmp, fmt};

use proxmox_sys::linux::procfs;

use crate::setup::{LocaleInfo, NetworkInfo};
use crate::utils::{CidrAddress, Fqdn};
use crate::SummaryOption;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BtrfsRaidLevel {
    Raid0,
    Raid1,
    Raid10,
}

impl fmt::Display for BtrfsRaidLevel {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use BtrfsRaidLevel::*;
        match self {
            Raid0 => write!(f, "RAID0"),
            Raid1 => write!(f, "RAID1"),
            Raid10 => write!(f, "RAID10"),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ZfsRaidLevel {
    Raid0,
    Raid1,
    Raid10,
    RaidZ,
    RaidZ2,
    RaidZ3,
}

impl fmt::Display for ZfsRaidLevel {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use ZfsRaidLevel::*;
        match self {
            Raid0 => write!(f, "RAID0"),
            Raid1 => write!(f, "RAID1"),
            Raid10 => write!(f, "RAID10"),
            RaidZ => write!(f, "RAIDZ-1"),
            RaidZ2 => write!(f, "RAIDZ-2"),
            RaidZ3 => write!(f, "RAIDZ-3"),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FsType {
    Ext4,
    Xfs,
    Zfs(ZfsRaidLevel),
    Btrfs(BtrfsRaidLevel),
}

impl fmt::Display for FsType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use FsType::*;
        match self {
            Ext4 => write!(f, "ext4"),
            Xfs => write!(f, "XFS"),
            Zfs(level) => write!(f, "ZFS ({level})"),
            Btrfs(level) => write!(f, "Btrfs ({level})"),
        }
    }
}

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

#[derive(Clone, Debug)]
pub struct LvmBootdiskOptions {
    pub total_size: u64,
    pub swap_size: u64,
    pub max_root_size: u64,
    pub max_data_size: u64,
    pub min_lvm_free: u64,
}

impl LvmBootdiskOptions {
    /// Sets the default values as described in the documentation:
    /// https://pve.proxmox.com/pve-docs/pve-admin-guide.html#advanced_lvm_options
    pub fn defaults_from(disk: &Disk) -> Self {
        let mem_total = procfs::read_meminfo()
            .map(|m| m.memtotal)
            .unwrap_or(4 * 1024 * 1024 * 1024);

        // Clamp to 4 GiB <= total system memory <= 8 GiB
        let swap_size = mem_total.clamp(4 * 1024 * 1024 * 1024, 8 * 1024 * 1024 * 1024);

        // If the disk size > 128 GiB use 16 GiB, else 1/8 of the disk
        let min_lvm_free = if disk.size > 128 * 1024 * 1024 {
            16 * 1024 * 1024 * 1024
        } else {
            disk.size / 8
        };

        Self {
            total_size: disk.size,
            swap_size,
            max_root_size: 0,
            max_data_size: 0,
            min_lvm_free,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BtrfsBootdiskOptions {
    pub disk_size: u64,
}

impl BtrfsBootdiskOptions {
    pub fn defaults_from(disk: &Disk) -> Self {
        Self {
            disk_size: disk.size,
        }
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ZfsCompressOption {
    #[default]
    On,
    Off,
    Lzjb,
    Lz4,
    Zle,
    Gzip,
    Zstd,
}

impl fmt::Display for ZfsCompressOption {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", format!("{self:?}").to_lowercase())
    }
}

impl From<&ZfsCompressOption> for String {
    fn from(value: &ZfsCompressOption) -> Self {
        value.to_string()
    }
}

pub const ZFS_COMPRESS_OPTIONS: &[ZfsCompressOption] = {
    use ZfsCompressOption::*;
    &[On, Off, Lzjb, Lz4, Zle, Gzip, Zstd]
};

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ZfsChecksumOption {
    #[default]
    On,
    Off,
    Fletcher2,
    Fletcher4,
    Sha256,
}

impl fmt::Display for ZfsChecksumOption {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", format!("{self:?}").to_lowercase())
    }
}

impl From<&ZfsChecksumOption> for String {
    fn from(value: &ZfsChecksumOption) -> Self {
        value.to_string()
    }
}

pub const ZFS_CHECKSUM_OPTIONS: &[ZfsChecksumOption] = {
    use ZfsChecksumOption::*;
    &[On, Off, Fletcher2, Fletcher4, Sha256]
};

#[derive(Clone, Debug)]
pub struct ZfsBootdiskOptions {
    pub ashift: usize,
    pub compress: ZfsCompressOption,
    pub checksum: ZfsChecksumOption,
    pub copies: usize,
    pub disk_size: u64,
}

impl ZfsBootdiskOptions {
    pub fn defaults_from(disk: &Disk) -> Self {
        Self {
            ashift: 12,
            compress: ZfsCompressOption::default(),
            checksum: ZfsChecksumOption::default(),
            copies: 1,
            disk_size: disk.size,
        }
    }
}

#[derive(Clone, Debug)]
pub enum AdvancedBootdiskOptions {
    Lvm(LvmBootdiskOptions),
    Zfs(ZfsBootdiskOptions),
    Btrfs(BtrfsBootdiskOptions),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Disk {
    pub path: String,
    pub size: u64,
}

impl fmt::Display for Disk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO: Format sizes properly with `proxmox-human-byte` once merged
        // https://lists.proxmox.com/pipermail/pbs-devel/2023-May/006125.html
        write!(
            f,
            "{} ({:.2} GiB)",
            self.path,
            (self.size as f64) / 1024. / 1024. / 1024.
        )
    }
}

impl From<&Disk> for String {
    fn from(value: &Disk) -> Self {
        value.to_string()
    }
}

impl cmp::PartialOrd for Disk {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        self.path.partial_cmp(&other.path)
    }
}

impl cmp::Ord for Disk {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.path.cmp(&other.path)
    }
}

#[derive(Clone, Debug)]
pub struct BootdiskOptions {
    pub disks: Vec<Disk>,
    pub fstype: FsType,
    pub advanced: AdvancedBootdiskOptions,
}

impl BootdiskOptions {
    pub fn defaults_from(disk: &Disk) -> Self {
        Self {
            disks: vec![disk.clone()],
            fstype: FsType::Ext4,
            advanced: AdvancedBootdiskOptions::Lvm(LvmBootdiskOptions::defaults_from(disk)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TimezoneOptions {
    pub country: String,
    pub timezone: String,
    pub kb_layout: String,
}

impl Default for TimezoneOptions {
    fn default() -> Self {
        Self {
            country: "at".to_owned(),
            timezone: "Europe/Vienna".to_owned(),
            kb_layout: "en-us".to_owned(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PasswordOptions {
    pub email: String,
    pub root_password: String,
}

impl Default for PasswordOptions {
    fn default() -> Self {
        Self {
            email: "mail@example.invalid".to_owned(),
            root_password: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct NetworkOptions {
    pub ifname: String,
    pub fqdn: Fqdn,
    pub address: CidrAddress,
    pub gateway: IpAddr,
    pub dns_server: IpAddr,
}

impl Default for NetworkOptions {
    fn default() -> Self {
        // TODO: Retrieve automatically
        Self {
            ifname: String::new(),
            fqdn: "pve.example.invalid".parse().unwrap(),
            // Safety: The provided mask will always be valid.
            address: CidrAddress::new(Ipv4Addr::UNSPECIFIED, 0).unwrap(),
            gateway: Ipv4Addr::UNSPECIFIED.into(),
            dns_server: Ipv4Addr::UNSPECIFIED.into(),
        }
    }
}

impl From<&NetworkInfo> for NetworkOptions {
    fn from(info: &NetworkInfo) -> Self {
        let mut this = Self::default();

        if let Some(ip) = info.dns.dns.first() {
            this.dns_server = *ip;
        }

        if let Some(domain) = &info.dns.domain {
            this.fqdn = domain.clone();
        }

        let mut filled = false;
        if let Some(gw) = &info.routes.gateway4 {
            if let Some(iface) = info.interfaces.get(&gw.dev) {
                if let Some(addr) = iface.addresses.iter().find(|addr| addr.is_ipv4()) {
                    this.ifname = iface.name.clone();
                    this.gateway = gw.gateway;
                    this.address = addr.clone();
                    filled = true;
                }
            }
        }
        if !filled {
            if let Some(gw) = &info.routes.gateway6 {
                if let Some(iface) = info.interfaces.get(&gw.dev) {
                    if let Some(addr) = iface.addresses.iter().find(|addr| addr.is_ipv6()) {
                        this.ifname = iface.name.clone();
                        this.gateway = gw.gateway;
                        this.address = addr.clone();
                    }
                }
            }
        }

        this
    }
}

#[derive(Clone, Debug)]
pub struct InstallerOptions {
    pub bootdisk: BootdiskOptions,
    pub timezone: TimezoneOptions,
    pub password: PasswordOptions,
    pub network: NetworkOptions,
    pub reboot: bool,
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
            SummaryOption::new("Administator email", &self.password.email),
            SummaryOption::new("Management interface", &self.network.ifname),
            SummaryOption::new("Hostname", self.network.fqdn.to_string()),
            SummaryOption::new("Host IP (CIDR)", self.network.address.to_string()),
            SummaryOption::new("Gateway", self.network.gateway.to_string()),
            SummaryOption::new("DNS", self.network.dns_server.to_string()),
        ]
    }
}
