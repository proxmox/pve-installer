use anyhow::{Result, bail};
use std::{fs, io, path::PathBuf};

use crate::utils::get_nic_list;
use proxmox_installer_common::{RUNTIME_DIR, setup::SetupInfo};
use proxmox_installer_types::{NetworkInterface, SystemInfo};

pub fn get() -> Result<SystemInfo> {
    let path = PathBuf::from(RUNTIME_DIR).join("iso-info.json").to_owned();
    let setup_info: SetupInfo = match fs::File::open(path) {
        Ok(iso_info_file) => {
            let reader = io::BufReader::new(iso_info_file);
            serde_json::from_reader(reader)?
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => SetupInfo::mocked(),
        Err(err) => bail!("failed to open iso-info.json - {err}"),
    };

    Ok(SystemInfo {
        product: setup_info.config,
        iso: setup_info.iso_info,
        network_interfaces: get_all_network_interfaces()?,
        dmi: proxmox_installer_common::dmi::get()?,
    })
}

fn get_all_network_interfaces() -> Result<Vec<NetworkInterface>> {
    let mut result: Vec<NetworkInterface> = Vec::new();

    let links = get_nic_list()?;
    for link in links {
        let mac = fs::read_to_string(format!("/sys/class/net/{link}/address"))?;
        result.push(NetworkInterface {
            link,
            mac: mac.trim().parse()?,
        });
    }
    Ok(result)
}
