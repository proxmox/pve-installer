//! This tool can be used to prepare a Proxmox installation ISO for automated installations.
//! Additional uses are to validate the format of an answer file or to test match filters and print
//! information on the properties to match against for the current hardware.

#![forbid(unsafe_code)]

use anyhow::{Context, Result, anyhow, bail, format_err};
use glob::Pattern;
use proxmox_sys::{crypt::verify_crypt_pw, linux::tty::read_password};
use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, File},
    io::{self, IsTerminal, Read},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    str::FromStr,
};

use proxmox_auto_installer::{
    sysinfo,
    utils::{
        AutoInstSettings, FetchAnswerFrom, HttpOptions, default_partition_label,
        get_matched_udev_indexes, get_nic_list, get_single_udev_index, verify_disks_settings,
        verify_email_and_root_password_settings, verify_first_boot_settings,
        verify_locale_settings, verify_network_settings,
    },
};
use proxmox_installer_common::{FIRST_BOOT_EXEC_MAX_SIZE, FIRST_BOOT_EXEC_NAME, cli};
use proxmox_installer_types::answer::{AutoInstallerConfig, FilterMatch};

static PROXMOX_ISO_FLAG: &str = "/auto-installer-capable";

/// Locale information as raw JSON, can be parsed into a
/// [LocaleInfo](`proxmox_installer_common::setup::LocaleInfo`) struct.
const LOCALE_INFO: &str = include_str!("../../locale-info.json");

#[derive(Debug, PartialEq)]
struct CdInfo {
    product_name: String,
    release: String,
    isorelease: String,
}

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
            .unwrap_or_default();

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
    /// Interactively verify the hashed root password.
    verify_password: bool,
}

impl cli::Subcommand for CommandValidateAnswerArgs {
    fn parse(args: &mut cli::Arguments) -> Result<Self> {
        Ok(Self {
            debug: args.contains(["-d", "--debug"]),
            verify_password: args.contains("--verify-root-password"),
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
  -d, --debug                 Also show the full answer as parsed.
      --verify-root-password  Interactively verify the hashed root password.
  -h, --help                  Print this help
  -V, --version               Print version
    "#,
            env!("CARGO_PKG_NAME")
        );
    }

    fn run(&self) -> Result<()> {
        if self.verify_password && !std::io::stdin().is_terminal() {
            Self::print_usage();
            bail!("Verifying the root password requires an interactive terminal.");
        }
        validate_answer(self)
    }
}

#[derive(Copy, Clone)]
enum PxeLoader {
    Ipxe,
}

impl FromStr for PxeLoader {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "ipxe" => Ok(PxeLoader::Ipxe),
            _ => bail!("unknown PXE loader '{s}'"),
        }
    }
}

/// Arguments for the `prepare-iso` command.
struct CommandPrepareISOArgs {
    /// Path to the source ISO to prepare.
    input: PathBuf,

    /// Path to store the final ISO to, defaults to an auto-generated file name depending on mode
    /// and the same directory as the source file is located in.
    /// If '--pxe' is specified, the path must be a directory.
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

    /// Instead of producing an ISO file, generate a 'initrd.img' and 'vmlinuz' file for use with
    /// (i)PXE servers. The '--output' option must point to a directory to place these files in.
    ///
    /// See also '--pxe-loader'.
    pxe: bool,

    /// Optional. The only possible value is 'ipxe'. If <LOADER> is specified, a
    /// configuration file is additionally produced for the specified PXE loader.
    ///
    /// Implies '--pxe'.
    pxe_loader: Option<PxeLoader>,

    /// Only useful in combination with '--fetch-from http'. Token the automated installer should
    /// use for retrieving an answer file.
    ///
    /// If set, the automated installer will include an 'Authorization' header in the HTTP POST
    /// for retrieving the answer, in the format 'Authorization: Bearer <TOKEN>'.
    answer_auth_token: Option<String>,
}

