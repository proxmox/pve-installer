use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use glob::Pattern;
use regex::Regex;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use proxmox_auto_installer::{
    answer::Answer,
    answer::FilterMatch,
    sysinfo,
    utils::{
        get_matched_udev_indexes, get_nic_list, get_single_udev_index, AutoInstModes,
        AutoInstSettings,
    },
};

static PROXMOX_ISO_FLAG: &str = "/auto-installer-capable";

/// This tool can be used to prepare a Proxmox installation ISO for automated installations.
/// Additional uses are to validate the format of an answer file or to test match filters and
/// print information on the properties to match against for the current hardware.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    PrepareIso(CommandPrepareISO),
    ValidateAnswer(CommandValidateAnswer),
    DeviceMatch(CommandDeviceMatch),
    DeviceInfo(CommandDeviceInfo),
    Identifiers(CommandIdentifiers),
}

/// Show device information that can be used for filters
#[derive(Args, Debug)]
struct CommandDeviceInfo {
    /// For which device type information should be shown
    #[arg(name="type", short, long, value_enum, default_value_t=AllDeviceTypes::All)]
    device: AllDeviceTypes,
}

/// Test which devices the given filter matches against
///
/// Filters support the following syntax:
/// ?          Match a single character
/// *          Match any number of characters
/// [a], [0-9] Specifc character or range of characters
/// [!a]       Negate a specific character of range
///
/// To avoid globbing characters being interpreted by the shell, use single quotes.
/// Multiple filters can be defined.
///
/// Examples:
/// Match disks against the serial number and device name, both must match:
///
/// proxmox-auto-install-assistant match --filter-match all disk 'ID_SERIAL_SHORT=*2222*' 'DEVNAME=*nvme*'
#[derive(Args, Debug)]
#[command(verbatim_doc_comment)]
struct CommandDeviceMatch {
    /// Device type to match the filter against
    r#type: Devicetype,

    /// Filter in the format KEY=VALUE where the key is the UDEV key and VALUE the filter string.
    /// Multiple filters are possible, separated by a space.
    filter: Vec<String>,

    /// Defines if any filter or all filters must match.
    #[arg(long, value_enum, default_value_t=FilterMatch::Any)]
    filter_match: FilterMatch,
}

/// Validate if an answer file is formatted correctly.
#[derive(Args, Debug)]
struct CommandValidateAnswer {
    /// Path to the answer file
    path: PathBuf,
    #[arg(short, long, default_value_t = false)]
    debug: bool,
}

/// Prepare an ISO for automated installation.
///
/// The final ISO will try to fetch an answer file automatically. It will first search for a
/// partition / file-system called "PROXMOX-INST-SRC" (or lowercase) and a file in the root named
/// "answer.toml".
///
/// If that is not found, it will try to fetch an answer file via an HTTP Post request. The URL for
/// it can be defined for the ISO with the '--url', '-u' argument. If not present, it will try to
/// get a URL from a DHCP option (250, TXT) or by querying a DNS TXT record at
/// 'proxmox-auto-installer.{search domain}'.
///
/// The TLS certificate fingerprint can either be defined via the '--cert-fingerprint', '-c'
/// argument or alternatively via the custom DHCP option (251, TXT) or in a DNS TXT record located
/// at 'proxmox-auto-installer-cert-fingerprint.{search domain}'.
///
/// The latter options to provide the TLS fingerprint will only be used if the same method was used
/// to retrieve the URL. For example, the DNS TXT record for the fingerprint will only be used, if
/// no one was configured with the '--cert-fingerprint' parameter and if the URL was retrieved via
/// the DNS TXT record.
///
/// The behavior of how to fetch an answer file can be overridden with the '--install-mode', '-i'
/// parameter. The answer file can be{n}
/// * integrated into the ISO itself ('included'){n}
/// * needs to be present in a partition / file-system with the label 'PROXMOX-INST-SRC'
///   ('partition'){n}
/// * get requested via an HTTP Post request ('http').
#[derive(Args, Debug)]
struct CommandPrepareISO {
    /// Path to the source ISO
    source: PathBuf,

    /// Path to store the final ISO to.
    #[arg(short, long)]
    target: Option<PathBuf>,

    /// Where to fetch the answer file from.
    #[arg(short, long, value_enum, default_value_t=AutoInstModes::Auto)]
    install_mode: AutoInstModes,

