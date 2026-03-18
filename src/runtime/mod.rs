pub mod log_level;
pub mod run_header;

pub use log_level::{get_log_level, set_log_level, LogLevel};
pub use run_header::RunHeader;

use crate::journal::{
    rotation, AsyncWriter, FileWriter, Journal, LogPolicy, MultiWriter, RotatingFileWriter,
    StdoutWriter,
};
use crate::render::{ConsoleStyle, FileStyle};
use parking_lot::{Mutex, RwLock};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// Maximum number of records the journal keeps in memory at any one time.
const MAX_JOURNAL_RECORDS: usize = 500;

/// How many `emit` calls between trim passes.
const TRIM_INTERVAL: usize = 128;

/// Bounded queue capacity for the background writer thread.
const WRITER_QUEUE_CAP: usize = 2048;

// ---------------------------------------------------------------------------
// Config snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Config {
    console_enabled: bool,
    console_level: LogLevel,
    console_color: bool,
    console_duration: bool,
    console_timestamp: bool,

    file_enabled: bool,
    file_level: LogLevel,
    file_path: Option<PathBuf>,

    file_policy_max_bytes: Option<u64>,
    file_policy_keep_backups: Option<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            console_enabled: true,
            console_level: LogLevel::Info,
            console_color: true,
            console_duration: false,
            console_timestamp: false,

            file_enabled: false,
            file_level: LogLevel::Debug,
            file_path: None,

            file_policy_max_bytes: None,
            file_policy_keep_backups: None,
        }
    }
}

impl Config {
    fn file_policy(&self) -> Option<LogPolicy> {
        Some(LogPolicy {
            max_bytes: self.file_policy_max_bytes?,
            keep_backups: self.file_policy_keep_backups?,
        })
    }

    fn effective_level(&self) -> LogLevel {
        let mut level = LogLevel::Off;

        if self.console_enabled && self.console_level < level {
            level = self.console_level;
        }

        if self.file_enabled && self.file_level < level {
            level = self.file_level;
        }

        level
    }
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

pub struct Runtime {
    journal: Mutex<Journal>,
    writer: Mutex<Box<dyn crate::journal::Writer + Send>>,
    config: RwLock<Config>,
    last_cfg: Mutex<Option<Config>>,
    emit_count: AtomicUsize,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn rt() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime {
        journal: Mutex::new(Journal::new()),
        writer: Mutex::new(Box::new(crate::journal::AsyncWriter::spawn(
            Box::new(NoopWriter),
            WRITER_QUEUE_CAP,
        ))),
        config: RwLock::new(Config::default()),
        last_cfg: Mutex::new(None),
        emit_count: AtomicUsize::new(0),
    })
}

// ---------------------------------------------------------------------------
// Public API — initialisation & configuration
// ---------------------------------------------------------------------------

/// Initialise the runtime. Idempotent; safe to call multiple times.
pub async fn init() {
    let _ = rt();
    rebuild_writers();
}

pub fn enable_console_output(enable: bool) {
    rt().config.write().console_enabled = enable;
    rebuild_writers();
}

pub fn set_console_level(level: LogLevel) {
    rt().config.write().console_level = level;
    rebuild_writers();
}

pub fn enable_console_color(enable: bool) {
    rt().config.write().console_color = enable;
    rebuild_writers();
}

pub fn enable_console_duration(enable: bool) {
    rt().config.write().console_duration = enable;
    rebuild_writers();
}

pub fn enable_console_timestamp(enable: bool) {
    rt().config.write().console_timestamp = enable;
    rebuild_writers();
}

pub fn enable_file_output(path: impl AsRef<Path>) -> io::Result<()> {
    let mut cfg = rt().config.write();
    cfg.file_enabled = true;
    cfg.file_path = Some(path.as_ref().to_path_buf());
    cfg.file_level = LogLevel::Debug;
    cfg.file_policy_max_bytes = None;
    cfg.file_policy_keep_backups = None;
    drop(cfg);
    rebuild_writers();
    Ok(())
}

pub fn set_file_level(level: LogLevel) {
    rt().config.write().file_level = level;
    rebuild_writers();
}

pub fn enable_file_output_rotating(
    path: impl AsRef<Path>,
    policy: LogPolicy,
    header: Option<RunHeader>,
) -> io::Result<()> {
    let path = path.as_ref();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let needs_separator = match fs::metadata(path) {
        Ok(m) if m.len() >= policy.max_bytes => {
            rotation::rotate(path, policy.keep_backups)?;
            false
        }
        Ok(m) if m.len() > 0 => true,
        _ => false,
    };

    if let Some(hdr) = header {
        let mut f = OpenOptions::new().create(true).append(true).open(path)?;
        if needs_separator {
            f.write_all(b"\n")?;
        }
        f.write_all(hdr.render().as_bytes())?;
        f.write_all(b"\n")?;
        f.flush()?;
    }

    {
        let mut cfg = rt().config.write();
        cfg.file_enabled = true;
        cfg.file_level = LogLevel::Debug;
        cfg.file_path = Some(path.to_path_buf());
        cfg.file_policy_max_bytes = Some(policy.max_bytes);
        cfg.file_policy_keep_backups = Some(policy.keep_backups);
    }
    rebuild_writers();
    Ok(())
}

pub fn disable_file_output() {
    {
        let mut cfg = rt().config.write();
        cfg.file_enabled = false;
        cfg.file_path = None;
        cfg.file_policy_max_bytes = None;
        cfg.file_policy_keep_backups = None;
    }
    rebuild_writers();
}

pub fn disable_all_output() {
    {
        let mut cfg = rt().config.write();
        cfg.console_enabled = false;
        cfg.file_enabled = false;
        cfg.file_path = None;
        cfg.file_policy_max_bytes = None;
        cfg.file_policy_keep_backups = None;
    }
    rebuild_writers();
}

