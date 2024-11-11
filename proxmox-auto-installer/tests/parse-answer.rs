use std::path::{Path, PathBuf};

use serde_json::Value;
use std::fs;

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

fn get_answer(path: PathBuf) -> Result<Answer, String> {
    let answer_raw = std::fs::read_to_string(path).unwrap();
    let answer: answer::Answer = toml::from_str(&answer_raw)
        .map_err(|err| format!("error parsing answer.toml: {err}"))
        .unwrap();

    Ok(answer)
}

pub fn setup_test_basic(path: &Path) -> (SetupInfo, LocaleInfo, RuntimeInfo, UdevInfo) {
    let (installer_info, locale_info, mut runtime_info) = load_installer_setup_files(path).unwrap();

    let udev_info: UdevInfo = {
        let mut path = path.to_path_buf();
        path.push("run-env-udev.json");

        read_json(&path)
            .map_err(|err| format!("Failed to retrieve udev info details: {err}"))
            .unwrap()
    };

    runtime_info.disks.sort();
    assert!(!runtime_info.disks.is_empty(), "disk list cannot be empty");

    (installer_info, locale_info, runtime_info, udev_info)
}

#[test]
fn test_parse_answers() {
    let path = get_test_resource_path().unwrap();
    let (setup_info, locales, runtime_info, udev_info) = setup_test_basic(&path);
    let mut tests_path = path.clone();
    tests_path.push("parse_answer");
    let test_dir = fs::read_dir(tests_path.clone()).unwrap();

    for file_entry in test_dir {
        let file = file_entry.unwrap();
        if !file.file_type().unwrap().is_file() || file.file_name() == "readme" {
            continue;
        }
        let p = file.path();
        let name = p.file_stem().unwrap().to_str().unwrap();
        let extension = p.extension().unwrap().to_str().unwrap();
        if extension == "toml" {
            println!("Test: {name}");
            let answer = get_answer(p.clone()).unwrap();
            let config =
                &parse_answer(&answer, &udev_info, &runtime_info, &locales, &setup_info).unwrap();
            println!("Selected disks: {:#?}", &config.disk_selection);
            let config_json = serde_json::to_string(config);
            let config: Value = serde_json::from_str(config_json.unwrap().as_str()).unwrap();
            let mut path = tests_path.clone();
            path.push(format!("{name}.json"));
            let compare_raw = std::fs::read_to_string(&path).unwrap();
            let compare: Value = serde_json::from_str(&compare_raw).unwrap();
            if config != compare {
                panic!(
                    "Test {} failed:\nleft:  {:#?}\nright: {:#?}\n",
                    name, config, compare
                );
            }
        }
    }
}
