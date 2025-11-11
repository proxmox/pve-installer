use anyhow::{Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use std::sync::OnceLock;
use std::{cmp, fmt};

use crate::disk_checks::check_raid_min_disks;
use crate::net::MAX_IFNAME_LEN;
use crate::setup::{LocaleInfo, NetworkInfo, RuntimeInfo, SetupInfo};
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

impl BtrfsRaidLevel {
    pub fn get_min_disks(&self) -> usize {
        match self {
            BtrfsRaidLevel::Raid0 => 1,
            BtrfsRaidLevel::Raid1 => 2,
            BtrfsRaidLevel::Raid10 => 4,
        }
    }

    /// Checks whether a user-supplied Btrfs RAID setup is valid or not, such as minimum
    /// number of disks.
    ///
    /// # Arguments
    ///
    /// * `disks` - List of disks designated as RAID targets.
    pub fn check_raid_disks_setup(&self, disks: &[Disk]) -> Result<(), String> {
        check_raid_min_disks(disks, self.get_min_disks())?;
        Ok(())
    }
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

impl ZfsRaidLevel {
    pub fn get_min_disks(&self) -> usize {
        match self {
            ZfsRaidLevel::Raid0 => 1,
            ZfsRaidLevel::Raid1 => 2,
            ZfsRaidLevel::Raid10 => 4,
            ZfsRaidLevel::RaidZ => 3,
            ZfsRaidLevel::RaidZ2 => 4,
            ZfsRaidLevel::RaidZ3 => 5,
        }
    }

    fn check_mirror_size(&self, disk1: &Disk, disk2: &Disk) -> Result<(), String> {
        if (disk1.size - disk2.size).abs() > disk1.size / 10. {
            Err(format!(
                "Mirrored disks must have same size:\n\n  * {disk1}\n  * {disk2}"
            ))
        } else {
            Ok(())
        }
    }

    /// Checks whether a user-supplied ZFS RAID setup is valid or not, such as disk sizes andminimum
    /// number of disks.
    ///
    /// # Arguments
    ///
    /// * `disks` - List of disks designated as RAID targets.
    pub fn check_raid_disks_setup(&self, disks: &[Disk]) -> Result<(), String> {
        check_raid_min_disks(disks, self.get_min_disks())?;

        match self {
            ZfsRaidLevel::Raid0 => {}
            ZfsRaidLevel::Raid10 => {
                if !disks.len().is_multiple_of(2) {
                    return Err(format!(
                        "Needs an even number of disks, currently selected: {}",
                        disks.len(),
                    ));
                }

                // Pairs need to have the same size
                for i in (0..disks.len()).step_by(2) {
                    self.check_mirror_size(&disks[i], &disks[i + 1])?;
                }
            }
            ZfsRaidLevel::Raid1
            | ZfsRaidLevel::RaidZ
            | ZfsRaidLevel::RaidZ2
            | ZfsRaidLevel::RaidZ3 => {
                for disk in disks {
                    self.check_mirror_size(&disks[0], disk)?;
                }
            }
        }

        Ok(())
    }
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

    pub fn get_min_disks(&self) -> usize {
        match self {
            FsType::Ext4 => 1,
            FsType::Xfs => 1,
            FsType::Zfs(level) => level.get_min_disks(),
            FsType::Btrfs(level) => level.get_min_disks(),
        }
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
    pub fn defaults_from(runinfo: &RuntimeInfo) -> Self {
        let disk = &runinfo.disks[0];
        Self {
            ashift: 12,
            compress: ZfsCompressOption::default(),
            checksum: ZfsChecksumOption::default(),
            copies: 1,
            arc_max: runinfo.default_zfs_arc_max,
            disk_size: disk.size,
            selected_disks: (0..runinfo.disks.len()).collect(),
        }
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

impl Disk {
    #[cfg(test)]
    pub fn dummy(index: usize) -> Disk {
        Disk {
            index: index.to_string(),
            path: format!("/dev/dummy{index}"),
            model: Some("Dummy disk".to_owned()),
            size: 1024. * 1024. * 1024. * 8.,
            block_size: Some(512),
        }
    }
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

/// Options controlling the behaviour of the network interface pinning (by
/// creating appropriate systemd.link files) during the installation.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct NetworkInterfacePinningOptions {
    /// Maps MAC address to custom name
    #[serde(default)]
    pub mapping: HashMap<String, String>,
}

impl NetworkInterfacePinningOptions {
    /// Default prefix to prepend to the pinned interface ID as received from the low-level
    /// installer.
    pub const DEFAULT_PREFIX: &str = "nic";

    /// Does some basic checks on the options.
    ///
    /// This includes checks for:
    /// - empty interface names
    /// - overlong interface names
    /// - duplicate interface names
    /// - only contains ASCII alphanumeric characters and underscore, as
    ///   enforced by our `pve-iface` json schema.
    pub fn verify(&self) -> Result<()> {
        let mut reverse_mapping = HashMap::<String, String>::new();
        for (mac, name) in self.mapping.iter() {
            if name.is_empty() {
                bail!("interface name for '{mac}' cannot be empty");
            }

            if name.len() > MAX_IFNAME_LEN {
                bail!(
                    "interface name '{name}' for '{mac}' cannot be longer than {} characters",
                    MAX_IFNAME_LEN
                );
            }

            if name.chars().all(char::is_numeric) {
                bail!(
                    "interface name '{name}' for '{mac}' is invalid: name must not be fully numeric"
                );
            }

            // Mimicking the `pve-iface` schema verification
            if name.starts_with(|c: char| c.is_ascii_digit()) {
                bail!(
                    "interface name '{name}' for '{mac}' is invalid: name must not start with a number"
                );
            }

            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                bail!(
                    "interface name '{name}' for '{mac}' is invalid: name must only consist of alphanumeric characters and underscores"
                );
            }

            if let Some(duplicate_mac) = reverse_mapping.get(name)
                && mac != duplicate_mac
            {
                bail!("duplicate interface name mapping '{name}' for: {mac}, {duplicate_mac}");
            }

            reverse_mapping.insert(name.clone(), mac.clone());
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NetworkOptions {
    pub ifname: String,
    pub fqdn: Fqdn,
    pub address: CidrAddress,
    pub gateway: IpAddr,
    pub dns_server: IpAddr,
    pub pinning_opts: Option<NetworkInterfacePinningOptions>,
}

impl NetworkOptions {
    const DEFAULT_DOMAIN: &'static str = "example.invalid";

    pub fn defaults_from(
        setup: &SetupInfo,
        network: &NetworkInfo,
        default_domain: Option<&str>,
        pinning_opts: Option<&NetworkInterfacePinningOptions>,
    ) -> Self {
        // Sets up sensible defaults as much as possible, such that even in the
        // worse case nothing breaks down *completely*.
        let mut this = Self {
            ifname: String::new(),
            fqdn: Self::construct_fqdn(
                network,
                setup.config.product.default_hostname(),
                default_domain,
            ),
            // Safety: The provided IP address/mask is always valid.
            // These are the same as used in the GTK-based installer.
            address: CidrAddress::new(Ipv4Addr::new(192, 168, 100, 2), 24).unwrap(),
            gateway: Ipv4Addr::new(192, 168, 100, 1).into(),
            dns_server: Ipv4Addr::new(192, 168, 100, 1).into(),
            pinning_opts: pinning_opts.cloned(),
        };

        if let Some(ip) = network.dns.dns.first() {
            this.dns_server = *ip;
        }

        if let Some(routes) = &network.routes
            && let Some(gw) = &routes.gateway4
            && let Some(iface) = network.interfaces.get(&gw.dev)
        {
            // we got some ipv4 connectivity, so use that

            if let Some(opts) = pinning_opts {
                this.ifname.clone_from(&iface.to_pinned(opts).name);
            } else {
                this.ifname.clone_from(&iface.name);
            }

            if let Some(addr) = iface.addresses.iter().find(|addr| addr.is_ipv4()) {
                this.gateway = gw.gateway;
                this.address = addr.clone();
            } else if let Some(gw) = &routes.gateway6
                && let Some(iface) = network.interfaces.get(&gw.dev)
                && let Some(addr) = iface.addresses.iter().find(|addr| addr.is_ipv6())
            {
                // no ipv4, but ipv6 connectivity
                if let Some(opts) = pinning_opts {
                    this.ifname.clone_from(&iface.to_pinned(opts).name);
                } else {
                    this.ifname.clone_from(&iface.name);
                }

                this.gateway = gw.gateway;
                this.address = addr.clone();
            }
        }

        // In case no there are no routes defined at all (e.g. no DHCP lease),
        // try to set the interface name to *some* valid values. At least one
        // NIC should always be present here, as the installation will abort
        // earlier in that case, so use the first one enumerated.
        if this.ifname.is_empty()
            && let Some(iface) = network.interfaces.values().min_by_key(|v| v.index)
        {
            if let Some(opts) = pinning_opts {
                this.ifname.clone_from(&iface.to_pinned(opts).name);
            } else {
                this.ifname.clone_from(&iface.name);
            }
        }

        if let Some(ref mut opts) = this.pinning_opts {
            // Ensure that all unique interfaces indeed have an entry in the map,
            // as required by the low-level installer
            for iface in network.interfaces.values() {
                let pinned_name = iface.to_pinned(opts).name;
                opts.mapping.entry(iface.mac.clone()).or_insert(pinned_name);
            }
        }

        this
    }

    pub fn construct_fqdn(
        network: &NetworkInfo,
        default_hostname: &str,
        default_domain: Option<&str>,
    ) -> Fqdn {
        let hostname = network.hostname.as_deref().unwrap_or(default_hostname);

        // First, use the provided default domain if provided. If that is unset,
        // use the one from the host network configuration, i.e. as and if provided by DHCP.
        // As last fallback, use [`Self::DEFAULT_DOMAIN`].
        let domain = default_domain.unwrap_or_else(|| {
            network
                .dns
                .domain
                .as_deref()
                .unwrap_or(Self::DEFAULT_DOMAIN)
        });

        Fqdn::from(&format!("{hostname}.{domain}")).unwrap_or_else(|_| {
            // Safety: This will always result in a valid FQDN, as we control & know
            // the values of default_hostname (one of "pve", "pmg", "pbs" or "pdm") and
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
    use crate::{
        setup::{Dns, Gateway, Interface, InterfaceState, NetworkInfo, Routes, SetupInfo},
        utils::CidrAddress,
    };
    use std::collections::BTreeMap;
    use std::net::{IpAddr, Ipv4Addr};

    fn dummy_disks(num: usize) -> Vec<Disk> {
        (0..num).map(Disk::dummy).collect()
    }

    #[test]
    fn btrfs_raid() {
        let disks = dummy_disks(10);

        let btrfs_raid_variants = [
            BtrfsRaidLevel::Raid0,
            BtrfsRaidLevel::Raid1,
            BtrfsRaidLevel::Raid10,
        ];

        for v in btrfs_raid_variants {
            assert!(v.check_raid_disks_setup(&[]).is_err());
            assert!(
                v.check_raid_disks_setup(&disks[..v.get_min_disks() - 1])
                    .is_err()
            );
            assert!(
                v.check_raid_disks_setup(&disks[..v.get_min_disks()])
                    .is_ok()
            );
            assert!(v.check_raid_disks_setup(&disks).is_ok());
        }
    }

    #[test]
    fn zfs_raid() {
        let disks = dummy_disks(10);

        let zfs_raid_variants = [
            ZfsRaidLevel::Raid0,
            ZfsRaidLevel::Raid1,
            ZfsRaidLevel::Raid10,
            ZfsRaidLevel::RaidZ,
            ZfsRaidLevel::RaidZ2,
            ZfsRaidLevel::RaidZ3,
        ];

        for v in zfs_raid_variants {
            assert!(v.check_raid_disks_setup(&[]).is_err());
            assert!(
                v.check_raid_disks_setup(&disks[..v.get_min_disks() - 1])
                    .is_err()
            );
            assert!(
                v.check_raid_disks_setup(&disks[..v.get_min_disks()])
                    .is_ok()
            );
            assert!(v.check_raid_disks_setup(&disks).is_ok());
        }
    }

    fn mock_setup_network() -> (SetupInfo, NetworkInfo) {
        let mut interfaces = BTreeMap::new();
        interfaces.insert(
            "eth0".to_owned(),
            Interface {
                name: "eth0".to_owned(),
                index: 0,
                pinned_id: "0".to_owned(),
                state: InterfaceState::Up,
                driver: "dummy".to_owned(),
                mac: "01:23:45:67:89:ab".to_owned(),
                addresses: vec![CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap()],
            },
        );

        let info = NetworkInfo {
            dns: Dns {
                domain: Some("bar.com".to_owned()),
                dns: Vec::new(),
            },
            routes: Some(Routes {
                gateway4: Some(Gateway {
                    dev: "eth0".to_owned(),
                    gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                }),
                gateway6: None,
            }),
            interfaces,
            hostname: Some("foo".to_owned()),
        };

        (SetupInfo::mocked(), info)
    }

    #[test]
    fn network_options_from_setup_network_info() {
        let (setup, mut info) = mock_setup_network();

        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info, None, None),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("foo.bar.com").unwrap(),
                address: CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                dns_server: Ipv4Addr::new(192, 168, 100, 1).into(),
                pinning_opts: None,
            }
        );

        info.hostname = None;
        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info, None, None),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("pve.bar.com").unwrap(),
                address: CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                dns_server: Ipv4Addr::new(192, 168, 100, 1).into(),
                pinning_opts: None,
            }
        );

        info.dns.domain = None;
        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info, None, None),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("pve.example.invalid").unwrap(),
                address: CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                dns_server: Ipv4Addr::new(192, 168, 100, 1).into(),
                pinning_opts: None,
            }
        );

        info.hostname = Some("foo".to_owned());
        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info, None, None),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("foo.example.invalid").unwrap(),
                address: CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                dns_server: Ipv4Addr::new(192, 168, 100, 1).into(),
                pinning_opts: None,
            }
        );
    }

    #[test]
    fn network_options_correctly_handles_user_supplied_default_domain() {
        let (setup, mut info) = mock_setup_network();

        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info, None, None),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("foo.bar.com").unwrap(),
                address: CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                dns_server: Ipv4Addr::new(192, 168, 100, 1).into(),
                pinning_opts: None,
            }
        );

        info.dns.domain = None;
        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info, Some("custom.local"), None),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("foo.custom.local").unwrap(),
                address: CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                dns_server: Ipv4Addr::new(192, 168, 100, 1).into(),
                pinning_opts: None,
            }
        );

        info.dns.domain = Some("some.domain.local".to_owned());
        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info, Some("custom.local"), None),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("foo.custom.local").unwrap(),
                address: CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                dns_server: Ipv4Addr::new(192, 168, 100, 1).into(),
                pinning_opts: None,
            }
        );
    }

    #[test]
    fn network_options_default_addresses_are_sane() {
        let mut interfaces = BTreeMap::new();
        interfaces.insert(
            "eth0".to_owned(),
            Interface {
                name: "eth0".to_owned(),
                index: 0,
                pinned_id: "0".to_owned(),
                state: InterfaceState::Up,
                driver: "dummy".to_owned(),
                mac: "01:23:45:67:89:ab".to_owned(),
                addresses: vec![],
            },
        );

        let info = NetworkInfo {
            dns: Dns {
                domain: None,
                dns: vec![],
            },
            routes: None,
            interfaces,
            hostname: None,
        };

        let setup = SetupInfo::mocked();

        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info, None, None),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("pve.example.invalid").unwrap(),
                address: CidrAddress::new(Ipv4Addr::new(192, 168, 100, 2), 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 100, 1)),
                dns_server: Ipv4Addr::new(192, 168, 100, 1).into(),
                pinning_opts: None,
            }
        );
    }

    #[test]
    fn network_interface_pinning_options_fail_on_empty_name() {
        let mut options = NetworkInterfacePinningOptions::default();
        options
            .mapping
            .insert("ab:cd:ef:12:34:56".to_owned(), String::new());

        let res = options.verify();
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "interface name for 'ab:cd:ef:12:34:56' cannot be empty"
        )
    }

    #[test]
    fn network_interface_pinning_options_fail_on_overlong_name() {
        let mut options = NetworkInterfacePinningOptions::default();
        options.mapping.insert(
            "ab:cd:ef:12:34:56".to_owned(),
            "waytoolonginterfacename".to_owned(),
        );

        let res = options.verify();
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "interface name 'waytoolonginterfacename' for 'ab:cd:ef:12:34:56' cannot be longer than 15 characters"
        )
    }

    #[test]
    fn network_interface_pinning_options_fail_on_duplicate_name() {
        let mut options = NetworkInterfacePinningOptions::default();
        options
            .mapping
            .insert("ab:cd:ef:12:34:56".to_owned(), "nic0".to_owned());
        options
            .mapping
            .insert("12:34:56:ab:cd:ef".to_owned(), "nic0".to_owned());

        let res = options.verify();
        assert!(res.is_err());
        let err = res.unwrap_err().to_string();

        // [HashMap] does not guarantee iteration order, so just check for the substrings
        // we expect to find
        assert!(err.contains("duplicate interface name mapping 'nic0' for: "));
        assert!(err.contains("12:34:56:ab:cd:ef"));
        assert!(err.contains("ab:cd:ef:12:34:56"));
    }

    #[test]
    fn network_interface_pinning_options_fail_on_invalid_characters() {
        let mut options = NetworkInterfacePinningOptions::default();
        options
            .mapping
            .insert("ab:cd:ef:12:34:56".to_owned(), "nic-".to_owned());

        let res = options.verify();
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "interface name 'nic-' for 'ab:cd:ef:12:34:56' is invalid: name must only consist of alphanumeric characters and underscores"
        )
    }

    #[test]
    fn network_interface_pinning_options_fail_on_name_starting_with_number() {
        let mut options = NetworkInterfacePinningOptions::default();
        options
            .mapping
            .insert("ab:cd:ef:12:34:56".to_owned(), "0nic".to_owned());

        let res = options.verify();
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "interface name '0nic' for 'ab:cd:ef:12:34:56' is invalid: name must not start with a number"
        )
    }

    #[test]
    fn network_interface_pinning_options_fail_on_fully_numeric_name() {
        let mut options = NetworkInterfacePinningOptions::default();
        options
            .mapping
            .insert("ab:cd:ef:12:34:56".to_owned(), "12345".to_owned());

        let res = options.verify();
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "interface name '12345' for 'ab:cd:ef:12:34:56' is invalid: name must not be fully numeric"
        )
    }
}
