//! Post installation hook for the Proxmox installer, mainly for combination
//! with the auto-installer.
//!
//! If a `[post-installation-webhook]` section is specified in the given answer file, it will send
//! a HTTP POST request to that URL, with an optional certificate fingerprint for usage with
//! (self-signed) TLS certificates.
//! In the body of the request, information about the newly installed system is sent.
//!
//! Relies on `proxmox-chroot` as an external dependency to (bind-)mount the
//! previously installed system.

use std::{
    collections::HashSet,
    ffi::CStr,
    fs::{self, File},
    io::BufReader,
    os::unix::fs::FileExt,
    path::PathBuf,
    process::{Command, ExitCode},
};

use anyhow::{anyhow, bail, Context, Result};
use proxmox_auto_installer::{
    answer::{Answer, PostNotificationHookInfo},
    udevinfo::{UdevInfo, UdevProperties},
};
use proxmox_installer_common::{
    options::{Disk, FsType},
    setup::{
        load_installer_setup_files, BootType, InstallConfig, IsoInfo, ProxmoxProduct, RuntimeInfo,
        SetupInfo,
    },
    sysinfo::SystemDMI,
    utils::CidrAddress,
};
use serde::Serialize;

/// Information about the system boot status.
#[derive(Serialize)]
struct BootInfo {
    /// Whether the system is booted using UEFI or legacy BIOS.
    mode: BootType,
    /// Whether SecureBoot is enabled for the installation.
    #[serde(skip_serializing_if = "Option::is_none")]
    secureboot: Option<bool>,
}

/// Holds all the public keys for the different algorithms available.
#[derive(Serialize)]
struct SshPublicHostKeys {
    // ECDSA-based public host key
    ecdsa: String,
    // ED25519-based public host key
    ed25519: String,
    // RSA-based public host key
    rsa: String,
}

/// Holds information about a single disk in the system.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct DiskInfo {
    /// Size in bytes
    size: usize,
    /// Set to true if the disk is used for booting.
    #[serde(skip_serializing_if = "Option::is_none")]
    is_bootdisk: Option<bool>,
    /// Properties about the device as given by udev.
    udev_properties: UdevProperties,
}

/// Holds information about the management network interface.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct NetworkInterfaceInfo {
    /// MAC address of the interface
    mac: String,
    /// (Designated) IP address of the interface
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<CidrAddress>,
    /// Set to true if the interface is the chosen management interface during
    /// installation.
    #[serde(skip_serializing_if = "Option::is_none")]
    is_management: Option<bool>,
    /// Properties about the device as given by udev.
    udev_properties: UdevProperties,
}

/// Information about the installed product itself.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct ProductInfo {
    /// Full name of the product
    fullname: String,
    /// Product abbreviation
    short: ProxmoxProduct,
    /// Version of the installed product
    version: String,
}

/// The current kernel version.
/// Aligns with the format as used by the /nodes/<node>/status API of each product.
#[derive(Serialize)]
struct KernelVersionInformation {
    /// The systemname/nodename
    pub sysname: String,
    /// The kernel release number
    pub release: String,
    /// The kernel version
    pub version: String,
    /// The machine architecture
    pub machine: String,
}

/// Information about the CPU(s) installed in the system
#[derive(Serialize)]
struct CpuInfo {
    /// Number of physical CPU cores.
    cores: usize,
    /// Number of logical CPU cores aka. threads.
    cpus: usize,
    /// CPU feature flag set as a space-delimited list.
    flags: String,
    /// Whether hardware-accelerated virtualization is supported.
    hvm: bool,
    /// Reported model of the CPU(s)
    model: String,
    /// Number of physical CPU sockets
    sockets: usize,
}

/// Metadata of the hook, such as schema version of the document.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct PostHookInfoSchema {
    /// major.minor version describing the schema version of this document, in a semanticy-version
    /// way.
    ///
    /// major: Incremented for incompatible/breaking API changes, e.g. removing an existing
    /// field.
    /// minor: Incremented when adding functionality in a backwards-compatible matter, e.g.
    /// adding a new field.
    version: String,
}