// ---------------------------------------------------------------------------
// Hot path — emit
// ---------------------------------------------------------------------------

pub fn emit(kind: crate::core::EventKind, message: String, fields: crate::journal::Fields) {
    let (record, scope) = {
        let mut j = rt().journal.lock();
        let (_, record, scope) = j.record_no_write(kind, message, fields);
        (record, scope)
    };

    let count = rt().emit_count.fetch_add(1, Ordering::Relaxed);
    if count % TRIM_INTERVAL == 0 {
        let mut j = rt().journal.lock();
        j.trim_records(MAX_JOURNAL_RECORDS);
        j.trim_scopes(MAX_JOURNAL_RECORDS);
    }

    let _ = rt().writer.lock().write_record(&record, scope.as_ref());
}

// ---------------------------------------------------------------------------
// Scope API
// ---------------------------------------------------------------------------

pub fn scope_guard(name: impl Into<String>) -> crate::core::RuntimeScopeGuard {
    crate::core::RuntimeScopeGuard::enter(name)
}

pub fn enter_scope(name: impl Into<String>) -> crate::core::ScopeId {
    rt().journal.lock().enter_scope(name)
}

pub fn set_scope_exit_messages(id: crate::core::ScopeId, msgs: crate::core::ExitMessages) {
    rt().journal.lock().set_scope_exit_messages(id, msgs);
}

pub fn exit_scope(id: crate::core::ScopeId, outcome: crate::core::Outcome) {
    let payload = {
        let mut j = rt().journal.lock();
        let (_, payload) = j.exit_scope_no_write(id, outcome);
        payload
    };

    if let Some((record, scope)) = payload {
        let _ = rt().writer.lock().write_record(&record, Some(&scope));
    }
}

// ---------------------------------------------------------------------------
// Misc public API
// ---------------------------------------------------------------------------

pub fn flush() -> io::Result<()> {
    rt().writer.lock().flush()
}

pub fn records() -> Vec<crate::core::Record> {
    rt().journal.lock().records()
}

pub fn scopes() -> Vec<crate::core::Scope> {
    rt().journal.lock().scopes()
}

// ---------------------------------------------------------------------------
// Internal — filtering helpers
// ---------------------------------------------------------------------------

fn record_enabled_at_level(record: &crate::core::Record, min_level: LogLevel) -> bool {
    match &record.kind {
        crate::core::RecordKind::Event { kind, .. } => {
            let record_level = match kind {
                crate::core::EventKind::Debug => LogLevel::Debug,
                crate::core::EventKind::Info => LogLevel::Info,
                crate::core::EventKind::Warning => LogLevel::Warning,
                crate::core::EventKind::Error => LogLevel::Error,
            };
            record_level >= min_level
        }
        crate::core::RecordKind::ScopeExit { .. } => true,
    }
}

struct FilteredWriter<W> {
    inner: W,
    min_level: LogLevel,
}

impl<W> FilteredWriter<W> {
    fn new(inner: W, min_level: LogLevel) -> Self {
        Self { inner, min_level }
    }
}

impl<W> crate::journal::Writer for FilteredWriter<W>
where
    W: crate::journal::Writer,
{
    fn write_record(
        &mut self,
        record: &crate::core::Record,
        scope: Option<&crate::core::Scope>,
    ) -> io::Result<()> {
        if record_enabled_at_level(record, self.min_level) {
            self.inner.write_record(record, scope)
        } else {
            Ok(())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

// ---------------------------------------------------------------------------
// Internal — writer construction
// ---------------------------------------------------------------------------

fn rebuild_writers() {
    let cfg = rt().config.read().clone();

    {
        let mut last = rt().last_cfg.lock();
        if last.as_ref() == Some(&cfg) {
            return;
        }
        *last = Some(cfg.clone());
    }

    set_log_level(cfg.effective_level());

    let inner: Box<dyn crate::journal::Writer + Send> = {
        let mut mw = MultiWriter::new();

        if cfg.console_enabled {
            let stdout = StdoutWriter::with_style(ConsoleStyle {
                color: cfg.console_color,
                show_scope: true,
                show_duration: cfg.console_duration,
                show_timestamp: cfg.console_timestamp,
            });
            mw.add(FilteredWriter::new(stdout, cfg.console_level));
        }

        if cfg.file_enabled {
            if let Some(path) = &cfg.file_path {
                let file_style = FileStyle {
                    show_timestamp: true,
                    show_scope: true,
                };

                match cfg.file_policy() {
                    Some(policy) => match RotatingFileWriter::with_style(path, file_style, policy) {
                        Ok(rfw) => mw.add(FilteredWriter::new(rfw, cfg.file_level)),
                        Err(e) => eprintln!("[eventline] failed to open rotating log file: {e}"),
                    },
                    None => match FileWriter::with_style(path, file_style) {
                        Ok(fw) => mw.add(FilteredWriter::new(fw, cfg.file_level)),
                        Err(e) => eprintln!("[eventline] failed to open log file: {e}"),
                    },
                }
            }
        }

        if mw.is_empty() {
            Box::new(NoopWriter)
        } else {
            Box::new(mw)
        }
    };

    let async_writer = AsyncWriter::spawn(inner, WRITER_QUEUE_CAP);
    *rt().writer.lock() = Box::new(async_writer);
}

// ---------------------------------------------------------------------------
// NoopWriter
// ---------------------------------------------------------------------------

struct NoopWriter;

impl crate::journal::Writer for NoopWriter {
    fn write_record(
        &mut self,
        _record: &crate::core::Record,
        _scope: Option<&crate::core::Scope>,
    ) -> io::Result<()> {
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
