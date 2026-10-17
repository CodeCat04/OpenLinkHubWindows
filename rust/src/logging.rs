//! Logger writing to `%APPDATA%\OpenLinkHub\openlinkhub.log` (previous run kept as `.old.log`),
//! stderr, and an in-memory tail shown on the dashboard.

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const TAIL_LINES: usize = 300;

struct Logger {
    file: Mutex<Option<File>>,
    tail: Mutex<VecDeque<String>>,
    level: LevelFilter,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub fn init(debug: bool) {
    let path = crate::config::path("openlinkhub.log");
    let _ = std::fs::rename(&path, crate::config::path("openlinkhub.old.log"));
    let file = OpenOptions::new().create(true).write(true).truncate(true).open(&path).ok();
    let level = if debug { LevelFilter::Debug } else { LevelFilter::Info };
    let logger = LOGGER.get_or_init(|| Logger { file: Mutex::new(file), tail: Mutex::new(VecDeque::new()), level });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}

/// Most recent log lines, oldest first.
pub fn tail() -> Vec<String> {
    LOGGER.get().map(|l| l.tail.lock().unwrap().iter().cloned().collect()).unwrap_or_default()
}

fn timestamp() -> String {
    // UTC wall clock without pulling in a date crate.
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}Z")
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level && metadata.target().starts_with(env!("CARGO_CRATE_NAME"))
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            Level::Error => "ERROR",
            Level::Warn => "WARN ",
            Level::Info => "INFO ",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        };
        let line = format!("{} {level} {}", timestamp(), record.args());
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = writeln!(f, "{line}");
        }
        eprintln!("{line}");
        let mut tail = self.tail.lock().unwrap();
        if tail.len() >= TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = f.flush();
        }
    }
}
