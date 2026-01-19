use crate::core::{Record, Scope};
use std::io::{self, Write};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::Arc;
use parking_lot::Mutex;

use crate::render::{self, ConsoleStyle, FileStyle};

/// Trait for writing journal records to various outputs.
pub trait Writer: Send + Sync {
    fn write_record(&mut self, record: &Record, scope: Option<&Scope>) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

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

/// Writer that appends to a file (detailed / audit-friendly).
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
