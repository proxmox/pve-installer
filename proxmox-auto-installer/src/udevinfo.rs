use serde::Deserialize;
use std::collections::BTreeMap;

/// Uses a BTreeMap to have the keys sorted
pub type UdevProperties = BTreeMap<String, String>;

#[derive(Clone, Deserialize, Debug)]
pub struct UdevInfo {
    pub disks: BTreeMap<String, UdevProperties>,
    pub nics: BTreeMap<String, UdevProperties>,
}
