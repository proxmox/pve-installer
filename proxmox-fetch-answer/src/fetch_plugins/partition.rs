use anyhow::{Error, Result};
use log::info;
use std::{fs::read_to_string, path::Path};

use crate::fetch_plugins::utils::mount_proxmoxinst_part;

static ANSWER_FILE: &str = "answer.toml";

pub struct FetchFromPartition;

impl FetchFromPartition {
    /// Returns the contents of the answer file
    pub fn get_answer() -> Result<String> {
        info!("Checking for answer file on partition.");
        let mount_path = mount_proxmoxinst_part()?;
        let answer = Self::get_answer_file(&mount_path)?;
        info!("Found answer file on partition.");
        Ok(answer)
    }

    /// Searches for answer file and returns contents if found
    fn get_answer_file(mount_path: &str) -> Result<String> {
        let answer_path = Path::new(mount_path).join(ANSWER_FILE);
        match answer_path.try_exists() {
            Ok(true) => Ok(read_to_string(answer_path)?),
            _ => Err(Error::msg(format!(
                "could not find answer file expected at: {}",
                answer_path.display()
            ))),
        }
    }
}
