use anyhow::{bail, format_err, Result};
use log::{error, info, LevelFilter};
use std::{
    env,
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::ExitCode,
};

use proxmox_installer_common::{
    http,
    setup::{
        installer_setup, read_json, spawn_low_level_installer, LocaleInfo, LowLevelMessage,
        RuntimeInfo, SetupInfo,
    },
    FIRST_BOOT_EXEC_MAX_SIZE, FIRST_BOOT_EXEC_NAME, RUNTIME_DIR,
};

use proxmox_auto_installer::{
    answer::{Answer, FirstBootHookInfo, FirstBootHookSourceMode},
    log::AutoInstLogger,
    udevinfo::UdevInfo,
    utils::parse_answer,
};

static LOGGER: AutoInstLogger = AutoInstLogger;

pub fn init_log() -> Result<()> {
    AutoInstLogger::init("/tmp/auto_installer.log")?;
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(LevelFilter::Info))
        .map_err(|err| format_err!(err))
}

fn setup_first_boot_executable(first_boot: &FirstBootHookInfo) -> Result<()> {
    let content = match first_boot.source {
        FirstBootHookSourceMode::FromUrl => {
            if let Some(url) = &first_boot.url {
                info!("Fetching first-boot hook from {url} ..");
                Some(http::get(url, first_boot.cert_fingerprint.as_deref())?)
            } else {
                bail!("first-boot hook source set to URL, but none specified!");
            }
        }
        FirstBootHookSourceMode::FromIso => Some(fs::read_to_string(format!(
            "/cdrom/{FIRST_BOOT_EXEC_NAME}"
        ))?),
    };

    if let Some(content) = content {
        if content.len() > FIRST_BOOT_EXEC_MAX_SIZE {
            bail!(
                "Maximum file size for first-boot executable file is {} MiB",
                FIRST_BOOT_EXEC_MAX_SIZE / 1024 / 1024
            )
        }

        Ok(fs::write(
            format!("/{RUNTIME_DIR}/{FIRST_BOOT_EXEC_NAME}"),
            content,
        )?)
    } else {
        Ok(())
    }
}

fn auto_installer_setup(in_test_mode: bool) -> Result<(Answer, UdevInfo)> {
    let base_path = if in_test_mode { "./testdir" } else { "/" };
    let mut path = PathBuf::from(base_path);

    path.push("run");
    path.push("proxmox-installer");

    let udev_info: UdevInfo = {
        let mut path = path.clone();
        path.push("run-env-udev.json");

        read_json(&path)
            .map_err(|err| format_err!("Failed to retrieve udev info details: {err}"))?
    };

    let answer = Answer::try_from_reader(std::io::stdin().lock())?;

    if let Some(first_boot) = &answer.first_boot {
        setup_first_boot_executable(first_boot)?;
    }

    Ok((answer, udev_info))
}

fn main() -> ExitCode {
    if let Err(err) = init_log() {
        panic!("could not initialize logging: {}", err);
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

    if answer.global.reboot_on_error {
        if let Err(err) = File::create("/run/proxmox-reboot-on-error") {
            error!("failed to create reboot-on-error flag-file: {err}");
        }
    }

    match run_installation(&answer, &locales, &runtime_info, &udevadm_info, &setup_info) {
        Ok(_) => {
            info!("Installation done.");
            ExitCode::SUCCESS
        },
        Err(err) => {
            error!("Installation failed: {err:#}");
            ExitCode::FAILURE
        }
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

    let mut inner = || -> Result<()> {
        let reader = child
            .stdout
            .take()
            .map(BufReader::new)
            .ok_or(format_err!("failed to get stdout reader"))?;
        let mut writer = child
            .stdin
            .take()
            .ok_or(format_err!("failed to get stdin writer"))?;

        serde_json::to_writer(&mut writer, &config)
            .map_err(|err| format_err!("failed to serialize install config: {err}"))?;
        writeln!(writer).map_err(|err| format_err!("failed to write install config: {err}"))?;

        let mut lowlevel_log = File::create("/tmp/install-low-level.log")
            .map_err(|err| format_err!("failed to open low-level installer logfile: {err}"))?;

        let mut last_progress_percentage = 0.;
        let mut last_progress_text = None;

        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(_) => break,
            };

            // The low-level installer also spews the output of any command it runs on its
            // stdout. Use a very simple heuricstic to determine whether it is actually JSON
            // or not.
            if !line.starts_with('{') || !line.ends_with('}') {
                let _ = writeln!(lowlevel_log, "{}", line);
                continue;
            }

            let msg = match serde_json::from_str::<LowLevelMessage>(&line) {
                Ok(msg) => msg,
                Err(err) => {
                    // Not a fatal error, so don't abort the installation by returning
                    eprintln!("low-level installer: error while parsing message: '{err}'");
                    eprintln!("    original message was: '{line}'");
                    continue;
                }
            };

            match msg.clone() {
                LowLevelMessage::Info { message } => info!("{message}"),
                LowLevelMessage::Error { message } => error!("{message}"),
                LowLevelMessage::Prompt { query } => {
                    bail!("Got interactive prompt I cannot answer: {query}")
                }
                LowLevelMessage::Progress { ratio, text } => {
                    let percentage = ratio * 100.;
                    if let Some(text) = text {
                        info!("progress {percentage:>5.1} % - {text}");
                        last_progress_percentage = percentage;
                        last_progress_text = Some(text);
                    } else if (percentage - last_progress_percentage) > 0.1 {
                        if let Some(text) = &last_progress_text {
                            info!("progress {percentage:>5.1} % - {text}");
                        } else {
                            info!("progress {percentage:>5.1} %");
                        }

                        last_progress_percentage = percentage;
                    }
                }
                LowLevelMessage::Finished { state, message } => {
                    if state == "err" {
                        bail!("{message}");
                    }
                    info!("Finished: '{state}' {message}");
                }
            };
        }
        Ok(())
    };
    inner().map_err(|err| format_err!("low level installer returned early: {err}"))
}
