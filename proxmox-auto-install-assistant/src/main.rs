//! This tool can be used to prepare a Proxmox installation ISO for automated installations.
//! Additional uses are to validate the format of an answer file or to test match filters and print
//! information on the properties to match against for the current hardware.

#![forbid(unsafe_code)]

use anyhow::{Context, Result, bail, format_err};
use glob::Pattern;
use std::{
    collections::BTreeMap,
    fmt, fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    str::FromStr,
};

use proxmox_auto_installer::{
    answer::{Answer, FilterMatch},
    sysinfo::SysInfo,
    utils::{
        AutoInstSettings, FetchAnswerFrom, HttpOptions, default_partition_label,
        get_matched_udev_indexes, get_nic_list, get_single_udev_index, verify_disks_settings,
        verify_email_and_root_password_settings, verify_first_boot_settings,
        verify_locale_settings,
    },
};
use proxmox_installer_common::{FIRST_BOOT_EXEC_MAX_SIZE, FIRST_BOOT_EXEC_NAME, cli};

static PROXMOX_ISO_FLAG: &str = "/auto-installer-capable";

/// Locale information as raw JSON, can be parsed into a
/// [LocaleInfo](`proxmox_installer_common::setup::LocaleInfo`) struct.
const LOCALE_INFO: &str = include_str!("../../locale-info.json");

/// Arguments for the `device-info` command.
struct CommandDeviceInfoArgs {
    /// Device type for which information should be shown.
    device_type: AllDeviceTypes,
}

impl cli::Subcommand for CommandDeviceInfoArgs {
    fn parse(args: &mut cli::Arguments) -> Result<Self> {
        Ok(Self {
            device_type: args
                .opt_value_from_str(["-t", "--type"])?
                .unwrap_or(AllDeviceTypes::All),
        })
    }

    fn print_usage() {
        eprintln!(
            r#"Show device information that can be used for filters.

USAGE:
  {} device-info [OPTIONS]

OPTIONS:
  -t, --type <type>  For which device type information should be shown [default: all] [possible values: all, network, disk]
  -h, --help         Print this help
  -V, --version      Print version
    "#,
            env!("CARGO_PKG_NAME")
        );
    }

    fn run(&self) -> Result<()> {
        info(self)
    }
}

/// Arguments for the `device-match` command.
struct CommandDeviceMatchArgs {
    /// Device type to match the filter against.
    device_type: DeviceType,

    /// Filter in the format KEY=VALUE where the key is the UDEV key and VALUE the filter string.
    /// Multiple filters are possible, separated by a space.
    filter: Vec<String>,

    /// Defines if any filter or all filters must match.
    filter_match: FilterMatch,
}

impl cli::Subcommand for CommandDeviceMatchArgs {
    fn parse(args: &mut cli::Arguments) -> Result<Self> {
        let filter_match = args
            .opt_value_from_str("--filter-match")?
            .unwrap_or(FilterMatch::Any);

        let device_type = args.free_from_str().context("parsing device type")?;
        let mut filter = vec![];
        while let Some(s) = args.opt_free_from_str()? {
            filter.push(s);
        }

        Ok(Self {
            device_type,
            filter,
            filter_match,
        })
    }

    fn print_usage() {
        eprintln!(
            r#"Test which devices the given filter matches against.

Filters support the following syntax:
- `?`               Match a single character
- `*`               Match any number of characters
- `[a]`, `[0-9]`  Specific character or range of characters
- `[!a]`          Negate a specific character of range

To avoid globbing characters being interpreted by the shell, use single quotes.
Multiple filters can be defined.

Examples:
Match disks against the serial number and device name, both must match:

$ proxmox-auto-install-assistant match --filter-match all disk 'ID_SERIAL_SHORT=*2222*' 'DEVNAME=*nvme*'

USAGE:
  {} device-match [OPTIONS] <TYPE> [FILTER]...

ARGUMENTS:
  <TYPE>
          Device type to match the filter against

          [possible values: network, disk]

  [FILTER]...
          Filter in the format KEY=VALUE where the key is the UDEV key and VALUE the filter string. Multiple filters are possible, separated by a space.

OPTIONS:
      --filter-match <FILTER_MATCH>
          Defines if any filter or all filters must match [default: any] [possible values: any, all]

  -h, --help         Print this help
  -V, --version      Print version
    "#,
            env!("CARGO_PKG_NAME")
        );
    }

