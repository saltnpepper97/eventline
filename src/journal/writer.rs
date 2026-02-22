use crate::core::{Record, Scope};
use crate::journal::rotation::{self, LogPolicy};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use crate::render::{self, ConsoleStyle, FileStyle};

/// Trait for writing journal records to various outputs.
pub trait Writer: Send + Sync {
    fn write_record(&mut self, record: &Record, scope: Option<&Scope>) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

// ---------------------------------------------------------------------------
// AsyncWriter (background thread)
// ---------------------------------------------------------------------------

/// Background-thread writer wrapper.
///
/// - `write_record()` never blocks on I/O.
/// - Uses a bounded queue; when full, records are dropped to avoid unbounded
///   memory growth.
/// - `flush()` is synchronous (waits for the background thread to flush).
pub struct AsyncWriter {
    tx: mpsc::SyncSender<Msg>,
    // We keep the handle so the thread isn't "detached forever".
    // Dropping joins to avoid leaving a runaway thread at shutdown.
    handle: Option<thread::JoinHandle<()>>,
}

enum Msg {
    Record(Record, Option<Scope>),
    Flush(mpsc::Sender<io::Result<()>>),
    Shutdown,
}

impl AsyncWriter {
    /// Spawn a background thread that owns `inner` and processes messages.
    ///
    /// `queue_cap` bounds memory use; when full, `write_record` drops the event.
    pub fn spawn(inner: Box<dyn Writer + Send>, queue_cap: usize) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Msg>(queue_cap.max(1));

        let handle = thread::Builder::new()
            .name("eventline-writer".to_string())
            .spawn(move || {
                let mut w = inner;

                while let Ok(msg) = rx.recv() {
                    match msg {
                        Msg::Record(rec, scope) => {
                            // Best-effort: ignore writer errors here to avoid wedging.
                            let _ = w.write_record(&rec, scope.as_ref());
                        }
                        Msg::Flush(reply) => {
                            let res = w.flush();
                            let _ = reply.send(res);
                        }
                        Msg::Shutdown => {
                            let _ = w.flush();
                            break;
                        }
                    }
                }

                let _ = w.flush();
            })
            .ok();

        Self {
            tx,
            handle,
        }
    }
}

impl Drop for AsyncWriter {
    fn drop(&mut self) {
        // Best-effort shutdown.
        let _ = self.tx.try_send(Msg::Shutdown);

        if let Some(h) = self.handle.take() {
            // Join to avoid leaving a thread around at process shutdown.
            // If the thread already exited, this is instant.
            let _ = h.join();
        }
    }
}

impl Writer for AsyncWriter {
    fn write_record(&mut self, record: &Record, scope: Option<&Scope>) -> io::Result<()> {
        // Clone into the queue so the background thread owns the data.
        // This is the price paid to keep the hot path non-blocking.
        let rec = record.clone();
        let sc = scope.cloned();

        // Never block the caller: drop if the queue is full.
        let _ = self.tx.try_send(Msg::Record(rec, sc));
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        let (reply_tx, reply_rx) = mpsc::channel();
        // If we can't enqueue flush, treat as success (nothing we can do).
        if self.tx.try_send(Msg::Flush(reply_tx)).is_ok() {
            // Wait for the writer thread to flush.
            match reply_rx.recv() {
                Ok(res) => res,
                Err(_) => Ok(()),
            }
        } else {
            Ok(())
        }
    }
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

pub struct RotatingFileWriter {
    path: PathBuf,
    file: File,
    style: FileStyle,
    policy: LogPolicy,
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