    /// Include the specified answer file in the ISO. Requires the '--install-mode', '-i' parameter
    /// to be set to 'included'.
    #[arg(short, long)]
    answer_file: Option<PathBuf>,

    /// Specify URL for fetching the answer file via HTTP
    #[arg(short, long)]
    url: Option<String>,

    /// Pin the ISO to the specified SHA256 TLS certificate fingerprint.
    #[arg(short, long)]
    cert_fingerprint: Option<String>,

    /// Tmp directory to use.
    #[arg(long)]
    tmp: Option<String>,
}

/// Show identifiers for the current machine. This information is part of the POST request to fetch
/// an answer file.
#[derive(Args, Debug)]
struct CommandIdentifiers {}

#[derive(Args, Debug)]
struct GlobalOpts {
    /// Output format
    #[arg(long, short, value_enum)]
    format: OutputFormat,
}

#[derive(Clone, Debug, ValueEnum, PartialEq)]
enum AllDeviceTypes {
    All,
    Network,
    Disk,
}

#[derive(Clone, Debug, ValueEnum)]
enum Devicetype {
    Network,
    Disk,
}

#[derive(Clone, Debug, ValueEnum)]
enum OutputFormat {
    Pretty,
    Json,
}

#[derive(Serialize)]
struct Devs {
    disks: Option<BTreeMap<String, BTreeMap<String, String>>>,
    nics: Option<BTreeMap<String, BTreeMap<String, String>>>,
}

fn main() {
    let args = Cli::parse();
    let res = match &args.command {
        Commands::PrepareIso(args) => prepare_iso(args),
        Commands::ValidateAnswer(args) => validate_answer(args),
        Commands::DeviceInfo(args) => info(args),
        Commands::DeviceMatch(args) => match_filter(args),
        Commands::Identifiers(args) => show_identifiers(args),
    };
    if let Err(err) = res {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn info(args: &CommandDeviceInfo) -> Result<()> {
    let mut devs = Devs {
        disks: None,
        nics: None,
    };

    if args.device == AllDeviceTypes::Network || args.device == AllDeviceTypes::All {
        match get_nics() {
            Ok(res) => devs.nics = Some(res),
            Err(err) => bail!("Error getting NIC data: {err}"),
        }
    }
    if args.device == AllDeviceTypes::Disk || args.device == AllDeviceTypes::All {
        match get_disks() {
            Ok(res) => devs.disks = Some(res),
            Err(err) => bail!("Error getting disk data: {err}"),
        }
    }
    println!("{}", serde_json::to_string_pretty(&devs).unwrap());
    Ok(())
}

fn match_filter(args: &CommandDeviceMatch) -> Result<()> {
    let devs: BTreeMap<String, BTreeMap<String, String>> = match args.r#type {
        Devicetype::Disk => get_disks().unwrap(),
        Devicetype::Network => get_nics().unwrap(),
    };
    // parse filters

    let mut filters: BTreeMap<String, String> = BTreeMap::new();

    for f in &args.filter {
        match f.split_once('=') {
            Some((key, value)) => {
                if key.is_empty() || value.is_empty() {
                    bail!("Filter key or value is empty in filter: '{f}'");
                }
                filters.insert(String::from(key), String::from(value));
            }
            None => {
                bail!("Could not find separator '=' in filter: '{f}'");
            }
        }
    }

    // align return values
    let result = match args.r#type {
        Devicetype::Disk => {
            get_matched_udev_indexes(filters, &devs, args.filter_match == FilterMatch::All)
        }
        Devicetype::Network => get_single_udev_index(filters, &devs).map(|r| vec![r]),
    };

    match result {
        Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
        Err(err) => bail!("Error matching filters: {err}"),
    }
    Ok(())
}

fn validate_answer(args: &CommandValidateAnswer) -> Result<()> {
    let answer = parse_answer(&args.path)?;
    if args.debug {
        println!("Parsed data from answer file:\n{:#?}", answer);
    }
    Ok(())
}

fn show_identifiers(_args: &CommandIdentifiers) -> Result<()> {
    match sysinfo::get_sysinfo(true) {
        Ok(res) => println!("{res}"),
        Err(err) => eprintln!("Error fetching system identifiers: {err}"),
    }
    Ok(())
}