impl PostHookInfoSchema {
    const SCHEMA_VERSION: &str = "1.0";
}

impl Default for PostHookInfoSchema {
    fn default() -> Self {
        Self {
            version: Self::SCHEMA_VERSION.to_owned(),
        }
    }
}

/// All data sent as request payload with the post-installation-webhook POST request.
///
/// NOTE: The format is versioned through `schema.version` (`$schema.version` in the
/// resulting JSON), ensure you update it when this struct or any of its members gets modified.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct PostHookInfo {
    // This field is prefixed by `$` on purpose, to indicate that it is document metadata and not
    // part of the actual content itself. (E.g. JSON Schema uses a similar naming scheme)
    #[serde(rename = "$schema")]
    schema: PostHookInfoSchema,
    /// major.minor version of Debian as installed, retrieved from /etc/debian_version
    debian_version: String,
    /// PVE/PMG/PBS version as reported by `pveversion`, `pmgversion` or
    /// `proxmox-backup-manager version`, respectively.
    product: ProductInfo,
    /// Release information for the ISO used for the installation.
    iso: IsoInfo,
    /// Installed kernel version
    kernel_version: KernelVersionInformation,
    /// Describes the boot mode of the machine and the SecureBoot status.
    boot_info: BootInfo,
    /// Information about the installed CPU(s)
    cpu_info: CpuInfo,
    /// DMI information about the system
    dmi: SystemDMI,
    /// Filesystem used for boot disk(s)
    filesystem: FsType,
    /// Fully qualified domain name of the installed system
    fqdn: String,
    /// Unique systemd-id128 identifier of the installed system (128-bit, 16 bytes)
    machine_id: String,
    /// All disks detected on the system.
    disks: Vec<DiskInfo>,
    /// All network interfaces detected on the system.
    network_interfaces: Vec<NetworkInterfaceInfo>,
    /// Public parts of SSH host keys of the installed system
    ssh_public_host_keys: SshPublicHostKeys,
}

/// Defines the size of a gibibyte in bytes.
const SIZE_GIB: usize = 1024 * 1024 * 1024;

impl PostHookInfo {
    /// Gathers all needed information about the newly installed system for sending
    /// it to a specified server.
    ///
    /// # Arguments
    ///
    /// * `target_path` - Path to where the chroot environment root is mounted
    /// * `answer` - Answer file as provided by the user
    fn gather(target_path: &str, answer: &Answer) -> Result<Self> {
        println!("Gathering installed system data ...");

        let config: InstallConfig =
            serde_json::from_reader(BufReader::new(File::open("/tmp/low-level-config.json")?))?;

        let (setup_info, _, run_env) =
            load_installer_setup_files(proxmox_installer_common::RUNTIME_DIR)
                .map_err(|err| anyhow!("Failed to load setup files: {err}"))?;

        let udev: UdevInfo = {
            let path =
                PathBuf::from(proxmox_installer_common::RUNTIME_DIR).join("run-env-udev.json");
            serde_json::from_reader(BufReader::new(File::open(path)?))?
        };

        // Opens a file, specified by an absolute path _inside_ the chroot
        // from the target.
        let open_file = |path: &str| {
            File::open(format!("{}/{}", target_path, path))
                .with_context(|| format!("failed to open '{path}'"))
        };

        // Reads a file, specified by an absolute path _inside_ the chroot
        // from the target.
        let read_file = |path: &str| {
            fs::read_to_string(format!("{}/{}", target_path, path))
                .map(|s| s.trim().to_owned())
                .with_context(|| format!("failed to read '{path}'"))
        };

        // Runs a command inside the target chroot.
        let run_cmd = |cmd: &[&str]| {
            Command::new("chroot")
                .arg(target_path)
                .args(cmd)
                .output()
                .with_context(|| format!("failed to run '{cmd:?}'"))
                .and_then(|r| Ok(String::from_utf8(r.stdout)?))
        };

        Ok(Self {
            schema: PostHookInfoSchema::default(),
            debian_version: read_file("/etc/debian_version")?,
            product: Self::gather_product_info(&setup_info, &run_cmd)?,
            iso: setup_info.iso_info.clone(),
            kernel_version: Self::gather_kernel_version(&run_cmd, &open_file)?,
            boot_info: BootInfo {
                mode: run_env.boot_type,
                secureboot: run_env.secure_boot,
            },
            cpu_info: Self::gather_cpu_info(&run_env)?,
            dmi: SystemDMI::get()?,
            filesystem: answer.disks.fs_type,
            fqdn: answer.global.fqdn.to_string(),
            machine_id: read_file("/etc/machine-id")?,
            disks: Self::gather_disks(&config, &run_env, &udev)?,
            network_interfaces: Self::gather_nic(&config, &run_env, &udev)?,
            ssh_public_host_keys: SshPublicHostKeys {
                ecdsa: read_file("/etc/ssh/ssh_host_ecdsa_key.pub")?,
                ed25519: read_file("/etc/ssh/ssh_host_ed25519_key.pub")?,
                rsa: read_file("/etc/ssh/ssh_host_rsa_key.pub")?,
            },
        })
    }

