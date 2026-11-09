use anyhow::{Context, Result, anyhow, bail};
use glob::Pattern;
use log::{info, warn};
use std::{
    collections::{BTreeMap, HashSet},
    process::Command,
};

use proxmox_installer_types::{
    ProxmoxProduct, UdevInfo,
    answer::{
        AutoInstallerConfig, DiskSelection, Filesystem, FilesystemOptions, FilesystemType,
        FilterMatch, FirstBootHookSourceMode, FqdnConfig, FqdnFromDhcpConfig, FqdnSourceMode,
        NetworkBondOptions, NetworkConfig,
    },
};

use proxmox_installer_common::{
    ROOT_PASSWORD_MIN_LENGTH,
    disk_checks::check_swapsize,
    options::{
        FilesystemDiskInfo, NetworkInterfacePinningOptions, NetworkOptions, email_validate,
        is_bond_mode_unsuitable_for_bridge,
    },
    setup::{
        InstallBtrfsOption, InstallConfig, InstallFirstBootSetup, InstallRootPassword,
        InstallZfsOption, LocaleInfo, NetworkInfo, RuntimeInfo, SetupInfo,
    },
};
use proxmox_network_types::MacAddress;

use serde::{Deserialize, Serialize};

fn get_network_settings(
    answer: &AutoInstallerConfig,
    udev_info: &UdevInfo,
    runtime_info: &RuntimeInfo,
    setup_info: &SetupInfo,
) -> Result<NetworkOptions> {
    info!("Setting up network configuration");

    let pinning_opts = answer
        .network
        .interface_name_pinning()
        .filter(|answer| answer.enabled)
        .map(|answer| answer.into());

    let bond_opts = answer
        .network
        .bond()
        .map(|opts| resolve_bond_options(opts, udev_info, &runtime_info.network))
        .transpose()?;

    let mut network_options = match &answer.global.fqdn.fqdn {
        // If the user set a static FQDN in the answer file, override it
        FqdnConfig::Simple(name) => {
            let mut opts = NetworkOptions::defaults_from(
                setup_info,
                &runtime_info.network,
                None,
                pinning_opts.as_ref(),
                bond_opts.as_ref(),
            );
            opts.fqdn = name.to_owned();
            opts
        }
        FqdnConfig::FromDhcp(FqdnFromDhcpConfig {
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
                pinning_opts.as_ref(),
                bond_opts.as_ref(),
            )
        }
    };

    if let NetworkConfig::FromAnswer(settings) = &answer.network {
        network_options.address = settings.cidr;
        network_options.dns_server = settings.dns;
        network_options.gateway = settings.gateway;
        network_options.vlan_opts = settings.vlan.clone();

        let vlan_id = settings.vlan.as_ref().map(|opts| opts.vid);
        if let Some(live_vlan_id) = runtime_info.network.vlan_id
            && vlan_id != Some(live_vlan_id)
        {
            warn!(
                "The installation environment uses VLAN {live_vlan_id}, but the answer file sets {}",
                vlan_id.map_or("no VLAN tag".to_owned(), |id| format!("VLAN {id}"))
            );
        }

        if network_options.bond_opts.is_none() {
            let ifname = get_single_udev_index(&settings.filter, &udev_info.nics)?;
            // the post-installation hook identifies the management interface by its final name
            network_options.ifname = network_options
                .pinning_opts
                .as_ref()
                .zip(runtime_info.network.interfaces.get(&ifname))
                .and_then(|(opts, iface)| iface.to_pinned(opts))
                .map_or(ifname, |iface| iface.name);
        } else if !settings.filter.is_empty() {
            warn!("Ignoring network interface filter, the management interface is the bond");
        }
    }

    // the lease may be from a network other than the bond's
    if let NetworkConfig::FromDhcp(_) = &answer.network
        && let Some(bond_opts) = &network_options.bond_opts
        && !bond_opts
            .members
            .iter()
            .filter_map(|mac| runtime_info.network.physical_interface_by_mac(mac))
            .any(|iface| iface.addresses.contains(&network_options.address))
    {
        bail!(
            "address {} from DHCP was not received on a bond member, set the network configuration with source \"from-answer\" instead",
            network_options.address
        );
    }

    if let Some(opts) = &network_options.pinning_opts {
        info!("Network interface name pinning is enabled");
        opts.verify()?;
    }

    if let Some(opts) = &network_options.bond_opts {
        let members = opts
            .members
            .iter()
            .map(|mac| {
                runtime_info
                    .network
                    .physical_interface_by_mac(mac)
                    .map_or_else(
                        || "<unknown>".to_owned(),
                        |iface| iface.installed_name(network_options.pinning_opts.as_ref()),
                    )
            })
            .collect::<Vec<_>>()
            .join(", ");

        info!("Network interface bonding enabled on: {members}");

        if setup_info.config.product == ProxmoxProduct::Pve
            && is_bond_mode_unsuitable_for_bridge(opts.mode)
        {
            warn!(
                "Bond mode '{}' is not suitable if guests use the bridge on top of the bond",
                opts.mode
            );
        }
    }

    if let Some(opts) = &network_options.vlan_opts {
        info!("Management interface reachable on VLAN {}", opts.vid);
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub struct AutoInstSettings {
    pub mode: FetchAnswerFrom,
    #[serde(default = "default_partition_label")]
    pub partition_label: String,
    #[serde(default)]
    pub http: HttpOptions,
    /// VLAN tag of the network the installation environment uses, if tagged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan_id: Option<u16>,
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
        if link.ifname == *"lo" || is_live_vlan_link(&link.ifname) {
            continue;
        }
        links.push(link.ifname);
    }

    Ok(links)
}

/// Returns whether the link is one of the VLAN links that `unconfigured.sh` sets up for a tagged
/// installation environment, named `live<ifindex>.<tag>`.
fn is_live_vlan_link(name: &str) -> bool {
    let is_number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());

    name.strip_prefix("live")
        .and_then(|rest| rest.split_once('.'))
        .is_some_and(|(index, tag)| is_number(index) && is_number(tag))
}

