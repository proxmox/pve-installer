use std::{collections::HashMap, fs};

use anyhow::{Result, bail};
use proxmox_installer_types::SystemDMI;

const DMI_PATH: &str = "/sys/devices/virtual/dmi/id";

pub fn get() -> Result<SystemDMI> {
    let system_files = [
        "product_serial",
        "product_sku",
        "product_uuid",
        "product_name",
    ];
    let baseboard_files = ["board_asset_tag", "board_serial", "board_name"];
    let chassis_files = ["chassis_serial", "chassis_sku", "chassis_asset_tag"];

    Ok(SystemDMI {
        system: get_dmi_infos_for(&system_files)?,
        baseboard: get_dmi_infos_for(&baseboard_files)?,
        chassis: get_dmi_infos_for(&chassis_files)?,
    })
}

fn get_dmi_infos_for(files: &[&str]) -> Result<HashMap<String, String>> {
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
