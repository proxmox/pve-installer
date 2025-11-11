use anyhow::{Context, Result, bail};
use glob::Pattern;
use log::{info, warn};
use std::{
    collections::{BTreeMap, HashSet},
    process::Command,
};

use crate::{
    answer::{
        self, Answer, DiskSelection, FirstBootHookSourceMode, FqdnConfig, FqdnExtendedConfig,
        FqdnSourceMode, Network,
    },
    udevinfo::UdevInfo,
};
use proxmox_installer_common::{
    ROOT_PASSWORD_MIN_LENGTH,
    disk_checks::check_swapsize,
    options::{FsType, NetworkOptions, ZfsChecksumOption, ZfsCompressOption, email_validate},
    setup::{
        InstallBtrfsOption, InstallConfig, InstallFirstBootSetup, InstallRootPassword,
        InstallZfsOption, LocaleInfo, RuntimeInfo, SetupInfo,
    },
};
use serde::{Deserialize, Serialize};

fn get_network_settings(
    answer: &Answer,
    udev_info: &UdevInfo,
    runtime_info: &RuntimeInfo,
    setup_info: &SetupInfo,
) -> Result<NetworkOptions> {
    info!("Setting up network configuration");

    let mut network_options = match &answer.global.fqdn {
        // If the user set a static FQDN in the answer file, override it
        FqdnConfig::Simple(name) => {
            let mut opts = NetworkOptions::defaults_from(
                setup_info,
                &runtime_info.network,
                None,
                answer.network.interface_name_pinning.as_ref(),
            );
            opts.fqdn = name.to_owned();
            opts
        }
        FqdnConfig::Extended(FqdnExtendedConfig {
            source: FqdnSourceMode::FromDhcp,
            domain,
        }) => {
            // A FQDN from DHCP information and/or defaults is constructed in
            // `NetworkOptions::defaults_from()` below, just check that the DHCP server actually
            // provided a hostname.
            if runtime_info.network.hostname.is_none() {
                bail!(
                    "`global.fqdn.source` set to \"from-dhcp\", but DHCP server did not provide a hostname!"
                );
            }

            // Either a domain must be received from the DHCP server or it must be set manually
            // (even just as fallback) in the answer file.
            if runtime_info.network.dns.domain.is_none() && domain.is_none() {
                bail!("no domain received from DHCP server and `global.fqdn.domain` is unset!");
            }

            NetworkOptions::defaults_from(
                setup_info,
                &runtime_info.network,
                domain.as_deref(),
                answer.network.interface_name_pinning.as_ref(),
            )
        }
    };

    if let answer::NetworkSettings::Manual(settings) = &answer.network.network_settings {
        network_options.address = settings.cidr.clone();
        network_options.dns_server = settings.dns;
        network_options.gateway = settings.gateway;
        network_options.ifname = get_single_udev_index(&settings.filter, &udev_info.nics)?;
    }

    if let Some(opts) = &network_options.pinning_opts {
        info!("Network interface name pinning is enabled");
        opts.verify()?;
    }

    info!("Network interface used is '{}'", &network_options.ifname);
    Ok(network_options)
}

