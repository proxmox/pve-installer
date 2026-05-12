use anyhow::{Result, bail};
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use std::{
    cmp,
    collections::HashMap,
    fmt,
    net::{IpAddr, Ipv4Addr},
    sync::OnceLock,
};

use crate::disk_checks::check_raid_min_disks;
use crate::net::{MAX_IFNAME_LEN, MIN_IFNAME_LEN};
use crate::setup::{LocaleInfo, NetworkInfo, RuntimeInfo, SetupInfo};
use proxmox_installer_types::{
    EMAIL_DEFAULT_PLACEHOLDER,
    answer::{
        BtrfsCompressOption, BtrfsRaidLevel, FilesystemType, NetworkInterfacePinningOptionsAnswer,
        ZfsChecksumOption, ZfsCompressOption, ZfsRaidLevel,
    },
};
use proxmox_network_types::{fqdn::Fqdn, ip_address::Cidr};

pub trait RaidLevel {
    /// Returns the minimum number of disks needed for this RAID level.
    fn get_min_disks(&self) -> usize;

    /// Checks whether a user-supplied Btrfs RAID setup is valid or not, such as minimum
    /// number of disks.
    ///
    /// # Arguments
    ///
    /// * `disks` - List of disks designated as RAID targets.
    fn check_raid_disks_setup(&self, disks: &[Disk]) -> Result<(), String>;

    /// Checks whether the given disk sizes are compatible for the RAID level, if it is a mirror.
    fn check_mirror_size(&self, _disk1: &Disk, _disk2: &Disk) -> Result<(), String> {
        Ok(())
    }
}

impl RaidLevel for BtrfsRaidLevel {
    fn get_min_disks(&self) -> usize {
        match self {
            Self::Raid0 => 1,
            Self::Raid1 => 2,
            Self::Raid10 => 4,
        }
    }

    fn check_raid_disks_setup(&self, disks: &[Disk]) -> Result<(), String> {
        check_raid_min_disks(disks, self.get_min_disks())?;
        Ok(())
    }
}

impl RaidLevel for ZfsRaidLevel {
    fn get_min_disks(&self) -> usize {
        match self {
            ZfsRaidLevel::Raid0 => 1,
            ZfsRaidLevel::Raid1 => 2,
            ZfsRaidLevel::Raid10 => 4,
            ZfsRaidLevel::RaidZ => 3,
            ZfsRaidLevel::RaidZ2 => 4,
            ZfsRaidLevel::RaidZ3 => 5,
        }
    }

    fn check_raid_disks_setup(&self, disks: &[Disk]) -> Result<(), String> {
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

    fn check_mirror_size(&self, disk1: &Disk, disk2: &Disk) -> Result<(), String> {
        if (disk1.size - disk2.size).abs() > disk1.size / 10. {
            Err(format!(
                "Mirrored disks must have same size:\n\n  * {disk1}\n  * {disk2}"
            ))
        } else {
            Ok(())
        }
    }
}

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

pub trait FilesystemDiskInfo {
    /// Returns the minimum number of disks needed for this filesystem.
    fn get_min_disks(&self) -> usize;
}

impl FilesystemDiskInfo for FilesystemType {
    fn get_min_disks(&self) -> usize {
        match self {
            FilesystemType::Ext4 => 1,
            FilesystemType::Xfs => 1,
            FilesystemType::Zfs(level) => level.get_min_disks(),
            FilesystemType::Btrfs(level) => level.get_min_disks(),
        }
    }
}

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
    pub fstype: FilesystemType,
    pub advanced: AdvancedBootdiskOptions,
}

impl BootdiskOptions {
    pub fn defaults_from(disk: &Disk) -> Self {
        Self {
            disks: vec![disk.clone()],
            fstype: FilesystemType::Ext4,
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
        // Mimicking the `pve-iface` schema verification
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            RegexBuilder::new(r"^[a-z][a-z0-9_]{1,20}([:\.]\d+)?$")
                .case_insensitive(true)
                .build()
                .unwrap()
        });

