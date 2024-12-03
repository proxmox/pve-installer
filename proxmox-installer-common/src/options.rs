use anyhow::{bail, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::sync::OnceLock;
use std::{cmp, fmt};

use crate::setup::{
    LocaleInfo, NetworkInfo, ProductConfig, ProxmoxProduct, RuntimeInfo, SetupInfo,
};
use crate::utils::{CidrAddress, Fqdn};

#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all(deserialize = "lowercase", serialize = "UPPERCASE"))]
pub enum BtrfsRaidLevel {
    #[serde(alias = "RAID0")]
    Raid0,
    #[serde(alias = "RAID1")]
    Raid1,
    #[serde(alias = "RAID10")]
    Raid10,
}

serde_plain::derive_display_from_serialize!(BtrfsRaidLevel);

#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all(deserialize = "lowercase", serialize = "UPPERCASE"))]
pub enum ZfsRaidLevel {
    #[serde(alias = "RAID0")]
    Raid0,
    #[serde(alias = "RAID1")]
    Raid1,
    #[serde(alias = "RAID10")]
    Raid10,
    #[serde(
        alias = "RAIDZ-1",
        rename(deserialize = "raidz-1", serialize = "RAIDZ-1")
    )]
    RaidZ,
    #[serde(
        alias = "RAIDZ-2",
        rename(deserialize = "raidz-2", serialize = "RAIDZ-2")
    )]
    RaidZ2,
    #[serde(
        alias = "RAIDZ-3",
        rename(deserialize = "raidz-3", serialize = "RAIDZ-3")
    )]
    RaidZ3,
}

serde_plain::derive_display_from_serialize!(ZfsRaidLevel);

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FsType {
    Ext4,
    Xfs,
    Zfs(ZfsRaidLevel),
    Btrfs(BtrfsRaidLevel),
}

impl FsType {
    pub fn is_btrfs(&self) -> bool {
        matches!(self, FsType::Btrfs(_))
    }

    /// Returns true if the filesystem is used on top of LVM, e.g. ext4 or XFS.
    pub fn is_lvm(&self) -> bool {
        matches!(self, FsType::Ext4 | FsType::Xfs)
    }
}

impl fmt::Display for FsType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Values displayed to the user in the installer UI
        match self {
            FsType::Ext4 => write!(f, "ext4"),
            FsType::Xfs => write!(f, "XFS"),
            FsType::Zfs(level) => write!(f, "ZFS ({level})"),
            FsType::Btrfs(level) => write!(f, "BTRFS ({level})"),
        }
    }
}

impl Serialize for FsType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // These values must match exactly what the low-level installer expects
        let value = match self {
            // proxinstall::$fssetup
            FsType::Ext4 => "ext4",
            FsType::Xfs => "xfs",
            // proxinstall::get_zfs_raid_setup()
            FsType::Zfs(level) => &format!("zfs ({level})"),
            // proxinstall::get_btrfs_raid_setup()
            FsType::Btrfs(level) => &format!("btrfs ({level})"),
        };

        serializer.collect_str(value)
    }
}

impl FromStr for FsType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ext4" => Ok(FsType::Ext4),
            "xfs" => Ok(FsType::Xfs),
            "zfs (RAID0)" => Ok(FsType::Zfs(ZfsRaidLevel::Raid0)),
            "zfs (RAID1)" => Ok(FsType::Zfs(ZfsRaidLevel::Raid1)),
            "zfs (RAID10)" => Ok(FsType::Zfs(ZfsRaidLevel::Raid10)),
            "zfs (RAIDZ-1)" => Ok(FsType::Zfs(ZfsRaidLevel::RaidZ)),
            "zfs (RAIDZ-2)" => Ok(FsType::Zfs(ZfsRaidLevel::RaidZ2)),
            "zfs (RAIDZ-3)" => Ok(FsType::Zfs(ZfsRaidLevel::RaidZ3)),
            "btrfs (RAID0)" => Ok(FsType::Btrfs(BtrfsRaidLevel::Raid0)),
            "btrfs (RAID1)" => Ok(FsType::Btrfs(BtrfsRaidLevel::Raid1)),
            "btrfs (RAID10)" => Ok(FsType::Btrfs(BtrfsRaidLevel::Raid10)),
            _ => Err(format!("Could not find file system: {s}")),
        }
    }
}

