use std::{collections::HashMap, fs::File, io::BufReader, path::Path};

use serde::{Deserialize, Deserializer};

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxmoxProduct {
    PVE,
    PBS,
    PMG,
}

#[derive(Clone, Deserialize)]
pub struct ProductConfig {
    pub fullname: String,
    pub product: ProxmoxProduct,
    #[serde(deserialize_with = "deserialize_bool_from_int")]
    pub enable_btrfs: bool,
}

#[derive(Clone, Deserialize)]
pub struct IsoInfo {
    pub release: String,
    pub isorelease: String,
}

#[derive(Clone, Deserialize)]
pub struct SetupInfo {
    #[serde(rename = "product-cfg")]
    pub product_cfg: ProductConfig,
    #[serde(rename = "iso-info")]
    pub iso_info: IsoInfo,
}

#[derive(Clone, Deserialize)]
pub struct CountryInfo {
    pub name: String,
    #[serde(default)]
    pub zone: String,
    pub kmap: String,
}

#[derive(Clone, Deserialize)]
pub struct KeyboardMapping {
    pub name: String,
    #[serde(rename = "x11")]
    pub layout: String,
    #[serde(rename = "x11var")]
    pub variant: String,
}

#[derive(Clone, Deserialize)]
pub struct LocaleInfo {
    #[serde(deserialize_with = "deserialize_cczones_map")]
    pub cczones: HashMap<String, Vec<String>>,
    pub country: HashMap<String, CountryInfo>,
    pub kmap: HashMap<String, KeyboardMapping>,
}

pub fn read_json<T: for<'de> Deserialize<'de>, P: AsRef<Path>>(path: P) -> Result<T, String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    let reader = BufReader::new(file);

    serde_json::from_reader(reader).map_err(|err| format!("failed to parse JSON: {err}"))
}

fn deserialize_bool_from_int<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let val: u32 = Deserialize::deserialize(deserializer)?;
    Ok(val != 0)
}

fn deserialize_cczones_map<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let map: HashMap<String, HashMap<String, u32>> = Deserialize::deserialize(deserializer)?;

    let mut result = HashMap::new();
    for (cc, list) in map.into_iter() {
        result.insert(cc, list.into_keys().collect());
    }

    Ok(result)
}
