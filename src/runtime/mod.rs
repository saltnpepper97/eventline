pub mod log_level;
pub mod run_header;

pub use log_level::{LogLevel, get_log_level, get_record_level, set_log_level, set_record_level};
pub use run_header::RunHeader;

use crate::journal::{
    AsyncWriter, FileWriter, Journal, LogPolicy, MultiWriter, RotatingFileWriter, StdoutWriter,
    rotation,
};
use crate::render::{ConsoleStyle, FileFormat, FileStyle};
use parking_lot::{Mutex, RwLock};
use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Default number of records the runtime journal keeps in memory.
const DEFAULT_MAX_JOURNAL_RECORDS: usize = 500;

/// How many `emit` calls between trim passes.
const TRIM_INTERVAL: usize = 128;

/// Bounded queue capacity for the background writer thread.
const WRITER_QUEUE_CAP: usize = 2048;

thread_local! {
    static SCOPE_STACK: RefCell<Vec<crate::core::ScopeId>> = const { RefCell::new(Vec::new()) };
}

// ---------------------------------------------------------------------------
// Config snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Config {
    console_enabled: bool,
    console_level: LogLevel,
    console_color: bool,
    console_show_scope: bool,
    console_show_scope_exit: bool,
    console_duration: bool,
    console_timestamp: bool,

    file_enabled: bool,
    file_level: LogLevel,
    file_path: Option<PathBuf>,
    file_format: FileFormat,

    file_policy_max_bytes: Option<u64>,
    file_policy_keep_backups: Option<u32>,

    max_journal_records: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            console_enabled: true,
            console_level: LogLevel::Info,
            console_color: true,
            console_show_scope: true,
            console_show_scope_exit: true,
            console_duration: false,
            console_timestamp: false,

            file_enabled: false,
            file_level: LogLevel::Debug,
            file_path: None,
            file_format: FileFormat::Text,

            file_policy_max_bytes: None,
            file_policy_keep_backups: None,

            max_journal_records: DEFAULT_MAX_JOURNAL_RECORDS,
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
    writer_dropped: Arc<AtomicUsize>,
    last_writer_error: Mutex<Option<String>>,
    config: RwLock<Config>,
    last_cfg: Mutex<Option<Config>>,
    emit_count: AtomicUsize,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn rt() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        let writer_dropped = Arc::new(AtomicUsize::new(0));
        Runtime {
            journal: Mutex::new(Journal::new()),
            writer: Mutex::new(Box::new(
                crate::journal::AsyncWriter::spawn_with_dropped_counter(
                    Box::new(NoopWriter),
                    WRITER_QUEUE_CAP,
                    writer_dropped.clone(),
                ),
            )),
            writer_dropped,
            last_writer_error: Mutex::new(None),
            config: RwLock::new(Config::default()),
            last_cfg: Mutex::new(None),
            emit_count: AtomicUsize::new(0),
        }
    })
}

// ---------------------------------------------------------------------------
// Public API — initialisation & configuration
// ---------------------------------------------------------------------------

/// Initialise the runtime. Idempotent; safe to call multiple times.
pub async fn init() {
    init_sync();
}

