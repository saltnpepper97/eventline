use std::fs::{OpenOptions, create_dir_all};
use std::io::Write;
use std::path::PathBuf;
use std::sync::RwLock;

/// Live logging file path (thread-safe)
static LIVE_LOG_PATH: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Enable live logging to a file. Will create directories if missing.
pub fn enable(path: PathBuf) {
    if let Some(parent) = path.parent() {
        let _ = create_dir_all(parent);
    }
    *LIVE_LOG_PATH.write().unwrap() = Some(path);
}

/// Write a line to the live log if enabled.
pub fn append(line: &str) {
    if let Some(path) = LIVE_LOG_PATH.read().unwrap().as_ref() {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{}", line);
        }
    }
}