    /// Retrieves all needed information about the boot disks that were selected during
    /// installation, most notable the udev properties.
    ///
    /// # Arguments
    ///
    /// * `config` - Low-level installation configuration
    /// * `run_env` - Runtime envirornment information gathered by the installer at the start
    /// * `udev` - udev information for all system devices
    fn gather_disks(
        config: &InstallConfig,
        run_env: &RuntimeInfo,
        udev: &UdevInfo,
    ) -> Result<Vec<DiskInfo>> {
        let get_udev_properties = |disk: &Disk| {
            udev.disks
                .get(&disk.index)
                .with_context(|| {
                    format!("could not find udev information for disk '{}'", disk.path)
                })
                .cloned()
        };

        let disks = if config.filesys.is_lvm() {
            // If the filesystem is LVM, there is only boot disk. The path (aka. /dev/..)
            // can be found in `config.target_hd`.
            run_env
                .disks
                .iter()
                .flat_map(|disk| {
                    let is_bootdisk = config
                        .target_hd
                        .as_ref()
                        .and_then(|hd| (*hd == disk.path).then_some(true));

                    anyhow::Ok(DiskInfo {
                        size: (config.hdsize * (SIZE_GIB as f64)) as usize,
                        is_bootdisk,
                        udev_properties: get_udev_properties(disk)?,
                    })
                })
                .collect()
        } else {
            // If the filesystem is not LVM-based (thus Btrfs or ZFS), `config.disk_selection`
            // contains a list of indices identifiying the boot disks, as given by udev.
            let selected_disks_indices: Vec<&String> = config.disk_selection.values().collect();

            run_env
                .disks
                .iter()
                .flat_map(|disk| {
                    let is_bootdisk = selected_disks_indices
                        .contains(&&disk.index)
                        .then_some(true);

                    anyhow::Ok(DiskInfo {
                        size: (config.hdsize * (SIZE_GIB as f64)) as usize,
                        is_bootdisk,
                        udev_properties: get_udev_properties(disk)?,
                    })
                })
                .collect()
        };

        Ok(disks)
    }

