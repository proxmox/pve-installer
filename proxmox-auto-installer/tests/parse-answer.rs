use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use proxmox_auto_installer::answer;
use proxmox_auto_installer::answer::Answer;
use proxmox_auto_installer::udevinfo::UdevInfo;
use proxmox_auto_installer::utils::parse_answer;

use proxmox_installer_common::setup::{
    load_installer_setup_files, read_json, LocaleInfo, RuntimeInfo, SetupInfo,
};

fn get_test_resource_path() -> Result<PathBuf, String> {
    Ok(std::env::current_dir()
        .expect("current dir failed")
        .join("tests/resources"))
}

fn get_answer(path: impl AsRef<Path>) -> Result<Answer, String> {
    let answer_raw = fs::read_to_string(path).unwrap();
    let answer: answer::Answer = toml::from_str(&answer_raw)
        .map_err(|err| format!("error parsing answer.toml: {err}"))
        .unwrap();

    Ok(answer)
}

fn setup_test_basic(path: impl AsRef<Path>) -> (SetupInfo, LocaleInfo, RuntimeInfo, UdevInfo) {
    let (installer_info, locale_info, mut runtime_info) =
        load_installer_setup_files(&path).unwrap();

    let udev_info: UdevInfo = {
        let mut path = path.as_ref().to_path_buf();
        path.push("run-env-udev.json");

        read_json(&path)
            .map_err(|err| format!("Failed to retrieve udev info details: {err}"))
            .unwrap()
    };

    runtime_info.disks.sort();
    assert!(!runtime_info.disks.is_empty(), "disk list cannot be empty");

    (installer_info, locale_info, runtime_info, udev_info)
}

fn run_named_test(name: &str) {
    let resource_path = get_test_resource_path().unwrap();
    let (setup_info, locales, runtime_info, udev_info) = setup_test_basic(&resource_path);

    let answer_path = resource_path.join(format!("parse_answer/{name}.toml"));

    let answer = get_answer(&answer_path).unwrap();
    let config = &parse_answer(&answer, &udev_info, &runtime_info, &locales, &setup_info).unwrap();

    let config_json = serde_json::to_string(config);
    let config: Value = serde_json::from_str(config_json.unwrap().as_str()).unwrap();

    let json_path = resource_path.join(format!("parse_answer/{name}.json"));
    let compare_raw = fs::read_to_string(&json_path).unwrap();
    let compare: Value = serde_json::from_str(&compare_raw).unwrap();

    pretty_assertions::assert_eq!(config, compare);
}

mod tests {
    mod parse_answer {
        use super::super::run_named_test;

        macro_rules! declare_named_tests {
            ($name:ident, $( $rest:ident ),* $(,)?) => { declare_named_tests!($name); declare_named_tests!($( $rest ),+); };
            ($name:ident) => {
                #[test]
                fn $name() {
                    run_named_test(&stringify!($name));
                }
            };
        }

        declare_named_tests!(
            btrfs,
            btrfs_raid_level_uppercase,
            disk_match,
            disk_match_all,
            disk_match_any,
            first_boot,
            hashed_root_password,
            minimal,
            nic_matching,
            specific_nic,
            zfs,
            zfs_raid_level_uppercase,
        );
    }
}