fn prepare_iso(args: &CommandPrepareISO) -> Result<()> {
    check_prepare_requirements(args)?;

    if args.install_mode == AutoInstModes::Included {
        if args.answer_file.is_none() {
            bail!("Missing path to answer file needed for 'direct' install mode.");
        }
        if args.cert_fingerprint.is_some() {
            bail!("No certificate fingerprint needed for direct install mode. Drop the parameter!");
        }
        if args.url.is_some() {
            bail!("No URL needed for direct install mode. Drop the parameter!");
        }
    } else if args.install_mode == AutoInstModes::Partition {
        if args.cert_fingerprint.is_some() {
            bail!(
                "No certificate fingerprint needed for partition install mode. Drop the parameter!"
            );
        }
        if args.url.is_some() {
            bail!("No URL needed for partition install mode. Drop the parameter!");
        }
    }
    if args.answer_file.is_some() && args.install_mode != AutoInstModes::Included {
        bail!("Set '-i', '--install-mode' to 'included' to place the answer file directly in the ISO.");
    }

    if let Some(file) = &args.answer_file {
        println!("Checking provided answer file...");
        parse_answer(file)?;
    }

    let mut tmp_base = PathBuf::new();
    if args.tmp.is_some() {
        tmp_base.push(args.tmp.as_ref().unwrap());
    } else {
        tmp_base.push(args.source.parent().unwrap());
        tmp_base.push(".proxmox-iso-prepare");
    }
    fs::create_dir_all(&tmp_base)?;

    let mut tmp_iso = tmp_base.clone();
    tmp_iso.push("proxmox.iso");
    let mut tmp_answer = tmp_base.clone();
    tmp_answer.push("answer.toml");

    println!("Copying source ISO to temporary location...");
    fs::copy(&args.source, &tmp_iso)?;
    println!("Done copying source ISO");

    println!("Preparing ISO...");
    let install_mode = AutoInstSettings {
        mode: args.install_mode.clone(),
        http_url: args.url.clone(),
        cert_fingerprint: args.cert_fingerprint.clone(),
    };
    let mut instmode_file_tmp = tmp_base.clone();
    instmode_file_tmp.push("auto-installer-mode.toml");
    fs::write(&instmode_file_tmp, toml::to_string_pretty(&install_mode)?)?;

    inject_file_to_iso(&tmp_iso, &instmode_file_tmp, "/auto-installer-mode.toml")?;

    if let Some(answer) = &args.answer_file {
        fs::copy(answer, &tmp_answer)?;
        inject_file_to_iso(&tmp_iso, &tmp_answer, "/answer.toml")?;
    }

    println!("Done preparing iso.");
    println!("Move ISO to target location...");
    let iso_target = final_iso_location(args);
    fs::rename(&tmp_iso, &iso_target)?;
    println!("Cleaning up...");
    fs::remove_dir_all(&tmp_base)?;
    println!("Final ISO is available at {}.", &iso_target.display());

    Ok(())
}

fn final_iso_location(args: &CommandPrepareISO) -> PathBuf {
    if let Some(specified) = args.target.clone() {
        return specified;
    }
    let mut suffix: String = match args.install_mode {
        AutoInstModes::Auto => "auto".into(),
        AutoInstModes::Http => "auto-http".into(),
        AutoInstModes::Included => "auto-answer-included".into(),
        AutoInstModes::Partition => "auto-part".into(),
    };

    if args.url.is_some() {
        suffix.push_str("-url");
    }
    if args.cert_fingerprint.is_some() {
        suffix.push_str("-fp");
    }

    let base = args.source.parent().unwrap();
    let iso = args.source.file_stem().unwrap();

    let mut target = base.to_path_buf();
    target.push(format!("{}-{}.iso", iso.to_str().unwrap(), suffix));

    target.to_path_buf()
}

fn inject_file_to_iso(iso: &PathBuf, file: &PathBuf, location: &str) -> Result<()> {
    let result = Command::new("xorriso")
        .arg("--boot_image")
        .arg("any")
        .arg("keep")
        .arg("-dev")
        .arg(iso)
        .arg("-map")
        .arg(file)
        .arg(location)
        .output()?;
    if !result.status.success() {
        bail!(
            "Error injecting {} into {}: {}",
            file.display(),
            iso.display(),
            String::from_utf8(result.stderr)?
        );
    }
    Ok(())
}

