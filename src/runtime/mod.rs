pub mod log_level;
pub mod run_header;

pub use log_level::{get_log_level, set_log_level, LogLevel};
pub use run_header::RunHeader;

use crate::journal::{rotation, FileWriter, Journal, LogPolicy, MultiWriter, RotatingFileWriter, StdoutWriter};
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
///
/// Trimming is O(n) so we only do it periodically rather than every call.
/// At 500 records max, trim overhead is negligible; at high log volume, this
/// amortises it across 128 calls instead of paying it every time.
const TRIM_INTERVAL: usize = 128;

// ---------------------------------------------------------------------------
// Config snapshot (all writer-relevant state in one place)
// ---------------------------------------------------------------------------

/// All writer-relevant configuration in a single struct so that:
/// - `rebuild_writers` acquires exactly **one** lock instead of six.
/// - Change detection (`last_cfg`) is a single `PartialEq` compare.
#[derive(Debug, Clone, PartialEq)]
struct Config {
    console_enabled:   bool,
    console_color:     bool,
    console_duration:  bool,
    console_timestamp: bool,

    file_enabled: bool,
    file_path:    Option<PathBuf>,

    /// Rotation policy fields stored separately because `LogPolicy` doesn't
    /// implement `PartialEq`.
    file_policy_max_bytes:    Option<u64>,
    file_policy_keep_backups: Option<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            console_enabled:          true,
            console_color:            true,
            console_duration:         false,
            console_timestamp:        false,
            file_enabled:             false,
            file_path:                None,
            file_policy_max_bytes:    None,
            file_policy_keep_backups: None,
        }
    }
}

impl Config {
    fn file_policy(&self) -> Option<LogPolicy> {
        Some(LogPolicy {
            max_bytes:    self.file_policy_max_bytes?,
            keep_backups: self.file_policy_keep_backups?,
        })
    }
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// Global runtime state.
///
/// ### Lock hierarchy (always acquire in this order to avoid deadlock)
///
/// 1. `config`  (`RwLock<Config>`)   — cheapest; short critical section.
/// 2. `journal` (`Mutex<Journal>`)   — buffer push + scope look-up only.
/// 3. `writer`  (`Mutex<…>`)         — I/O; acquired *after* journal is
///                                      released so other threads can log
///                                      concurrently.
///
/// ### Invariants
///
/// - The journal records the full structured history up to `MAX_JOURNAL_RECORDS`.
/// - Log level and output toggles only gate *emission* (writers), not recording.
/// - Writers are rebuilt lazily: `rebuild_writers` is a no-op when the effective
///   config has not changed since the last rebuild.
pub struct Runtime {
    /// Structured history + scope tracker.
    journal: Mutex<Journal>,

    /// The active writer.  Held in a **separate** mutex from `journal` so that
    /// I/O does not block threads that are pushing records into the buffer.
    writer: Mutex<Box<dyn crate::journal::Writer + Send>>,

    /// All writer-relevant configuration in one lock.
    config: RwLock<Config>,

    /// Snapshot of the config used the last time writers were rebuilt.
    /// Lets `rebuild_writers` skip expensive reconstruction when nothing changed.
    last_cfg: Mutex<Option<Config>>,

    /// Counter used to amortise trim overhead (trim every `TRIM_INTERVAL` emits).
    emit_count: AtomicUsize,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn rt() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime {
        journal:    Mutex::new(Journal::new()),
        writer:     Mutex::new(Box::new(NoopWriter)),
        config:     RwLock::new(Config::default()),
        last_cfg:   Mutex::new(None),
        emit_count: AtomicUsize::new(0),
    })
}

// ---------------------------------------------------------------------------
// Public API — initialisation & configuration
// ---------------------------------------------------------------------------

/// Initialise the runtime.  Idempotent; safe to call multiple times.
pub async fn init() {
    let _ = rt();
    rebuild_writers();
}

/// Enable/disable console output.
pub fn enable_console_output(enable: bool) {
    rt().config.write().console_enabled = enable;
    rebuild_writers();
}

/// Enable/disable ANSI colors in console output.
pub fn enable_console_color(enable: bool) {
    rt().config.write().console_color = enable;
    rebuild_writers();
}