/// Initialise the runtime. Idempotent; safe to call multiple times.
pub fn init_sync() {
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

pub fn enable_console_scope_labels(enable: bool) {
    rt().config.write().console_show_scope = enable;
    rebuild_writers();
}

pub fn enable_console_scope_exits(enable: bool) {
    rt().config.write().console_show_scope_exit = enable;
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
    cfg.file_format = FileFormat::Text;
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

pub fn set_file_format(format: FileFormat) {
    rt().config.write().file_format = format;
    rebuild_writers();
}

pub fn set_journal_retention(max_records: usize) {
    rt().config.write().max_journal_records = max_records.max(1);
}

pub fn enable_file_output_jsonl(path: impl AsRef<Path>) -> io::Result<()> {
    let mut cfg = rt().config.write();
    cfg.file_enabled = true;
    cfg.file_path = Some(path.as_ref().to_path_buf());
    cfg.file_level = LogLevel::Debug;
    cfg.file_format = FileFormat::Jsonl;
    cfg.file_policy_max_bytes = None;
    cfg.file_policy_keep_backups = None;
    drop(cfg);
    rebuild_writers();
    Ok(())
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
    let active_scope = current_scope();
    let (record, scope) = {
        let mut j = rt().journal.lock();
        let (_, record, scope) = j.record_no_write_in_scope(kind, message, fields, active_scope);
        (record, scope)
    };

    let count = rt().emit_count.fetch_add(1, Ordering::Relaxed);
    if count.is_multiple_of(TRIM_INTERVAL) {
        let max_records = rt().config.read().max_journal_records;
        let mut j = rt().journal.lock();
        j.trim_records(max_records);
        j.trim_scopes(max_records);
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
    let parent = current_scope();
    let id = rt().journal.lock().enter_scope_with_parent(name, parent);
    SCOPE_STACK.with(|stack| stack.borrow_mut().push(id));
    id
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
        remove_scope_from_stack(id);
        let _ = rt().writer.lock().write_record(&record, Some(&scope));
    }
}

fn current_scope() -> Option<crate::core::ScopeId> {
    SCOPE_STACK.with(|stack| stack.borrow().last().copied())
}

fn remove_scope_from_stack(id: crate::core::ScopeId) {
    SCOPE_STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        if stack.last().copied() == Some(id) {
            stack.pop();
        } else if let Some(pos) = stack.iter().rposition(|scope_id| *scope_id == id) {
            stack.remove(pos);
        }
    });
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

pub fn records_jsonl() -> Vec<String> {
    rt().journal.lock().records_jsonl()
}

pub fn dropped_writer_records() -> usize {
    rt().writer_dropped.load(Ordering::Relaxed)
}

pub fn last_writer_error() -> Option<String> {
    rt().last_writer_error.lock().clone()
}

pub fn clear() {
    rt().journal.lock().clear();
    rt().emit_count.store(0, Ordering::Relaxed);
    rt().writer_dropped.store(0, Ordering::Relaxed);
    *rt().last_writer_error.lock() = None;
    SCOPE_STACK.with(|stack| stack.borrow_mut().clear());
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

    let mut last_error = None;
    let inner: Box<dyn crate::journal::Writer + Send> = {
        let mut mw = MultiWriter::new();

        if cfg.console_enabled {
            let stdout = StdoutWriter::with_style(ConsoleStyle {
                color: cfg.console_color,
                show_scope: cfg.console_show_scope,
                show_scope_exit: cfg.console_show_scope_exit,
                show_duration: cfg.console_duration,
                show_timestamp: cfg.console_timestamp,
            });
            mw.add(FilteredWriter::new(stdout, cfg.console_level));
        }

        if cfg.file_enabled
            && let Some(path) = &cfg.file_path
        {
            let file_style = FileStyle {
                show_timestamp: true,
                show_scope: true,
                format: cfg.file_format,
            };

            match cfg.file_policy() {
                Some(policy) => match RotatingFileWriter::with_style(path, file_style, policy) {
                    Ok(rfw) => mw.add(FilteredWriter::new(rfw, cfg.file_level)),
                    Err(e) => {
                        last_error = Some(format!(
                            "failed to open rotating log file '{}': {e}",
                            path.display()
                        ))
                    }
                },
                None => match FileWriter::with_style(path, file_style) {
                    Ok(fw) => mw.add(FilteredWriter::new(fw, cfg.file_level)),
                    Err(e) => {
                        last_error =
                            Some(format!("failed to open log file '{}': {e}", path.display()))
                    }
                },
            }
        }

        if mw.is_empty() {
            Box::new(NoopWriter)
        } else {
            Box::new(mw)
        }
    };

    *rt().last_writer_error.lock() = last_error;

    let async_writer = AsyncWriter::spawn_with_dropped_counter(
        inner,
        WRITER_QUEUE_CAP,
        rt().writer_dropped.clone(),
    );
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

#[cfg(test)]
mod tests {
    use super::{Config, LogLevel};
    use crate::core::{EventKind, RecordKind};

    #[test]
    fn effective_level_uses_most_verbose_enabled_sink() {
        let cfg = Config {
            console_enabled: true,
            console_level: LogLevel::Info,
            file_enabled: true,
            file_level: LogLevel::Debug,
            ..Config::default()
        };

        assert_eq!(cfg.effective_level(), LogLevel::Debug);
    }

    #[test]
    fn effective_level_ignores_disabled_sinks() {
        let cfg = Config {
            console_enabled: false,
            file_enabled: true,
            file_level: LogLevel::Warning,
            ..Config::default()
        };

        assert_eq!(cfg.effective_level(), LogLevel::Warning);
    }

    #[test]
    fn filtered_output_still_records() {
        super::clear();
        super::disable_all_output();
        super::set_record_level(LogLevel::Debug);

        crate::debug!("hidden but recorded");

        let records = super::records();
        assert!(records.iter().any(|record| matches!(
            &record.kind,
            RecordKind::Event {
                kind: EventKind::Debug,
                name,
                ..
            } if name == "hidden but recorded"
        )));
    }

    #[test]
    fn nested_runtime_scopes_restore_parent_scope() {
        super::clear();
        super::disable_all_output();
        super::set_record_level(LogLevel::Debug);

        let parent = super::enter_scope("parent");
        crate::info!("parent event");
        let child = super::enter_scope("child");
        crate::info!("child event");
        super::exit_scope(child, crate::core::Outcome::Success);
        crate::info!("parent event after child");
        super::exit_scope(parent, crate::core::Outcome::Success);

        let records = super::records();
        let event_scope = |message: &str| {
            records.iter().find_map(|record| match &record.kind {
                RecordKind::Event { name, .. } if name == message => record.scope,
                _ => None,
            })
        };

        assert_eq!(event_scope("parent event"), Some(parent));
        assert_eq!(event_scope("child event"), Some(child));
        assert_eq!(event_scope("parent event after child"), Some(parent));
    }

    #[test]
    fn runtime_scopes_are_thread_local() {
        super::clear();
        super::disable_all_output();
        super::set_record_level(LogLevel::Debug);

        let first = std::thread::spawn(|| {
            let id = super::enter_scope("thread-a");
            crate::info!("event-a");
            super::exit_scope(id, crate::core::Outcome::Success);
            id
        });
        let second = std::thread::spawn(|| {
            let id = super::enter_scope("thread-b");
            crate::info!("event-b");
            super::exit_scope(id, crate::core::Outcome::Success);
            id
        });

        let first = first.join().unwrap();
        let second = second.join().unwrap();
        let records = super::records();
        let event_scope = |message: &str| {
            records.iter().find_map(|record| match &record.kind {
                RecordKind::Event { name, .. } if name == message => record.scope,
                _ => None,
            })
        };

        assert_eq!(event_scope("event-a"), Some(first));
        assert_eq!(event_scope("event-b"), Some(second));
    }
}
