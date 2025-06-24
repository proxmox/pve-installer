use anyhow::{Result, format_err};
use proxmox_installer_common::{
    options::{
        BtrfsCompressOption, BtrfsRaidLevel, FsType, ZfsChecksumOption, ZfsCompressOption,
        ZfsRaidLevel,
    },
    utils::{CidrAddress, Fqdn},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::BufRead, net::IpAddr};

// NOTE New answer file properties must use kebab-case, but should allow snake_case for backwards
// compatibility. TODO Remove the snake_cased variants in a future major version (e.g. PVE 10).

// BTreeMap is used to store filters as the order of the filters will be stable, compared to
// storing them in a HashMap

#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Answer {
    pub global: Global,
    pub network: Network,
    #[serde(rename = "disk-setup")]
    pub disks: Disks,
    #[serde(default)]
    pub post_installation_webhook: Option<PostNotificationHookInfo>,
    #[serde(default)]
    pub first_boot: Option<FirstBootHookInfo>,
}

impl Answer {
    pub fn try_from_reader(reader: impl BufRead) -> Result<Self> {
        let mut buffer = String::new();
        let lines = reader.lines();
        for line in lines {
            buffer.push_str(&line.unwrap());
            buffer.push('\n');
        }

        toml::from_str(&buffer).map_err(|err| format_err!("Failed parsing answer file: {err}"))
    }
}

#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Global {
    pub country: String,
    /// FQDN to set for the installed system.
    pub fqdn: FqdnConfig,
    pub keyboard: KeyboardLayout,
    pub mailto: String,
    pub timezone: String,
    #[serde(alias = "root_password")]
    pub root_password: Option<String>,
    #[serde(alias = "root_password_hashed")]
    pub root_password_hashed: Option<String>,
    #[serde(alias = "reboot_on_error", default)]
    pub reboot_on_error: bool,
    #[serde(alias = "reboot_mode", default)]
    pub reboot_mode: RebootMode,
    #[serde(alias = "root_ssh_keys", default)]
    pub root_ssh_keys: Vec<String>,
}

#[derive(Copy, Clone, Deserialize, Serialize, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum RebootMode {
    #[default]
    Reboot,
    PowerOff,
}

/// Allow the user to either set the FQDN of the installation to either some
/// fixed value or retrieve it dynamically via e.g.DHCP.
#[derive(Clone, Deserialize, Debug)]
#[serde(
    untagged,
    expecting = "either a fully-qualified domain name or extendend configuration for usage with DHCP must be specified"
)]
pub enum FqdnConfig {
    /// Sets the FQDN to the exact value.
    Simple(Fqdn),
    /// Extended configuration, e.g. to use hostname and domain from DHCP.
    Extended(FqdnExtendedConfig),
}

/// Extended configuration for retrieving the FQDN from external sources.
#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct FqdnExtendedConfig {
    /// Source to gather the FQDN from.
    #[serde(default)]
    pub source: FqdnSourceMode,
    /// Domain to use if none is received via DHCP.
    pub domain: Option<String>,
}

/// Describes the source to retrieve the FQDN of the installation.
#[derive(Clone, Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum FqdnSourceMode {
    #[default]
    FromDhcp,
}

#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct PostNotificationHookInfo {
    /// URL to send a POST request to
    pub url: String,
    /// SHA256 cert fingerprint if certificate pinning should be used.
    #[serde(alias = "cert_fingerprint")]
    pub cert_fingerprint: Option<String>,
}

/// Possible sources for the optional first-boot hook script/executable file.
#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum FirstBootHookSourceMode {
    /// Fetch the executable file from an URL, specified in the parent.
    FromUrl,
    /// The executable file has been baked into the ISO at a known location,
    /// and should be retrieved from there.
    FromIso,
}

/// Possible orderings for the `proxmox-first-boot` systemd service.
///
/// Determines the final value of `Unit.Before` and `Unit.Wants` in the service
/// file.
// Must be kept in sync with Proxmox::Install::Config and the service files in the
// proxmox-first-boot package.
#[derive(Clone, Default, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum FirstBootHookServiceOrdering {
    /// Needed for bringing up the network itself, runs before any networking is attempted.
    BeforeNetwork,
    /// Network needs to be already online, runs after networking was brought up.
    NetworkOnline,
    /// Runs after the system has successfully booted up completely.
    #[default]
    FullyUp,
}

