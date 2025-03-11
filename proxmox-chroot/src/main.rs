use std::{
    fs, io,
    path::{self, Path, PathBuf},
    process::Command,
};

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use proxmox_installer_common::{
    RUNTIME_DIR,
    options::FsType,
    setup::{InstallConfig, SetupInfo},
};
use regex::Regex;

const ANSWER_MP: &str = "answer";
static BINDMOUNTS: [&str; 4] = ["dev", "proc", "run", "sys"];
const TARGET_DIR: &str = "/target";
const ZPOOL_NAME: &str = "rpool";

/// Helper tool to prepare everything to `chroot` into an installation
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Prepare(CommandPrepare),
    Cleanup(CommandCleanup),
}

/// Mount the root file system and bind mounts in preparation to chroot into the installation
#[derive(Args, Debug)]
struct CommandPrepare {
    /// Filesystem used for the installation. Will try to automatically detect it after a
    /// successful installation.
    #[arg(short, long, value_enum)]
    filesystem: Option<Filesystems>,

    /// Numerical ID of `rpool` ZFS pool to import. Needed if multiple pools of name `rpool` are present.
    #[arg(long)]
    rpool_id: Option<u64>,

    /// UUID of the BTRFS file system to mount. Needed if multiple BTRFS file systems are present.
    #[arg(long)]
    btrfs_uuid: Option<String>,
}

/// Unmount everything. Use once done with chroot.
#[derive(Args, Debug)]
struct CommandCleanup {
    /// Filesystem used for the installation. Will try to automatically detect it by default.
    #[arg(short, long, value_enum)]
    filesystem: Option<Filesystems>,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Filesystems {
    Zfs,
    Ext4,
    Xfs,
    Btrfs,
}

impl From<FsType> for Filesystems {
    fn from(fs: FsType) -> Self {
        match fs {
            FsType::Xfs => Self::Xfs,
            FsType::Ext4 => Self::Ext4,
            FsType::Zfs(_) => Self::Zfs,
            FsType::Btrfs(_) => Self::Btrfs,
        }
    }
}

fn main() {
    let args = Cli::parse();
    let res = match &args.command {
        Commands::Prepare(args) => prepare(args),
        Commands::Cleanup(args) => cleanup(args),
    };
    if let Err(err) = res {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

fn prepare(args: &CommandPrepare) -> Result<()> {
    let fs = get_fs(args.filesystem)?;

    fs::create_dir_all(TARGET_DIR)?;

    match fs {
        Filesystems::Zfs => mount_zpool(args.rpool_id)?,
        Filesystems::Xfs => mount_fs()?,
        Filesystems::Ext4 => mount_fs()?,
        Filesystems::Btrfs => mount_btrfs(args.btrfs_uuid.clone())?,
    }

    if let Err(e) = bindmount() {
        eprintln!("{e}")
    }

    println!("Done. You can now use 'chroot /target /bin/bash'!");
    Ok(())
}

fn cleanup(args: &CommandCleanup) -> Result<()> {
    let fs = get_fs(args.filesystem)?;

    if let Err(e) = bind_umount() {
        eprintln!("{e:#}")
    }

    match fs {
        Filesystems::Zfs => umount_zpool(),
        Filesystems::Btrfs | Filesystems::Xfs | Filesystems::Ext4 => umount(Path::new(TARGET_DIR))?,
    }

    println!("Chroot cleanup done. You can now reboot or leave the shell.");
    Ok(())
}

fn get_fs(filesystem: Option<Filesystems>) -> Result<Filesystems> {
    let fs = match filesystem {
        None => {
            let low_level_config = match get_low_level_config() {
                Ok(c) => c,
                Err(_) => bail!(
                    "Could not fetch config from previous installation. Please specify file system with -f."
                ),
            };
            Filesystems::from(low_level_config.filesys)
        }
        Some(fs) => fs,
    };

    Ok(fs)
}

fn get_low_level_config() -> Result<InstallConfig> {
    let file = fs::File::open("/tmp/low-level-config.json")?;
    let reader = io::BufReader::new(file);
    let config: InstallConfig = serde_json::from_reader(reader)?;
    Ok(config)
}

fn get_iso_info() -> Result<SetupInfo> {
    let path = PathBuf::from(RUNTIME_DIR).join("iso-info.json");
    let reader = io::BufReader::new(fs::File::open(path)?);
    let setup_info: SetupInfo = serde_json::from_reader(reader)?;
    Ok(setup_info)
}

fn mount_zpool(pool_id: Option<u64>) -> Result<()> {
    println!("importing ZFS pool to {TARGET_DIR}");
    let mut import = Command::new("zpool");
    import.arg("import").args(["-R", TARGET_DIR]);
    match pool_id {
        None => {
            import.arg(ZPOOL_NAME);
        }
        Some(id) => {
            import.arg(id.to_string());
        }
    }
    match import.status() {
        Ok(s) if !s.success() => bail!("Could not import ZFS pool. Abort!"),
        _ => (),
    }
    println!("successfully imported ZFS pool to {TARGET_DIR}");
    Ok(())
}

fn umount_zpool() {
    match Command::new("zpool").arg("export").arg(ZPOOL_NAME).status() {
        Ok(s) if !s.success() => println!("failure on exporting {ZPOOL_NAME}"),
        _ => (),
    }
}

fn mount_fs() -> Result<()> {
    let iso_info = get_iso_info()?;
    let product = iso_info.config.product;

    println!("Activating VG '{product}'");
    let res = Command::new("vgchange")
        .arg("-ay")
        .arg(product.to_string())
        .output();
    match res {
        Err(e) => bail!("{e:#}"),
        Ok(output) => {
            if output.status.success() {
                println!(
                    "successfully activated VG '{product}': {}",
                    String::from_utf8(output.stdout)?
                );
            } else {
                bail!(
                    "activation of VG '{product}' failed: {}",
                    String::from_utf8(output.stderr)?
                )
            }
        }
    }

    match Command::new("mount")
        .arg(format!("/dev/mapper/{product}-root"))
        .arg(TARGET_DIR)
        .output()
    {
        Err(e) => bail!("{e}"),
        Ok(output) => {
            if output.status.success() {
                println!("mounted root file system successfully");
            } else {
                bail!(
                    "mounting of root file system failed: {}",
                    String::from_utf8(output.stderr)?
                )
            }
        }
    }

    Ok(())
}

fn umount(path: &Path) -> Result<()> {
    match Command::new("umount")
        .arg("--recursive")
        .arg("--verbose")
        .arg(path)
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                Ok(())
            } else {
                bail!(
                    "unmounting of {path:?} failed: {}",
                    String::from_utf8(output.stderr)?
                )
            }
        }
        Err(err) => bail!("unmounting of {path:?} failed: {:#}", err),
    }
}