    /// Retrieves all needed information about the management network interface that was selected
    /// during installation, most notable the udev properties.
    ///
    /// # Arguments
    ///
    /// * `config` - Low-level installation configuration
    /// * `run_env` - Runtime envirornment information gathered by the installer at the start
    /// * `udev` - udev information for all system devices
    fn gather_nic(
        config: &InstallConfig,
        run_env: &RuntimeInfo,
        udev: &UdevInfo,
    ) -> Result<Vec<NetworkInterfaceInfo>> {
        Ok(run_env
            .network
            .interfaces
            .values()
            .flat_map(|nic| {
                let udev_properties = udev
                    .nics
                    .get(&nic.name)
                    .with_context(|| {
                        format!("could not find udev information for NIC '{}'", nic.name)
                    })?
                    .clone();

                if config.mngmt_nic == nic.name {
                    // Use the actual IP address from the low-level install config, as the runtime info
                    // contains the original IP address from DHCP.
                    anyhow::Ok(NetworkInterfaceInfo {
                        mac: nic.mac.clone(),
                        address: Some(config.cidr.clone()),
                        is_management: Some(true),
                        udev_properties,
                    })
                } else {
                    anyhow::Ok(NetworkInterfaceInfo {
                        mac: nic.mac.clone(),
                        address: None,
                        is_management: None,
                        udev_properties,
                    })
                }
            })
            .collect())
    }

    /// Retrieves the version of the installed product from the chroot.
    ///
    /// # Arguments
    ///
    /// * `setup_info` - Filled-out struct with information about the product
    /// * `run_cmd` - Callback to run a command inside the target chroot.
    fn gather_product_info(
        setup_info: &SetupInfo,
        run_cmd: &dyn Fn(&[&str]) -> Result<String>,
    ) -> Result<ProductInfo> {
        let package = match setup_info.config.product {
            ProxmoxProduct::PVE => "pve-manager",
            ProxmoxProduct::PMG => "pmg-api",
            ProxmoxProduct::PBS => "proxmox-backup-server",
        };

        let version = run_cmd(&[
            "dpkg-query",
            "--showformat",
            "${Version}",
            "--show",
            package,
        ])
        .with_context(|| format!("failed to retrieve version of {package}"))?;

        Ok(ProductInfo {
            fullname: setup_info.config.fullname.clone(),
            short: setup_info.config.product,
            version,
        })
    }

    /// Extracts the version string from the *installed* kernel image.
    ///
    /// First, it determines the exact path to the kernel image (aka. `/boot/vmlinuz-<version>`)
    /// by looking at the installed kernel package, then reads the string directly from the image
    /// from the well-defined kernel header. See also [0] for details.
    ///
    /// [0] https://www.kernel.org/doc/html/latest/arch/x86/boot.html
    ///
    /// # Arguments
    ///
    /// * `run_cmd` - Callback to run a command inside the target chroot.
    /// * `open_file` - Callback to open a file inside the target chroot.
    #[cfg(target_arch = "x86_64")]
    fn gather_kernel_version(
        run_cmd: &dyn Fn(&[&str]) -> Result<String>,
        open_file: &dyn Fn(&str) -> Result<File>,
    ) -> Result<KernelVersionInformation> {
        let file = open_file(&Self::find_kernel_image_path(run_cmd)?)?;

        // Read the 2-byte `kernel_version` field at offset 0x20e [0] from the file ..
        // https://www.kernel.org/doc/html/latest/arch/x86/boot.html#the-real-mode-kernel-header
        let mut buffer = [0u8; 2];
        file.read_exact_at(&mut buffer, 0x20e)
            .context("could not read kernel_version offset from image")?;

        // .. which gives us the offset of the kernel version string inside the image, minus 0x200.
        // https://www.kernel.org/doc/html/latest/arch/x86/boot.html#details-of-header-fields
        let offset = u16::from_le_bytes(buffer) + 0x200;

        // The string is usually somewhere around 80-100 bytes long, so 256 bytes is more than
        // enough to cover all cases.
        let mut buffer = [0u8; 256];
        file.read_exact_at(&mut buffer, offset.into())
            .context("could not read kernel version string from image")?;

        // Now just consume the buffer until the NUL byte
        let kernel_version = CStr::from_bytes_until_nul(&buffer)
            .context("did not find a NUL-terminator in kernel version string")?
            .to_str()
            .context("could not convert kernel version string")?;

        // The version string looks like:
        //   6.8.4-2-pve (build@proxmox) #1 SMP PREEMPT_DYNAMIC PMX 6.8.4-2 (2024-04-10T17:36Z) x86_64 GNU/Linux
        //
        // Thus split it into three parts, as we are interested in the release version
        // and everything starting at the build number
        let parts: Vec<&str> = kernel_version.splitn(3, ' ').collect();

        if parts.len() != 3 {
            bail!("failed to split kernel version string");
        }

        Ok(KernelVersionInformation {
            machine: std::env::consts::ARCH.to_owned(),
            sysname: "Linux".to_owned(),
            release: parts
                .first()
                .context("kernel release not found")?
                .to_string(),
            version: parts
                .get(2)
                .context("kernel version not found")?
                .to_string(),
        })
    }

