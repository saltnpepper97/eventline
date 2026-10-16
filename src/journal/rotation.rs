use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024; // 5 MiB
pub const DEFAULT_KEEP_BACKUPS: u32 = 5;

/// Controls when and how log files are rotated.
///
/// Rotation scheme: `app.log` → `app.log.1` → `app.log.2` ... up to `keep_backups`.
/// The oldest backup beyond `keep_backups` is silently dropped.
#[derive(Debug, Clone)]
pub struct LogPolicy {
    /// Maximum file size in bytes before rotation is triggered.
    pub max_bytes: u64,
    /// Number of rotated backups to keep. Set to 0 to delete immediately on rotation.
    pub keep_backups: u32,
}

impl Default for LogPolicy {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            keep_backups: DEFAULT_KEEP_BACKUPS,
        }
    }
}

impl LogPolicy {
    pub fn new(max_bytes: u64, keep_backups: u32) -> Self {
        Self {
            max_bytes,
            keep_backups,
        }
    }
}

/// Rotates existing backups and moves the active log file to `.1`.
///
/// If `keep_backups` is 0 the active log is simply deleted.
/// Missing files are harmless. Other removal or rename failures are returned
/// to the caller so rotation cannot silently append to the wrong file.
pub fn rotate(path: &Path, keep_backups: u32) -> io::Result<()> {
    if keep_backups == 0 {
        return ignore_missing(fs::remove_file(path));
    }

    // Shift existing backups up by one slot, dropping the oldest.
    for i in (1..keep_backups).rev() {
        let from = rotated_name(path, i);
        let to = rotated_name(path, i + 1);
        ignore_missing(fs::rename(&from, &to))?;
    }

    let first = rotated_name(path, 1);
    ignore_missing(fs::rename(path, first))
}

pub fn rotated_name(base: &Path, n: u32) -> PathBuf {
    let mut name = base.as_os_str().to_os_string();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

fn ignore_missing(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
