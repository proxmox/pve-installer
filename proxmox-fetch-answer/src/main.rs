use std::process::ExitCode;
use std::{fs, path::PathBuf};

use anyhow::{bail, format_err, Result};
use log::{error, info, LevelFilter};

use proxmox_auto_installer::{
    log::AutoInstLogger,
    utils::{AutoInstModes, AutoInstSettings},
};

use fetch_plugins::{http::FetchFromHTTP, partition::FetchFromPartition};

mod fetch_plugins;

static LOGGER: AutoInstLogger = AutoInstLogger;
static AUTOINST_MODE_FILE: &str = "/cdrom/auto-installer-mode.toml";

pub fn init_log() -> Result<()> {
    AutoInstLogger::init("/tmp/fetch_answer.log")?;
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(LevelFilter::Info))
        .map_err(|err| format_err!(err))
}

fn fetch_answer(install_settings: &AutoInstSettings) -> Result<String> {
    info!("Fetching answer file in mode {:?}:", &install_settings.mode);
    match install_settings.mode {
        AutoInstModes::Auto => {
            match FetchFromPartition::get_answer() {
                Ok(answer) => return Ok(answer),
                Err(err) => info!("Fetching answer file from partition failed: {err}"),
            }
            match FetchFromHTTP::get_answer(install_settings) {
                Ok(answer) => return Ok(answer),
                Err(err) => info!("Fetching answer file via HTTP failed: {err}"),
            }
        }
        AutoInstModes::Included => {
            let answer_path = PathBuf::from("/cdrom/answer.toml");
            match fs::read_to_string(&answer_path) {
                Ok(answer) => return Ok(answer),
                Err(err) => info!("Fetching answer file from ISO failed: {err}"),
            }
        }
        AutoInstModes::Partition => match FetchFromPartition::get_answer() {
            Ok(answer) => return Ok(answer),
            Err(err) => info!("Fetching answer file from partition failed: {err}"),
        },
        AutoInstModes::Http => match FetchFromHTTP::get_answer(install_settings) {
            Ok(answer) => return Ok(answer),
            Err(err) => info!("Fetching answer file via HTTP failed: {err}"),
        },
    }
    bail!("Could not find any answer file!");
}

fn main() -> ExitCode {
    if let Err(err) = init_log() {
        panic!("could not initialize logging: {err}");
    }

    let raw_install_settings = match fs::read_to_string(AUTOINST_MODE_FILE) {
        Ok(f) => f,
        Err(err) => {
            error!("Could not find needed file '{AUTOINST_MODE_FILE}' in live environment: {err}");
            return ExitCode::FAILURE;
        }
    };
    let install_settings: AutoInstSettings = match toml::from_str(raw_install_settings.as_str()) {
        Ok(content) => content,
        Err(err) => {
            error!("Failed to parse '{AUTOINST_MODE_FILE}': {err}");
            return ExitCode::FAILURE;
        }
    };

    let answer = match fetch_answer(&install_settings) {
        Ok(answer) => answer,
        Err(err) => {
            error!("Aborting: {}", err);
            return ExitCode::FAILURE;
        }
    };
    info!("queried answer file for automatic installation successfully");

    println!("{answer}");

    return ExitCode::SUCCESS;
}