impl FirstBootHookServiceOrdering {
    /// Maps the enum to the appropriate systemd target name, without the '.target' suffix.
    pub fn as_systemd_target_name(&self) -> &str {
        match self {
            FirstBootHookServiceOrdering::BeforeNetwork => "network-pre",
            FirstBootHookServiceOrdering::NetworkOnline => "network-online",
            FirstBootHookServiceOrdering::FullyUp => "multi-user",
        }
    }
}

/// Describes from where to fetch the first-boot hook script, either being baked into the ISO or
/// from a URL.
#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct FirstBootHookInfo {
    /// Mode how to retrieve the first-boot executable file, either from an URL or from the ISO if
    /// it has been baked-in.
    pub source: FirstBootHookSourceMode,
    /// Determines the service order when the hook will run on first boot.
    #[serde(default)]
    pub ordering: FirstBootHookServiceOrdering,
    /// Retrieve the post-install script from a URL, if source == "from-url".
    pub url: Option<String>,
    /// SHA256 cert fingerprint if certificate pinning should be used, if source == "from-url".
    #[serde(alias = "cert_fingerprint")]
    pub cert_fingerprint: Option<String>,
}

#[derive(Clone, Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
enum NetworkConfigMode {
    #[default]
    FromDhcp,
    FromAnswer,
}

#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct NetworkInAnswer {
    #[serde(default)]
    pub source: NetworkConfigMode,
    pub cidr: Option<CidrAddress>,
    pub dns: Option<IpAddr>,
    pub gateway: Option<IpAddr>,
    pub filter: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Deserialize, Debug)]
#[serde(try_from = "NetworkInAnswer", deny_unknown_fields)]
pub struct Network {
    pub network_settings: NetworkSettings,
}

impl TryFrom<NetworkInAnswer> for Network {
    type Error = &'static str;

    fn try_from(network: NetworkInAnswer) -> Result<Self, Self::Error> {
        if network.source == NetworkConfigMode::FromAnswer {
            if network.cidr.is_none() {
                return Err("Field 'cidr' must be set.");
            }
            if network.dns.is_none() {
                return Err("Field 'dns' must be set.");
            }
            if network.gateway.is_none() {
                return Err("Field 'gateway' must be set.");
            }
            if network.filter.is_none() {
                return Err("Field 'filter' must be set.");
            }

            Ok(Network {
                network_settings: NetworkSettings::Manual(NetworkManual {
                    cidr: network.cidr.unwrap(),
                    dns: network.dns.unwrap(),
                    gateway: network.gateway.unwrap(),
                    filter: network.filter.unwrap(),
                }),
            })
        } else {
            if network.cidr.is_some() {
                return Err("Field 'cidr' not supported for 'from-dhcp' config.");
            }
            if network.dns.is_some() {
                return Err("Field 'dns' not supported for 'from-dhcp' config.");
            }
            if network.gateway.is_some() {
                return Err("Field 'gateway' not supported for 'from-dhcp' config.");
            }
            if network.filter.is_some() {
                return Err("Field 'filter' not supported for 'from-dhcp' config.");
            }

            Ok(Network {
                network_settings: NetworkSettings::FromDhcp,
            })
        }
    }
}

#[derive(Clone, Debug)]
pub enum NetworkSettings {
    FromDhcp,
    Manual(NetworkManual),
}

#[derive(Clone, Debug)]
pub struct NetworkManual {
    pub cidr: CidrAddress,
    pub dns: IpAddr,
    pub gateway: IpAddr,
    pub filter: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct DiskSetup {
    pub filesystem: Filesystem,
    #[serde(alias = "disk_list", default)]
    pub disk_list: Vec<String>,
    pub filter: Option<BTreeMap<String, String>>,
    #[serde(alias = "filter_match")]
    pub filter_match: Option<FilterMatch>,
    pub zfs: Option<ZfsOptions>,
    pub lvm: Option<LvmOptions>,
    pub btrfs: Option<BtrfsOptions>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "DiskSetup", deny_unknown_fields)]
pub struct Disks {
    pub fs_type: FsType,
    pub disk_selection: DiskSelection,
    pub filter_match: Option<FilterMatch>,
    pub fs_options: FsOptions,
}

impl TryFrom<DiskSetup> for Disks {
    type Error = &'static str;