    fn run(&self) -> Result<()> {
        match_filter(self)
    }
}

/// Arguments for the `validate-answer` command.
struct CommandValidateAnswerArgs {
    /// Path to the answer file.
    path: PathBuf,
    /// Whether to also show the full answer as parsed.
    debug: bool,
}

impl cli::Subcommand for CommandValidateAnswerArgs {
    fn parse(args: &mut cli::Arguments) -> Result<Self> {
        Ok(Self {
            debug: args.contains(["-d", "--debug"]),
            // Needs to be last
            path: args.free_from_str()?,
        })
    }

    fn print_usage() {
        eprintln!(
            r#"Validate if an answer file is formatted correctly.

USAGE:
  {} validate-answer [OPTIONS] <PATH>

ARGUMENTS:
  <PATH>  Path to the answer file.

OPTIONS:
  -d, --debug        Also show the full answer as parsed.
  -h, --help         Print this help
  -V, --version      Print version
    "#,
            env!("CARGO_PKG_NAME")
        );
    }

    fn run(&self) -> Result<()> {
        validate_answer(self)
    }
}

/// Arguments for the `prepare-iso` command.
struct CommandPrepareISOArgs {
    /// Path to the source ISO to prepare.
    input: PathBuf,

    /// Path to store the final ISO to, defaults to an auto-generated file name depending on mode
    /// and the same directory as the source file is located in.
    output: Option<PathBuf>,

    /// Where the automatic installer should fetch the answer file from.
    fetch_from: FetchAnswerFrom,

    /// Include the specified answer file in the ISO. Requires the '--fetch-from'  parameter
    /// to be set to 'iso'.
    answer_file: Option<PathBuf>,

    /// Specify URL for fetching the answer file via HTTP.
    url: Option<String>,

    /// Pin the ISO to the specified SHA256 TLS certificate fingerprint.
    cert_fingerprint: Option<String>,

    /// Staging directory to use for preparing the new ISO file. Defaults to the directory of the
    /// input ISO file.
    tmp: Option<String>,

    /// Can be used in combination with `--fetch-from partition` to set the partition label
    /// the auto-installer will search for.
    // FAT can only handle 11 characters (per specification at least, drivers might allow more),
    // so shorten "Automated Installer Source" to "AIS" to be safe.
    partition_label: String,

    /// Executable file to include, which should be run on the first system boot after the
    /// installation. Can be used for further bootstrapping the new system.
    ///
    /// Must be appropriately enabled in the answer file.
    on_first_boot: Option<PathBuf>,
}

impl cli::Subcommand for CommandPrepareISOArgs {
    fn parse(args: &mut cli::Arguments) -> Result<Self> {
        Ok(Self {
            output: args.opt_value_from_str("--output")?,
            fetch_from: args.value_from_str("--fetch-from")?,
            answer_file: args.opt_value_from_str("--answer-file")?,
            url: args.opt_value_from_str("--url")?,
            cert_fingerprint: args.opt_value_from_str("--cert-fingerprint")?,
            tmp: args.opt_value_from_str("--tmp")?,
            partition_label: args
                .opt_value_from_str("--partition-label")?
                .unwrap_or_else(default_partition_label),
            on_first_boot: args.opt_value_from_str("--on-first-boot")?,
            // Needs to be last
            input: args.free_from_str()?,
        })
    }

