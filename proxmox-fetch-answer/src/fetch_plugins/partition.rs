use anyhow::{bail, format_err, Result};
use log::{info, warn};
use std::{
    fs::{self, create_dir_all},
    path::{Path, PathBuf},
    process::Command,
};

static ANSWER_FILE: &str = "answer.toml";
static ANSWER_MP: &str = "/mnt/answer";
// FAT can only handle 11 characters, so shorten Automated Installer Source to AIS
static PARTLABEL: &str = "proxmox-ais";
static DISK_BY_ID_PATH: &str = "/dev/disk/by-label";

pub struct FetchFromPartition;

impl FetchFromPartition {
    /// Returns the contents of the answer file
    pub fn get_answer() -> Result<String> {
        info!("Checking for answer file on partition.");

        let mut mount_path = PathBuf::from(mount_proxmoxinst_part()?);
        mount_path.push(ANSWER_FILE);
        let answer = fs::read_to_string(mount_path)
            .map_err(|err| format_err!("failed to read answer file - {err}"))?;

        info!("Found answer file on partition.");

        Ok(answer)
    }
}

fn path_exists_logged(file_name: &str, search_path: &str) -> Option<PathBuf> {
    let path = Path::new(search_path).join(&file_name);
    info!("Testing partition search path {path:?}");
    match path.try_exists() {
        Ok(true) => Some(path),
        Ok(false) => None,
        Err(err) => {
            info!("Encountered issue, accessing '{path:?}': {err}");
            None
        }
    }
}

/// Searches for upper and lower case existence of the partlabel in the search_path
///
/// # Arguemnts
/// * `partlabel_source` - Partition Label, used as upper and lower case
/// * `search_path` - Path where to search for the partiiton label
fn scan_partlabels(partlabel: &str, search_path: &str) -> Result<PathBuf> {
    let partlabel_upper_case = partlabel.to_uppercase();
    if let Some(path) = path_exists_logged(&partlabel_upper_case, search_path) {
            info!("Found partition with label '{partlabel_upper_case}'");
            return Ok(path);
    }

    let partlabel_lower_case = partlabel.to_lowercase();
    if let Some(path) = path_exists_logged(&partlabel_upper_case, search_path) {
            info!("Found partition with label '{partlabel_lower_case}'");
            return Ok(path);
    }

    bail!("Could not detect upper or lower case labels for '{partlabel}'");
}

/// Will search and mount a partition/FS labeled PARTLABEL (proxmox-ais) in lower or uppercase
/// to ANSWER_MP
fn mount_proxmoxinst_part() -> Result<String> {
    if let Ok(true) = check_if_mounted(ANSWER_MP) {
        info!("Skipping: '{ANSWER_MP}' is already mounted.");
        return Ok(ANSWER_MP.into());
    }
    let part_path = scan_partlabels(PARTLABEL, DISK_BY_ID_PATH)?;
    info!("Mounting partition at {ANSWER_MP}");
    // create dir for mountpoint
    create_dir_all(ANSWER_MP)?;
    match Command::new("mount")
        .args(["-o", "ro"])
        .arg(part_path)
        .arg(ANSWER_MP)
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                Ok(ANSWER_MP.into())
            } else {
                warn!("Error mounting: {}", String::from_utf8(output.stderr)?);
                Ok(ANSWER_MP.into())
            }
        }
        Err(err) => bail!("Error mounting: {err}"),
    }
}

fn check_if_mounted(target_path: &str) -> Result<bool> {
    let mounts = fs::read_to_string("/proc/mounts")?;
    for line in mounts.lines() {
        if let Some(mp) = line.split(' ').nth(1) {
            if mp == target_path {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
