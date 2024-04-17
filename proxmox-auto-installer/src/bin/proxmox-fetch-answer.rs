use anyhow::{anyhow, Error, Result};
use log::{error, info, LevelFilter};
use proxmox_auto_installer::{fetch_plugins::partition::FetchFromPartition, log::AutoInstLogger};
use std::io::Write;
use std::process::{Command, ExitCode, Stdio};

static LOGGER: AutoInstLogger = AutoInstLogger;

pub fn init_log() -> Result<()> {
    AutoInstLogger::init("/tmp/fetch_answer.log")?;
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(LevelFilter::Info))
        .map_err(|err| anyhow!(err))
}

fn fetch_answer() -> Result<String> {
    match FetchFromPartition::get_answer() {
        Ok(answer) => return Ok(answer),
        Err(err) => info!("Fetching answer file from partition failed: {err}"),
    }
    // TODO: add more options to get an answer file, e.g. download from url where url could be
    // fetched via txt records on predefined subdomain, kernel param, dhcp option, ...

    Err(Error::msg("Could not find any answer file!"))
}

fn main() -> ExitCode {
    if let Err(err) = init_log() {
        panic!("could not initialize logging: {err}");
    }

    info!("Fetching answer file");
    let answer = match fetch_answer() {
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
