use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Deserialize, Debug)]
pub struct UdevInfo {
    // use BTreeMap to have keys sorted
    pub disks: BTreeMap<String, BTreeMap<String, String>>,
    pub nics: BTreeMap<String, BTreeMap<String, String>>,
}
