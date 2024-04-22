use anyhow::{anyhow, bail, Error, Result};
use log::{error, info, LevelFilter};
use std::{
    env,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::ExitCode,
    thread,
    time::Duration,
};

use proxmox_installer_common::setup::{
    installer_setup, read_json, spawn_low_level_installer, LocaleInfo, RuntimeInfo, SetupInfo,
};

use proxmox_auto_installer::{
    answer::Answer,
    log::AutoInstLogger,
    udevinfo::UdevInfo,
    utils::{parse_answer, run_cmds, LowLevelMessage},
};

static LOGGER: AutoInstLogger = AutoInstLogger;

pub fn init_log() -> Result<()> {
    AutoInstLogger::init("/tmp/auto_installer.log")?;
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(LevelFilter::Info))
        .map_err(|err| anyhow!(err))
}

fn auto_installer_setup(in_test_mode: bool) -> Result<(Answer, UdevInfo)> {
    let base_path = if in_test_mode { "./testdir" } else { "/" };
    let mut path = PathBuf::from(base_path);

    path.push("run");
    path.push("proxmox-installer");

    let udev_info: UdevInfo = {
        let mut path = path.clone();
        path.push("run-env-udev.json");

        read_json(&path).map_err(|err| anyhow!("Failed to retrieve udev info details: {err}"))?
    };

    let mut buffer = String::new();
    let lines = std::io::stdin().lock().lines();
    for line in lines {
        buffer.push_str(&line.unwrap());
        buffer.push('\n');
    }

    let answer: Answer =
        toml::from_str(&buffer).map_err(|err| anyhow!("Failed parsing answer file: {err}"))?;

    Ok((answer, udev_info))
}

fn main() -> ExitCode {
    if let Err(err) = init_log() {
        panic!("could not initilize logging: {}", err);
    }

    let in_test_mode = match env::args().nth(1).as_deref() {
        Some("-t") => true,
        // Always force the test directory in debug builds
        _ => cfg!(debug_assertions),
    };
    info!("Starting auto installer");

    let (setup_info, locales, runtime_info) = match installer_setup(in_test_mode) {
        Ok(result) => result,
        Err(err) => {
            error!("Installer setup error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let (answer, udevadm_info) = match auto_installer_setup(in_test_mode) {
        Ok(result) => result,
        Err(err) => {
            error!("Autoinstaller setup error: {err}");
            return ExitCode::FAILURE;
        }
    };

    match run_installation(&answer, &locales, &runtime_info, &udevadm_info, &setup_info) {
        Ok(_) => info!("Installation done."),
        Err(err) => {
            error!("Installation failed: {err}");
            return exit_failure(answer.global.reboot_on_error);
        }
    }

    run_postinstallation(&answer);

    // TODO: (optionally) do a HTTP post with basic system info, like host SSH public key(s) here

    for secs in (0..=5).rev() {
        info!("Installation finished - auto-rebooting in {secs} seconds ..");
        thread::sleep(Duration::from_secs(1));
    }

    ExitCode::SUCCESS
}

/// When we exit with a failure, the installer will not automatically reboot.
/// Default value for reboot_on_error is false
fn exit_failure(reboot_on_error: bool) -> ExitCode {
    if reboot_on_error {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn run_installation(
    answer: &Answer,
    locales: &LocaleInfo,
    runtime_info: &RuntimeInfo,
    udevadm_info: &UdevInfo,
    setup_info: &SetupInfo,
) -> Result<()> {
    let config = parse_answer(answer, udevadm_info, runtime_info, locales, setup_info)?;
    info!("Calling low-level installer");

    let mut child = match spawn_low_level_installer(false) {
        Ok(child) => child,
        Err(err) => {
            bail!("Low level installer could not be started: {}", err);
        }
    };

    let mut cur_counter = 111;
    let mut inner = || -> Result<()> {
        let reader = child
            .stdout
            .take()
            .map(BufReader::new)
            .ok_or(anyhow!("failed to get stdout reader"))?;
        let mut writer = child
            .stdin
            .take()
            .ok_or(anyhow!("failed to get stdin writer"))?;

        serde_json::to_writer(&mut writer, &config)
            .map_err(|err| anyhow!("failed to serialize install config: {err}"))?;
        writeln!(writer).map_err(|err| anyhow!("failed to write install config: {err}"))?;

        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(_) => break,
            };
            let msg = match serde_json::from_str::<LowLevelMessage>(&line) {
                Ok(msg) => msg,
                Err(_) => {
                    // Not a fatal error, so don't abort the installation by returning
                    continue;
                }
            };

            match msg.clone() {
                LowLevelMessage::Info { message } => info!("{message}"),
                LowLevelMessage::Error { message } => error!("{message}"),
                LowLevelMessage::Prompt { query } => {
                    bail!("Got interactive prompt I cannot answer: {query}")
                }
                LowLevelMessage::Progress { ratio, text: _ } => {
                    let counter = (ratio * 100.).floor() as usize;
                    if counter != cur_counter {
                        cur_counter = counter;
                        info!("Progress: {counter:>3}%");
                    }
                }
                LowLevelMessage::Finished { state, message } => {
                    if state == "err" {
                        bail!("{message}");
                    }
                    // Do not print anything if the installation was successful,
                    // as we handle that here ourselves
                }
            };
        }
        Ok(())
    };
    match inner() {
        Err(err) => Err(Error::msg(format!(
            "low level installer returned early: {err}"
        ))),
        _ => Ok(()),
    }
}

fn run_postinstallation(answer: &Answer) {
    if let Some(system) = &answer.system {
        if !system.root_ssh_keys.is_empty() {
            // FIXME: move handling this into the low-level installer and just pass in installation
            // config, as doing parts of the installation/configuration here and parts in the
            // low-level installer is not nice (seemingly spooky actions at a distance).
            info!("Adding root ssh-keys to the installed system ..");
            run_cmds(
                "ssh-key-setup",
                true,
                &[
                    "mkdir -p /target/root/.ssh",
                    &format!(
                        "printf '{}' >>/target/root/.ssh/authorized_keys",
                        system.root_ssh_keys.join("\n"),
                    ),
                ],
            );
        }
    }
}
