//! Append-only log at `~/.dbclient/log.txt`, rotated to `log.prev.txt` past 1 MB.
//! `DBCLIENT_DEBUG=1` enables debug lines.

use crate::paths::{restrict_file, AppPaths};
use std::sync::{Mutex, OnceLock};

const MAX_LOG_SIZE: u64 = 1024 * 1024;

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub struct Logger {
    paths: AppPaths,
    lock: Mutex<()>,
    verbose: bool,
}

/// Installs the global logger. Later calls are ignored.
pub fn init(paths: AppPaths) {
    let verbose = std::env::var("DBCLIENT_DEBUG").map(|v| v == "1").unwrap_or(false);
    let _ = LOGGER.set(Logger { paths, lock: Mutex::new(()), verbose });
}

pub fn debug(message: impl AsRef<str>) {
    if let Some(l) = LOGGER.get() {
        if l.verbose {
            l.write("DEBUG", message.as_ref());
        }
    }
}
pub fn info(message: impl AsRef<str>) {
    write("INFO", message.as_ref());
}
pub fn warn(message: impl AsRef<str>) {
    write("WARN", message.as_ref());
}
pub fn error(message: impl AsRef<str>) {
    write("ERROR", message.as_ref());
}

fn write(level: &str, message: &str) {
    match LOGGER.get() {
        Some(l) => l.write(level, message),
        None => eprintln!("[{level}] {message}"),
    }
}

impl Logger {
    fn write(&self, level: &str, message: &str) {
        let line = format!("[{}] [{level}] {message}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.paths.ensure_directory().is_err() {
            return;
        }
        self.rotate_if_needed();
        let path = self.paths.log_file();
        let is_new = !path.exists();
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = f.write_all(line.as_bytes());
        }
        if is_new {
            restrict_file(&path);
        }
    }

    fn rotate_if_needed(&self) {
        let path = self.paths.log_file();
        let Ok(meta) = std::fs::metadata(&path) else { return };
        if meta.len() <= MAX_LOG_SIZE {
            return;
        }
        let prev = self.paths.prev_log_file();
        let _ = std::fs::remove_file(&prev);
        if std::fs::rename(&path, &prev).is_ok() {
            restrict_file(&prev);
        }
    }
}
