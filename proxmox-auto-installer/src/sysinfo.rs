use anyhow::{bail, Result};
use proxmox_installer_common::setup::SetupInfo;
use serde::Serialize;
use std::{collections::HashMap, fs, io, path::Path};

use crate::utils::get_nic_list;

const DMI_PATH: &str = "/sys/devices/virtual/dmi/id";

pub fn get_sysinfo(pretty: bool) -> Result<String> {
    let system_files = vec![
        "product_serial",
        "product_sku",
        "product_uuid",
        "product_name",
    ];
    let baseboard_files = vec!["board_asset_tag", "board_serial", "board_name"];
    let chassis_files = vec!["chassis_serial", "chassis_sku", "chassis_asset_tag"];

    let system = get_dmi_infos(system_files)?;
    let baseboard = get_dmi_infos(baseboard_files)?;
    let chassis = get_dmi_infos(chassis_files)?;

    let mut mac_addresses: Vec<String> = Vec::new();
    let links = get_nic_list()?;
    for link in links {
        let address = fs::read_to_string(format!("/sys/class/net/{link}/address"))?;
        let address = String::from(address.trim());
        mac_addresses.push(address);
    }

    let iso_info = Path::new("/run/proxmox-installer/iso-info.json");
    let mut product = String::from("Not available. Would be one of the following: pve, pmg, pbs");
    if iso_info.exists() {
        let file = fs::File::open("/run/proxmox-installer/iso-info.json")?;
        let reader = io::BufReader::new(file);
        let setup_info: SetupInfo = serde_json::from_reader(reader)?;
        product = setup_info.config.product.to_string();
    }

    let sysinfo = SysInfo {
        product,
        system,
        baseboard,
        chassis,
        mac_addresses,
    };
    if pretty {
        return Ok(serde_json::to_string_pretty(&sysinfo)?);
    }
    Ok(serde_json::to_string(&sysinfo)?)
}

#[derive(Debug, Serialize)]
struct SysInfo {
    product: String,
    system: HashMap<String, String>,
    baseboard: HashMap<String, String>,
    chassis: HashMap<String, String>,
    mac_addresses: Vec<String>,
}

fn get_dmi_infos(files: Vec<&str>) -> Result<HashMap<String, String>> {
    let mut res: HashMap<String, String> = HashMap::new();

    for file in files {
        let path = format!("{DMI_PATH}/{file}");
        let content = match fs::read_to_string(&path) {
            Err(ref err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(ref err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                bail!("Could not read data. Are you running as root or with sudo?")
            }
            Err(err) => bail!("Error: '{err}' on '{path}'"),
            Ok(content) => content.trim().into(),
        };
        let key = file.splitn(2, '_').last().unwrap();
        res.insert(key.into(), content);
    }

    Ok(res)
}
