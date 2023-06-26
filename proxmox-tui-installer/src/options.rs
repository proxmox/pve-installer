use std::net::{IpAddr, Ipv4Addr};
use std::{cmp, fmt};

use crate::setup::{LocaleInfo, NetworkInfo, RuntimeInfo};
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
    pub total_size: f64,
    pub swap_size: Option<f64>,
    pub max_root_size: Option<f64>,
    pub max_data_size: Option<f64>,
    pub min_lvm_free: Option<f64>,
}

impl LvmBootdiskOptions {
    pub fn defaults_from(disk: &Disk) -> Self {
        Self {
            total_size: disk.size,
            swap_size: None,
            max_root_size: None,
            max_data_size: None,
            min_lvm_free: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BtrfsBootdiskOptions {
    pub disk_size: f64,
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
    pub disk_size: f64,
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

#[derive(Clone, Debug, PartialEq)]
pub struct Disk {
    pub index: String,
    pub path: String,
    pub model: Option<String>,
    pub size: f64,
}

impl fmt::Display for Disk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO: Format sizes properly with `proxmox-human-byte` once merged
        // https://lists.proxmox.com/pipermail/pbs-devel/2023-May/006125.html
        f.write_str(&self.path)?;
        if let Some(model) = &self.model {
            // FIXME: ellipsize too-long names?
            write!(f, " ({model})")?;
        }
        write!(f, " ({:.2} GiB)", self.size)
    }
}

impl From<&Disk> for String {
    fn from(value: &Disk) -> Self {
        value.to_string()
    }
}

impl cmp::Eq for Disk {}

impl cmp::PartialOrd for Disk {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        self.index.partial_cmp(&other.index)
    }
}

impl cmp::Ord for Disk {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.index.cmp(&other.index)
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

impl TimezoneOptions {
    pub fn defaults_from(runtime: &RuntimeInfo, locales: &LocaleInfo) -> Self {
        let country = runtime.country.clone().unwrap_or_else(|| "at".to_owned());

        let timezone = locales
            .cczones
            .get(&country)
            .and_then(|zones| zones.get(0))
            .cloned()
            .unwrap_or_else(|| "UTC".to_owned());

        let kb_layout = locales
            .countries
            .get(&country)
            .and_then(|c| {
                if c.kmap.is_empty() {
                    None
                } else {
                    Some(c.kmap.clone())
                }
            })
            .unwrap_or_else(|| "en-us".to_owned());

        Self {
            country,
            timezone,
            kb_layout,
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
            email: "mail@example.invalid".to_string(),
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
        let fqdn = format!(
            "{}.example.invalid",
            crate::current_product().default_hostname()
        );
        // TODO: Retrieve automatically
        Self {
            ifname: String::new(),
            fqdn: fqdn.parse().unwrap(),
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
            let hostname = crate::current_product().default_hostname();
            this.fqdn =
                Fqdn::from(&format!("{hostname}.{domain}")).unwrap_or_else(|_| domain.clone());
        }

        if let Some(routes) = &info.routes {
            let mut filled = false;
            if let Some(gw) = &routes.gateway4 {
                if let Some(iface) = info.interfaces.get(&gw.dev) {
                    this.ifname = iface.name.clone();
                    if let Some(addresses) = &iface.addresses {
                        if let Some(addr) = addresses.iter().find(|addr| addr.is_ipv4()) {
                            this.gateway = gw.gateway;
                            this.address = addr.clone();
                            filled = true;
                        }
                    }
                }
            }
            if !filled {
                if let Some(gw) = &routes.gateway6 {
                    if let Some(iface) = info.interfaces.get(&gw.dev) {
                        if let Some(addresses) = &iface.addresses {
                            if let Some(addr) = addresses.iter().find(|addr| addr.is_ipv6()) {
                                this.ifname = iface.name.clone();
                                this.gateway = gw.gateway;
                                this.address = addr.clone();
                            }
                        }
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
            SummaryOption::new("Administator email", &self.password.email),
            SummaryOption::new("Management interface", &self.network.ifname),
            SummaryOption::new("Hostname", self.network.fqdn.to_string()),
            SummaryOption::new("Host IP (CIDR)", self.network.address.to_string()),
            SummaryOption::new("Gateway", self.network.gateway.to_string()),
            SummaryOption::new("DNS", self.network.dns_server.to_string()),
        ]
    }
}