/// Returns the bond options of the answer with the members selected by its filter, if it has one,
/// and checks that all members are physical network interfaces.
fn resolve_bond_options(
    opts: &NetworkBondOptions,
    udev_info: &UdevInfo,
    network: &NetworkInfo,
) -> Result<NetworkBondOptions> {
    opts.verify()?;

    let mut opts = opts.clone();
    if !opts.filter.is_empty() {
        opts.members = get_bond_members_by_filter(&opts, udev_info, network)?;
        opts.filter.clear();
        opts.filter_match = None;
        opts.verify()?;
    }

    for mac in &opts.members {
        if network.physical_interface_by_mac(mac).is_none() {
            bail!("bond member {mac:x} does not match any physical network interface");
        }
    }

    Ok(opts)
}

/// Returns the MAC addresses of the physical network interfaces the filter of the bond options
/// selects.
fn get_bond_members_by_filter(
    opts: &NetworkBondOptions,
    udev_info: &UdevInfo,
    network: &NetworkInfo,
) -> Result<Vec<MacAddress>> {
    let match_all = opts.filter_match.unwrap_or_default() == FilterMatch::All;
    let names = get_matched_udev_indexes(&opts.filter, &udev_info.nics, match_all)
        .map_err(|err| anyhow!("cannot select bond members - {err:#}"))?;

    let mut members = Vec::new();
    let mut member_names = Vec::new();
    for name in names {
        if let Some(iface) = network.interfaces.get(&name)
            && network
                .physical_interface_by_mac(&iface.mac)
                .is_some_and(|member| member.name == name)
        {
            members.push(iface.mac);
            member_names.push(name);
        }
    }

    if members.len() < 2 {
        bail!(
            "bond member filter must match at least 2 physical network interfaces, matched: {}",
            if member_names.is_empty() {
                "none".to_owned()
            } else {
                member_names.join(", ")
            }
        );
    }

    Ok(members)
}

