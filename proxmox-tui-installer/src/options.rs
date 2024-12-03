use crate::SummaryOption;

use proxmox_installer_common::{
    options::{
        BootdiskOptions, BtrfsRaidLevel, FsType, NetworkOptions, TimezoneOptions, ZfsRaidLevel,
    },
    setup::LocaleInfo,
    EMAIL_DEFAULT_PLACEHOLDER,
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

#[cfg(test)]
mod tests {
    use super::*;
    use proxmox_installer_common::{
        setup::{
            Dns, Gateway, Interface, InterfaceState, IsoInfo, IsoLocations, NetworkInfo,
            ProductConfig, ProxmoxProduct, Routes, SetupInfo,
        },
        utils::{CidrAddress, Fqdn},
    };
    use std::net::{IpAddr, Ipv4Addr};
    use std::{collections::BTreeMap, path::PathBuf};

    fn dummy_setup_info() -> SetupInfo {
        SetupInfo {
            config: ProductConfig {
                fullname: "Proxmox VE".to_owned(),
                product: ProxmoxProduct::PVE,
                enable_btrfs: true,
            },
            iso_info: IsoInfo {
                release: String::new(),
                isorelease: String::new(),
            },
            locations: IsoLocations {
                iso: PathBuf::new(),
            },
        }
    }

    #[test]
    fn network_options_from_setup_network_info() {
        let setup = dummy_setup_info();

        let mut interfaces = BTreeMap::new();
        interfaces.insert(
            "eth0".to_owned(),
            Interface {
                name: "eth0".to_owned(),
                index: 0,
                state: InterfaceState::Up,
                mac: "01:23:45:67:89:ab".to_owned(),
                addresses: Some(vec![
                    CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap()
                ]),
            },
        );

        let mut info = NetworkInfo {
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

        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("foo.bar.com").unwrap(),
                address: CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                dns_server: Ipv4Addr::UNSPECIFIED.into(),
            }
        );

        info.hostname = None;
        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("pve.bar.com").unwrap(),
                address: CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                dns_server: Ipv4Addr::UNSPECIFIED.into(),
            }
        );

        info.dns.domain = None;
        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("pve.example.invalid").unwrap(),
                address: CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                dns_server: Ipv4Addr::UNSPECIFIED.into(),
            }
        );

        info.hostname = Some("foo".to_owned());
        pretty_assertions::assert_eq!(
            NetworkOptions::defaults_from(&setup, &info),
            NetworkOptions {
                ifname: "eth0".to_owned(),
                fqdn: Fqdn::from("foo.example.invalid").unwrap(),
                address: CidrAddress::new(Ipv4Addr::new(192, 168, 0, 2), 24).unwrap(),
                gateway: IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)),
                dns_server: Ipv4Addr::UNSPECIFIED.into(),
            }
        );
    }
}
