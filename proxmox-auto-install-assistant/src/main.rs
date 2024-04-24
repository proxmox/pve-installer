use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use glob::Pattern;
use regex::Regex;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use proxmox_auto_installer::{
    answer::Answer,
    answer::FilterMatch,
    sysinfo::SysInfo,
    utils::{
        get_matched_udev_indexes, get_nic_list, get_single_udev_index, AutoInstSettings,
        FetchAnswerFrom, HttpOptions,
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
    SystemInfo(CommandSystemInfo),
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
/// The behavior of how to fetch an answer file must be set with the '--fetch-from', parameter. The
/// answer file can be:{n}
/// * integrated into the ISO itself ('iso'){n}
/// * needs to be present in a partition / file-system with the label 'PROXMOX-AIS' (Proxmox
/// Automated Installer Source) ('partition'){n}
/// * get requested via an HTTP Post request ('http').
///
/// The URL for the HTTP mode can be defined for the ISO with the '--url' argument. If not present,
/// it will try to get a URL from a DHCP option (250, TXT) or by querying a DNS TXT record at
/// 'proxmox-auto-installer.{search domain}'.
///
/// The TLS certificate fingerprint can either be defined via the '--cert-fingerprint' argument or
/// alternatively via the custom DHCP option (251, TXT) or in a DNS TXT record located at
/// 'proxmox-auto-installer-cert-fingerprint.{search domain}'.
///
/// The latter options to provide the TLS fingerprint will only be used if the same method was used
/// to retrieve the URL. For example, the DNS TXT record for the fingerprint will only be used, if
/// no one was configured with the '--cert-fingerprint' parameter and if the URL was retrieved via
/// the DNS TXT record.
#[derive(Args, Debug)]
struct CommandPrepareISO {
    /// Path to the source ISO to prepare
    input: PathBuf,

    /// Path to store the final ISO to, defaults to an auto-generated file name depending on mode
    /// and the same directory as the source file is located in.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Where the automatic installer should fetch the answer file from.
    #[arg(long, value_enum)]
    fetch_from: FetchAnswerFrom,

    /// Include the specified answer file in the ISO. Requires the '--fetch-from'  parameter
    /// to be set to 'iso'.
    #[arg(long)]
    answer_file: Option<PathBuf>,

    /// Specify URL for fetching the answer file via HTTP
    #[arg(long)]
    url: Option<String>,

    /// Pin the ISO to the specified SHA256 TLS certificate fingerprint.
    #[arg(long)]
    cert_fingerprint: Option<String>,

    /// Staging directory to use for preparing the new ISO file. Defaults to the directory of the
    /// input ISO file.
    #[arg(long)]
    tmp: Option<String>,
}

/// Show the system information that can be used to identify a host.
///
/// The shown information is sent as POST HTTP request when fetching the answer file for the
/// automatic installation through HTTP, You can, for example, use this to return a dynamically
/// assembled answer file.
#[derive(Args, Debug)]
struct CommandSystemInfo {}

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
        Commands::SystemInfo(args) => show_system_info(args),
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
            get_matched_udev_indexes(&filters, &devs, args.filter_match == FilterMatch::All)
        }
        Devicetype::Network => get_single_udev_index(&filters, &devs).map(|r| vec![r]),
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

fn show_system_info(_args: &CommandSystemInfo) -> Result<()> {
    match SysInfo::as_json_pretty() {
        Ok(res) => println!("{res}"),
        Err(err) => eprintln!("Error fetching system info: {err}"),
    }
    Ok(())
}

fn prepare_iso(args: &CommandPrepareISO) -> Result<()> {
    check_prepare_requirements(args)?;

    if args.fetch_from == FetchAnswerFrom::Iso && args.answer_file.is_none() {
        bail!("Missing path to the answer file required for the fetch-from 'iso' mode.");
    }
    if args.url.is_some() && args.fetch_from != FetchAnswerFrom::Http {
        bail!(
            "Setting a URL is incompatible with the fetch-from '{:?}' mode, only works with the 'http' mode",
            args.fetch_from,
        );
    }
    if args.cert_fingerprint.is_some() && args.fetch_from != FetchAnswerFrom::Http {
        bail!(
            "Setting a certificate fingerprint incompatible is fetch-from '{:?}' mode, only works for 'http' mode.",
            args.fetch_from,
        );
    }
    if args.answer_file.is_some() && args.fetch_from != FetchAnswerFrom::Iso {
        bail!("Set '-i', '--install-mode' to 'included' to place the answer file directly in the ISO.");
    }

    if let Some(file) = &args.answer_file {
        println!("Checking provided answer file...");
        parse_answer(file)?;
    }

    let iso_target = final_iso_location(args);
    let iso_target_file_name = match iso_target.file_name() {
        None => bail!("no base filename in target ISO path found"),
        Some(source_file_name) => source_file_name.to_string_lossy(),
    };

    let mut tmp_base = PathBuf::new();
    match args.tmp.as_ref() {
        Some(tmp_dir) => tmp_base.push(tmp_dir),
        None => tmp_base.push(iso_target.parent().unwrap()),
    }

    let mut tmp_iso = tmp_base.clone();
    tmp_iso.push(format!("{iso_target_file_name}.tmp",));

    println!("Copying source ISO to temporary location...");
    fs::copy(&args.input, &tmp_iso)?;

    println!("Preparing ISO...");
    let config = AutoInstSettings {
        mode: args.fetch_from.clone(),
        http: HttpOptions {
            url: args.url.clone(),
            cert_fingerprint: args.cert_fingerprint.clone(),
        },
    };
    let mut instmode_file_tmp = tmp_base.clone();
    instmode_file_tmp.push("auto-installer-mode.toml");
    fs::write(&instmode_file_tmp, toml::to_string_pretty(&config)?)?;

    inject_file_to_iso(&tmp_iso, &instmode_file_tmp, "/auto-installer-mode.toml")?;

    if let Some(answer_file) = &args.answer_file {
        inject_file_to_iso(&tmp_iso, answer_file, "/answer.toml")?;
    }

    println!("Moving prepared ISO to target location...");
    fs::rename(&tmp_iso, &iso_target)?;
    println!("Final ISO is available at {iso_target:?}.");

    Ok(())
}

fn final_iso_location(args: &CommandPrepareISO) -> PathBuf {
    if let Some(specified) = args.output.clone() {
        return specified;
    }
    let mut suffix: String = match args.fetch_from {
        FetchAnswerFrom::Http => "auto-from-http",
        FetchAnswerFrom::Iso => "auto-from-iso",
        FetchAnswerFrom::Partition => "auto-from-partition",
    }
    .into();

    if args.url.is_some() {
        suffix.push_str("-url");
    }
    if args.cert_fingerprint.is_some() {
        suffix.push_str("-fp");
    }

    let base = args.input.parent().unwrap();
    let iso = args.input.file_stem().unwrap();

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
            "Error injecting {file:?} into {iso:?}: {}",
            String::from_utf8_lossy(&result.stderr)
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
    let re_props = Regex::new(r"(?m)^E: ([^=]+)=(.*)$")?;

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
        bail!("could not run udevadm successfully for {path:?}");
    }
    Ok(String::from_utf8(udev_output.stdout)?)
}

fn parse_answer(path: &PathBuf) -> Result<Answer> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) => bail!("Opening answer file {path:?} failed: {err}"),
    };
    let mut contents = String::new();
    if let Err(err) = file.read_to_string(&mut contents) {
        bail!("Reading from file {path:?} failed: {err}");
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
    match Path::try_exists(&args.input) {
        Ok(true) => (),
        Ok(false) => bail!("Source file does not exist."),
        Err(_) => bail!("Source file does not exist."),
    }

    match Command::new("xorriso")
        .arg("-dev")
        .arg(&args.input)
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
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            bail!("Could not find the 'xorriso' binary. Please install it.")
        }
        Err(err) => bail!("unexpected error when trying to execute 'xorriso' - {err}"),
    };

    Ok(())
}
