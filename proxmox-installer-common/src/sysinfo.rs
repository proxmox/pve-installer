use std::{collections::HashMap, fs};

use anyhow::{bail, Result};
use serde::Serialize;

const DMI_PATH: &str = "/sys/devices/virtual/dmi/id";

#[derive(Debug, Serialize)]
pub struct SystemDMI {
    system: HashMap<String, String>,
    baseboard: HashMap<String, String>,
    chassis: HashMap<String, String>,
}

impl SystemDMI {
    pub fn get() -> Result<Self> {
        let system_files = [
            "product_serial",
            "product_sku",
            "product_uuid",
            "product_name",
        ];
        let baseboard_files = ["board_asset_tag", "board_serial", "board_name"];
        let chassis_files = ["chassis_serial", "chassis_sku", "chassis_asset_tag"];

        Ok(Self {
            system: Self::get_dmi_infos(&system_files)?,
            baseboard: Self::get_dmi_infos(&baseboard_files)?,
            chassis: Self::get_dmi_infos(&chassis_files)?,
        })
    }

    fn get_dmi_infos(files: &[&str]) -> Result<HashMap<String, String>> {
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
}
