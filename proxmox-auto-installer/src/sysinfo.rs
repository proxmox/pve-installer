use anyhow::{bail, Result};
use proxmox_installer_common::setup::{IsoInfo, ProductConfig, SetupInfo};
use serde::Serialize;
use std::{collections::HashMap, fs, io};

use crate::utils::get_nic_list;

const DMI_PATH: &str = "/sys/devices/virtual/dmi/id";

#[derive(Debug, Serialize)]
pub struct SysInfo {
    product: ProductConfig,
    iso: IsoInfo,
    dmi: SystemDMI,
    network_interfaces: Vec<NetdevWithMac>,
}

impl SysInfo {
    pub fn get() -> Result<Self> {
        let setup_info: SetupInfo = match fs::File::open("/run/proxmox-installer/iso-info.json") {
            Ok(iso_info_file) => {
                let reader = io::BufReader::new(iso_info_file);
                serde_json::from_reader(reader)?
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => SetupInfo::mocked(),
            Err(err) => bail!("failed to open iso-info.json - {err}"),
        };

        Ok(Self {
            product: setup_info.config,
            iso: setup_info.iso_info,
            network_interfaces: NetdevWithMac::get_all()?,
            dmi: SystemDMI::get()?,
        })
    }

    pub fn as_json_pretty() -> Result<String> {
        let info = Self::get()?;
        Ok(serde_json::to_string_pretty(&info)?)
    }

    pub fn as_json() -> Result<String> {
        let info = Self::get()?;
        Ok(serde_json::to_string(&info)?)
    }
}

#[derive(Debug, Serialize)]
struct NetdevWithMac {
    /// The network link name
    pub link: String,
    /// The MAC address of the network device
    pub mac: String,
}

impl NetdevWithMac {
    fn get_all() -> Result<Vec<Self>> {
        let mut result: Vec<Self> = Vec::new();

        let links = get_nic_list()?;
        for link in links {
            let mac = fs::read_to_string(format!("/sys/class/net/{link}/address"))?;
            let mac = String::from(mac.trim());
            result.push(Self { link, mac });
        }
        Ok(result)
    }
}

#[derive(Debug, Serialize)]
struct SystemDMI {
    system: HashMap<String, String>,
    baseboard: HashMap<String, String>,
    chassis: HashMap<String, String>,
}

impl SystemDMI {
    pub(crate) fn get() -> Result<Self> {
        let system_files = vec![
            "product_serial",
            "product_sku",
            "product_uuid",
            "product_name",
        ];
        let baseboard_files = vec!["board_asset_tag", "board_serial", "board_name"];
        let chassis_files = vec!["chassis_serial", "chassis_sku", "chassis_asset_tag"];

        Ok(Self {
            system: Self::get_dmi_infos(system_files)?,
            baseboard: Self::get_dmi_infos(baseboard_files)?,
            chassis: Self::get_dmi_infos(chassis_files)?,
        })
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
}
