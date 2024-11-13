use anyhow::{bail, Result};
use proxmox_installer_common::{
    setup::{IsoInfo, ProductConfig, SetupInfo},
    sysinfo::SystemDMI,
    RUNTIME_DIR,
};
use serde::Serialize;
use std::{fs, io, path::PathBuf};

use crate::utils::get_nic_list;

#[derive(Debug, Serialize)]
pub struct SysInfo {
    product: ProductConfig,
    iso: IsoInfo,
    dmi: SystemDMI,
    network_interfaces: Vec<NetdevWithMac>,
}

impl SysInfo {
    pub fn get() -> Result<Self> {
        let path = PathBuf::from(RUNTIME_DIR).join("iso-info.json").to_owned();
        let setup_info: SetupInfo = match fs::File::open(path) {
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
