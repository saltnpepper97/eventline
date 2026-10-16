pub mod log_level;
pub mod run_header;
mod scope_context;

pub use log_level::{LogLevel, get_log_level, get_record_level, set_log_level, set_record_level};
pub use run_header::RunHeader;
pub use scope_context::{ScopeContext, WithScopeContext, scope_async, scope_async_result};

use crate::journal::{
    AsyncWriter, FileWriter, Journal, LogPolicy, MultiWriter, RotatingFileWriter, StdoutWriter,
    WriteMode, WriterDiagnostics, WriterState,
};
use crate::render::{ConsoleStyle, FileFormat, FileStyle};
use parking_lot::{Mutex, RwLock};
use std::cell::RefCell;
use std::io;
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
    writer_mode: WriteMode,
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
            writer_mode: WriteMode::BestEffort,
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
    writer_state: Arc<WriterState>,
    config: RwLock<Config>,
    last_cfg: Mutex<Option<Config>>,
    emit_count: AtomicUsize,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn rt() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime {
        journal: Mutex::new(Journal::new()),
        writer: Mutex::new(Box::new(NoopWriter)),
        writer_state: Arc::new(WriterState::default()),
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
    init_sync();
}

/// Initialise the runtime. Idempotent; safe to call multiple times.
pub fn init_sync() {
    let _ = rt();
    let _ = update_writers(|_| {}, None, false);
}

pub fn enable_console_output(enable: bool) {
    let _ = update_writers(|cfg| cfg.console_enabled = enable, None, false);
}

pub fn set_console_level(level: LogLevel) {
    let _ = update_writers(|cfg| cfg.console_level = level, None, false);
}

pub fn enable_console_color(enable: bool) {
    let _ = update_writers(|cfg| cfg.console_color = enable, None, false);
}

pub fn enable_console_scope_labels(enable: bool) {
    let _ = update_writers(|cfg| cfg.console_show_scope = enable, None, false);
}

pub fn enable_console_scope_exits(enable: bool) {
    let _ = update_writers(|cfg| cfg.console_show_scope_exit = enable, None, false);
}

pub fn enable_console_duration(enable: bool) {
    let _ = update_writers(|cfg| cfg.console_duration = enable, None, false);
}

pub fn enable_console_timestamp(enable: bool) {
    let _ = update_writers(|cfg| cfg.console_timestamp = enable, None, false);
}

/// Enable plain file output, preserving the configured file level.
/// Opening errors are returned and retained in [`writer_diagnostics`]. The
/// previous output configuration remains active if setup fails.
pub fn enable_file_output(path: impl AsRef<Path>) -> io::Result<()> {
    update_writers(
        |cfg| {
            cfg.file_enabled = true;
            cfg.file_path = Some(path.as_ref().to_path_buf());
            cfg.file_format = FileFormat::Text;
            cfg.file_policy_max_bytes = None;
            cfg.file_policy_keep_backups = None;
        },
        None,
        true,
    )
}

pub fn set_file_level(level: LogLevel) {
    let _ = update_writers(|cfg| cfg.file_level = level, None, false);
}

pub fn set_file_format(format: FileFormat) {
    let _ = update_writers(|cfg| cfg.file_format = format, None, false);
}

pub fn set_journal_retention(max_records: usize) {
    let mut last = rt().last_cfg.lock();
    let max_records = max_records.max(1);
    rt().config.write().max_journal_records = max_records;
    if let Some(cfg) = last.as_mut() {
        cfg.max_journal_records = max_records;
    }
}

/// Change queue-pressure behavior without adding fields to [`crate::Setup`].
/// Blocking mode waits for queue capacity on the calling thread. It prevents
/// queue-pressure drops, but does not retry failed I/O or guarantee fsync.
/// Use [`flush`] to wait for output and observe background write failures.
///
/// ```
/// eventline::set_writer_mode(eventline::WriteMode::Blocking)?;
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn set_writer_mode(mode: WriteMode) -> io::Result<()> {
    update_writers(|cfg| cfg.writer_mode = mode, None, false)
}

/// Enable JSONL output. Setup failures leave the previous sinks active.
pub fn enable_file_output_jsonl(path: impl AsRef<Path>) -> io::Result<()> {
    update_writers(
        |cfg| {
            cfg.file_enabled = true;
            cfg.file_path = Some(path.as_ref().to_path_buf());
            cfg.file_format = FileFormat::Jsonl;
            cfg.file_policy_max_bytes = None;
            cfg.file_policy_keep_backups = None;
        },
        None,
        true,
    )
}

