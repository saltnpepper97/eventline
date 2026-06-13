use crate::core::{Record, Scope};
use crate::journal::rotation::{self, LogPolicy};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    tx: Option<mpsc::SyncSender<Msg>>,
    dropped: Arc<AtomicUsize>,
    // We keep the handle so the thread isn't "detached forever".
    // Dropping joins to avoid leaving a runaway thread at shutdown.
    handle: Option<thread::JoinHandle<()>>,
}

enum Msg {
    Record(Box<Record>, Option<Box<Scope>>),
    Flush(mpsc::Sender<io::Result<()>>),
}

impl AsyncWriter {
    /// Spawn a background thread that owns `inner` and processes messages.
    ///
    /// `queue_cap` bounds memory use; when full, `write_record` drops the event.
    pub fn spawn(inner: Box<dyn Writer + Send>, queue_cap: usize) -> Self {
        Self::spawn_with_dropped_counter(inner, queue_cap, Arc::new(AtomicUsize::new(0)))
    }

    pub(crate) fn spawn_with_dropped_counter(
        inner: Box<dyn Writer + Send>,
        queue_cap: usize,
        dropped: Arc<AtomicUsize>,
    ) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Msg>(queue_cap.max(1));

        let handle = thread::Builder::new()
            .name("eventline-writer".to_string())
            .spawn(move || {
                let mut w = inner;

                while let Ok(msg) = rx.recv() {
                    match msg {
                        Msg::Record(rec, scope) => {
                            // Best-effort: ignore writer errors here to avoid wedging.
                            let _ = w.write_record(&rec, scope.as_deref());
                        }
                        Msg::Flush(reply) => {
                            let res = w.flush();
                            let _ = reply.send(res);
                        }
                    }
                }

                let _ = w.flush();
            })
            .ok();

        Self {
            tx: Some(tx),
            dropped,
            handle,
        }
    }

    pub fn dropped_count(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for AsyncWriter {
    fn drop(&mut self) {
        // Closing the sender lets the worker drain all queued records and exit
        // without needing to enqueue a shutdown message into a possibly full queue.
        drop(self.tx.take());

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
        let rec = Box::new(record.clone());
        let sc = scope.cloned().map(Box::new);

        // Never block the caller: drop if the queue is full.
        let Some(tx) = &self.tx else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        };
        match tx.try_send(Msg::Record(rec, sc)) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) | Err(mpsc::TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let Some(tx) = &self.tx else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "eventline async writer is shut down",
            ));
        };

        tx.send(Msg::Flush(reply_tx)).map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "eventline async writer thread is not available",
            )
        })?;

        match reply_rx.recv() {
            Ok(res) => res,
            Err(_) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "eventline async writer did not acknowledge flush",
            )),
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
        if line.is_empty() {
            return Ok(());
        }
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
        let file = OpenOptions::new().create(true).append(true).open(path)?;

        Ok(Self {
            file,
            style: FileStyle::default(),
        })
    }

    pub fn with_style(path: impl AsRef<Path>, style: FileStyle) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;

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

        let file = OpenOptions::new().create(true).append(true).open(&path)?;

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
        let line = render::render_file(record, scope, self.style);
        let bytes = line.as_bytes();
        let write_len = bytes.len() as u64 + 1;

        if self.policy.max_bytes > 0
            && self.current_size > 0
            && self.current_size.saturating_add(write_len) > self.policy.max_bytes
        {
            self.rotate_and_reopen()?;
        }

        self.file.write_all(bytes)?;
        self.file.write_all(b"\n")?;
        self.current_size += write_len;
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
        Self {
            writers: Vec::new(),
        }
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

#[cfg(test)]
mod tests {
    use super::{AsyncWriter, RotatingFileWriter, Writer};
    use crate::core::{EventKind, Record, RecordId, RecordKind};
    use crate::journal::{Fields, LogPolicy};
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct SlowWriter;

    impl Writer for SlowWriter {
        fn write_record(
            &mut self,
            _record: &crate::core::Record,
            _scope: Option<&crate::core::Scope>,
        ) -> io::Result<()> {
            thread::sleep(Duration::from_millis(5));
            Ok(())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn record(message: &str) -> Record {
        Record {
            id: RecordId(1),
            scope: None,
            time_ns: 0,
            kind: RecordKind::Event {
                kind: EventKind::Info,
                name: message.to_string(),
                fields: Fields::new(),
            },
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("eventline-{name}-{unique}.log"))
    }

    #[test]
    fn async_writer_counts_dropped_records_when_queue_is_full() {
        let mut writer = AsyncWriter::spawn(Box::new(SlowWriter), 1);
        let record = record("queued");

        for _ in 0..100 {
            writer.write_record(&record, None).unwrap();
        }

        assert!(writer.dropped_count() > 0);
    }

    #[test]
    fn rotating_writer_rotates_before_next_write_crosses_limit() {
        let path = temp_path("rotate");
        let rotated = PathBuf::from(format!("{}.1", path.display()));

        {
            let mut writer = RotatingFileWriter::new(&path, LogPolicy::new(15, 1)).unwrap();
            writer.write_record(&record("first"), None).unwrap();
            writer.write_record(&record("second"), None).unwrap();
            writer.flush().unwrap();
        }

        let active = fs::read_to_string(&path).unwrap();
        let backup = fs::read_to_string(&rotated).unwrap();

        assert!(active.contains("second"));
        assert!(!active.contains("first"));
        assert!(backup.contains("first"));

        let _ = fs::remove_file(path);
        let _ = fs::remove_file(rotated);
    }
}