fn get_disks() -> Result<BTreeMap<String, BTreeMap<String, String>>> {
    let unwantend_block_devs = vec![
        "ram[0-9]*",
        "loop[0-9]*",
        "md[0-9]*",
        "dm-*",
        "fd[0-9]*",
        "sr[0-9]*",
    ];

    // compile Regex here once and not inside the loop
    let re_disk = Regex::new(r"(?m)^E: DEVTYPE=disk")?;
    let re_cdrom = Regex::new(r"(?m)^E: ID_CDROM")?;
    let re_iso9660 = Regex::new(r"(?m)^E: ID_FS_TYPE=iso9660")?;

    let re_name = Regex::new(r"(?m)^N: (.*)$")?;
    let re_props = Regex::new(r"(?m)^E: (.*)=(.*)$")?;

    let mut disks: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

    'outer: for entry in fs::read_dir("/sys/block")? {
        let entry = entry.unwrap();
        let filename = entry.file_name().into_string().unwrap();

        for p in &unwantend_block_devs {
            if Pattern::new(p)?.matches(&filename) {
                continue 'outer;
            }
        }

        let output = match get_udev_properties(&entry.path()) {
            Ok(output) => output,
            Err(err) => {
                eprint!("{err}");
                continue 'outer;
            }
        };

        if !re_disk.is_match(&output) {
            continue 'outer;
        };
        if re_cdrom.is_match(&output) {
            continue 'outer;
        };
        if re_iso9660.is_match(&output) {
            continue 'outer;
        };

        let mut name = filename;
        if let Some(cap) = re_name.captures(&output) {
            if let Some(res) = cap.get(1) {
                name = String::from(res.as_str());
            }
        }

        let mut udev_props: BTreeMap<String, String> = BTreeMap::new();

        for line in output.lines() {
            if let Some(caps) = re_props.captures(line) {
                let key = String::from(caps.get(1).unwrap().as_str());
                let value = String::from(caps.get(2).unwrap().as_str());
                udev_props.insert(key, value);
            }
        }

        disks.insert(name, udev_props);
    }
    Ok(disks)
}

fn get_nics() -> Result<BTreeMap<String, BTreeMap<String, String>>> {
    let re_props = Regex::new(r"(?m)^E: (.*)=(.*)$")?;
    let mut nics: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

    let links = get_nic_list()?;
    for link in links {
        let path = format!("/sys/class/net/{link}");

        let output = match get_udev_properties(&PathBuf::from(path)) {
            Ok(output) => output,
            Err(err) => {
                eprint!("{err}");
                continue;
            }
        };

        let mut udev_props: BTreeMap<String, String> = BTreeMap::new();

        for line in output.lines() {
            if let Some(caps) = re_props.captures(line) {
                let key = String::from(caps.get(1).unwrap().as_str());
                let value = String::from(caps.get(2).unwrap().as_str());
                udev_props.insert(key, value);
            }
        }

        nics.insert(link, udev_props);
    }
    Ok(nics)
}

fn get_udev_properties(path: &PathBuf) -> Result<String> {
    let udev_output = Command::new("udevadm")
        .arg("info")
        .arg("--path")
        .arg(path)
        .arg("--query")
        .arg("all")
        .output()?;
    if !udev_output.status.success() {
        bail!("could not run udevadm successfully for {}", path.display());
    }
    Ok(String::from_utf8(udev_output.stdout)?)
}

fn parse_answer(path: &PathBuf) -> Result<Answer> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) => bail!("Opening answer file '{}' failed: {err}", path.display()),
    };
    let mut contents = String::new();
    if let Err(err) = file.read_to_string(&mut contents) {
        bail!("Reading from file '{}' failed: {err}", path.display());
    }
    match toml::from_str(&contents) {
        Ok(answer) => {
            println!("The file was parsed successfully, no syntax errors found!");
            Ok(answer)
        }
        Err(err) => bail!("Error parsing answer file: {err}"),
    }
}

fn check_prepare_requirements(args: &CommandPrepareISO) -> Result<()> {
    match Path::try_exists(&args.source) {
        Ok(true) => (),
        Ok(false) => bail!("Source file does not exist."),
        Err(_) => bail!("Source file does not exist."),
    }

    match Command::new("xorriso")
        .arg("-dev")
        .arg(&args.source)
        .arg("-find")
        .arg(PROXMOX_ISO_FLAG)
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .status()
    {
        Ok(v) => {
            if !v.success() {
                bail!("The source ISO file is not able to be installed automatically. Please try a more current one.");
            }
        }
        Err(_) => bail!("Could not run 'xorriso'. Please install it."),
    };

    Ok(())
}
