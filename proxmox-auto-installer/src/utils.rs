use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use glob::Pattern;
use log::info;
use std::{collections::BTreeMap, process::Command};

use crate::{
    answer::{self, Answer},
    udevinfo::UdevInfo,
};
use proxmox_installer_common::{
    options::{email_validate, FsType, NetworkOptions, ZfsChecksumOption, ZfsCompressOption},
    setup::{
        InstallBtrfsOption, InstallConfig, InstallRootPassword, InstallZfsOption, LocaleInfo,
        RuntimeInfo, SetupInfo,
    },
};
use serde::{Deserialize, Serialize};

fn get_network_settings(
    answer: &Answer,
    udev_info: &UdevInfo,
    runtime_info: &RuntimeInfo,
    setup_info: &SetupInfo,
) -> Result<NetworkOptions> {
    let mut network_options = NetworkOptions::defaults_from(setup_info, &runtime_info.network);

    info!("Setting network configuration");

    // Always use the FQDN from the answer file
    network_options.fqdn = answer.global.fqdn.clone();

    if let answer::NetworkSettings::Manual(settings) = &answer.network.network_settings {
        network_options.address = settings.cidr.clone();
        network_options.dns_server = settings.dns;
        network_options.gateway = settings.gateway;
        network_options.ifname = get_single_udev_index(&settings.filter, &udev_info.nics)?;
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

#[derive(Deserialize, Serialize, Debug, Clone, ValueEnum, PartialEq)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum FetchAnswerFrom {
    Iso,
    Http,
    Partition,
}

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

fn default_partition_label() -> String {
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
                None => bail!("disk in 'disk_selection' not found"),
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

fn verify_locale_settings(answer: &Answer, locales: &LocaleInfo) -> Result<()> {
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

fn verify_email_and_root_password_settings(answer: &Answer) -> Result<()> {
    info!("Verifying email and root password settings");

    email_validate(&answer.global.mailto).with_context(|| answer.global.mailto.clone())?;

    if answer.global.root_password.is_some() && answer.global.root_password_hashed.is_some() {
        bail!("`global.root_password` and `global.root_password_hashed` cannot be set at the same time");
    } else if answer.global.root_password.is_none() && answer.global.root_password_hashed.is_none()
    {
        bail!("One of `global.root_password` or `global.root_password_hashed` must be set");
    } else {
        Ok(())
    }
}

pub fn parse_answer(
    answer: &Answer,
    udev_info: &UdevInfo,
    runtime_info: &RuntimeInfo,
    locales: &LocaleInfo,
    setup_info: &SetupInfo,
) -> Result<InstallConfig> {
    info!("Parsing answer file");
    info!("Setting File system");
    let filesystem = answer.disks.fs_type;
    info!("File system selected: {}", filesystem);

    let network_settings = get_network_settings(answer, udev_info, runtime_info, setup_info)?;

    verify_locale_settings(answer, locales)?;
    verify_email_and_root_password_settings(answer)?;

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

        root_password: InstallRootPassword {
            plain: answer.global.root_password.clone(),
            hashed: answer.global.root_password_hashed.clone(),
        },
        mailto: answer.global.mailto.clone(),
        root_ssh_keys: answer.global.root_ssh_keys.clone(),

        mngmt_nic: network_settings.ifname,

        hostname: network_settings.fqdn.host().unwrap().to_string(),
        domain: network_settings.fqdn.domain(),
        cidr: network_settings.address,
        gateway: network_settings.gateway,
        dns: network_settings.dns_server,
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
                arc_max: zfs.arc_max.unwrap_or(2048),
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
    Ok(config)
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LowLevelMessage {
    #[serde(rename = "message")]
    Info {
        message: String,
    },
    Error {
        message: String,
    },
    Prompt {
        query: String,
    },
    Finished {
        state: String,
        message: String,
    },
    Progress {
        ratio: f32,
        text: String,
    },
}