fn mount_btrfs(btrfs_uuid: Option<String>) -> Result<()> {
    let uuid = match btrfs_uuid {
        Some(uuid) => uuid,
        None => get_btrfs_uuid()?,
    };

    match Command::new("mount")
        .arg("--uuid")
        .arg(uuid)
        .arg(TARGET_DIR)
        .output()
    {
        Err(e) => bail!("{e}"),
        Ok(output) => {
            if output.status.success() {
                println!("mounted BTRFS root file system successfully");
            } else {
                bail!(
                    "mounting of BTRFS root file system failed: {}",
                    String::from_utf8(output.stderr)?
                )
            }
        }
    }

    Ok(())
}

fn get_btrfs_uuid() -> Result<String> {
    let output = Command::new("btrfs")
        .arg("filesystem")
        .arg("show")
        .output()?;
    if !output.status.success() {
        bail!(
            "Error checking for BTRFS file systems: {}",
            String::from_utf8(output.stderr)?
        );
    }
    let out = String::from_utf8(output.stdout)?;
    let mut uuids = Vec::new();

    let re_uuid =
        Regex::new(r"uuid: ([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$")?;
    for line in out.lines() {
        if let Some(cap) = re_uuid.captures(line) {
            if let Some(uuid) = cap.get(1) {
                uuids.push(uuid.as_str());
            }
        }
    }
    match uuids.len() {
        0 => bail!("Could not find any BTRFS UUID"),
        i if i > 1 => {
            let uuid_list = uuids
                .iter()
                .fold(String::new(), |acc, &arg| format!("{acc}\n{arg}"));
            bail!(
                "Found {i} UUIDs:{uuid_list}\nPlease specify the UUID to use with the --btrfs-uuid parameter"
            )
        }
        _ => (),
    }
    Ok(uuids[0].into())
}

fn do_bindmount(source: &Path, target: &Path) -> Result<()> {
    match Command::new("mount")
        .arg("--bind")
        .arg(source)
        .arg(target)
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                Ok(())
            } else {
                bail!(
                    "bind-mount of {source:?} to {target:?} failed: {}",
                    String::from_utf8(output.stderr)?
                )
            }
        }
        Err(err) => bail!("bind-mount of {source:?} to {target:?} failed: {:#}", err),
    }
}

fn bindmount() -> Result<()> {
    println!("Bind-mounting virtual filesystems");

    for item in BINDMOUNTS {
        let source = path::Path::new("/").join(item);
        let target = path::Path::new(TARGET_DIR).join(item);

        println!("Bind-mounting {source:?} to {target:?}");
        do_bindmount(&source, &target)?;
    }

    let answer_path = path::Path::new("/mnt").join(ANSWER_MP);

    if answer_path.exists() {
        let target = path::Path::new(TARGET_DIR).join("mnt").join(ANSWER_MP);

        println!("Creating target directory {target:?}");
        fs::create_dir_all(&target)?;

        println!("Bind-mounting {answer_path:?} to {target:?}");
        do_bindmount(&answer_path, &target)?;
    }
    Ok(())
}

fn bind_umount() -> Result<()> {
    for item in BINDMOUNTS {
        let target = path::Path::new(TARGET_DIR).join(item);
        println!("Unmounting {target:?}");
        if let Err(e) = umount(target.as_path()) {
            eprintln!("{e}");
        }
    }

    let answer_target = path::Path::new(TARGET_DIR).join("mnt").join(ANSWER_MP);
    if answer_target.exists() {
        println!("Unmounting and removing answer mountpoint");
        if let Err(e) = umount(&answer_target) {
            eprintln!("{e}");
        }
        if let Err(e) = fs::remove_dir(answer_target) {
            eprintln!("{e}");
        }
    }

    Ok(())
}