pub fn get_matched_udev_indexes(
    filter: &BTreeMap<String, String>,
    udev_list: &BTreeMap<String, BTreeMap<String, String>>,
    match_all: bool,
) -> Result<Vec<String>> {
    let patterns = filter
        .iter()
        .map(|(key, value)| {
            let pattern =
                Pattern::new(value).with_context(|| format!("invalid glob '{value}' for {key}"))?;
            Ok((key, pattern))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut matches = vec![];
    for (dev, dev_values) in udev_list {
        let mut results = patterns.iter().map(|(key, pattern)| {
            dev_values
                .get(*key)
                .is_some_and(|value| pattern.matches(value))
        });
        let is_match = if match_all {
            results.all(|matched| matched)
        } else {
            results.any(|matched| matched)
        };
        if is_match {
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
    answer: &AutoInstallerConfig,
    udev_info: &UdevInfo,
    runtime_info: &RuntimeInfo,
    config: &mut InstallConfig,
) -> Result<()> {
    match config.filesys {
        FilesystemType::Ext4 | FilesystemType::Xfs => {
            set_single_disk(answer, udev_info, runtime_info, config)
        }
        FilesystemType::Zfs(_) | FilesystemType::Btrfs(_) => {
            set_selected_disks(answer, udev_info, runtime_info, config)
        }
    }
}

fn set_single_disk(
    answer: &AutoInstallerConfig,
    udev_info: &UdevInfo,
    runtime_info: &RuntimeInfo,
    config: &mut InstallConfig,
) -> Result<()> {
    match answer.disks.disk_selection()? {
        DiskSelection::Selection(disk_list) => {
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
        DiskSelection::Filter(filter) => {
            let disk_index = get_single_udev_index(&filter, &udev_info.disks)?;
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
    answer: &AutoInstallerConfig,
    udev_info: &UdevInfo,
    runtime_info: &RuntimeInfo,
    config: &mut InstallConfig,
) -> Result<()> {
    match answer.disks.disk_selection()? {
        DiskSelection::Selection(disk_list) => {
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
        DiskSelection::Filter(filter) => {
            info!("No disk list found, looking for disk filters");
            let filter_match = answer.disks.filter_match.unwrap_or_default();
            let selected_disk_indexes = get_matched_udev_indexes(
                &filter,
                &udev_info.disks,
                filter_match == FilterMatch::All,
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

fn verify_filesystem_settings(
    answer: &AutoInstallerConfig,
    setup_info: &SetupInfo,
) -> Result<FilesystemOptions> {
    info!("Verifying filesystem settings");

    let fs_options = answer.disks.filesystem_details()?;
    if answer.disks.filesystem == Filesystem::Btrfs && !setup_info.config.enable_btrfs {
        bail!(
            "BTRFS is not supported as a root filesystem for the product or the release of this ISO."
        );
    }

    Ok(fs_options)
}

pub fn verify_locale_settings(answer: &AutoInstallerConfig, locales: &LocaleInfo) -> Result<()> {
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
pub fn verify_email_and_root_password_settings(answer: &AutoInstallerConfig) -> Result<()> {
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

pub fn verify_disks_settings(answer: &AutoInstallerConfig) -> Result<()> {
    let fs_options = answer.disks.filesystem_details()?;

    if let DiskSelection::Selection(selection) = answer.disks.disk_selection()? {
        let min_disks = fs_options.to_type().get_min_disks();

        if selection.len() < min_disks {
            bail!(
                "{}: need at least {} disks",
                fs_options.to_type(),
                min_disks
            );
        }

        let mut disk_set = HashSet::new();
        for disk in &selection {
            if !disk_set.insert(disk) {
                bail!("List of disks contains duplicate device {disk}");
            }
        }
    }

    match fs_options {
        FilesystemOptions::Ext4(lvm) | FilesystemOptions::Xfs(lvm) => {
            if let Some((swapsize, hdsize)) = lvm.swapsize.zip(lvm.hdsize) {
                check_swapsize(swapsize, hdsize)?;
            }
        }
        _ => {}
    }

    Ok(())
}

pub fn verify_first_boot_settings(answer: &AutoInstallerConfig) -> Result<()> {
    info!("Verifying first boot settings");

    if let Some(first_boot) = &answer.first_boot
        && first_boot.source == FirstBootHookSourceMode::FromUrl
        && first_boot.url.is_none()
    {
        bail!("first-boot executable source set to URL, but none specified!");
    }

    Ok(())
}

pub fn verify_network_settings(
    network: &NetworkConfig,
    run_env: Option<&RuntimeInfo>,
) -> Result<()> {
    info!("Verifying network settings");

    let pin_opts: Option<NetworkInterfacePinningOptions> =
        network.interface_name_pinning().map(|v| v.into());

    if let Some(pin_opts) = pin_opts {
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
                        "found unknown address '{mac:x}' (mapped to '{name}') in network interface pinning options"
                    );
                }
            }
        }
    }

    if let Some(opts) = network.bond() {
        opts.verify()?;
    }

    if let Some(opts) = network.vlan() {
        opts.verify()?;
    }

    Ok(())
}

/// Parses the TOML of an answer file. Errors give the position, but leave out the excerpt of the
/// source that TOML errors usually include, as it may show a password or passphrase, and the value
/// of the wrong type or out of range that serde names, for example a passphrase given in place of
/// a section.
pub fn parse_answer_toml<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T> {
    toml::from_str(raw).map_err(|err| {
        let message = without_rejected_value(err.message().trim_end());
        match err.span().and_then(|span| raw.get(..span.start)) {
            Some(before) => {
                let line = before.matches('\n').count() + 1;
                let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
                anyhow!("line {line}, column {column}: {message}")
            }
            None => anyhow!("{message}"),
        }
    })
}

/// Turns, for example, "invalid type: string \"secret\", expected u32" into "invalid type: string,
/// expected u32". serde quotes the value with backticks, or double quotes for strings.
fn without_rejected_value(message: &str) -> String {
    for prefix in ["invalid type: ", "invalid value: "] {
        if let Some(rest) = message.strip_prefix(prefix)
            // the last one, as the value itself may contain the separator
            && let Some((unexpected, expected)) = rest.rsplit_once(", expected ")
        {
            let kind = unexpected.split(['`', '"']).next().unwrap_or("").trim_end();
            return format!("{prefix}{kind}, expected {expected}");
        }
    }
    message.to_string()
}

pub fn parse_answer(
    answer: &AutoInstallerConfig,
    udev_info: &UdevInfo,
    runtime_info: &RuntimeInfo,
    locales: &LocaleInfo,
    setup_info: &SetupInfo,
) -> Result<InstallConfig> {
    info!("Parsing answer file");

    let fs_options = verify_filesystem_settings(answer, setup_info)?;

    info!("Setting File system");
    info!("File system selected: {}", fs_options.to_type());

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
        filesys: fs_options.to_type(),
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
        subscription_key: answer.global.subscription_key.clone(),

        mngmt_nic: network_settings.ifname,
        mngmt_bond: network_settings.bond_opts.as_ref().map(Into::into),
        mngmt_vlan: network_settings.vlan_opts.as_ref().map(Into::into),
        network_interface_pin_map: network_settings
            .pinning_opts
            .map(|o| o.mapping)
            .unwrap_or_default(),

        hostname: network_settings.fqdn.host().to_owned(),
        domain: network_settings.fqdn.domain(),
        cidr: network_settings.address,
        gateway: network_settings.gateway,
        dns: network_settings.dns_server,

        first_boot: InstallFirstBootSetup::default(),
    };

    set_disks(answer, udev_info, runtime_info, &mut config)?;
    match fs_options {
        FilesystemOptions::Ext4(lvm) | FilesystemOptions::Xfs(lvm) => {
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
        FilesystemOptions::Zfs(zfs) => {
            let first_selected_disk = get_first_selected_disk(&config);

            config.hdsize = zfs
                .hdsize
                .unwrap_or(runtime_info.disks[first_selected_disk].size);
            config.zfs_opts = Some(InstallZfsOption {
                ashift: zfs.ashift.unwrap_or(12) as usize,
                arc_max: zfs
                    .arc_max
                    .map(|v| v as usize)
                    .unwrap_or(runtime_info.default_zfs_arc_max),
                compress: zfs.compress.unwrap_or_default(),
                checksum: zfs.checksum.unwrap_or_default(),
                copies: zfs.copies.unwrap_or(1) as usize,
            });
        }
        FilesystemOptions::Btrfs(btrfs) => {
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

#[cfg(test)]
mod tests {
    use super::{
        AutoInstSettings, FetchAnswerFrom, HttpOptions, is_live_vlan_link, parse_answer_toml,
    };
    use proxmox_installer_types::answer::AutoInstallerConfig;

    #[test]
    fn answer_parse_errors_leave_out_the_source() {
        let raw = "[global]\nroot-password = \"secret-value\" trailing\n";
        assert!(
            toml::from_str::<toml::Value>(raw)
                .unwrap_err()
                .to_string()
                .contains("secret-value")
        );

        let err = parse_answer_toml::<toml::Value>(raw).unwrap_err();
        assert!(err.to_string().starts_with("line 2, column 32: "), "{err}");
        assert!(!format!("{err:?}").contains("secret-value"), "{err:?}");

        let err = parse_answer_toml::<AutoInstallerConfig>(raw).unwrap_err();
        assert!(!format!("{err:?}").contains("secret-value"), "{err:?}");

        let raw = "[global]\nroot-password = 12345678\n";
        let err = parse_answer_toml::<AutoInstallerConfig>(raw).unwrap_err();
        assert!(err.to_string().starts_with("line "), "{err}");
        assert!(!err.to_string().contains("12345678"), "{err}");

        #[derive(Debug, serde::Deserialize)]
        struct Typed {
            #[allow(dead_code)]
            number: u8,
        }
        let err = parse_answer_toml::<Typed>("number = \"secret, expected x\"\n").unwrap_err();
        assert_eq!(
            err.to_string(),
            "line 1, column 10: invalid type: string, expected u8"
        );
        let err = parse_answer_toml::<Typed>("number = 4321\n").unwrap_err();
        assert!(!err.to_string().contains("4321"), "{err}");
        assert!(
            err.to_string()
                .contains("invalid value: integer, expected u8"),
            "{err}"
        );
    }

    #[test]
    fn auto_inst_settings_leave_out_an_unset_vlan_tag() {
        // older ISOs reject unknown keys in these settings
        let mut settings = AutoInstSettings {
            mode: FetchAnswerFrom::Iso,
            partition_label: "proxmox-ais".to_owned(),
            http: HttpOptions::default(),
            vlan_id: None,
        };
        assert!(!toml::to_string_pretty(&settings).unwrap().contains("vlan"));

        settings.vlan_id = Some(100);
        assert!(
            toml::to_string_pretty(&settings)
                .unwrap()
                .contains("vlan_id = 100")
        );
    }

    #[test]
    fn live_vlan_links_are_recognized() {
        assert!(is_live_vlan_link("live2.100"));
        assert!(is_live_vlan_link("live12.4094"));

        for name in [
            "eno1",
            "live",
            "live2",
            "live2.",
            "live.100",
            "livex.100",
            "vlan100",
        ] {
            assert!(!is_live_vlan_link(name), "{name}");
        }
    }
}
