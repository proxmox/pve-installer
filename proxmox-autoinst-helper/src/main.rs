use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use glob::Pattern;
use regex::Regex;
use serde::Serialize;
use std::{collections::BTreeMap, fs, io::Read, path::PathBuf, process::Command};

use proxmox_auto_installer::{
    answer::Answer,
    answer::FilterMatch,
    sysinfo,
    utils::{get_matched_udev_indexes, get_nic_list, get_single_udev_index},
};

/// This tool validates the format of an answer file. Additionally it can test match filters and
/// print information on the properties to match against for the current hardware.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
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
/// proxmox-autoinst-helper match --filter-match all disk 'ID_SERIAL_SHORT=*2222*' 'DEVNAME=*nvme*'
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
    let mut file = match fs::File::open(&args.path) {
        Ok(file) => file,
        Err(err) => bail!(
            "Opening answer file '{}' failed: {err}",
            args.path.display()
        ),
    };
    let mut contents = String::new();
    if let Err(err) = file.read_to_string(&mut contents) {
        bail!("Reading from file '{}' failed: {err}", args.path.display());
    }

    let answer: Answer = match toml::from_str(&contents) {
        Ok(answer) => {
            println!("The file was parsed successfully, no syntax errors found!");
            answer
        }
        Err(err) => bail!("Error parsing answer file: {err}"),
    };
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