serde_plain::derive_deserialize_from_fromstr!(FsType, "valid filesystem");

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

/// See the accompanying mount option in btrfs(5).
#[derive(Copy, Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all(deserialize = "lowercase"))]
pub enum BtrfsCompressOption {
    On,
    #[default]
    Off,
    Zlib,
    Lzo,
    Zstd,
}

impl fmt::Display for BtrfsCompressOption {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", format!("{self:?}").to_lowercase())
    }
}

impl From<&BtrfsCompressOption> for String {
    fn from(value: &BtrfsCompressOption) -> Self {
        value.to_string()
    }
}

pub const BTRFS_COMPRESS_OPTIONS: &[BtrfsCompressOption] = {
    use BtrfsCompressOption::*;
    &[On, Off, Zlib, Lzo, Zstd]
};

#[derive(Clone, Debug)]
pub struct BtrfsBootdiskOptions {
    pub disk_size: f64,
    pub selected_disks: Vec<usize>,
    pub compress: BtrfsCompressOption,
}

impl BtrfsBootdiskOptions {
    /// This panics if the provided slice is empty.
    pub fn defaults_from(disks: &[Disk]) -> Self {
        let disk = &disks[0];
        Self {
            disk_size: disk.size,
            selected_disks: (0..disks.len()).collect(),
            compress: BtrfsCompressOption::default(),
        }
    }
}

#[derive(Copy, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
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

serde_plain::derive_display_from_serialize!(ZfsCompressOption);

impl From<&ZfsCompressOption> for String {
    fn from(value: &ZfsCompressOption) -> Self {
        value.to_string()
    }
}

pub const ZFS_COMPRESS_OPTIONS: &[ZfsCompressOption] = {
    use ZfsCompressOption::*;
    &[On, Off, Lzjb, Lz4, Zle, Gzip, Zstd]
};

#[derive(Copy, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ZfsChecksumOption {
    #[default]
    On,
    Fletcher4,
    Sha256,
}

serde_plain::derive_display_from_serialize!(ZfsChecksumOption);

impl From<&ZfsChecksumOption> for String {
    fn from(value: &ZfsChecksumOption) -> Self {
        value.to_string()
    }
}

pub const ZFS_CHECKSUM_OPTIONS: &[ZfsChecksumOption] = {
    use ZfsChecksumOption::*;
    &[On, Fletcher4, Sha256]
};

#[derive(Clone, Debug)]
pub struct ZfsBootdiskOptions {
    pub ashift: usize,
    pub compress: ZfsCompressOption,
    pub checksum: ZfsChecksumOption,
    pub copies: usize,
    pub arc_max: usize,
    pub disk_size: f64,
    pub selected_disks: Vec<usize>,
}

impl ZfsBootdiskOptions {
    /// Panics if the disk list is empty.
    pub fn defaults_from(runinfo: &RuntimeInfo, product_conf: &ProductConfig) -> Self {
        let disk = &runinfo.disks[0];
        Self {
            ashift: 12,
            compress: ZfsCompressOption::default(),
            checksum: ZfsChecksumOption::default(),
            copies: 1,
            arc_max: default_zfs_arc_max(product_conf.product, runinfo.total_memory),
            disk_size: disk.size,
            selected_disks: (0..runinfo.disks.len()).collect(),
        }
    }
}

/// Calculates the default upper limit for the ZFS ARC size.
/// See also <https://bugzilla.proxmox.com/show_bug.cgi?id=4829> and
/// https://openzfs.github.io/openzfs-docs/Performance%20and%20Tuning/Module%20Parameters.html#zfs-arc-max
///
/// # Arguments
/// * `product` - The product to be installed
/// * `total_memory` - Total memory installed in the system, in MiB
///
/// # Returns
/// The default ZFS maximum ARC size in MiB for this system.
fn default_zfs_arc_max(product: ProxmoxProduct, total_memory: usize) -> usize {
    if product != ProxmoxProduct::PVE {
        // For products other the PVE, just let ZFS decide on its own. Setting `0`
        // causes the installer to skip writing the `zfs_arc_max` module parameter.
        0
    } else {
        ((total_memory as f64) / 10.)
            .round()
            .clamp(64., 16. * 1024.) as usize
    }
}

