use anyhow::Result;
use log::info;
use std::path::PathBuf;

use crate::fetch_plugins::utils::{get_answer_file, mount_proxmoxinst_part};

static ANSWER_FILE: &str = "answer.toml";

pub struct FetchFromPartition;

impl FetchFromPartition {
    /// Returns the contents of the answer file
    pub fn get_answer() -> Result<String> {
        info!("Checking for answer file on partition.");
        let mut mount_path = PathBuf::from(mount_proxmoxinst_part()?);
        mount_path.push(ANSWER_FILE);
        let answer = get_answer_file(&mount_path)?;
        info!("Found answer file on partition.");
        Ok(answer)
    }
}
