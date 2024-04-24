use clap::ValueEnum;
use proxmox_installer_common::{
    options::{BtrfsRaidLevel, FsType, ZfsChecksumOption, ZfsCompressOption, ZfsRaidLevel},
    utils::{CidrAddress, Fqdn},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::IpAddr};

// BTreeMap is used to store filters as the order of the filters will be stable, compared to
// storing them in a HashMap

#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Answer {
    pub global: Global,
    pub network: Network,
    #[serde(rename = "disk-setup")]
    pub disks: Disks,
}

#[derive(Clone, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Global {
    pub country: String,
    pub fqdn: Fqdn,
    pub keyboard: KeyboardLayout,
    pub mailto: String,
    pub timezone: String,
    pub root_password: String,
    #[serde(default)]
    pub reboot_on_error: bool,
    #[serde(default)]
    pub root_ssh_keys: Vec<String>,
}

#[derive(Clone, Deserialize, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
enum NetworkConfigMode {
    #[default]
    #[serde(rename = "from-dhcp")]
    FromDhcp,
    #[serde(rename = "from-answer")]
    FromAnswer,
}

#[derive(Clone, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct DiskSetup {
    pub filesystem: Filesystem,
    #[serde(default)]
    pub disk_list: Vec<String>,
    pub filter: Option<BTreeMap<String, String>>,
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
            return Err("Need either 'disk_list' or 'filter' set");
        }
        if !source.disk_list.is_empty() && source.filter.is_some() {
            return Err("Cannot use both, 'disk_list' and 'filter'");
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
                        return Err("ZFS raid level 'zfs.raid' must be set")
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
                        return Err("BTRFS raid level 'btrfs.raid' must be set")
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
#[derive(Clone, Deserialize, Debug, PartialEq, ValueEnum)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum FilterMatch {
    Any,
    All,
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Filesystem {
    Ext4,
    Xfs,
    Zfs,
    Btrfs,
}

#[derive(Clone, Copy, Default, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ZfsOptions {
    pub raid: Option<ZfsRaidLevel>,
    pub ashift: Option<usize>,
    pub arc_max: Option<usize>,
    pub checksum: Option<ZfsChecksumOption>,
    pub compress: Option<ZfsCompressOption>,
    pub copies: Option<usize>,
    pub hdsize: Option<f64>,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct LvmOptions {
    pub hdsize: Option<f64>,
    pub swapsize: Option<f64>,
    pub maxroot: Option<f64>,
    pub maxvz: Option<f64>,
    pub minfree: Option<f64>,
}

#[derive(Clone, Copy, Default, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct BtrfsOptions {
    pub hdsize: Option<f64>,
    pub raid: Option<BtrfsRaidLevel>,
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

impl std::fmt::Display for KeyboardLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keyboard_layout = serde_json::to_value(self).unwrap().to_string();
        write!(f, "{}", keyboard_layout.trim_matches('\"'))
    }
}
