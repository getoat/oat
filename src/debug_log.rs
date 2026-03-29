#[cfg(not(test))]
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

#[cfg(not(test))]
use chrono::Utc;

#[cfg(not(test))]
fn log_file() -> &'static Option<Mutex<File>> {
    static LOG_FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    LOG_FILE.get_or_init(|| {
        if !debug_logging_enabled() {
            return None;
        }
        let path = std::env::var_os("OAT_DEBUG_LOG_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp/oat-debug.log"));
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(Mutex::new)
    })
}

#[cfg(not(test))]
fn debug_logging_enabled() -> bool {
    std::env::var("OAT_DEBUG_LOG").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[cfg(not(test))]
pub(crate) fn log_debug(component: &str, message: impl AsRef<str>) {
    let Some(file) = log_file().as_ref() else {
        return;
    };
    let mut file = file.lock().expect("debug log file lock");
    let _ = writeln!(
        file,
        "[{}] [{}] {}",
        Utc::now().to_rfc3339(),
        component,
        message.as_ref()
    );
}

#[cfg(test)]
pub(crate) fn log_debug(_component: &str, _message: impl AsRef<str>) {}