        let mut reverse_mapping = HashMap::<String, String>::new();
        for (mac, name) in self.mapping.iter() {
            if name.len() < MIN_IFNAME_LEN {
                bail!(
                    "interface name for '{mac}' must be at least {MIN_IFNAME_LEN} characters long"
                );
            }

            if name.len() > MAX_IFNAME_LEN {
                bail!(
                    "interface name '{name}' for '{mac}' cannot be longer than {} characters",
                    MAX_IFNAME_LEN
                );
            }

            if !re.is_match(name) {
                bail!(
                    "interface name '{name}' for '{mac}' is invalid: name must start with a letter and contain only ascii characters, digits and underscores"
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

impl From<&NetworkInterfacePinningOptionsAnswer> for NetworkInterfacePinningOptions {
    fn from(answer: &NetworkInterfacePinningOptionsAnswer) -> Self {
        if answer.enabled {
            Self {
                // convert all MAC addresses to lowercase before further usage,
                // to enable easy comparison
                mapping: answer
                    .mapping
                    .iter()
                    .map(|(k, v)| (k.to_lowercase(), v.clone()))
                    .collect(),
            }
        } else {
            Self::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NetworkOptions {
    pub ifname: String,
    pub fqdn: Fqdn,
    pub address: Cidr,
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
            fqdn: Self::construct_fqdn(network, &setup.config.product.to_string(), default_domain),
            // Safety: The provided IP address/mask is always valid.
            // These are the same as used in the GTK-based installer.
            address: Cidr::new_v4([192, 168, 100, 2], 24).unwrap(),
            gateway: Ipv4Addr::new(192, 168, 100, 1).into(),
            dns_server: Ipv4Addr::new(192, 168, 100, 1).into(),
            pinning_opts: pinning_opts.cloned(),
        };

        let iface = if let Some(routes) = &network.routes {
            if let Some(gw) = &routes.gateway4
                && let Some(iface) = network.interfaces.get(&gw.dev)
            {
                this.gateway = gw.gateway;

                if let Some(addr) = iface.addresses.iter().find(|addr| addr.is_ipv4()) {
                    this.address = *addr;
                }

                if let Some(addr) = network.dns.dns.iter().find(|addr| addr.is_ipv4()) {
                    this.dns_server = *addr;
                }

                Some(iface)
            } else if let Some(gw) = &routes.gateway6
                && let Some(iface) = network.interfaces.get(&gw.dev)
            {
                this.gateway = gw.gateway;

                if let Some(addr) = iface.addresses.iter().find(|addr| addr.is_ipv6()) {
                    this.address = *addr;
                }

                if let Some(addr) = network.dns.dns.iter().find(|addr| addr.is_ipv6()) {
                    this.dns_server = *addr;
                }

                Some(iface)
            } else {
                None
            }
        } else {
            None
        }
        .unwrap_or_else(|| {
            // Safety: In case no there are no routes defined at all (e.g. no DHCP lease), try to
            // set the interface name to *some* valid values. At least one NIC must always be
            // present here, as the installation will abort earlier otherwise, so use the first one
            // enumerated.
            network
                .interfaces
                .values()
                .min_by_key(|v| v.index)
                .expect("at least one NIC must be present")
        });

        // Use pinned network interface name, if enabled
        if let Some(pinned) = pinning_opts.and_then(|opts| iface.to_pinned(opts)) {
            this.ifname.clone_from(&pinned.name);
        } else {
            this.ifname.clone_from(&iface.name);
        }

        if let Some(ref mut opts) = this.pinning_opts {
            // Ensure that all unique, pinnable interfaces indeed have an entry in the map, as
            // required by the low-level installer
            for iface in network.interfaces.values() {
                if let Some(pinned) = iface.to_pinned(opts) {
                    opts.mapping.entry(iface.mac.clone()).or_insert(pinned.name);
                }
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
    } else if email == EMAIL_DEFAULT_PLACEHOLDER {
        bail!("Invalid (default) email address")
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::{Dns, Gateway, Interface, InterfaceState, NetworkInfo, Routes, SetupInfo};
    use std::collections::BTreeMap;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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
                pinned_id: Some("0".to_owned()),
                state: InterfaceState::Up,
                driver: "dummy".to_owned(),
                mac: "01:23:45:67:89:ab".to_owned(),
                addresses: vec![Cidr::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap()],
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
                address: Cidr::new_v4([192, 168, 0, 2], 24).unwrap(),
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
                address: Cidr::new_v4([192, 168, 0, 2], 24).unwrap(),
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
                address: Cidr::new_v4([192, 168, 0, 2], 24).unwrap(),
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
                address: Cidr::new_v4([192, 168, 0, 2], 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                dns_server: Ipv4Addr::new(192, 168, 100, 1).into(),
                pinning_opts: None,
            }
        );
    }

    fn mock_setup_network_v6_only() -> (SetupInfo, NetworkInfo) {
        let mut interfaces = BTreeMap::new();
        interfaces.insert(
            "eth0".to_owned(),
            Interface {
                name: "eth0".to_owned(),
                index: 0,
                pinned_id: Some("0".to_owned()),
                state: InterfaceState::Up,
                driver: "dummy".to_owned(),
                mac: "01:23:45:67:89:ab".to_owned(),
                addresses: vec![
                    Cidr::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2), 64).unwrap(),
                ],
            },
        );

        let info = NetworkInfo {
            dns: Dns {
                domain: Some("bar.com".to_owned()),
                dns: vec![IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x53))],
            },
            routes: Some(Routes {
                gateway4: None,
                gateway6: Some(Gateway {
                    dev: "eth0".to_owned(),
                    gateway: IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
                }),
            }),
            interfaces,
            hostname: Some("foo".to_owned()),
        };

        (SetupInfo::mocked(), info)
    }

    #[test]
    fn network_options_from_setup_network_info_ipv6_only() {
        let (setup, info) = mock_setup_network_v6_only();

        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info, None, None),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("foo.bar.com").unwrap(),
                address: Cidr::new_v6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 2), 64).unwrap(),
                gateway: IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
                dns_server: IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x53)),
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
                address: Cidr::new_v4([192, 168, 0, 2], 24).unwrap(),
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
                address: Cidr::new_v4([192, 168, 0, 2], 24).unwrap(),
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
                address: Cidr::new_v4([192, 168, 0, 2], 24).unwrap(),
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
                pinned_id: Some("0".to_owned()),
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
                address: Cidr::new_v4([192, 168, 100, 2], 24).unwrap(),
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
            "interface name for 'ab:cd:ef:12:34:56' must be at least 2 characters long"
        )
    }

    #[test]
    fn network_interface_pinning_options_fail_on_too_short_name() {
        let mut options = NetworkInterfacePinningOptions::default();
        options
            .mapping
            .insert("ab:cd:ef:12:34:56".to_owned(), "a".to_owned());

        let res = options.verify();
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "interface name for 'ab:cd:ef:12:34:56' must be at least 2 characters long"
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
            "interface name 'nic-' for 'ab:cd:ef:12:34:56' is invalid: name must start with a letter and contain only ascii characters, digits and underscores"
        )
    }

    #[test]
    fn network_interface_pinning_options_fail_on_nonletter_first_char() {
        let mut options = NetworkInterfacePinningOptions::default();
        options
            .mapping
            .insert("ab:cd:ef:12:34:56".to_owned(), "0nic".to_owned());

        let res = options.verify();
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "interface name '0nic' for 'ab:cd:ef:12:34:56' is invalid: name must start with a letter and contain only ascii characters, digits and underscores"
        );

        options
            .mapping
            .insert("ab:cd:ef:12:34:56".to_owned(), "_a".to_owned());

        let res = options.verify();
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "interface name '_a' for 'ab:cd:ef:12:34:56' is invalid: name must start with a letter and contain only ascii characters, digits and underscores"
        );
    }

    #[test]
    fn network_interface_pinning_options_pass_on_uppercase_char() {
        let mut options = NetworkInterfacePinningOptions::default();
        options
            .mapping
            .insert("ab:cd:ef:12:34:56".to_owned(), "Nic0".to_owned());

        let res = options.verify();
        assert!(res.is_ok());

        options
            .mapping
            .insert("ab:cd:ef:12:34:56".to_owned(), "nIc0".to_owned());

        let res = options.verify();
        assert!(res.is_ok());

        options
            .mapping
            .insert("ab:cd:ef:12:34:56".to_owned(), "nic0".to_owned());

        let res = options.verify();
        assert!(res.is_ok());
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
            "interface name '12345' for 'ab:cd:ef:12:34:56' is invalid: name must start with a letter and contain only ascii characters, digits and underscores"
        )
    }
}
