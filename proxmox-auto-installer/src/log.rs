use anyhow::{Result, bail};
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

    /// Logs to both, stderr and into a log file
    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{}: {}", record.level(), record.args());
            let mut file = LOGFILE
                .get()
                .expect("could not get LOGFILE")
                .lock()
                .expect("could not get mutex for LOGFILE");
            writeln!(file, "{}: {}", record.level(), record.args())
                .expect("could not write to LOGFILE");
        }
    }

    fn flush(&self) {
        LOGFILE
            .get()
            .expect("could not get LOGFILE")
            .lock()
            .expect("could not get mutex for LOGFILE")
            .flush()
            .expect("could not flush LOGFILE");
    }
}