    fn print_usage() {
        eprintln!(
            r#"Prepare an ISO for automated installation.

The behavior of how to fetch an answer file must be set with the '--fetch-from' parameter.
The answer file can be:
 * integrated into the ISO itself ('iso')
 * present on a partition / file-system, matched by its label ('partition')
 * requested via an HTTP Post request ('http').

The URL for the HTTP mode can be defined for the ISO with the '--url' argument. If not present, it
will try to get a URL from a DHCP option (250, TXT) or by querying a DNS TXT record for the domain
'proxmox-auto-installer.{{search domain}}'.

The TLS certificate fingerprint can either be defined via the '--cert-fingerprint' argument or
alternatively via the custom DHCP option (251, TXT) or in a DNS TXT record located at
'proxmox-auto-installer-cert-fingerprint.{{search domain}}'.

The latter options to provide the TLS fingerprint will only be used if the same method was used to
retrieve the URL. For example, the DNS TXT record for the fingerprint will only be used, if no one
was configured with the '--cert-fingerprint' parameter and if the URL was retrieved via the DNS TXT
record.

If the 'partition' mode is used, the '--partition-label' parameter can be used to set the partition
label the auto-installer should search for. This defaults to 'proxmox-ais'.

USAGE:
  {} prepare-iso [OPTIONS] --fetch-from <FETCH_FROM> <INPUT>

ARGUMENTS:
  <INPUT>
          Path to the source ISO to prepare

OPTIONS:
      --output <OUTPUT>
          Path to store the final ISO to, defaults to an auto-generated file name depending on mode
          and the same directory as the source file is located in.

      --fetch-from <FETCH_FROM>
          Where the automatic installer should fetch the answer file from.

          [possible values: iso, http, partition]

      --answer-file <ANSWER_FILE>
          Include the specified answer file in the ISO. Requires the '--fetch-from' parameter to
          be set to 'iso'.

      --url <URL>
          Specify URL for fetching the answer file via HTTP.

      --cert-fingerprint <CERT_FINGERPRINT>
          Pin the ISO to the specified SHA256 TLS certificate fingerprint.

      --tmp <TMP>
          Staging directory to use for preparing the new ISO file. Defaults to the directory of the
          input ISO file.

      --partition-label <PARTITION_LABEL>
          Can be used in combination with `--fetch-from partition` to set the partition label the
          auto-installer will search for.

          [default: proxmox-ais]

      --on-first-boot <ON_FIRST_BOOT>
          Executable file to include, which should be run on the first system boot after the
          installation. Can be used for further bootstrapping the new system.

          Must be appropriately enabled in the answer file.

  -h, --help         Print this help
  -V, --version      Print version
    "#,
            env!("CARGO_PKG_NAME")
        );
    }

    fn run(&self) -> Result<()> {
        prepare_iso(self)
    }
}

/// Arguments for the `system-info` command.
struct CommandSystemInfoArgs;

impl cli::Subcommand for CommandSystemInfoArgs {
    fn parse(_: &mut cli::Arguments) -> Result<Self> {
        Ok(Self)
    }

    fn print_usage() {
        eprintln!(
            r#"Show the system information that can be used to identify a host.

The shown information is sent as POST HTTP request when fetching the answer file for the
automatic installation through HTTP, You can, for example, use this to return a dynamically
assembled answer file.

USAGE:
  {} system-info [OPTIONS]

OPTIONS:
  -h, --help         Print this help
  -V, --version      Print version
    "#,
            env!("CARGO_PKG_NAME")
        );
    }

    fn run(&self) -> Result<()> {
        show_system_info(self)
    }
}

#[derive(PartialEq)]
enum AllDeviceTypes {
    All,
    Network,
    Disk,
}

impl FromStr for AllDeviceTypes {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_ref() {
            "all" => Ok(AllDeviceTypes::All),
            "network" => Ok(AllDeviceTypes::Network),
            "disk" => Ok(AllDeviceTypes::Disk),
            _ => bail!("unknown device type '{s}'"),
        }
    }
}

enum DeviceType {
    Network,
    Disk,
}

impl FromStr for DeviceType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_ref() {
            "network" => Ok(DeviceType::Network),
            "disk" => Ok(DeviceType::Disk),
            _ => bail!("unknown device type '{s}'"),
        }
    }
}