impl cli::Subcommand for CommandPrepareISOArgs {
    fn parse(args: &mut cli::Arguments) -> Result<Self> {
        let pxe_loader = args.opt_value_from_str("--pxe-loader")?;

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
            pxe: args.contains("--pxe") || pxe_loader.is_some(),
            pxe_loader,
            answer_auth_token: args.opt_value_from_str("--answer-auth-token")?,
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

          If '--pxe' is specified, the given path must be a directory, otherwise it will default
          to the directory of the source file.

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
          Can be used in combination with '--fetch-from partition' to set the partition label the
          auto-installer will search for.

          [default: proxmox-ais]

      --on-first-boot <ON_FIRST_BOOT>
          Executable file to include, which should be run on the first system boot after the
          installation. Can be used for further bootstrapping the new system.

          Must be appropriately enabled in the answer file.

      --pxe
          Instead of only producing an ISO file, additionally generate 'initrd.img' and 'vmlinuz'
          file for use with (i)PXE servers. If given, the '--output' option must point to a
          directory to place the files in, instead of a filename.

          See also '--pxe-loader'.

          [default: off]

      --pxe-loader <LOADER>
          Optional. The only possible value is 'ipxe'. If <LOADER> is specified, a configuration
          file is additionally produced for the specified PXE loader.

          Implies '--pxe'.

      --answer-auth-token <TOKEN>
          Only useful in combination with '--fetch-from http'. Token the automated installer should
          use for retrieving an answer file.

          <TOKEN> must be of format '<name>:<secret>'.

          If set, the automated installer will include an 'Authorization' header in the HTTP POST
          for retrieving the answer, in the format 'Authorization: Bearer <TOKEN>'.

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

/// Arguments for the `inspect-iso` command.
struct CommandInspectISOArgs {
    /// Path to the ISO to inspect.
    input: PathBuf,
    /// Print sensitive fields verbatim instead of redacting them.
    show_sensitive: bool,
}

impl cli::Subcommand for CommandInspectISOArgs {
    fn parse(args: &mut cli::Arguments) -> Result<Self> {
        Ok(Self {
            show_sensitive: args.contains("--show-sensitive"),
            // Needs to be last
            input: args.free_from_str()?,
        })
    }

    fn print_usage() {
        eprintln!(
            r#"Inspect a Proxmox installer ISO and report whether it has been prepared for
automated installation, which fetch mode it uses, and -- for fetch-from 'iso'
-- the embedded answer file with sensitive fields redacted.

The embedded answer file is always re-serialized through the TOML formatter
so its on-screen shape stays consistent regardless of '--show-sensitive';
comments and the original key ordering are not preserved.

USAGE:
  {} inspect-iso [OPTIONS] <INPUT>

ARGUMENTS:
  <INPUT>  Path to the ISO to inspect.

OPTIONS:
      --show-sensitive  Print sensitive fields (root password, subscription key,
                        HTTP auth token) verbatim instead of as <redacted>.
  -h, --help            Print this help
  -V, --version         Print version
"#,
            env!("CARGO_PKG_NAME")
        );
    }

    fn run(&self) -> Result<()> {
        inspect_iso(self)
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
  inspect-iso      Inspect an ISO and report its automated installation configuration
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
            Some("inspect-iso") => cli::handle_command::<CommandInspectISOArgs>(args),
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

fn verify_hashed_password_interactive(answer: &AutoInstallerConfig) -> Result<()> {
    if let Some(hashed) = &answer.global.root_password_hashed {
        println!("Verifying hashed root password.");

        let password = String::from_utf8(read_password("Enter root password to verify: ")?)?;
        verify_crypt_pw(&password, hashed).context("Failed to verify hashed root password")?;

        println!("Password matches hashed root password.");
        Ok(())
    } else {
        bail!("'root-password-hashed' not set in answer file, cannot verify.");
    }
}

fn validate_answer(args: &CommandValidateAnswerArgs) -> Result<()> {
    let mut valid = validate_answer_file_keys(&args.path)?;

    match parse_answer(&args.path) {
        Ok(answer) => {
            if args.debug {
                println!("Parsed data from answer file:\n{:#?}", answer);
            }
            if args.verify_password
                && let Err(err) = verify_hashed_password_interactive(&answer)
            {
                eprintln!("{err:#}");
                valid = false;
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
    let info = sysinfo::get().context("fetching system info")?;
    println!("{}", serde_json::to_string_pretty(&info)?);

    Ok(())
}

const REDACTED: &str = "<redacted>";

/// Per-section answer-file fields that hold secrets and must be redacted unless `--show-sensitive`.
/// Each section lists both the canonical kebab-case key and the legacy snake_case alias.
const ANSWER_SENSITIVE_FIELDS: &[(&str, &[&str])] = &[
    (
        "global",
        &[
            "root-password",
            "root_password",
            "root-password-hashed",
            "root_password_hashed",
            "subscription-key",
            "subscription_key",
        ],
    ),
    ("post-installation-webhook", &["auth-token", "auth_token"]),
];

fn inspect_iso(args: &CommandInspectISOArgs) -> Result<()> {
    if !fs::exists(&args.input).context("checking if input ISO exists")? {
        bail!("input ISO {:?} does not exist", args.input);
    }

    println!("Source ISO:    {}", args.input.display());

    // Random name + O_EXCL + 0700 + RAII cleanup defeats the predictable-name and symlink-replace
    // attack surface; xorriso fundamentally needs a real path, so memfd/pipe are not an option.
    let tmp_dir = tempfile::Builder::new()
        .prefix("proxmox-inspect-iso.")
        .tempdir()
        .context("creating temp dir for ISO extraction")?;
    inspect_iso_inner(args, tmp_dir.path())
}

fn inspect_iso_inner(args: &CommandInspectISOArgs, tmp_dir: &Path) -> Result<()> {
    let disk_info_raw = try_extract_iso_text(&args.input, "/.disk/info", tmp_dir)
        .context("reading /.disk/info to identify ISO")?
        .ok_or_else(|| {
            anyhow!(
                "/.disk/info is missing; {:?} is not a recognized installer ISO",
                args.input,
            )
        })?;
    let info = parse_disk_info_kv(&disk_info_raw);
    let product_long = info.get("PRODUCTLONG").map(String::as_str).unwrap_or("");
    if !product_long.starts_with("Proxmox") {
        bail!(
            "PRODUCTLONG={product_long:?} in /.disk/info does not look like a Proxmox installer ISO",
        );
    }
    let release = info.get("RELEASE").map(String::as_str).unwrap_or("?");
    let iso_release = info
        .get("ISORELEASE")
        .filter(|s| !s.is_empty())
        .map(|s| format!("-{s}"))
        .unwrap_or_default();
    println!("Product:       {product_long} {release}{iso_release}");

    let Some(mode_toml) = try_extract_iso_text(&args.input, "/auto-installer-mode.toml", tmp_dir)
        .context("reading /auto-installer-mode.toml")?
    else {
        println!("Auto-install:  not configured (use 'prepare-iso' to enable)");
        return Ok(());
    };
    println!("Auto-install:  enabled");

    let settings: AutoInstSettings =
        toml::from_str(&mode_toml).context("parsing /auto-installer-mode.toml")?;
    let mode_str = match settings.mode {
        FetchAnswerFrom::Iso => "iso",
        FetchAnswerFrom::Http => "http",
        FetchAnswerFrom::Partition => "partition",
    };
    println!("Fetch mode:    {mode_str}");
    if settings.partition_label != default_partition_label() {
        println!("Partition label: {}", settings.partition_label);
    }
    if let Some(url) = &settings.http.url {
        println!("HTTP URL:      {url}");
    }
    if let Some(fp) = &settings.http.cert_fingerprint {
        println!("HTTP cert fingerprint: {fp}");
    }
    if let Some(token) = &settings.http.token {
        let shown = if args.show_sensitive {
            token.as_str()
        } else {
            REDACTED
        };
        println!("HTTP auth token: {shown}");
    }

    if settings.mode == FetchAnswerFrom::Iso {
        let answer_toml = try_extract_iso_text(&args.input, "/answer.toml", tmp_dir)
            .context("reading /answer.toml")?
            .ok_or_else(|| {
                anyhow!("auto-install mode is 'iso' but /answer.toml is not present in the ISO")
            })?;
        let displayed = format_answer_toml(&answer_toml, !args.show_sensitive)
            .context("formatting answer file")?;
        println!();
        println!("Embedded answer file (/answer.toml):");
        print!("{displayed}");
        if !displayed.ends_with('\n') {
            println!();
        }
    }

    Ok(())
}

/// Parse the shell-env-style `KEY='value'` lines that the cd-builder writes
/// into `/.disk/info`. Unknown lines are ignored; quoting variants are accepted.
fn parse_disk_info_kv(s: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches(|c| c == '\'' || c == '"');
        out.insert(key.trim().to_string(), value.to_string());
    }
    out
}

/// Extract `iso_path` from `iso` into `tmp_dir/<basename>` and return its UTF-8 contents.
/// `Ok(None)` only on xorriso's "not in ISO" diagnostic; any other failure propagates as an error.
fn try_extract_iso_text(iso: &Path, iso_path: &str, tmp_dir: &Path) -> Result<Option<String>> {
    let base_name = Path::new(iso_path)
        .file_name()
        .ok_or_else(|| anyhow!("invalid ISO-internal path {iso_path:?}: no file component"))?;
    let out_path = tmp_dir.join(base_name);
    let _ = fs::remove_file(&out_path);

    if let Err(err) = extract_file_from_iso(iso, Path::new(iso_path), &out_path) {
        // xorriso prints "Cannot determine attributes of (ISO) source file" for paths absent from
        // the ISO; treat that as not-present and propagate everything else so real errors surface.
        let err_text = err.to_string();
        if err_text.contains("Cannot determine attributes of (ISO) source file") {
            return Ok(None);
        }
        return Err(err);
    }
    let content =
        fs::read_to_string(&out_path).with_context(|| format!("reading extracted {out_path:?}"))?;
    let _ = fs::remove_file(&out_path);
    Ok(Some(content))
}

/// Re-serialize the answer file through the TOML formatter, optionally redacting known-sensitive
/// fields. The output shape stays identical regardless of `--show-sensitive`; comments are lost.
fn format_answer_toml(content: &str, redact: bool) -> Result<String> {
    let mut value: toml::Value = toml::from_str(content).context("parsing answer file as TOML")?;
    if redact {
        for (section, keys) in ANSWER_SENSITIVE_FIELDS {
            let Some(table) = value.get_mut(*section).and_then(|v| v.as_table_mut()) else {
                continue;
            };
            for key in *keys {
                if table.contains_key(*key) {
                    table.insert(
                        (*key).to_string(),
                        toml::Value::String(REDACTED.to_string()),
                    );
                }
            }
        }
    }
    Ok(toml::to_string_pretty(&value)?)
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

    if let Some(path) = &args.output {
        if args.pxe {
            if !fs::exists(path)? || !fs::metadata(path)?.is_dir() {
                // If PXE output is enabled and an output was specified, it must point to a
                // directory, as we produce multiple files there.
                bail!("'--output' must point to an existing directory when '--pxe' is specified.");
            }
        } else if fs::exists(path)? && !fs::metadata(path)?.is_file() {
            // .. otherwise, the output file must either not exist yet or point to a file which
            // gets overwritten.
            bail!(
                "Path specified by '--output' already exists but is not a file, cannot overwrite."
            );
        }
    }

    if let Some(file) = &args.answer_file {
        println!("Checking provided answer file...");
        parse_answer(file)?;
    }

    let iso_target = final_iso_location(args)?;
    let iso_target_file_name = match iso_target.file_name() {
        None => bail!("no base filename in target ISO path found"),
        Some(source_file_name) => source_file_name.to_string_lossy(),
    };

    let tmp_base = args
        .tmp
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&args.input.parent().unwrap()));

    let mut tmp_iso = tmp_base.clone();
    tmp_iso.push(format!("{iso_target_file_name}.tmp"));

    println!("Copying source ISO to temporary location...");
    fs::copy(&args.input, &tmp_iso)?;

    println!("Preparing ISO...");
    let config = AutoInstSettings {
        mode: args.fetch_from.clone(),
        partition_label: args.partition_label.clone(),
        http: HttpOptions {
            url: args.url.clone(),
            cert_fingerprint: args.cert_fingerprint.clone(),
            token: args.answer_auth_token.clone(),
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
    let _ = fs::remove_file(&instmode_file_tmp);

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

    if args.pxe {
        prepare_pxe_compatible_files(args, &tmp_base, &tmp_iso, &iso_target, &uuid)?;
        let _ = fs::remove_file(tmp_iso);
    } else {
        println!("Moving prepared ISO to target location...");
        fs::copy(&tmp_iso, &iso_target)?;
        let _ = fs::remove_file(tmp_iso);
        println!("Final ISO is available at {}.", iso_target.display());
    }

    Ok(())
}

/// Creates and prepares all files needing for PXE-booting the installer.
///
/// The general flow here is:
/// 1. Extract the kernel and initrd image to the given target folder
/// 2. Recompress the initrd image from zstd to gzip, as PXE loaders generally
///    only support gzip.
/// 3. Remove the `/boot` directory from the target ISO, to save nearly 100 MiB
/// 4. If a particular (supported) PXE loader was given on the command line,
///    generate a configuration file for it.
///
/// # Arguments
///
/// * `args` - Original command arguments given to the `prepare-iso` subcommand.
/// * `tmp_base` - Directory to use a scratch pad.
/// * `iso_source` - Source ISO file to extract kernel and initrd from.
/// * `iso_target` - Target ISO file to create, must be different from 'iso_source'.
/// * `iso_uuid` - UUID to set for the target ISO.
fn prepare_pxe_compatible_files(
    args: &CommandPrepareISOArgs,
    tmp_base: &Path,
    iso_source: &Path,
    iso_target: &Path,
    iso_uuid: &str,
) -> Result<()> {
    debug_assert_ne!(
        iso_source, iso_target,
        "source and target ISO files must be different"
    );

    println!("Creating vmlinuz and initrd.img for PXE booting...");

    let out_dir = match &args.output {
        Some(path) => path,
        None => args
            .input
            .parent()
            .ok_or_else(|| anyhow!("could not determine directory of input file"))?,
    };

    let cd_info_path = out_dir.join(".cd-info.tmp");
    extract_file_from_iso(iso_source, Path::new("/.cd-info"), &cd_info_path)?;
    let cd_info = parse_cd_info(&fs::read_to_string(&cd_info_path)?)?;

    extract_file_from_iso(
        iso_source,
        Path::new("/boot/linux26"),
        &out_dir.join("vmlinuz"),
    )?;

    let compressed_initrd = tmp_base.join("initrd.img.zst");
    extract_file_from_iso(
        iso_source,
        Path::new("/boot/initrd.img"),
        &compressed_initrd,
    )?;

    // re-compress the initrd from zstd to gzip, as iPXE does not support a
    // zstd-compressed initrd
    {
        println!("Recompressing initrd using gzip...");

        let input = File::open(&compressed_initrd)
            .with_context(|| format!("opening {compressed_initrd:?}"))?;

        let output_file = File::create(out_dir.join("initrd.img"))
            .with_context(|| format!("opening {out_dir:?}/initrd.img"))?;
        let mut output = flate2::write::GzEncoder::new(output_file, flate2::Compression::default());

        zstd::stream::copy_decode(input, &mut output)?;
        output.finish()?;
    }

    let iso_target_file_name = iso_target
        .file_name()
        .map(|name| name.to_string_lossy())
        .ok_or_else(|| anyhow!("no filename found for ISO target?"))?;
    println!("Creating ISO file {:?}...", iso_target_file_name);

    // need to remove the output file if it exists, as xorriso refuses to overwrite it
    if fs::exists(iso_target)? {
        fs::remove_file(iso_target).context("failed to remove existing target ISO file")?;
    }

    // remove the whole /boot folder from the ISO to save some space (nearly 100 MiB), as it is
    // unnecessary with PXE
    remove_file_from_iso(iso_source, iso_target, iso_uuid, "/boot")?;

    if let Some(loader) = args.pxe_loader {
        create_pxe_config_file(loader, &cd_info, &iso_target_file_name, out_dir)?;
    }

    // try to clean up all temporary files
    let _ = fs::remove_file(&cd_info_path);
    let _ = fs::remove_file(&compressed_initrd);
    println!("PXE-compatible files are available in {out_dir:?}.");

    Ok(())
}

fn final_iso_location(args: &CommandPrepareISOArgs) -> Result<PathBuf> {
    // if not in PXE mode and the user already specified a output file, use that
    if !args.pxe
        && let Some(specified) = &args.output
    {
        return Ok(specified.clone());
    }

    let mut filename = args
        .input
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("input name has no filename?"))?
        .to_owned();

    match args.fetch_from {
        FetchAnswerFrom::Http => filename.push_str("-auto-from-http"),
        FetchAnswerFrom::Iso => filename.push_str("-auto-from-iso"),
        FetchAnswerFrom::Partition => filename.push_str("-auto-from-partition"),
    }

    if args.url.is_some() {
        filename.push_str("-url");
    }
    if args.cert_fingerprint.is_some() {
        filename.push_str("-fp");
    }

    filename.push_str(".iso");

    if args.pxe
        && let Some(out_dir) = &args.output
    {
        // for PXE, place the file into the output directory if one was given
        Ok(out_dir.join(filename))
    } else {
        // .. otherwise, we default to the directory the input ISO lies in
        let path = args
            .input
            .parent()
            .ok_or_else(|| anyhow!("parent directory of input not found"))?
            .join(filename);
        Ok(path)
    }
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

/// Extracts a file from the given ISO9660 file.
///
/// If `file` points a directory inside the ISO, `outpath` must be a directory too.
///
/// # Arguments
///
/// * `iso` - Source ISO file to extract from
/// * `file` - Absolute file path inside the ISO file to extract.
/// * `outpath` - Output path to write the extracted file to.
fn extract_file_from_iso(iso: &Path, file: &Path, outpath: &Path) -> Result<()> {
    debug_assert!(fs::exists(iso).unwrap_or_default());

    let result = Command::new("xorriso")
        .arg("-osirrox")
        .arg("on")
        .arg("-indev")
        .arg(iso)
        .arg("-extract")
        .arg(file)
        .arg(outpath)
        .output()?;

    if !result.status.success() {
        bail!(
            "Error extracting {file:?} from {iso:?} to {outpath:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(())
}

/// Removes a file from the ISO9660 file.
///
/// # Arguments
///
/// * `iso_in` - Source ISO file to remove the file from.
/// * `iso_out` - Target ISO file to create with the given filepath removed, must be different from
///   'iso_in'
/// * `iso_uuid` - UUID to set for the target ISO.
/// * `path` - File path to remove from the ISO file.
fn remove_file_from_iso(
    iso_in: &Path,
    iso_out: &Path,
    iso_uuid: &str,
    path: impl AsRef<Path> + fmt::Debug,
) -> Result<()> {
    debug_assert_ne!(
        iso_in, iso_out,
        "source and target ISO files must be different"
    );

    let result = Command::new("xorriso")
        .arg("-boot_image")
        .arg("any")
        .arg("keep")
        .arg("-volume_date")
        .arg("uuid")
        .arg(iso_uuid)
        .arg("-indev")
        .arg(iso_in)
        .arg("-outdev")
        .arg(iso_out)
        .arg("-rm_r")
        .arg(path.as_ref())
        .output()?;

    if !result.status.success() {
        bail!(
            "Error removing {path:?} from {iso_in:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(())
}

struct PxeBootOption {
    /// Unique, short, single-word identifier among all entries
    id: &'static str,
    /// What to show in parenthesis in the menu select
    description: &'static str,
    /// Extra parameters to append to the kernel commandline
    extra_params: &'static str,
}

const DEFAULT_KERNEL_PARAMS: &str = "ramdisk_size=16777216 rw quiet";
const PXE_BOOT_OPTIONS: &[PxeBootOption] = &[
    PxeBootOption {
        id: "auto",
        description: "Automated",
        extra_params: "splash=silent proxmox-start-auto-installer",
    },
    PxeBootOption {
        id: "gui",
        description: "Graphical",
        extra_params: "splash=silent",
    },
    PxeBootOption {
        id: "tui",
        description: "Terminal UI",
        extra_params: "splash=silent proxmox-tui-mode vga=788",
    },
    PxeBootOption {
        id: "serial",
        description: "Terminal UI, Serial Console",
        extra_params: "splash=silent proxmox-tui-mode console=ttyS0,115200",
    },
    PxeBootOption {
        id: "debug",
        description: "Debug Mode",
        extra_params: "splash=verbose proxmox-debug vga=788",
    },
    PxeBootOption {
        id: "debugtui",
        description: "Terminal UI, Debug Mode",
        extra_params: "splash=verbose proxmox-debug proxmox-tui-mode vga=788",
    },
    PxeBootOption {
        id: "serialdebug",
        description: "Serial Console, Debug Mode",
        extra_params: "splash=verbose proxmox-debug proxmox-tui-mode console=ttyS0,115200",
    },
];

/// Creates a configuration file for the given PXE bootloader.
///
/// # Arguments
///
/// * `loader` - PXE bootloader to generate the configuration for
/// * `cd_info` - Information loaded from the ISO
/// * `iso_filename` - Final name of the ISO file, written to the PXE configuration
/// * `out_dir` - Output path to write the file(s) to
fn create_pxe_config_file(
    loader: PxeLoader,
    cd_info: &CdInfo,
    iso_filename: &str,
    out_dir: &Path,
) -> Result<()> {
    debug_assert!(fs::exists(out_dir).unwrap_or_default());

    let (filename, contents) = match loader {
        PxeLoader::Ipxe => {
            let default_kernel =
                format!("kernel vmlinuz {DEFAULT_KERNEL_PARAMS} initrd=initrd.img");

            let menu_items = PXE_BOOT_OPTIONS
                .iter()
                .map(|opt| {
                    format!(
                        "item {} Install {} ({})\n",
                        opt.id, cd_info.product_name, opt.description
                    )
                })
                .collect::<String>();

            let menu_options = PXE_BOOT_OPTIONS
                .iter()
                .map(|opt| {
                    format!(
                        r#":{}
    echo Loading {} {} Installer ...
    {default_kernel} {}
    goto load

"#,
                        opt.id, cd_info.product_name, opt.description, opt.extra_params
                    )
                })
                .collect::<String>();

            let script = format!(
                r#"#!ipxe

dhcp

menu Welcome to {} {}-{}
{menu_items}
choose --default auto --timeout 10000 target && goto ${{target}}

{menu_options}
:load
initrd initrd.img
initrd {iso_filename} proxmox.iso
boot
"#,
                cd_info.product_name, cd_info.release, cd_info.isorelease
            );

            println!("Creating boot.ipxe for iPXE booting...");
            ("boot.ipxe", script)
        }
    };

    let target_path = out_dir.join(filename);
    fs::create_dir_all(
        target_path
            .parent()
            .ok_or_else(|| anyhow!("expected parent path"))?,
    )?;

    fs::write(target_path, contents)?;
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
                .next_back()
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
            if let Some(prop) = line.strip_prefix(PROP_DEVTYP_PREFIX)
                && prop != "disk"
            {
                continue 'outer;
            }

            if line.starts_with(PROP_CDROM) || line.starts_with(PROP_ISO9660_FS) {
                continue 'outer;
            }

            if let Some(prop) = line.strip_prefix("N: ") {
                name = prop.to_owned();
            };

            if let Some(prop) = line.strip_prefix("E: ")
                && let Some((key, val)) = prop.split_once('=')
            {
                udev_props.insert(key.to_owned(), val.to_owned());
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
            if let Some(prop) = line.strip_prefix("E: ")
                && let Some((key, val)) = prop.split_once('=')
            {
                udev_props.insert(key.to_owned(), val.to_owned());
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

fn parse_answer(path: impl AsRef<Path> + fmt::Debug) -> Result<AutoInstallerConfig> {
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
            verify_network_settings(&answer.network, None)?;
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

/// Parses the simple key='value' .cd-info format as shipped in the installer.
///
/// # Parameters
///
/// * `raw_cd_info` - .cd-info file contents.
///
/// # Returns
///
/// If successful, a struct containing the long product name, the product version and ISO release
/// iteration.
fn parse_cd_info(raw_cd_info: &str) -> Result<CdInfo> {
    let mut info = CdInfo {
        product_name: "Proxmox VE".into(),
        release: String::new(),
        isorelease: String::new(),
    };

    for line in raw_cd_info.lines() {
        match line.split_once('=') {
            Some(("PRODUCTLONG", val)) => info.product_name = val.trim_matches('\'').parse()?,
            Some(("RELEASE", val)) => info.release = val.trim_matches('\'').to_owned(),
            Some(("ISORELEASE", val)) => info.isorelease = val.trim_matches('\'').to_owned(),
            Some(_) => {}
            None if line.is_empty() => {}
            _ => bail!("invalid cd-info line: {line}"),
        }
    }

    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::{CdInfo, parse_cd_info};
    use anyhow::Result;

    #[test]
    fn parse_cdinfo() -> Result<()> {
        let s = r#"
PRODUCT='pve'
PRODUCTLONG='Proxmox VE'
RELEASE='42.1'
ISORELEASE='1'
ISONAME='proxmox-ve'
"#;

        assert_eq!(
            parse_cd_info(s)?,
            CdInfo {
                product_name: "Proxmox VE".into(),
                release: "42.1".into(),
                isorelease: "1".into(),
            }
        );
        Ok(())
    }
}