/// Show/hide duration on console scope-exit lines.
pub fn enable_console_duration(enable: bool) {
    rt().config.write().console_duration = enable;
    rebuild_writers();
}

/// Show/hide timestamp prefix on console output.
pub fn enable_console_timestamp(enable: bool) {
    rt().config.write().console_timestamp = enable;
    rebuild_writers();
}

/// Enable file output (append) using canonical detailed format, no rotation.
pub fn enable_file_output(path: impl AsRef<Path>) -> io::Result<()> {
    let mut cfg = rt().config.write();
    cfg.file_enabled             = true;
    cfg.file_path                = Some(path.as_ref().to_path_buf());
    cfg.file_policy_max_bytes    = None;
    cfg.file_policy_keep_backups = None;
    drop(cfg);
    rebuild_writers();
    Ok(())
}

/// Enable file output with automatic rotation.
///
/// - If the existing log file is at or over `policy.max_bytes` it is rotated
///   immediately before the writer opens.
/// - If `header` is `Some`, a decorated header line is written as raw bytes
///   before structured records begin.  A blank separator line is inserted
///   automatically when appending to an existing non-empty file.
pub fn enable_file_output_rotating(
    path:   impl AsRef<Path>,
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
        let mut cfg             = rt().config.write();
        cfg.file_enabled             = true;
        cfg.file_path                = Some(path.to_path_buf());
        cfg.file_policy_max_bytes    = Some(policy.max_bytes);
        cfg.file_policy_keep_backups = Some(policy.keep_backups);
    }
    rebuild_writers();
    Ok(())
}

/// Disable file output (still records in memory).
pub fn disable_file_output() {
    {
        let mut cfg             = rt().config.write();
        cfg.file_enabled             = false;
        cfg.file_path                = None;
        cfg.file_policy_max_bytes    = None;
        cfg.file_policy_keep_backups = None;
    }
    rebuild_writers();
}

/// Disable all writers (still records in memory).
pub fn disable_all_output() {
    {
        let mut cfg             = rt().config.write();
        cfg.console_enabled          = false;
        cfg.file_enabled             = false;
        cfg.file_path                = None;
        cfg.file_policy_max_bytes    = None;
        cfg.file_policy_keep_backups = None;
    }
    rebuild_writers();
}

// ---------------------------------------------------------------------------
// Hot path — emit
// ---------------------------------------------------------------------------

/// Emit an event into the journal.
///
/// ### Performance notes
///
/// - The log-level check happens in the calling macro *before* `format!()` is
///   evaluated, so suppressed messages never allocate a `String`.
/// - The journal mutex is held only for the duration of a buffer push + a
///   scope look-up clone.  It is **released before** the writer is called.
/// - Writer I/O (`Mutex<writer>`) is therefore concurrent with buffer pushes
///   from other threads.
/// - Trim is amortised: the O(n) drain only runs every `TRIM_INTERVAL` calls.
pub fn emit(kind: crate::core::EventKind, message: String, fields: crate::journal::Fields) {
    // --- 1. Push to buffer; hold journal lock as briefly as possible. ---
    let (record, scope) = {
        let mut j = rt().journal.lock();
        let (_, record, scope) = j.record_no_write(kind, message, fields);
        (record, scope)
        // Mutex<Journal> is released here.
    };

    // --- 2. Amortised trim (every TRIM_INTERVAL emits). ---
    let count = rt().emit_count.fetch_add(1, Ordering::Relaxed);
    if count % TRIM_INTERVAL == 0 {
        let mut j = rt().journal.lock();
        j.trim_records(MAX_JOURNAL_RECORDS);
        j.trim_scopes(MAX_JOURNAL_RECORDS);
    }

    // --- 3. Write outside the journal lock. ---
    //
    // We re-check the log level here because `emit` can be called directly
    // (not from a macro), so the macro-side early exit may not have fired.
    if log_level::enabled_for_record(&record) {
        let _ = rt().writer.lock().write_record(&record, scope.as_ref());
    }
}

// ---------------------------------------------------------------------------
// Scope API
// ---------------------------------------------------------------------------