fn main() -> process::ExitCode {
    cli::run(cli::AppInfo {
        global_help: &format!(
            r#"This tool can be used to prepare a Proxmox installation ISO for automated installations.
Additional uses are to validate the format of an answer file or to test match filters and
print information on the properties to match against for the current hardware

USAGE:
  {} <COMMAND>

COMMANDS:
  prepare-iso      Prepare an ISO for automated installation
  validate-answer  Validate if an answer file is formatted correctly
  device-match     Test which devices the given filter matches against
  device-info      Show device information that can be used for filters
  system-info      Show the system information that can be used to identify a host

GLOBAL OPTIONS:
  -h, --help       Print help
  -V, --version    Print version
"#,
            env!("CARGO_PKG_NAME")
        ),
        on_command: |s, args| match s {
            Some("prepare-iso") => cli::handle_command::<CommandPrepareISOArgs>(args),
            Some("validate-answer") => cli::handle_command::<CommandValidateAnswerArgs>(args),
            Some("device-match") => cli::handle_command::<CommandDeviceMatchArgs>(args),
            Some("device-info") => cli::handle_command::<CommandDeviceInfoArgs>(args),
            Some("system-info") => cli::handle_command::<CommandSystemInfoArgs>(args),
            Some(s) => bail!("unknown subcommand '{s}'"),
            None => bail!("subcommand required"),
        },
    })
}

fn info(args: &CommandDeviceInfoArgs) -> Result<()> {
    let nics = if matches!(
        args.device_type,
        AllDeviceTypes::All | AllDeviceTypes::Network
    ) {
        match get_nics() {
            Ok(res) => Some(res),
            Err(err) => bail!("Error getting NIC data: {err}"),
        }
    } else {
        None
    };

    let disks = if matches!(args.device_type, AllDeviceTypes::All | AllDeviceTypes::Disk) {
        match get_disks() {
            Ok(res) => Some(res),
            Err(err) => bail!("Error getting disk data: {err}"),
        }
    } else {
        None
    };

    serde_json::to_writer_pretty(
        std::io::stdout(),
        &serde_json::json!({
            "disks": disks,
            "nics": nics,
        }),
    )?;
    Ok(())
}

