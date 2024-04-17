use anyhow::{bail, Result};
use log::{Level, Metadata, Record};
use std::{fs::File, io::Write, sync::Mutex, sync::OnceLock};

pub struct AutoInstLogger;
static LOGFILE: OnceLock<Mutex<File>> = OnceLock::new();

impl AutoInstLogger {
    pub fn init(path: &str) -> Result<()> {
        let f = File::create(path)?;
        if LOGFILE.set(Mutex::new(f)).is_err() {
            bail!("Cannot set LOGFILE")
        }
        Ok(())
    }
}

impl log::Log for AutoInstLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info
    }

    /// Logs to stdout without log level and into log file including log level
    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            println!("{}", record.args());
            let mut file = LOGFILE
                .get()
                .expect("could not get LOGFILE")
                .lock()
                .expect("could not get mutex for LOGFILE");
            file.write_all(format!("{} - {}\n", record.level(), record.args()).as_bytes())
                .expect("could not write to LOGFILE");
        }
    }

    fn flush(&self) {}
}
