use anyhow::{bail, Result};
use clap::ValueEnum;
use glob::Pattern;
use log::{debug, error, info};
use std::{
    collections::BTreeMap,
    process::{Command, Stdio},
};

use crate::{
    answer::{self, Answer},
    udevinfo::UdevInfo,
};
use proxmox_installer_common::{
    options::{FsType, NetworkOptions, ZfsChecksumOption, ZfsCompressOption},
    setup::{InstallConfig, InstallZfsOption, LocaleInfo, RuntimeInfo, SetupInfo},
};
use serde::{Deserialize, Serialize};

fn find_with_glob(pattern: &str, value: &str) -> Result<bool> {
    let p = Pattern::new(pattern)?;
    match p.matches(value) {
        true => Ok(true),
        false => Ok(false),
    }
}

pub fn get_network_settings(
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
        network_options.ifname = get_single_udev_index(settings.filter.clone(), &udev_info.nics)?;
    }
    info!("Network interface used is '{}'", &network_options.ifname);
    Ok(network_options)
}

pub fn get_single_udev_index(
    filter: BTreeMap<String, String>,
    udev_list: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<String> {
    if filter.is_empty() {
        bail!("no filter defined");
    }
    let mut dev_index: Option<String> = None;
    'outer: for (dev, dev_values) in udev_list {
        for (filter_key, filter_value) in &filter {
            for (udev_key, udev_value) in dev_values {
                if udev_key == filter_key && find_with_glob(filter_value, udev_value)? {
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
pub enum AutoInstModes {
    Auto,
    Included,
    Http,
    Partition,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub struct AutoInstSettings {
    pub mode: AutoInstModes,
    pub http_url: Option<String>,
    pub cert_fingerprint: Option<String>,
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
    filter: BTreeMap<String, String>,
    udev_list: &BTreeMap<String, BTreeMap<String, String>>,
    match_all: bool,
) -> Result<Vec<String>> {
    let mut matches = vec![];
    for (dev, dev_values) in udev_list {
        let mut did_match_once = false;
        let mut did_match_all = true;
        for (filter_key, filter_value) in &filter {
            for (udev_key, udev_value) in dev_values {
                if udev_key == filter_key && find_with_glob(filter_value, udev_value)? {
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

pub fn set_disks(
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
                Some(disk) => config.target_hd = Some(disk.clone()),
                None => bail!("disk in 'disk_selection' not found"),
            }
        }
        answer::DiskSelection::Filter(filter) => {
            let disk_index = get_single_udev_index(filter.clone(), &udev_info.disks)?;
            let disk = runtime_info
                .disks
                .iter()
                .find(|item| item.index == disk_index);
            config.target_hd = disk.cloned();
        }
    }
    info!("Selected disk: {}", config.target_hd.clone().unwrap().path);
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
            let disk_filters = filter.clone();
            let selected_disk_indexes = get_matched_udev_indexes(
                disk_filters,
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

pub fn get_first_selected_disk(config: &InstallConfig) -> usize {
    config
        .disk_selection
        .iter()
        .next()
        .expect("no disks found")
        .0
        .parse::<usize>()
        .expect("could not parse key to usize")
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
    if !locales.kmap.keys().any(|i| i == &answer.global.keyboard) {
        bail!("keyboard layout '{}' is not valid", &answer.global.keyboard);
    }
    if !locales
        .cczones
        .iter()
        .any(|(_, zones)| zones.contains(&answer.global.timezone))
    {
        bail!("timezone '{}' is not valid", &answer.global.timezone);
    }
    Ok(())
}

pub fn run_cmds(step: &str, in_chroot: bool, cmds: &[&str]) {
    let run = || {
        debug!("Running commands for '{step}':");
        for cmd in cmds {
            run_cmd(cmd)?;
        }
        Ok::<(), anyhow::Error>(())
    };

    if in_chroot {
        if let Err(err) = run_cmd("proxmox-chroot prepare") {
            error!("Failed to setup chroot for '{step}': {err}");
            return;
        }
    }

    if let Err(err) = run() {
        error!("Running commands for '{step}' failed: {err:?}");
    } else {
        debug!("Running commands in chroot for '{step}' finished");
    }

    if in_chroot {
        if let Err(err) = run_cmd("proxmox-chroot cleanup") {
            error!("Failed to clean up chroot for '{step}': {err}");
        }
    }
}

fn run_cmd(cmd: &str) -> Result<()> {
    debug!("Command '{cmd}':");
    let child = match Command::new("/bin/bash")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => bail!("error running command {cmd}: {err}"),
    };
    match child.wait_with_output() {
        Ok(output) => {
            if output.status.success() {
                debug!("{}", String::from_utf8(output.stdout).unwrap());
            } else {
                bail!("{}", String::from_utf8(output.stderr).unwrap());
            }
        }
        Err(err) => bail!("{err}"),
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
    info!("Setting File system");
    let filesystem = answer.disks.fs_type;
    info!("File system selected: {}", filesystem);

    let network_settings = get_network_settings(answer, udev_info, runtime_info, setup_info)?;

    verify_locale_settings(answer, locales)?;

    let mut config = InstallConfig {
        autoreboot: 0,
        filesys: filesystem,
        hdsize: 0.,
        swapsize: None,
        maxroot: None,
        minfree: None,
        maxvz: None,
        zfs_opts: None,
        target_hd: None,
        disk_selection: BTreeMap::new(),
        lvm_auto_rename: 1,

        country: answer.global.country.clone(),
        timezone: answer.global.timezone.clone(),
        keymap: answer.global.keyboard.clone(),

        password: answer.global.root_password.clone(),
        mailto: answer.global.mailto.clone(),

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
            config.hdsize = lvm.hdsize.unwrap_or(config.target_hd.clone().unwrap().size);
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
        }
    }

    // never print the auto reboot text after finishing to avoid the delay, as this is handled by
    // the auto-installer itself anyway. The auto-installer might still perform some post-install
    // steps after running the low-level installer.
    config.autoreboot = 0;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_glob_patterns() {
        let test_value = "foobar";
        assert_eq!(find_with_glob("*bar", test_value).unwrap(), true);
        assert_eq!(find_with_glob("foo*", test_value).unwrap(), true);
        assert_eq!(find_with_glob("foobar", test_value).unwrap(), true);
        assert_eq!(find_with_glob("oobar", test_value).unwrap(), false);
        assert_eq!(find_with_glob("f*bar", test_value).unwrap(), true);
        assert_eq!(find_with_glob("f?bar", test_value).unwrap(), false);
        assert_eq!(find_with_glob("fo?bar", test_value).unwrap(), true);
        assert_eq!(find_with_glob("f[!a]obar", test_value).unwrap(), true);
        assert_eq!(find_with_glob("f[oa]obar", test_value).unwrap(), true);
    }
}