fn match_filter(args: &CommandDeviceMatchArgs) -> Result<()> {
    let devs: BTreeMap<String, BTreeMap<String, String>> = match args.device_type {
        DeviceType::Disk => get_disks().unwrap(),
        DeviceType::Network => get_nics().unwrap(),
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
    let result = match args.device_type {
        DeviceType::Disk => {
            get_matched_udev_indexes(&filters, &devs, args.filter_match == FilterMatch::All)
        }
        DeviceType::Network => get_single_udev_index(&filters, &devs).map(|r| vec![r]),
    };

    match result {
        Ok(result) => serde_json::to_writer_pretty(std::io::stdout(), &result)?,
        Err(err) => bail!("Error matching filters: {err}"),
    }
    Ok(())
}

fn validate_answer_file_keys(path: impl AsRef<Path> + fmt::Debug) -> Result<bool> {
    let mut file =
        fs::File::open(&path).with_context(|| format!("Opening answer file {path:?} failed"))?;

    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .with_context(|| format!("Reading from file {path:?} failed"))?;

    fn validate(section: &[&str], v: toml::Value) -> bool {
        // These sections only hold udev properties, so don't validate them
        if section == ["network", "filter"] || section == ["disk-setup", "filter"] {
            return true;
        }

        let mut valid = true;
        if let toml::Value::Table(t) = v {
            for (k, v) in t {
                if k.contains('_') {
                    eprintln!(
                        "Warning: Section [{}] contains deprecated key `{k}`, use `{}` instead.",
                        section.join("."),
                        k.replace("_", "-")
                    );
                    valid = false;
                }
                valid &= validate(&[section, &[&k]].concat(), v);
            }
        }

        valid
    }

    let answers: toml::Value = toml::from_str(&contents).context("Error parsing answer file")?;

    if validate(&[], answers) {
        Ok(true)
    } else {
        eprintln!(
            "Warning: Answer file is using deprecated underscore keys. \
            Since PVE 8.4-1 and PBS 3.4-1, kebab-case style keys are now preferred."
        );
        Ok(false)
    }
}

fn validate_answer(args: &CommandValidateAnswerArgs) -> Result<()> {
    let mut valid = validate_answer_file_keys(&args.path)?;

    match parse_answer(&args.path) {
        Ok(answer) => {
            if args.debug {
                println!("Parsed data from answer file:\n{:#?}", answer);
            }
        }
        Err(err) => {
            eprintln!("{err:#}");
            valid = false;
        }
    }

    if valid {
        println!("The answer file was parsed successfully, no errors found!");
        Ok(())
    } else {
        bail!("Found issues in the answer file.");
    }
}

fn show_system_info(_args: &CommandSystemInfoArgs) -> Result<()> {
    match SysInfo::as_json_pretty() {
        Ok(res) => println!("{res}"),
        Err(err) => eprintln!("Error fetching system info: {err}"),
    }
    Ok(())
}

fn prepare_iso(args: &CommandPrepareISOArgs) -> Result<()> {
    check_prepare_requirements(args)?;
    let uuid = get_iso_uuid(&args.input)?;

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
        bail!("You must set '--fetch-from' to 'iso' to place the answer file directly in the ISO.");
    }

    if let Some(first_boot) = &args.on_first_boot {
        let metadata = fs::metadata(first_boot)?;

        if metadata.len() > FIRST_BOOT_EXEC_MAX_SIZE.try_into()? {
            bail!(
                "Maximum file size for first-boot executable file is {} MiB",
                FIRST_BOOT_EXEC_MAX_SIZE / 1024 / 1024
            )
        }
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
        partition_label: args.partition_label.clone(),
        http: HttpOptions {
            url: args.url.clone(),
            cert_fingerprint: args.cert_fingerprint.clone(),
        },
    };
    let mut instmode_file_tmp = tmp_base.clone();
    instmode_file_tmp.push("auto-installer-mode.toml");
    fs::write(&instmode_file_tmp, toml::to_string_pretty(&config)?)?;

    inject_file_to_iso(
        &tmp_iso,
        &instmode_file_tmp,
        "/auto-installer-mode.toml",
        &uuid,
    )?;

    if let Some(answer_file) = &args.answer_file {
        inject_file_to_iso(&tmp_iso, answer_file, "/answer.toml", &uuid)?;
    }

    if let Some(first_boot) = &args.on_first_boot {
        inject_file_to_iso(
            &tmp_iso,
            first_boot,
            &format!("/{FIRST_BOOT_EXEC_NAME}"),
            &uuid,
        )?;
    }

    println!("Moving prepared ISO to target location...");
    fs::rename(&tmp_iso, &iso_target)?;
    println!("Final ISO is available at {iso_target:?}.");

    Ok(())
}

