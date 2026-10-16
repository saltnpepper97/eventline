use crate::core::{Record, Scope};
use crate::journal::rotation::{self, LogPolicy};
use crate::journal::{WriterDiagnostics, WriterState};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

/// Queue-pressure policy for background output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WriteMode {
    /// Keep producers nonblocking; count records dropped when the queue is full.
    #[default]
    BestEffort,
    /// Block the calling thread until queue capacity is available.
    /// This prevents queue-pressure drops; I/O failures remain possible and
    /// are reported through diagnostics and `flush`. This does not call fsync.
    Blocking,
}

/// Background-thread writer wrapper.
///
/// Defaults to a bounded, nonblocking, best-effort queue. Use [`WriteMode::Blocking`]
/// for backpressure. [`Self::diagnostics`] retains failure counts and the latest
/// error in memory. `flush()` waits for queued work and reports the first write
/// failure since the previous flush, even if the underlying flush succeeds.
pub struct AsyncWriter {
    tx: Option<mpsc::SyncSender<Msg>>,
    state: Arc<WriterState>,
    mode: WriteMode,
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
        Self::spawn_with_mode(inner, queue_cap, WriteMode::BestEffort)
    }

    /// Spawn with an explicit queue-pressure policy.
    ///
    /// A thread-start failure is retained in diagnostics and subsequent writes
    /// return an error. Use [`Self::try_spawn_with_mode`] to return it immediately.
    pub fn spawn_with_mode(
        inner: Box<dyn Writer + Send>,
        queue_cap: usize,
        mode: WriteMode,
    ) -> Self {
        let state = Arc::new(WriterState::default());
        match Self::try_spawn_with_state(inner, queue_cap, mode, state.clone()) {
            Ok(writer) => writer,
            Err(error) => {
                state.record_error("starting background writer", &error);
                Self {
                    tx: None,
                    state,
                    mode,
                    handle: None,
                }
            }
        }
    }

    /// Spawn a worker, returning a thread-start error immediately.
    pub fn try_spawn_with_mode(
        inner: Box<dyn Writer + Send>,
        queue_cap: usize,
        mode: WriteMode,
    ) -> io::Result<Self> {
        Self::try_spawn_with_state(inner, queue_cap, mode, Arc::new(WriterState::default()))
    }

    pub(crate) fn try_spawn_with_state(
        inner: Box<dyn Writer + Send>,
        queue_cap: usize,
        mode: WriteMode,
        state: Arc<WriterState>,
    ) -> io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<Msg>(queue_cap.max(1));
        let worker_state = state.clone();

        let handle = thread::Builder::new()
            .name("eventline-writer".to_string())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut w = inner;
                    let mut pending_error = None;
                    while let Ok(msg) = rx.recv() {
                        match msg {
                            Msg::Record(rec, scope) => {
                                if let Err(error) = w.write_record(&rec, scope.as_deref()) {
                                    worker_state.record_error("writing record", &error);
                                    pending_error.get_or_insert(error);
                                }
                            }
                            Msg::Flush(reply) => {
                                let res = w.flush();
                                if let Err(error) = &res {
                                    worker_state.record_error("flushing writer", error);
                                }
                                let res = pending_error.take().map_or(res, Err);
                                let _ = reply.send(res);
                            }
                        }
                    }
                    if let Err(error) = w.flush() {
                        worker_state.record_error("flushing writer at shutdown", &error);
                    }
                }));
                if result.is_err() {
                    worker_state
                        .record_error("background writer", &io::Error::other("worker panicked"));
                }
            })?;

        Ok(Self {
            tx: Some(tx),
            state,
            mode,
            handle: Some(handle),
        })
    }

    pub fn dropped_count(&self) -> usize {
        self.state.dropped_count()
    }

    pub fn diagnostics(&self) -> WriterDiagnostics {
        self.state.snapshot()
    }

    fn unavailable(&self, message: &str) -> io::Error {
        let error = io::Error::new(io::ErrorKind::BrokenPipe, message.to_string());
        self.state.record_error("background writer queue", &error);
        error
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
        let rec = Box::new(record.clone());
        let sc = scope.cloned().map(Box::new);

        let Some(tx) = &self.tx else {
            self.state.drop_record();
            return Err(self.unavailable("eventline async writer is shut down"));
        };
        if self.mode == WriteMode::Blocking {
            return tx.send(Msg::Record(rec, sc)).map_err(|_| {
                self.state.drop_record();
                self.unavailable("eventline async writer thread is not available")
            });
        }
        match tx.try_send(Msg::Record(rec, sc)) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => {
                self.state.drop_record();
                Ok(())
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.state.drop_record();
                Err(self.unavailable("eventline async writer thread is not available"))
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let Some(tx) = &self.tx else {
            return Err(self.unavailable("eventline async writer is shut down"));
        };

        tx.send(Msg::Flush(reply_tx))
            .map_err(|_| self.unavailable("eventline async writer thread is not available"))?;

        match reply_rx.recv() {
            Ok(res) => res,
            Err(_) => Err(self.unavailable("eventline async writer did not acknowledge flush")),
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
            Ok(m) if m.is_dir() => {
                return Err(io::Error::new(
                    io::ErrorKind::IsADirectory,
                    "log path is a directory",
                ));
            }
            Ok(m) if policy.max_bytes > 0 && m.len() >= policy.max_bytes => {
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

    // Write session metadata through the opened file so it cannot be rotated
    // away by a second constructor pass. Headers intentionally bypass limits.
    pub(crate) fn write_header(&mut self, header: &[u8]) -> io::Result<()> {
        if self.current_size > 0 {
            self.file.write_all(b"\n")?;
            self.current_size += 1;
        }
        self.file.write_all(header)?;
        self.file.write_all(b"\n")?;
        self.current_size = self.current_size.saturating_add(header.len() as u64 + 1);
        self.file.flush()
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
        let mut first_error = None;
        for w in &mut self.writers {
            if let Err(error) = w.write_record(record, scope) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut first_error = None;
        for w in &mut self.writers {
            if let Err(error) = w.flush() {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::{AsyncWriter, MultiWriter, RotatingFileWriter, WriteMode, Writer};
    use crate::core::{EventKind, Record, RecordId, RecordKind};
    use crate::journal::{Fields, LogPolicy};
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
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

    struct FailingWriter {
        write: bool,
        flush: bool,
    }

    impl Writer for FailingWriter {
        fn write_record(&mut self, _: &Record, _: Option<&crate::core::Scope>) -> io::Result<()> {
            if self.write {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "write denied",
                ))
            } else {
                Ok(())
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.flush {
                Err(io::Error::other("flush failed"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn async_flush_reports_prior_write_failures_and_diagnostics_stay_sticky() {
        let mut writer = AsyncWriter::spawn(
            Box::new(FailingWriter {
                write: true,
                flush: false,
            }),
            8,
        );
        writer.write_record(&record("first"), None).unwrap();
        writer.write_record(&record("second"), None).unwrap();
        assert_eq!(
            writer.flush().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        let diagnostics = writer.diagnostics();
        assert_eq!(diagnostics.failed_operations, 2);
        assert_eq!(diagnostics.dropped_records, 0);
        assert!(
            diagnostics
                .last_error
                .unwrap()
                .contains("writing record: write denied")
        );
        writer.flush().unwrap();
        assert_eq!(writer.diagnostics().failed_operations, 2);
    }

    #[test]
    fn flush_and_shutdown_failures_are_counted() {
        let mut writer = AsyncWriter::spawn(
            Box::new(FailingWriter {
                write: false,
                flush: true,
            }),
            1,
        );
        assert!(
            writer
                .flush()
                .unwrap_err()
                .to_string()
                .contains("flush failed")
        );
        assert_eq!(writer.diagnostics().failed_operations, 1);
        let state = writer.state.clone();
        drop(writer);
        assert_eq!(state.snapshot().failed_operations, 2);
        assert!(state.snapshot().last_error.unwrap().contains("shutdown"));
    }

    struct CountingWriter {
        writes: Arc<AtomicUsize>,
        flushes: Arc<AtomicUsize>,
    }
    impl Writer for CountingWriter {
        fn write_record(&mut self, _: &Record, _: Option<&crate::core::Scope>) -> io::Result<()> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[test]
    fn multi_writer_attempts_every_sink_despite_a_failure() {
        let writes = Arc::new(AtomicUsize::new(0));
        let flushes = Arc::new(AtomicUsize::new(0));
        let mut writer = MultiWriter::new();
        writer.add(FailingWriter {
            write: true,
            flush: true,
        });
        writer.add(CountingWriter {
            writes: writes.clone(),
            flushes: flushes.clone(),
        });
        assert_eq!(
            writer
                .write_record(&record("event"), None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(writer.flush().is_err());
        assert_eq!(writes.load(Ordering::Relaxed), 1);
        assert_eq!(flushes.load(Ordering::Relaxed), 1);
    }

    struct GatedWriter {
        started: mpsc::Sender<()>,
        release: parking_lot::Mutex<mpsc::Receiver<()>>,
        writes: Arc<AtomicUsize>,
    }
    impl Writer for GatedWriter {
        fn write_record(&mut self, _: &Record, _: Option<&crate::core::Scope>) -> io::Result<()> {
            let _ = self.started.send(());
            self.release
                .lock()
                .recv_timeout(Duration::from_secs(5))
                .map_err(io::Error::other)?;
            self.writes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn gated_writer(
        mode: WriteMode,
    ) -> (
        AsyncWriter,
        mpsc::Receiver<()>,
        mpsc::Sender<()>,
        Arc<AtomicUsize>,
    ) {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let writes = Arc::new(AtomicUsize::new(0));
        let writer = AsyncWriter::try_spawn_with_mode(
            Box::new(GatedWriter {
                started: started_tx,
                release: parking_lot::Mutex::new(release_rx),
                writes: writes.clone(),
            }),
            1,
            mode,
        )
        .unwrap();
        (writer, started_rx, release_tx, writes)
    }

    #[test]
    fn best_effort_mode_counts_pressure_drops_without_counting_io_failures() {
        let (mut writer, started, release, writes) = gated_writer(WriteMode::BestEffort);
        writer.write_record(&record("first"), None).unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        writer.write_record(&record("second"), None).unwrap();
        writer.write_record(&record("dropped"), None).unwrap();
        assert_eq!(writer.dropped_count(), 1);
        release.send(()).unwrap();
        release.send(()).unwrap();
        writer.flush().unwrap();
        assert_eq!(writes.load(Ordering::Relaxed), 2);
        assert_eq!(writer.diagnostics().failed_operations, 0);
    }

    #[test]
    fn blocking_mode_waits_for_capacity_and_delivers_every_record() {
        let (mut writer, started, release, writes) = gated_writer(WriteMode::Blocking);
        writer.write_record(&record("first"), None).unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        writer.write_record(&record("second"), None).unwrap();
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let producer = thread::spawn(move || {
            attempt_tx.send(()).unwrap();
            writer.write_record(&record("third"), None).unwrap();
            done_tx.send(()).unwrap();
            writer
        });
        attempt_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release.send(()).unwrap();
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        release.send(()).unwrap();
        release.send(()).unwrap();
        let mut writer = producer.join().unwrap();
        writer.flush().unwrap();
        assert_eq!(writes.load(Ordering::Relaxed), 3);
        assert_eq!(writer.diagnostics(), crate::WriterDiagnostics::default());
    }

    #[test]
    fn worker_panic_and_disconnected_queue_are_observable() {
        struct PanickingWriter;
        impl Writer for PanickingWriter {
            fn write_record(
                &mut self,
                _: &Record,
                _: Option<&crate::core::Scope>,
            ) -> io::Result<()> {
                panic!("test worker panic");
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut writer = AsyncWriter::spawn(Box::new(PanickingWriter), 1);
        writer.write_record(&record("panic"), None).unwrap();
        assert_eq!(
            writer.flush().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            writer
                .write_record(&record("lost"), None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(writer.dropped_count(), 1);
        assert!(writer.diagnostics().failed_operations >= 2);
        assert!(writer.diagnostics().last_error.is_some());
    }
}