/// Enter a runtime scope and return an RAII guard that exits on drop.
pub fn scope_guard(name: impl Into<String>) -> crate::core::RuntimeScopeGuard {
    crate::core::RuntimeScopeGuard::enter(name)
}

/// Enter a scope; returns the scope id.
pub fn enter_scope(name: impl Into<String>) -> crate::core::ScopeId {
    rt().journal.lock().enter_scope(name)
}

/// Set per-outcome exit messages for an existing scope.
pub fn set_scope_exit_messages(id: crate::core::ScopeId, msgs: crate::core::ExitMessages) {
    rt().journal.lock().set_scope_exit_messages(id, msgs);
}

/// Exit a scope explicitly.
pub fn exit_scope(id: crate::core::ScopeId, outcome: crate::core::Outcome) {
    // Same two-phase approach: push record inside journal lock, write outside.
    let payload = {
        let mut j = rt().journal.lock();
        let (_, payload) = j.exit_scope_no_write(id, outcome);
        payload
        // Mutex<Journal> released.
    };

    if let Some((record, scope)) = payload {
        if log_level::enabled_for_record(&record) {
            let _ = rt().writer.lock().write_record(&record, Some(&scope));
        }
    }
}

// ---------------------------------------------------------------------------
// Misc public API
// ---------------------------------------------------------------------------

/// Flush the active writer.
pub fn flush() -> io::Result<()> {
    rt().writer.lock().flush()
}

/// Snapshot of all in-memory records.
pub fn records() -> Vec<crate::core::Record> {
    rt().journal.lock().records()
}

/// Snapshot of all in-memory scopes.
pub fn scopes() -> Vec<crate::core::Scope> {
    rt().journal.lock().scopes()
}

// ---------------------------------------------------------------------------
// Internal — writer construction
// ---------------------------------------------------------------------------

/// Rebuild the active writer from the current config.
///
/// This is a no-op when the config has not changed since the last rebuild,
/// so it is safe (and cheap) to call after every config setter.
fn rebuild_writers() {
    // Read config with a shared (read) lock — doesn't block other readers.
    let cfg = rt().config.read().clone();

    // Skip rebuild if nothing changed.
    {
        let mut last = rt().last_cfg.lock();
        if last.as_ref() == Some(&cfg) {
            return;
        }
        *last = Some(cfg.clone());
    }

    // Build the new writer outside any hot-path lock.
    let new_writer: Box<dyn crate::journal::Writer + Send> = {
        let mut mw = MultiWriter::new();

        if cfg.console_enabled {
            mw.add(StdoutWriter::with_style(ConsoleStyle {
                color:          cfg.console_color,
                show_scope:     true,
                show_duration:  cfg.console_duration,
                show_timestamp: cfg.console_timestamp,
            }));
        }

        if cfg.file_enabled {
            if let Some(path) = &cfg.file_path {
                let file_style = FileStyle {
                    show_timestamp: true,
                    show_scope:     true,
                };
                match cfg.file_policy() {
                    Some(policy) => {
                        match RotatingFileWriter::with_style(path, file_style, policy) {
                            Ok(rfw) => mw.add(rfw),
                            Err(e)  => eprintln!("[eventline] failed to open rotating log file: {e}"),
                        }
                    }
                    None => {
                        match FileWriter::with_style(path, file_style) {
                            Ok(fw)  => mw.add(fw),
                            Err(e)  => eprintln!("[eventline] failed to open log file: {e}"),
                        }
                    }
                }
            }
        }

        if mw.is_empty() {
            Box::new(NoopWriter)
        } else {
            Box::new(mw)
        }
    };

    // Swap in under the writer lock only.
    *rt().writer.lock() = new_writer;
}

// ---------------------------------------------------------------------------
// NoopWriter
// ---------------------------------------------------------------------------

/// Writer used when output is fully disabled.
struct NoopWriter;

impl crate::journal::Writer for NoopWriter {
    fn write_record(
        &mut self,
        _record: &crate::core::Record,
        _scope:  Option<&crate::core::Scope>,
    ) -> io::Result<()> {
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