fn final_iso_location(args: &CommandPrepareISOArgs) -> PathBuf {
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

fn inject_file_to_iso(
    iso: impl AsRef<Path> + fmt::Debug,
    file: &PathBuf,
    location: &str,
    uuid: &String,
) -> Result<()> {
    let result = Command::new("xorriso")
        .arg("-boot_image")
        .arg("any")
        .arg("keep")
        .arg("-volume_date")
        .arg("uuid")
        .arg(uuid)
        .arg("-dev")
        .arg(iso.as_ref())
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

fn get_iso_uuid(iso: impl AsRef<Path>) -> Result<String> {
    let result = Command::new("xorriso")
        .arg("-dev")
        .arg(iso.as_ref())
        .arg("-report_system_area")
        .arg("cmd")
        .output()?;
    if !result.status.success() {
        bail!(
            "Error determining the UUID of the source ISO: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let mut uuid = String::new();
    for line in String::from_utf8(result.stdout)?.lines() {
        if line.starts_with("-volume_date uuid") {
            uuid = line
                .split(' ')
                .last()
                .ok_or_else(|| format_err!("xorriso did behave unexpectedly"))?
                .replace('\'', "")
                .trim()
                .into();
            break;
        }
    }
    Ok(uuid)
}

fn get_disks() -> Result<BTreeMap<String, BTreeMap<String, String>>> {
    let unwanted_block_devs = [
        Pattern::new("ram[0-9]*")?,
        Pattern::new("loop[0-9]*")?,
        Pattern::new("md[0-9]*")?,
        Pattern::new("dm-*")?,
        Pattern::new("fd[0-9]*")?,
        Pattern::new("sr[0-9]*")?,
    ];

    const PROP_DEVTYP_PREFIX: &str = "E: DEVTYPE=";
    const PROP_CDROM: &str = "E: ID_CDROM";
    const PROP_ISO9660_FS: &str = "E: ID_FS_TYPE=iso9660";

    let mut disks: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

    'outer: for entry in fs::read_dir("/sys/block")? {
        let entry = entry.unwrap();
        let filename = entry.file_name().into_string().unwrap();

        for p in &unwanted_block_devs {
            if p.matches(&filename) {
                continue 'outer;
            }
        }

        let output = match get_udev_properties(entry.path()) {
            Ok(output) => output,
            Err(err) => {
                eprint!("{err}");
                continue 'outer;
            }
        };

        let mut name = filename;
        let mut udev_props: BTreeMap<String, String> = BTreeMap::new();
        for line in output.lines() {
            if let Some(prop) = line.strip_prefix(PROP_DEVTYP_PREFIX) {
                if prop != "disk" {
                    continue 'outer;
                }
            }

            if line.starts_with(PROP_CDROM) || line.starts_with(PROP_ISO9660_FS) {
                continue 'outer;
            }

            if let Some(prop) = line.strip_prefix("N: ") {
                name = prop.to_owned();
            };

            if let Some(prop) = line.strip_prefix("E: ") {
                if let Some((key, val)) = prop.split_once('=') {
                    udev_props.insert(key.to_owned(), val.to_owned());
                }
            }
        }

        disks.insert(name, udev_props);
    }
    Ok(disks)
}

fn get_nics() -> Result<BTreeMap<String, BTreeMap<String, String>>> {
    let mut nics: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

    let links = get_nic_list()?;
    for link in links {
        let path = format!("/sys/class/net/{link}");

        let output = match get_udev_properties(PathBuf::from(path)) {
            Ok(output) => output,
            Err(err) => {
                eprint!("{err}");
                continue;
            }
        };

        let mut udev_props: BTreeMap<String, String> = BTreeMap::new();

        for line in output.lines() {
            if let Some(prop) = line.strip_prefix("E: ") {
                if let Some((key, val)) = prop.split_once('=') {
                    udev_props.insert(key.to_owned(), val.to_owned());
                }
            }
        }

        nics.insert(link, udev_props);
    }
    Ok(nics)
}

fn get_udev_properties(path: impl AsRef<Path> + fmt::Debug) -> Result<String> {
    let udev_output = Command::new("udevadm")
        .arg("info")
        .arg("--path")
        .arg(path.as_ref())
        .arg("--query")
        .arg("all")
        .output()?;
    if !udev_output.status.success() {
        bail!("could not run udevadm successfully for {path:?}");
    }
    Ok(String::from_utf8(udev_output.stdout)?)
}

fn parse_answer(path: impl AsRef<Path> + fmt::Debug) -> Result<Answer> {
    let mut file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(err) => bail!("Opening answer file {path:?} failed: {err}"),
    };
    let mut contents = String::new();
    if let Err(err) = file.read_to_string(&mut contents) {
        bail!("Reading from file {path:?} failed: {err}");
    }

    match toml::from_str(&contents) {
        Ok(answer) => {
            verify_locale_settings(&answer, &serde_json::from_str(LOCALE_INFO)?)?;
            verify_disks_settings(&answer)?;
            verify_first_boot_settings(&answer)?;
            verify_email_and_root_password_settings(&answer)?;
            Ok(answer)
        }
        Err(err) => bail!("Error parsing answer file: {err}"),
    }
}

fn check_prepare_requirements(args: &CommandPrepareISOArgs) -> Result<()> {
    match Path::try_exists(&args.input) {
        Ok(true) => (),
        Ok(false) => bail!("Source file {:?} does not exist.", args.input),
        Err(err) => bail!("Failed to stat source file {:?}: {err:#}", args.input),
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
                bail!(
                    "The source ISO file is not able to be installed automatically. Please try a more current one."
                );
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            bail!("Could not find the 'xorriso' binary. Please install it.")
        }
        Err(err) => bail!("unexpected error when trying to execute 'xorriso' - {err}"),
    };

    Ok(())
}