    /// Retrieves the absolute path to the kernel image (aka. `/boot/vmlinuz-<version>`)
    /// inside the chroot by looking at the file list installed by the kernel package.
    ///
    /// # Arguments
    ///
    /// * `run_cmd` - Callback to run a command inside the target chroot.
    fn find_kernel_image_path(run_cmd: &dyn Fn(&[&str]) -> Result<String>) -> Result<String> {
        let pkg_name = Self::find_kernel_package_name(run_cmd)?;

        let all_files = run_cmd(&["dpkg-query", "--listfiles", &pkg_name])?;
        for file in all_files.lines() {
            if file.starts_with("/boot/vmlinuz-") {
                return Ok(file.to_owned());
            }
        }

        bail!("failed to find installed kernel image path")
    }

    /// Retrieves the full name of the kernel package installed inside the chroot.
    ///
    /// # Arguments
    ///
    /// * `run_cmd` - Callback to run a command inside the target chroot.
    fn find_kernel_package_name(run_cmd: &dyn Fn(&[&str]) -> Result<String>) -> Result<String> {
        let dpkg_arch = run_cmd(&["dpkg", "--print-architecture"])?
            .trim()
            .to_owned();

        let kernel_pkgs = run_cmd(&[
            "dpkg-query",
            "--showformat",
            "${db:Status-Abbrev}|${Architecture}|${Package}\\n",
            "--show",
            "proxmox-kernel-[0-9]*",
        ])?;

        // The output to parse looks like this:
        //   ii |all|proxmox-kernel-6.8
        //   un ||proxmox-kernel-6.8.8-2-pve
        //   ii |amd64|proxmox-kernel-6.8.8-2-pve-signed
        for pkg in kernel_pkgs.lines() {
            let parts = pkg.split('|').collect::<Vec<&str>>();

            if let [status, arch, name] = parts[..] {
                if status.trim() == "ii" && arch.trim() == dpkg_arch {
                    return Ok(name.trim().to_owned());
                }
            }
        }

        bail!("failed to find installed kernel package")
    }

    /// Retrieves some basic information about the CPU in the running system,
    /// reading them from /proc/cpuinfo.
    ///
    /// # Arguments
    ///
    /// * `run_env` - Runtime envirornment information gathered by the installer at the start
    fn gather_cpu_info(run_env: &RuntimeInfo) -> Result<CpuInfo> {
        let mut result = CpuInfo {
            cores: 0,
            cpus: 0,
            flags: String::new(),
            hvm: run_env.hvm_supported,
            model: String::new(),
            sockets: 0,
        };
        let mut sockets = HashSet::new();
        let mut cores = HashSet::new();

        // Does not matter if we read the file from inside the chroot or directly on the host.
        let cpuinfo = fs::read_to_string("/proc/cpuinfo")?;
        for line in cpuinfo.lines() {
            match line.split_once(':') {
                Some((key, _)) if key.trim() == "processor" => {
                    result.cpus += 1;
                }
                Some((key, value)) if key.trim() == "core id" => {
                    cores.insert(value);
                }
                Some((key, value)) if key.trim() == "physical id" => {
                    sockets.insert(value);
                }
                Some((key, value)) if key.trim() == "flags" => {
                    value.trim().clone_into(&mut result.flags);
                }
                Some((key, value)) if key.trim() == "model name" => {
                    value.trim().clone_into(&mut result.model);
                }
                _ => {}
            }
        }

        result.cores = cores.len();
        result.sockets = sockets.len();

        Ok(result)
    }
}

