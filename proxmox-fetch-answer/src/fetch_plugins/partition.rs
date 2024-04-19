use anyhow::{format_err, Error, Result};
use log::{info, warn};
use std::{
    fs::{self, create_dir_all},
    path::{Path, PathBuf},
    process::Command,
};

static ANSWER_FILE: &str = "answer.toml";
static ANSWER_MP: &str = "/mnt/answer";
static PARTLABEL: &str = "proxmox-inst-src";
static SEARCH_PATH: &str = "/dev/disk/by-label";

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

/// Searches for upper and lower case existence of the partlabel in the search_path
///
/// # Arguemnts
/// * `partlabel_source` - Partition Label, used as upper and lower case
/// * `search_path` - Path where to search for the partiiton label
fn scan_partlabels(partlabel_source: &str, search_path: &str) -> Result<PathBuf> {
    let partlabel = partlabel_source.to_uppercase();
    let path = Path::new(search_path).join(&partlabel);
    match path.try_exists() {
        Ok(true) => {
            info!("Found partition with label '{partlabel}'");
            return Ok(path);
        }
        Ok(false) => info!("Did not detect partition with label '{partlabel}'"),
        Err(err) => info!("Encountered issue, accessing '{path:?}': {err}"),
    }

    let partlabel = partlabel_source.to_lowercase();
    let path = Path::new(search_path).join(&partlabel);
    match path.try_exists() {
        Ok(true) => {
            info!("Found partition with label '{partlabel}'");
            return Ok(path);
        }
        Ok(false) => info!("Did not detect partition with label '{partlabel}'"),
        Err(err) => info!("Encountered issue, accessing '{path:?}': {err}"),
    }
    Err(Error::msg(format!(
        "Could not detect upper or lower case labels for '{partlabel_source}'"
    )))
}

/// Will search and mount a partition/FS labeled PARTLABEL (proxmox-inst-src) in lower or uppercase
/// to ANSWER_MP
fn mount_proxmoxinst_part() -> Result<String> {
    if let Ok(true) = check_if_mounted(ANSWER_MP) {
        info!("Skipping: '{ANSWER_MP}' is already mounted.");
        return Ok(ANSWER_MP.into());
    }
    let part_path = scan_partlabels(PARTLABEL, SEARCH_PATH)?;
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
        Err(err) => Err(Error::msg(format!("Error mounting: {err}"))),
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