#[derive(Clone, Debug)]
pub enum AdvancedBootdiskOptions {
    Lvm(LvmBootdiskOptions),
    Zfs(ZfsBootdiskOptions),
    Btrfs(BtrfsBootdiskOptions),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Disk {
    pub index: String,
    pub path: String,
    pub model: Option<String>,
    pub size: f64,
    pub block_size: Option<usize>,
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
        Some(self.cmp(other))
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
            .and_then(|zones| zones.first())
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

#[derive(Clone, Debug, PartialEq)]
pub struct NetworkOptions {
    pub ifname: String,
    pub fqdn: Fqdn,
    pub address: CidrAddress,
    pub gateway: IpAddr,
    pub dns_server: IpAddr,
}

impl NetworkOptions {
    const DEFAULT_DOMAIN: &'static str = "example.invalid";

    pub fn defaults_from(setup: &SetupInfo, network: &NetworkInfo) -> Self {
        let mut this = Self {
            ifname: String::new(),
            fqdn: Self::construct_fqdn(network, setup.config.product.default_hostname()),
            // Safety: The provided mask will always be valid.
            address: CidrAddress::new(Ipv4Addr::UNSPECIFIED, 0).unwrap(),
            gateway: Ipv4Addr::UNSPECIFIED.into(),
            dns_server: Ipv4Addr::UNSPECIFIED.into(),
        };

        if let Some(ip) = network.dns.dns.first() {
            this.dns_server = *ip;
        }

        if let Some(routes) = &network.routes {
            let mut filled = false;
            if let Some(gw) = &routes.gateway4 {
                if let Some(iface) = network.interfaces.get(&gw.dev) {
                    this.ifname.clone_from(&iface.name);
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
                    if let Some(iface) = network.interfaces.get(&gw.dev) {
                        if let Some(addresses) = &iface.addresses {
                            if let Some(addr) = addresses.iter().find(|addr| addr.is_ipv6()) {
                                this.ifname.clone_from(&iface.name);
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

    fn construct_fqdn(network: &NetworkInfo, default_hostname: &str) -> Fqdn {
        let hostname = network.hostname.as_deref().unwrap_or(default_hostname);

        let domain = network
            .dns
            .domain
            .as_deref()
            .unwrap_or(Self::DEFAULT_DOMAIN);

        Fqdn::from(&format!("{hostname}.{domain}")).unwrap_or_else(|_| {
            // Safety: This will always result in a valid FQDN, as we control & know
            // the values of default_hostname (one of "pve", "pmg" or "pbs") and
            // constant-defined DEFAULT_DOMAIN.
            Fqdn::from(&format!("{}.{}", default_hostname, Self::DEFAULT_DOMAIN)).unwrap()
        })
    }
}

/// Validates an email address using the regex for `<input type="email" />` elements
/// as defined in the [HTML specification].
/// Using that /should/ cover all possible cases that are encountered in the wild.
///
/// It additionally checks whether the email our default email placeholder value.
///
/// [HTML specification]: <https://html.spec.whatwg.org/multipage/input.html#valid-e-mail-address>
pub fn email_validate(email: &str) -> Result<()> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$").unwrap()
    });

    if !re.is_match(email) {
        bail!("Email does not look like a valid address (user@domain.tld)")
    } else if email == crate::EMAIL_DEFAULT_PLACEHOLDER {
        bail!("Invalid (default) email address")
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zfs_arc_limit() {
        const TESTS: &[(usize, usize)] = &[
            (16, 64), // at least 64 MiB
            (1024, 102),
            (4 * 1024, 410),
            (8 * 1024, 819),
            (150 * 1024, 15360),
            (160 * 1024, 16384),
            (1024 * 1024, 16384), // maximum of 16 GiB
        ];

        for (total_memory, expected) in TESTS {
            assert_eq!(
                default_zfs_arc_max(ProxmoxProduct::PVE, *total_memory),
                *expected
            );
            assert_eq!(default_zfs_arc_max(ProxmoxProduct::PBS, *total_memory), 0);
            assert_eq!(default_zfs_arc_max(ProxmoxProduct::PMG, *total_memory), 0);
        }
    }
}