    fn try_from(source: DiskSetup) -> Result<Self, Self::Error> {
        if source.disk_list.is_empty() && source.filter.is_none() {
            return Err("Need either 'disk-list' or 'filter' set");
        }
        if !source.disk_list.is_empty() && source.filter.is_some() {
            return Err("Cannot use both, 'disk-list' and 'filter'");
        }

        let disk_selection = if !source.disk_list.is_empty() {
            DiskSelection::Selection(source.disk_list.clone())
        } else {
            DiskSelection::Filter(source.filter.clone().unwrap())
        };

        let lvm_checks = |source: &DiskSetup| -> Result<(), Self::Error> {
            if source.zfs.is_some() || source.btrfs.is_some() {
                return Err("make sure only 'lvm' options are set");
            }
            if source.disk_list.len() > 1 {
                return Err("make sure to define only one disk for ext4 and xfs");
            }
            Ok(())
        };
        // TODO: improve checks for foreign FS options. E.g. less verbose and handling new FS types
        // automatically
        let (fs, fs_options) = match source.filesystem {
            Filesystem::Xfs => {
                lvm_checks(&source)?;
                (FsType::Xfs, FsOptions::LVM(source.lvm.unwrap_or_default()))
            }
            Filesystem::Ext4 => {
                lvm_checks(&source)?;
                (FsType::Ext4, FsOptions::LVM(source.lvm.unwrap_or_default()))
            }
            Filesystem::Zfs => {
                if source.lvm.is_some() || source.btrfs.is_some() {
                    return Err("make sure only 'zfs' options are set");
                }
                match source.zfs {
                    None | Some(ZfsOptions { raid: None, .. }) => {
                        return Err("ZFS raid level 'zfs.raid' must be set");
                    }
                    Some(opts) => (FsType::Zfs(opts.raid.unwrap()), FsOptions::ZFS(opts)),
                }
            }
            Filesystem::Btrfs => {
                if source.zfs.is_some() || source.lvm.is_some() {
                    return Err("make sure only 'btrfs' options are set");
                }
                match source.btrfs {
                    None | Some(BtrfsOptions { raid: None, .. }) => {
                        return Err("BTRFS raid level 'btrfs.raid' must be set");
                    }
                    Some(opts) => (FsType::Btrfs(opts.raid.unwrap()), FsOptions::BTRFS(opts)),
                }
            }
        };

        let res = Disks {
            fs_type: fs,
            disk_selection,
            filter_match: source.filter_match,
            fs_options,
        };
        Ok(res)
    }
}

#[derive(Clone, Debug)]
pub enum FsOptions {
    LVM(LvmOptions),
    ZFS(ZfsOptions),
    BTRFS(BtrfsOptions),
}

#[derive(Clone, Debug)]
pub enum DiskSelection {
    Selection(Vec<String>),
    Filter(BTreeMap<String, String>),
}

#[derive(Clone, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum FilterMatch {
    Any,
    All,
}

serde_plain::derive_fromstr_from_deserialize!(FilterMatch);

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Filesystem {
    Ext4,
    Xfs,
    Zfs,
    Btrfs,
}

#[derive(Clone, Copy, Default, Deserialize, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ZfsOptions {
    pub raid: Option<ZfsRaidLevel>,
    pub ashift: Option<usize>,
    #[serde(alias = "arc_max")]
    pub arc_max: Option<usize>,
    pub checksum: Option<ZfsChecksumOption>,
    pub compress: Option<ZfsCompressOption>,
    pub copies: Option<usize>,
    pub hdsize: Option<f64>,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct LvmOptions {
    pub hdsize: Option<f64>,
    pub swapsize: Option<f64>,
    pub maxroot: Option<f64>,
    pub maxvz: Option<f64>,
    pub minfree: Option<f64>,
}

#[derive(Clone, Copy, Default, Deserialize, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct BtrfsOptions {
    pub hdsize: Option<f64>,
    pub raid: Option<BtrfsRaidLevel>,
    pub compress: Option<BtrfsCompressOption>,
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum KeyboardLayout {
    De,
    DeCh,
    Dk,
    EnGb,
    EnUs,
    Es,
    Fi,
    Fr,
    FrBe,
    FrCa,
    FrCh,
    Hu,
    Is,
    It,
    Jp,
    Lt,
    Mk,
    Nl,
    No,
    Pl,
    Pt,
    PtBr,
    Se,
    Si,
    Tr,
}

serde_plain::derive_display_from_serialize!(KeyboardLayout);