/// Runs the specified callback with the mounted chroot, passing along the
/// absolute path to where / is mounted.
/// The callback is *not* run inside the chroot itself, that is left to the caller.
///
/// # Arguments
///
/// * `callback` - Callback to call with the absolute path where the chroot environment root is
///                mounted.
fn with_chroot<R, F: FnOnce(&str) -> Result<R>>(callback: F) -> Result<R> {
    let ec = Command::new("proxmox-chroot")
        .arg("prepare")
        .status()
        .context("failed to run proxmox-chroot")?;

    if !ec.success() {
        bail!("failed to create chroot for installed system");
    }

    // See also proxmox-chroot/src/main.rs w.r.t to the path, which is hard-coded there
    let result = callback("/target");

    let ec = Command::new("proxmox-chroot").arg("cleanup").status();
    // We do not want to necessarily fail here, as the install environment is about
    // to be teared down completely anyway.
    if ec.is_err() || !ec.map(|ec| ec.success()).unwrap_or(false) {
        eprintln!("failed to clean up chroot for installed system");
    }

    result
}

/// Reads the answer file from stdin, checks for a configured post-installation-webhook URL (+
/// optional certificate fingerprint for HTTPS). If configured, retrieves all relevant information
/// about the installed system and sends them to the given endpoint.
fn do_main() -> Result<()> {
    let answer = Answer::try_from_reader(std::io::stdin().lock())?;

    if let Some(PostNotificationHookInfo {
        url,
        cert_fingerprint,
    }) = &answer.post_installation_webhook
    {
        println!("Found post-installation-webhook; sending POST request to '{url}'.");

        let info = with_chroot(|target_path| PostHookInfo::gather(target_path, &answer))?;

        proxmox_installer_common::http::post(
            url,
            cert_fingerprint.as_deref(),
            serde_json::to_string(&info)?,
        )?;
    } else {
        println!("No post-installation-webhook configured; skipping");
    }

    Ok(())
}

