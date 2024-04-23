use std::process::ExitCode;
use std::{fs, path::PathBuf};

use anyhow::{bail, format_err, Result};
use log::{error, info, LevelFilter};

use proxmox_auto_installer::{
    log::AutoInstLogger,
    utils::{AutoInstSettings, FetchAnswerFrom},
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
        FetchAnswerFrom::Iso => {
            let answer_path = PathBuf::from("/cdrom/answer.toml");
            match fs::read_to_string(answer_path) {
                Ok(answer) => return Ok(answer),
                Err(err) => info!("Fetching answer file from ISO failed: {err}"),
            }
        }
        FetchAnswerFrom::Partition => match FetchFromPartition::get_answer() {
            Ok(answer) => return Ok(answer),
            Err(err) => info!("Fetching answer file from partition failed: {err}"),
        },
        FetchAnswerFrom::Http => match FetchFromHTTP::get_answer(&install_settings.http) {
            Ok(answer) => return Ok(answer),
            Err(err) => info!("Fetching answer file via HTTP failed: {err}"),
        },
    }
    bail!("Could not find any answer file!");
}

fn do_main() -> Result<()> {
    if let Err(err) = init_log() {
        bail!("could not initialize logging: {err}");
    }

    let raw_install_settings = fs::read_to_string(AUTOINST_MODE_FILE).map_err(|err| {
        format_err!("Could not find needed file '{AUTOINST_MODE_FILE}' in live environment: {err}")
    })?;
    let install_settings: AutoInstSettings = toml::from_str(raw_install_settings.as_str())
        .map_err(|err| format_err!("Failed to parse '{AUTOINST_MODE_FILE}': {err}"))?;

    let answer = fetch_answer(&install_settings).map_err(|err| format_err!("Aborting: {err}"))?;
    info!("queried answer file for automatic installation successfully");

    println!("{answer}");

    Ok(())
}

fn main() -> ExitCode {
    match do_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!("{err}");
            ExitCode::FAILURE
        }
    }
}