pub fn get_single_udev_index(
    filter: &BTreeMap<String, String>,
    udev_list: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<String> {
    if filter.is_empty() {
        bail!("no filter defined");
    }
    let mut dev_index: Option<String> = None;
    'outer: for (dev, dev_values) in udev_list {
        for (filter_key, filter_value) in filter {
            let filter_pattern =
                Pattern::new(filter_value).context("invalid glob in disk selection")?;
            for (udev_key, udev_value) in dev_values {
                if udev_key == filter_key && filter_pattern.matches(udev_value) {
                    dev_index = Some(dev.clone());
                    break 'outer; // take first match
                }
            }
        }
    }
    if dev_index.is_none() {
        bail!("filter did not match any device");
    }

    Ok(dev_index.unwrap())
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum FetchAnswerFrom {
    Iso,
    Http,
    Partition,
}

serde_plain::derive_fromstr_from_deserialize!(FetchAnswerFrom);

#[derive(Deserialize, Serialize, Clone, Default, PartialEq, Debug)]
pub struct HttpOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_fingerprint: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub struct AutoInstSettings {
    pub mode: FetchAnswerFrom,
    #[serde(default = "default_partition_label")]
    pub partition_label: String,
    #[serde(default)]
    pub http: HttpOptions,
}

pub fn default_partition_label() -> String {
    "proxmox-ais".to_owned()
}

#[derive(Deserialize, Debug)]
struct IpLinksUdevInfo {
    ifname: String,
}

/// Returns vec of usable NICs
pub fn get_nic_list() -> Result<Vec<String>> {
    let ip_output = Command::new("/usr/sbin/ip")
        .arg("-j")
        .arg("link")
        .output()?;
    let parsed_links: Vec<IpLinksUdevInfo> = serde_json::from_slice(&ip_output.stdout)?;
    let mut links: Vec<String> = Vec::new();

    for link in parsed_links {
        if link.ifname == *"lo" {
            continue;
        }
        links.push(link.ifname);
    }

    Ok(links)
}

pub fn get_matched_udev_indexes(
    filter: &BTreeMap<String, String>,
    udev_list: &BTreeMap<String, BTreeMap<String, String>>,
    match_all: bool,
) -> Result<Vec<String>> {
    let mut matches = vec![];
    for (dev, dev_values) in udev_list {
        let mut did_match_once = false;
        let mut did_match_all = true;
        for (filter_key, filter_value) in filter {
            let filter_pattern =
                Pattern::new(filter_value).context("invalid glob in disk selection")?;
            for (udev_key, udev_value) in dev_values {
                if udev_key == filter_key && filter_pattern.matches(udev_value) {
                    did_match_once = true;
                } else if udev_key == filter_key {
                    did_match_all = false;
                }
            }
        }
        if (match_all && did_match_all) || (!match_all && did_match_once) {
            matches.push(dev.clone());
        }
    }
    if matches.is_empty() {
        bail!("filter did not match any devices");
    }
    matches.sort();
    Ok(matches)
}

fn set_disks(
    answer: &Answer,
    udev_info: &UdevInfo,
    runtime_info: &RuntimeInfo,
    config: &mut InstallConfig,
) -> Result<()> {
    match config.filesys {
        FsType::Ext4 | FsType::Xfs => set_single_disk(answer, udev_info, runtime_info, config),
        FsType::Zfs(_) | FsType::Btrfs(_) => {
            set_selected_disks(answer, udev_info, runtime_info, config)
        }
    }
}

fn set_single_disk(
    answer: &Answer,
    udev_info: &UdevInfo,
    runtime_info: &RuntimeInfo,
    config: &mut InstallConfig,
) -> Result<()> {
    match &answer.disks.disk_selection {
        answer::DiskSelection::Selection(disk_list) => {
            let disk_name = disk_list[0].clone();
            let disk = runtime_info
                .disks
                .iter()
                .find(|item| item.path.ends_with(disk_name.as_str()));
            match disk {
                Some(disk) => config.target_hd = Some(disk.path.clone()),
                None => bail!("disk in 'disk-selection' not found"),
            }
        }
        answer::DiskSelection::Filter(filter) => {
            let disk_index = get_single_udev_index(filter, &udev_info.disks)?;
            let disk = runtime_info
                .disks
                .iter()
                .find(|item| item.index == disk_index);
            config.target_hd = disk.map(|d| d.path.clone());
        }
    }
    info!("Selected disk: {}", config.target_hd.clone().unwrap());
    Ok(())
}

fn set_selected_disks(
    answer: &Answer,
    udev_info: &UdevInfo,
    runtime_info: &RuntimeInfo,
    config: &mut InstallConfig,
) -> Result<()> {
    match &answer.disks.disk_selection {
        answer::DiskSelection::Selection(disk_list) => {
            info!("Disk selection found");
            for disk_name in disk_list.clone() {
                let disk = runtime_info
                    .disks
                    .iter()
                    .find(|item| item.path.ends_with(disk_name.as_str()));
                if let Some(disk) = disk {
                    config
                        .disk_selection
                        .insert(disk.index.clone(), disk.index.clone());
                }
            }
        }
        answer::DiskSelection::Filter(filter) => {
            info!("No disk list found, looking for disk filters");
            let filter_match = answer
                .disks
                .filter_match
                .clone()
                .unwrap_or(answer::FilterMatch::Any);
            let selected_disk_indexes = get_matched_udev_indexes(
                filter,
                &udev_info.disks,
                filter_match == answer::FilterMatch::All,
            )?;

            for i in selected_disk_indexes.into_iter() {
                let disk = runtime_info
                    .disks
                    .iter()
                    .find(|item| item.index == i)
                    .unwrap();
                config
                    .disk_selection
                    .insert(disk.index.clone(), disk.index.clone());
            }
        }
    }
    if config.disk_selection.is_empty() {
        bail!("No disks found matching selection.");
    }

    let mut selected_disks: Vec<String> = Vec::new();
    for i in config.disk_selection.keys() {
        selected_disks.push(
            runtime_info
                .disks
                .iter()
                .find(|item| item.index.as_str() == i)
                .unwrap()
                .clone()
                .path,
        );
    }
    info!(
        "Selected disks: {}",
        selected_disks
            .iter()
            .map(|x| x.to_string() + " ")
            .collect::<String>()
    );

    Ok(())
}

fn get_first_selected_disk(config: &InstallConfig) -> usize {
    config
        .disk_selection
        .iter()
        .next()
        .expect("no disks found")
        .0
        .parse::<usize>()
        .expect("could not parse key to usize")
}

fn verify_filesystem_settings(answer: &Answer, setup_info: &SetupInfo) -> Result<()> {
    info!("Verifying filesystem settings");

    if answer.disks.fs_type.is_btrfs() && !setup_info.config.enable_btrfs {
        bail!(
            "BTRFS is not supported as a root filesystem for the product or the release of this ISO."
        );
    }

    Ok(())
}

pub fn verify_locale_settings(answer: &Answer, locales: &LocaleInfo) -> Result<()> {
    info!("Verifying locale settings");
    if !locales
        .countries
        .keys()
        .any(|i| i == &answer.global.country)
    {
        bail!("country code '{}' is not valid", &answer.global.country);
    }
    if !locales
        .kmap
        .keys()
        .any(|i| i == &answer.global.keyboard.to_string())
    {
        bail!("keyboard layout '{}' is not valid", &answer.global.keyboard);
    }

    if !locales
        .cczones
        .iter()
        .any(|(_, zones)| zones.contains(&answer.global.timezone))
        && answer.global.timezone != "UTC"
    {
        bail!("timezone '{}' is not valid", &answer.global.timezone);
    }

    Ok(())
}

/// Validates the following options of an user-provided answer:
///
/// - `global.root-password`
/// - `global.root-password-hashed`
/// - `global.mailto`
///
/// Ensures that the provided email-address is of valid format and that one
/// of the two root password options is set appropriately.
pub fn verify_email_and_root_password_settings(answer: &Answer) -> Result<()> {
    info!("Verifying email and root password settings");

    email_validate(&answer.global.mailto).with_context(|| answer.global.mailto.clone())?;

    match (
        &answer.global.root_password,
        &answer.global.root_password_hashed,
    ) {
        (Some(_), Some(_)) => {
            bail!(
                "`global.root-password` and `global.root-password-hashed` cannot be set at the same time"
            );
        }
        (None, None) => {
            bail!("One of `global.root-password` or `global.root-password-hashed` must be set");
        }
        (Some(password), None) if password.len() < ROOT_PASSWORD_MIN_LENGTH => {
            bail!(
                "`global.root-password` must be at least {ROOT_PASSWORD_MIN_LENGTH} characters long"
            );
        }
        _ => Ok(()),
    }
}

pub fn verify_disks_settings(answer: &Answer) -> Result<()> {
    if let DiskSelection::Selection(selection) = &answer.disks.disk_selection {
        let min_disks = answer.disks.fs_type.get_min_disks();
        if selection.len() < min_disks {
            bail!(
                "{}: need at least {} disks",
                answer.disks.fs_type,
                min_disks
            );
        }

        let mut disk_set = HashSet::new();
        for disk in selection {
            if !disk_set.insert(disk) {
                bail!("List of disks contains duplicate device {disk}");
            }
        }
    }

    if let answer::FsOptions::LVM(lvm) = &answer.disks.fs_options
        && let Some((swapsize, hdsize)) = lvm.swapsize.zip(lvm.hdsize) {
            check_swapsize(swapsize, hdsize)?;
        }

    Ok(())
}

pub fn verify_first_boot_settings(answer: &Answer) -> Result<()> {
    info!("Verifying first boot settings");

    if let Some(first_boot) = &answer.first_boot
        && first_boot.source == FirstBootHookSourceMode::FromUrl && first_boot.url.is_none() {
            bail!("first-boot executable source set to URL, but none specified!");
        }

    Ok(())
}

pub fn verify_network_settings(network: &Network, run_env: Option<&RuntimeInfo>) -> Result<()> {
    info!("Verifying network settings");

    if let Some(pin_opts) = &network.interface_name_pinning {
        pin_opts.verify()?;

        if let Some(run_env) = run_env {
            for (mac, name) in pin_opts.mapping.iter() {
                if !run_env
                    .network
                    .interfaces
                    .values()
                    .any(|iface| iface.mac == *mac)
                {
                    warn!(
                        "found unknown address '{mac}' (mapped to '{name}') in network interface pinning options"
                    );
                }
            }
        }
    }

    Ok(())
}

pub fn parse_answer(
    answer: &Answer,
    udev_info: &UdevInfo,
    runtime_info: &RuntimeInfo,
    locales: &LocaleInfo,
    setup_info: &SetupInfo,
) -> Result<InstallConfig> {
    info!("Parsing answer file");

    verify_filesystem_settings(answer, setup_info)?;

    info!("Setting File system");
    let filesystem = answer.disks.fs_type;
    info!("File system selected: {}", filesystem);

    let network_settings = get_network_settings(answer, udev_info, runtime_info, setup_info)?;

    verify_locale_settings(answer, locales)?;
    verify_disks_settings(answer)?;
    verify_email_and_root_password_settings(answer)?;
    verify_first_boot_settings(answer)?;
    verify_network_settings(&answer.network, Some(runtime_info))?;

    let root_password = match (
        &answer.global.root_password,
        &answer.global.root_password_hashed,
    ) {
        (Some(password), None) => InstallRootPassword::Plain(password.to_owned()),
        (None, Some(hashed)) => InstallRootPassword::Hashed(hashed.to_owned()),
        // Make the compiler happy, won't be reached anyway due to above checks
        _ => bail!("invalid root password setting"),
    };

    let mut config = InstallConfig {
        autoreboot: 1_usize,
        filesys: filesystem,
        hdsize: 0.,
        swapsize: None,
        maxroot: None,
        minfree: None,
        maxvz: None,
        zfs_opts: None,
        btrfs_opts: None,
        target_hd: None,
        disk_selection: BTreeMap::new(),
        existing_storage_auto_rename: 1,

        country: answer.global.country.clone(),
        timezone: answer.global.timezone.clone(),
        keymap: answer.global.keyboard.to_string(),

        root_password,
        mailto: answer.global.mailto.clone(),
        root_ssh_keys: answer.global.root_ssh_keys.clone(),

        mngmt_nic: network_settings.ifname,
        network_interface_pin_map: network_settings
            .pinning_opts
            .map(|o| o.mapping)
            .unwrap_or_default(),

        hostname: network_settings
            .fqdn
            .host()
            .unwrap_or(setup_info.config.product.default_hostname())
            .to_string(),
        domain: network_settings.fqdn.domain(),
        cidr: network_settings.address,
        gateway: network_settings.gateway,
        dns: network_settings.dns_server,

        first_boot: InstallFirstBootSetup::default(),
    };

    set_disks(answer, udev_info, runtime_info, &mut config)?;
    match &answer.disks.fs_options {
        answer::FsOptions::LVM(lvm) => {
            let disk = runtime_info
                .disks
                .iter()
                .find(|d| Some(&d.path) == config.target_hd.as_ref());

            config.hdsize = lvm
                .hdsize
                .unwrap_or_else(|| disk.map(|d| d.size).unwrap_or_default());
            config.swapsize = lvm.swapsize;
            config.maxroot = lvm.maxroot;
            config.maxvz = lvm.maxvz;
            config.minfree = lvm.minfree;
        }
        answer::FsOptions::ZFS(zfs) => {
            let first_selected_disk = get_first_selected_disk(&config);

            config.hdsize = zfs
                .hdsize
                .unwrap_or(runtime_info.disks[first_selected_disk].size);
            config.zfs_opts = Some(InstallZfsOption {
                ashift: zfs.ashift.unwrap_or(12),
                arc_max: zfs.arc_max.unwrap_or(runtime_info.default_zfs_arc_max),
                compress: zfs.compress.unwrap_or(ZfsCompressOption::On),
                checksum: zfs.checksum.unwrap_or(ZfsChecksumOption::On),
                copies: zfs.copies.unwrap_or(1),
            });
        }
        answer::FsOptions::BTRFS(btrfs) => {
            let first_selected_disk = get_first_selected_disk(&config);

            config.hdsize = btrfs
                .hdsize
                .unwrap_or(runtime_info.disks[first_selected_disk].size);
            config.btrfs_opts = Some(InstallBtrfsOption {
                compress: btrfs.compress.unwrap_or_default(),
            })
        }
    }

    if let Some(first_boot) = &answer.first_boot {
        config.first_boot.enabled = true;
        config.first_boot.ordering_target =
            Some(first_boot.ordering.as_systemd_target_name().to_owned());
    }

    Ok(config)
}
