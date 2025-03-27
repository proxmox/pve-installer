use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use proxmox_auto_installer::answer::Answer;
use proxmox_auto_installer::udevinfo::UdevInfo;
use proxmox_auto_installer::utils::parse_answer;

use proxmox_installer_common::setup::{
    LocaleInfo, RuntimeInfo, SetupInfo, load_installer_setup_files, read_json,
};

fn get_test_resource_path() -> Result<PathBuf, String> {
    Ok(std::env::current_dir()
        .expect("current dir failed")
        .join("tests/resources"))
}

fn get_answer(path: impl AsRef<Path>) -> Result<Answer, String> {
    let answer_raw = fs::read_to_string(path).unwrap();
    toml::from_str(&answer_raw)
        .map_err(|err| format!("error parsing answer.toml: {}", err.message()))
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
    let (setup_info, locales, mut runtime_info, udev_info) = setup_test_basic(&resource_path);

    let test_run_env_path = resource_path.join(format!("parse_answer/{name}.run-env.json"));
    if test_run_env_path.exists() {
        runtime_info = read_json(test_run_env_path).unwrap()
    }

    let answer_path = resource_path.join(format!("parse_answer/{name}.toml"));

    let answer = get_answer(&answer_path).unwrap();
    let config = &parse_answer(&answer, &udev_info, &runtime_info, &locales, &setup_info).unwrap();

    let config_json = serde_json::to_string(config);
    let config: Value = serde_json::from_str(config_json.unwrap().as_str()).unwrap();

    let json_path = resource_path.join(format!("parse_answer/{name}.json"));
    let compare: Value = read_json(json_path).unwrap();

    pretty_assertions::assert_eq!(config, compare);
}

fn run_named_fail_parse_test(name: &str) {
    let resource_path = get_test_resource_path().unwrap();
    let (setup_info, locales, mut runtime_info, udev_info) = setup_test_basic(&resource_path);

    let test_run_env_path = resource_path.join(format!("parse_answer_fail/{name}.run-env.json"));
    if test_run_env_path.exists() {
        runtime_info = read_json(test_run_env_path).unwrap()
    }

    let answer_path = resource_path.join(format!("parse_answer_fail/{name}.toml"));

    let err_json: Value = {
        let path = resource_path.join(format!("parse_answer_fail/{name}.json"));
        read_json(path).unwrap()
    };

    let answer = match get_answer(&answer_path) {
        Ok(answer) => answer,
        Err(err) => {
            assert_eq!(err, err_json.get("parse-error").unwrap().as_str().unwrap());
            return;
        }
    };

    let config = parse_answer(&answer, &udev_info, &runtime_info, &locales, &setup_info);

    assert!(config.is_err());
    assert_eq!(
        config.unwrap_err().to_string(),
        err_json.get("error").unwrap().as_str().unwrap()
    );
}

mod tests {
    macro_rules! declare_tests {
        ($fn:ident, $name:ident, $( $rest:ident ),* $(,)?) => {
            declare_tests!($fn, $name);
            declare_tests!($fn, $( $rest ),+);
        };
        ($fn:ident, $name:ident) => {
            #[test]
            fn $name() {
                $fn(&stringify!($name));
            }
        };
    }

    mod parse_answer {
        use super::super::run_named_test;

        declare_tests!(
            run_named_test,
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

    mod parse_answer_fail {
        use super::super::run_named_fail_parse_test;

        declare_tests!(
            run_named_fail_parse_test,
            both_password_and_hashed_set,
            no_root_password_set,
            short_password
        );
    }
}
