use crate::core::{Record, Scope};
use crate::journal::rotation::{self, LogPolicy};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use parking_lot::Mutex;

use crate::render::{self, ConsoleStyle, FileStyle};

/// Trait for writing journal records to various outputs.
pub trait Writer: Send + Sync {
    fn write_record(&mut self, record: &Record, scope: Option<&Scope>) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

// ---------------------------------------------------------------------------
// StdoutWriter
// ---------------------------------------------------------------------------

/// Writer that outputs to stdout (simple / human-facing).
pub struct StdoutWriter {
    handle: io::Stdout,
    style: ConsoleStyle,
}

impl StdoutWriter {
    pub fn new() -> Self {
        Self {
            handle: io::stdout(),
            style: ConsoleStyle::default(),
        }
    }

    pub fn with_style(style: ConsoleStyle) -> Self {
        Self {
            handle: io::stdout(),
            style,
        }
    }
}

impl Default for StdoutWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl Writer for StdoutWriter {
    fn write_record(&mut self, record: &Record, scope: Option<&Scope>) -> io::Result<()> {
        let line = render::render_console(record, scope, self.style);
        writeln!(self.handle, "{line}")
    }

    fn flush(&mut self) -> io::Result<()> {
        self.handle.flush()
    }
}

// ---------------------------------------------------------------------------
// FileWriter  (simple, non-rotating)
// ---------------------------------------------------------------------------

/// Writer that appends to a file (detailed / audit-friendly).
///
/// Use [`RotatingFileWriter`] if you need automatic log rotation.
pub struct FileWriter {
    file: File,
    style: FileStyle,
}

impl FileWriter {
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        Ok(Self {
            file,
            style: FileStyle::default(),
        })
    }

    pub fn with_style(path: impl AsRef<Path>, style: FileStyle) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        Ok(Self { file, style })
    }
}

impl Writer for FileWriter {
    fn write_record(&mut self, record: &Record, scope: Option<&Scope>) -> io::Result<()> {
        let line = render::render_file(record, scope, self.style);
        writeln!(self.file, "{line}")
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

// ---------------------------------------------------------------------------
// RotatingFileWriter
// ---------------------------------------------------------------------------

/// Writer that appends to a file and rotates it when it exceeds a size limit.
///
/// Rotation happens *before* writing a record that would push the file past
/// `policy.max_bytes`. The active file is renamed to `<path>.1`, older
/// backups are shifted up, and a fresh file is opened.
///
/// The number of retained backups is controlled by `policy.keep_backups`.
/// Setting it to 0 simply deletes the active file on rotation.
///
/// # Note on run headers
///
/// If you want a run header at the top of every fresh log file (including
/// rotated ones), you can write it through [`runtime::write_run_header`] at
/// startup. The header written after rotation is handled automatically when
/// you supply a `RunHeader` to [`runtime::enable_file_output_rotating`].
pub struct RotatingFileWriter {
    path: PathBuf,
    file: File,
    style: FileStyle,
    policy: LogPolicy,
    /// Bytes written to the current file handle (used to avoid a syscall on
    /// every record; we accept a small over-shoot rather than exact tracking).
    current_size: u64,
}

impl RotatingFileWriter {
    pub fn new(path: impl AsRef<Path>, policy: LogPolicy) -> io::Result<Self> {
        Self::with_style(path, FileStyle::default(), policy)
    }

    pub fn with_style(
        path: impl AsRef<Path>,
        style: FileStyle,
        policy: LogPolicy,
    ) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Rotate *before* opening if the file is already at or past the limit.
        let current_size = match fs::metadata(&path) {
            Ok(m) if m.len() >= policy.max_bytes => {
                rotation::rotate(&path, policy.keep_backups)?;
                0
            }
            Ok(m) => m.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e),
        };

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        Ok(Self {
            path,
            file,
            style,
            policy,
            current_size,
        })
    }

    /// Rotate the current file and open a fresh one.
    fn rotate_and_reopen(&mut self) -> io::Result<()> {
        self.file.flush()?;
        rotation::rotate(&self.path, self.policy.keep_backups)?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.current_size = 0;
        Ok(())
    }
}

impl Writer for RotatingFileWriter {
    fn write_record(&mut self, record: &Record, scope: Option<&Scope>) -> io::Result<()> {
        if self.current_size >= self.policy.max_bytes {
            self.rotate_and_reopen()?;
        }

        let line = render::render_file(record, scope, self.style);
        let bytes = line.as_bytes();
        self.file.write_all(bytes)?;
        self.file.write_all(b"\n")?;
        self.current_size += bytes.len() as u64 + 1;
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

// ---------------------------------------------------------------------------
// MultiWriter
// ---------------------------------------------------------------------------

/// Combines multiple writers.
///
/// Semantics:
/// - All writers are attempted in order.
/// - First error stops processing and is returned.
/// - This keeps failure visible and avoids silent partial output.
pub struct MultiWriter {
    writers: Vec<Box<dyn Writer>>,
}

impl MultiWriter {
    pub fn new() -> Self {
        Self { writers: Vec::new() }
    }

    pub fn add(&mut self, writer: impl Writer + 'static) {
        self.writers.push(Box::new(writer));
    }

    pub fn is_empty(&self) -> bool {
        self.writers.is_empty()
    }

    pub fn len(&self) -> usize {
        self.writers.len()
    }
}

impl Default for MultiWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl Writer for MultiWriter {
    fn write_record(&mut self, record: &Record, scope: Option<&Scope>) -> io::Result<()> {
        for w in &mut self.writers {
            w.write_record(record, scope)?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        for w in &mut self.writers {
            w.flush()?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SyncWriter
// ---------------------------------------------------------------------------

/// Thread-safe writer wrapper.
///
/// This allows writers to be used behind shared runtime state without requiring
/// `&mut` access from multiple call sites.
pub struct SyncWriter {
    inner: Arc<Mutex<Box<dyn Writer>>>,
}

impl SyncWriter {
    pub fn new(writer: impl Writer + 'static) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Box::new(writer))),
        }
    }
}

impl Clone for SyncWriter {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Writer for SyncWriter {
    fn write_record(&mut self, record: &Record, scope: Option<&Scope>) -> io::Result<()> {
        self.inner.lock().write_record(record, scope)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.lock().flush()
    }
}