/// Enable rotating output, returning open, rotation, or header-write errors.
/// Queued output is drained before rotation or header insertion. The configured
/// level and format are preserved; failed setup keeps the previous sinks active.
pub fn enable_file_output_rotating(
    path: impl AsRef<Path>,
    policy: LogPolicy,
    header: Option<RunHeader>,
) -> io::Result<()> {
    update_writers(
        |cfg| {
            cfg.file_enabled = true;
            cfg.file_path = Some(path.as_ref().to_path_buf());
            cfg.file_policy_max_bytes = Some(policy.max_bytes);
            cfg.file_policy_keep_backups = Some(policy.keep_backups);
        },
        header,
        true,
    )
}

pub fn disable_file_output() {
    let _ = update_writers(
        |cfg| {
            cfg.file_enabled = false;
            cfg.file_path = None;
            cfg.file_policy_max_bytes = None;
            cfg.file_policy_keep_backups = None;
        },
        None,
        false,
    );
}

pub fn disable_all_output() {
    let _ = update_writers(
        |cfg| {
            cfg.console_enabled = false;
            cfg.file_enabled = false;
            cfg.file_path = None;
            cfg.file_policy_max_bytes = None;
            cfg.file_policy_keep_backups = None;
        },
        None,
        false,
    );
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

/// Enter a synchronous scope, exiting it when the guard is dropped.
///
/// To hold this guard across `.await`, run the future inside [`scope_async`] or
/// [`ScopeContext::wrap`]. Wrap concurrently polled sibling futures separately.
pub fn scope_guard(name: impl Into<String>) -> crate::core::RuntimeScopeGuard {
    crate::core::RuntimeScopeGuard::enter(name)
}

/// Run synchronous work in a scope whose outcome follows its returned `Result`.
///
/// `Ok` records Success, `Err` records Failure, and unwinding records Failure.
/// The original result is returned without cloning or formatting its contents.
/// Early `return` and `?` inside the closure return from that closure; propagate
/// the returned result at the call site to exit the surrounding function.
///
/// For async work, use [`scope_async_result`]. To choose Aborted explicitly,
/// use [`scope_guard`] with [`crate::RuntimeScopeGuard::aborted`] or
/// [`crate::RuntimeScopeGuard::exit_result`].
///
/// ```
/// fn load_config() -> Result<(), &'static str> {
///     eventline::scope_result("config", || {
///         Err("invalid config")?;
///         Ok(())
///     })
/// }
/// assert_eq!(load_config(), Err("invalid config"));
/// ```
pub fn scope_result<T, E>(
    name: impl Into<String>,
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let mut guard = scope_guard(name);
    let result = operation();
    guard.exit_result(&result);
    result
}

/// Enter a scope in the current thread or installed future context.
///
/// Async operations that keep scopes open across `.await` must be wrapped with
/// [`scope_async`] or [`ScopeContext::wrap`].
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

    remove_scope_from_stack(id);
    if let Some((record, scope)) = payload {
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

/// Wait for queued output. Returns the first write failure since the previous
/// flush, or a flush/worker error. Success does not imply fsync or erase diagnostics.
pub fn flush() -> io::Result<()> {
    rt().writer.lock().flush()
}

pub fn records() -> Vec<crate::core::Record> {
    rt().journal.lock().records()
}

pub fn scopes() -> Vec<crate::core::Scope> {
    rt().journal.lock().scopes()
}

/// Monotonic elapsed time for a retained runtime scope, frozen after it exits.
/// Returns `None` after the scope is cleared or trimmed from the journal.
pub fn scope_elapsed(id: crate::core::ScopeId) -> Option<std::time::Duration> {
    rt().journal.lock().scope_elapsed(id)
}

pub fn records_jsonl() -> Vec<String> {
    rt().journal.lock().records_jsonl()
}

pub fn dropped_writer_records() -> usize {
    rt().writer_state.dropped_count()
}

/// Most recently observed setup, write, flush, or worker failure.
/// Retained across successful operations until [`clear`].
pub fn last_writer_error() -> Option<String> {
    rt().writer_state.snapshot().last_error
}

/// Snapshot cumulative writer health. No extra diagnostic log files are written.
pub fn writer_diagnostics() -> WriterDiagnostics {
    rt().writer_state.snapshot()
}

pub fn clear() {
    let mut writer = rt().writer.lock();
    let _ = writer.flush();
    rt().journal.lock().clear();
    rt().emit_count.store(0, Ordering::Relaxed);
    rt().writer_state.clear();
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

pub(crate) fn apply_setup(s: crate::Setup) -> io::Result<()> {
    use crate::FileSetup;
    let shared_level = s.level.unwrap_or(if s.verbose {
        LogLevel::Debug
    } else {
        LogLevel::Info
    });
    let mut file = s.file;
    let header = match &mut file {
        Some(FileSetup::Rotating { header, .. }) => header.take(),
        _ => None,
    };
    let force = matches!(
        file,
        Some(FileSetup::Plain { .. } | FileSetup::Jsonl { .. } | FileSetup::Rotating { .. })
    );
    let file_format = if matches!(&file, Some(FileSetup::Jsonl { .. })) {
        FileFormat::Jsonl
    } else {
        FileFormat::Text
    };
    update_writers(
        move |cfg| {
            cfg.console_enabled = s.verbose;
            cfg.console_level = s.console_level.unwrap_or(shared_level);
            cfg.file_level = s.file_level.unwrap_or(shared_level);
            if let Some(max_records) = s.journal_retention {
                cfg.max_journal_records = max_records.max(1);
            }
            match file {
                None => {}
                Some(FileSetup::Off) => {
                    cfg.file_enabled = false;
                    cfg.file_path = None;
                    cfg.file_policy_max_bytes = None;
                    cfg.file_policy_keep_backups = None;
                }
                Some(FileSetup::Plain { path } | FileSetup::Jsonl { path }) => {
                    cfg.file_enabled = true;
                    cfg.file_path = Some(path);
                    cfg.file_format = file_format;
                    cfg.file_policy_max_bytes = None;
                    cfg.file_policy_keep_backups = None;
                }
                Some(FileSetup::Rotating { path, policy, .. }) => {
                    cfg.file_enabled = true;
                    cfg.file_path = Some(path);
                    cfg.file_policy_max_bytes = Some(policy.max_bytes);
                    cfg.file_policy_keep_backups = Some(policy.keep_backups);
                }
            }
        },
        header,
        force,
    )
}

fn update_writers(
    change: impl FnOnce(&mut Config),
    header: Option<RunHeader>,
    force: bool,
) -> io::Result<()> {
    // Serialize config updates through installation. Failed candidates are not
    // cached or committed, so repeating the same request can recover.
    let mut last = rt().last_cfg.lock();
    let mut cfg = rt().config.read().clone();
    change(&mut cfg);
    if !force && last.as_ref() == Some(&cfg) {
        return Ok(());
    }

    let mut writer = rt().writer.lock();
    // Drain before opening or rotating files. A past failure must not prevent
    // switching to a healthy sink; the worker already retained its diagnostics.
    let _ = writer.flush();
    let replacement = build_writers(&cfg, header.as_ref()).and_then(|inner| {
        AsyncWriter::try_spawn_with_state(
            inner,
            WRITER_QUEUE_CAP,
            cfg.writer_mode,
            rt().writer_state.clone(),
        )
    });
    let replacement = match replacement {
        Ok(writer) => writer,
        Err(error) => {
            let operation = match (&cfg.file_path, cfg.file_enabled) {
                (Some(path), true) => {
                    format!("configuring output for log file '{}'", path.display())
                }
                _ => "configuring output".to_string(),
            };
            rt().writer_state.record_error(&operation, &error);
            return Err(error);
        }
    };
    *writer = Box::new(replacement);
    set_log_level(cfg.effective_level());
    *rt().config.write() = cfg.clone();
    *last = Some(cfg);
    Ok(())
}

fn build_writers(
    cfg: &Config,
    header: Option<&RunHeader>,
) -> io::Result<Box<dyn crate::journal::Writer + Send>> {
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
            Some(policy) => {
                let mut file = RotatingFileWriter::with_style(path, file_style, policy)?;
                if let Some(header) = header {
                    file.write_header(header.render().as_bytes())?;
                }
                mw.add(FilteredWriter::new(file, cfg.file_level));
            }
            None => mw.add(FilteredWriter::new(
                FileWriter::with_style(path, file_style)?,
                cfg.file_level,
            )),
        }
    }
    if mw.is_empty() {
        Ok(Box::new(NoopWriter))
    } else {
        Ok(Box::new(mw))
    }
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
    static TEST_RUNTIME: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

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
        let _lock = TEST_RUNTIME.lock();
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
        let _lock = TEST_RUNTIME.lock();
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
        let _lock = TEST_RUNTIME.lock();
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
