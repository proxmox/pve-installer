use anyhow::{anyhow, Error, Result};
use fetch_plugins::{http::FetchFromHTTP, partition::FetchFromPartition};
use log::{error, info, LevelFilter};
use proxmox_auto_installer::{log::AutoInstLogger, utils::{AutoInstModes, AutoInstSettings}};
use std::{fs, path::PathBuf};
use std::io::Write;
use std::process::{Command, ExitCode, Stdio};

mod fetch_plugins;

static LOGGER: AutoInstLogger = AutoInstLogger;
static AUTOINST_MODE_FILE: &str = "/cdrom/autoinst-mode.toml";

pub fn init_log() -> Result<()> {
    AutoInstLogger::init("/tmp/fetch_answer.log")?;
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(LevelFilter::Info))
        .map_err(|err| anyhow!(err))
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
        },
        AutoInstModes::Included => {
            let answer_path = PathBuf::from("/cdrom/answer.toml");
            match fetch_plugins::utils::get_answer_file(&answer_path) {
                Ok(answer) => return Ok(answer),
                Err(err) => info!("Fetching answer file from ISO failed: {err}"),
            }
        },
        AutoInstModes::Partition => {
            match FetchFromPartition::get_answer() {
                Ok(answer) => return Ok(answer),
                Err(err) => info!("Fetching answer file from partition failed: {err}"),
            }
        },
        AutoInstModes::Http => {
            match FetchFromHTTP::get_answer(install_settings) {
                Ok(answer) => return Ok(answer),
                Err(err) => info!("Fetching answer file via HTTP failed: {err}"),
            }
        },

    }
    Err(Error::msg("Could not find any answer file!"))
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
        },
    };
    let install_settings: AutoInstSettings = match toml::from_str(raw_install_settings.as_str()) {
        Ok(content) => content,
        Err(err) => {
            error!("Failed to parse '{AUTOINST_MODE_FILE}': {err}");
            return ExitCode::FAILURE;
        },
    };

    let answer = match fetch_answer(&install_settings) {
        Ok(answer) => answer,
        Err(err) => {
            error!("Aborting: {}", err);
            return ExitCode::FAILURE;
        }
    };

    let mut child = match Command::new("proxmox-auto-installer")
        .stdout(Stdio::inherit())
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => panic!("Failed to start automatic installation: {err}"),
    };

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    std::thread::spawn(move || {
        stdin
            .write_all(answer.as_bytes())
            .expect("Failed to write to stdin");
    });

    match child.wait() {
        Ok(status) => {
            if status.success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE // Will be trapped
            }
        }
        Err(err) => {
            error!("Auto installer exited: {err}");
            ExitCode::FAILURE
        }
    }
}