fn main() -> ExitCode {
    match do_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("\nError occurred during post-installation-webhook:");
            eprintln!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::PostHookInfo;

    #[test]
    fn finds_correct_kernel_package_name() {
        let mocked_run_cmd = |cmd: &[&str]| {
            if cmd[0] == "dpkg" {
                assert_eq!(cmd, &["dpkg", "--print-architecture"]);
                Ok("amd64\n".to_owned())
            } else {
                assert_eq!(
                    cmd,
                    &[
                        "dpkg-query",
                        "--showformat",
                        "${db:Status-Abbrev}|${Architecture}|${Package}\\n",
                        "--show",
                        "proxmox-kernel-[0-9]*",
                    ]
                );
                Ok(r#"ii |all|proxmox-kernel-6.8
un ||proxmox-kernel-6.8.8-2-pve
ii |amd64|proxmox-kernel-6.8.8-2-pve-signed
                "#
                .to_owned())
            }
        };

        assert_eq!(
            PostHookInfo::find_kernel_package_name(&mocked_run_cmd).unwrap(),
            "proxmox-kernel-6.8.8-2-pve-signed"
        );
    }

    #[test]
    fn find_kernel_package_name_fails_on_wrong_architecture() {
        let mocked_run_cmd = |cmd: &[&str]| {
            if cmd[0] == "dpkg" {
                assert_eq!(cmd, &["dpkg", "--print-architecture"]);
                Ok("arm64\n".to_owned())
            } else {
                assert_eq!(
                    cmd,
                    &[
                        "dpkg-query",
                        "--showformat",
                        "${db:Status-Abbrev}|${Architecture}|${Package}\\n",
                        "--show",
                        "proxmox-kernel-[0-9]*",
                    ]
                );
                Ok(r#"ii |all|proxmox-kernel-6.8
un ||proxmox-kernel-6.8.8-2-pve
ii |amd64|proxmox-kernel-6.8.8-2-pve-signed
                "#
                .to_owned())
            }
        };

        assert_eq!(
            PostHookInfo::find_kernel_package_name(&mocked_run_cmd)
                .unwrap_err()
                .to_string(),
            "failed to find installed kernel package"
        );
    }

    #[test]
    fn find_kernel_package_name_fails_on_missing_package() {
        let mocked_run_cmd = |cmd: &[&str]| {
            if cmd[0] == "dpkg" {
                assert_eq!(cmd, &["dpkg", "--print-architecture"]);
                Ok("amd64\n".to_owned())
            } else {
                assert_eq!(
                    cmd,
                    &[
                        "dpkg-query",
                        "--showformat",
                        "${db:Status-Abbrev}|${Architecture}|${Package}\\n",
                        "--show",
                        "proxmox-kernel-[0-9]*",
                    ]
                );
                Ok(r#"ii |all|proxmox-kernel-6.8
un ||proxmox-kernel-6.8.8-2-pve
                "#
                .to_owned())
            }
        };

        assert_eq!(
            PostHookInfo::find_kernel_package_name(&mocked_run_cmd)
                .unwrap_err()
                .to_string(),
            "failed to find installed kernel package"
        );
    }

    #[test]
    fn finds_correct_absolute_kernel_image_path() {
        let mocked_run_cmd = |cmd: &[&str]| {
            if cmd[0] == "dpkg" {
                assert_eq!(cmd, &["dpkg", "--print-architecture"]);
                Ok("amd64\n".to_owned())
            } else if cmd[0..=1] == ["dpkg-query", "--showformat"] {
                assert_eq!(
                    cmd,
                    &[
                        "dpkg-query",
                        "--showformat",
                        "${db:Status-Abbrev}|${Architecture}|${Package}\\n",
                        "--show",
                        "proxmox-kernel-[0-9]*",
                    ]
                );
                Ok(r#"ii |all|proxmox-kernel-6.8
un ||proxmox-kernel-6.8.8-2-pve
ii |amd64|proxmox-kernel-6.8.8-2-pve-signed
                "#
                .to_owned())
            } else {
                assert_eq!(
                    cmd,
                    [
                        "dpkg-query",
                        "--listfiles",
                        "proxmox-kernel-6.8.8-2-pve-signed"
                    ]
                );
                Ok(r#"
/.
/boot
/boot/System.map-6.8.8-2-pve
/boot/config-6.8.8-2-pve
/boot/vmlinuz-6.8.8-2-pve
/lib
/lib/modules
/lib/modules/6.8.8-2-pve
/lib/modules/6.8.8-2-pve/kernel
/lib/modules/6.8.8-2-pve/kernel/arch
/lib/modules/6.8.8-2-pve/kernel/arch/x86
/lib/modules/6.8.8-2-pve/kernel/arch/x86/crypto
/lib/modules/6.8.8-2-pve/kernel/arch/x86/crypto/aegis128-aesni.ko
/lib/modules/6.8.8-2-pve/kernel/arch/x86/crypto/aesni-intel.ko
/lib/modules/6.8.8-2-pve/kernel/arch/x86/crypto/aria-aesni-avx-x86_64.ko
/lib/modules/6.8.8-2-pve/kernel/arch/x86/crypto/aria-aesni-avx2-x86_64.ko
/lib/modules/6.8.8-2-pve/kernel/arch/x86/crypto/aria-gfni-avx512-x86_64.ko
/lib/modules/6.8.8-2-pve/kernel/arch/x86/crypto/blowfish-x86_64.ko
/lib/modules/6.8.8-2-pve/kernel/arch/x86/crypto/camellia-aesni-avx-x86_64.ko
                "#
                .to_owned())
            }
        };

        assert_eq!(
            PostHookInfo::find_kernel_image_path(&mocked_run_cmd).unwrap(),
            "/boot/vmlinuz-6.8.8-2-pve"
        );
    }
}
