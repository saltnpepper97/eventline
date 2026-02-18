pub mod log_level;
pub mod run_header;

pub use log_level::{get_log_level, set_log_level, LogLevel};

use crate::journal::{FileWriter, Journal, MultiWriter, StdoutWriter};
use crate::render::{ConsoleStyle, FileStyle};
use parking_lot::Mutex;
use std::path::Path;
use std::sync::OnceLock;

/// Global runtime state.
///
/// SOLID boundaries:
/// - `core`: data types only.
/// - `journal`: append-only history + writer integration.
/// - `render`: canonical formatting.
/// - `runtime`: global config + sinks + filtering policy.
///
/// Important invariant:
/// - The journal records the full structured history.
/// - Log level and output toggles only gate *emission* (writers), not recording.
pub struct Runtime {
    journal: Mutex<Journal>,

    // Console sink configuration
    console_enabled: Mutex<bool>,
    console_color: Mutex<bool>,
    console_duration: Mutex<bool>,
    console_timestamp: Mutex<bool>,

    // File sink configuration (kept so console rebuilds don't drop file output)
    file_enabled: Mutex<bool>,
    file_path: Mutex<Option<std::path::PathBuf>>,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn rt() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime {
        journal: Mutex::new(Journal::new()),

        console_enabled: Mutex::new(true),
        console_color: Mutex::new(true),
        console_duration: Mutex::new(false),
        console_timestamp: Mutex::new(false),

        file_enabled: Mutex::new(false),
        file_path: Mutex::new(None),
    })
}

/// Initialize runtime.
///
/// This is idempotent; calling it multiple times is safe.
/// Defaults:
/// - console output enabled
/// - console color enabled
/// - console timestamps disabled
/// - console duration disabled
/// - global log level defaults to Info (in `log_level.rs`)
pub async fn init() {
    let _ = rt();
    rebuild_writers();
}

/// Enable/disable console output.
pub fn enable_console_output(enable: bool) {
    *rt().console_enabled.lock() = enable;
    rebuild_writers();
}

/// Enable/disable ANSI colors in console output.
pub fn enable_console_color(enable: bool) {
    *rt().console_color.lock() = enable;
    rebuild_writers();
}

/// Show/hide duration on console scope-exit lines.
pub fn enable_console_duration(enable: bool) {
    *rt().console_duration.lock() = enable;
    rebuild_writers();
}

/// Show/hide timestamp prefix on console output.
pub fn enable_console_timestamp(enable: bool) {
    *rt().console_timestamp.lock() = enable;
    rebuild_writers();
}

/// Enable file output (append) using canonical detailed format.
///
/// File output is intended for audit/post-mortem. It remains enabled across
/// console config changes (because file config is stored in `Runtime`).
pub fn enable_file_output(path: impl AsRef<Path>) -> std::io::Result<()> {
    *rt().file_enabled.lock() = true;
    *rt().file_path.lock() = Some(path.as_ref().to_path_buf());
    rebuild_writers();
    Ok(())
}

/// Disable file output (still records in memory).
pub fn disable_file_output() {
    *rt().file_enabled.lock() = false;
    *rt().file_path.lock() = None;
    rebuild_writers();
}

/// Disable all writers (still records in memory).
pub fn disable_all_output() {
    *rt().console_enabled.lock() = false;
    *rt().file_enabled.lock() = false;
    *rt().file_path.lock() = None;
    rebuild_writers();
}

/// Emit an event into the journal.
/// This is the core primitive macros call.
pub fn emit(kind: crate::core::EventKind, message: String, fields: crate::journal::Fields) {
    let mut j = rt().journal.lock();
    let _ = j.record(kind, message, fields);
}

/// Enter a runtime scope and return an RAII guard that exits on drop.
///
/// This is async-friendly and does not require `UnwindSafe`.
pub fn scope_guard(name: impl Into<String>) -> crate::core::RuntimeScopeGuard {
    crate::core::RuntimeScopeGuard::enter(name)
}

/// Enter a scope; returns the scope id.
pub fn enter_scope(name: impl Into<String>) -> crate::core::ScopeId {
    rt().journal.lock().enter_scope(name)
}

/// Set per-outcome exit messages for an existing scope.
///
/// These messages only affect the `done:` (ScopeExit) render.
pub fn set_scope_exit_messages(id: crate::core::ScopeId, msgs: crate::core::ExitMessages) {
    rt().journal.lock().set_scope_exit_messages(id, msgs);
}

/// Exit a scope explicitly.
pub fn exit_scope(id: crate::core::ScopeId, outcome: crate::core::Outcome) {
    let _ = rt().journal.lock().exit_scope(id, outcome);
}

/// Flush sinks.
pub fn flush() -> std::io::Result<()> {
    rt().journal.lock().flush()
}

/// Access snapshots (for saving/export).
pub fn records() -> Vec<crate::core::Record> {
    rt().journal.lock().records()
}

pub fn scopes() -> Vec<crate::core::Scope> {
    rt().journal.lock().scopes()
}

// ---------------- internal writer wiring ----------------

fn rebuild_writers() {
    let console_enabled = *rt().console_enabled.lock();
    let console_color = *rt().console_color.lock();
    let console_duration = *rt().console_duration.lock();
    let console_timestamp = *rt().console_timestamp.lock();

    let file_enabled = *rt().file_enabled.lock();
    let file_path = rt().file_path.lock().clone();

    let mut mw = MultiWriter::new();

    // Console sink (simple/human-facing)
    if console_enabled {
        mw.add(StdoutWriter::with_style(ConsoleStyle {
            color: console_color,
            show_scope: true,
            show_duration: console_duration,
            show_timestamp: console_timestamp,
        }));
    }

    // File sink (detailed/audit-friendly)
    if file_enabled {
        if let Some(path) = file_path {
            match FileWriter::with_style(
                path,
                FileStyle {
                    show_timestamp: true,
                    show_scope: true,
                },
            ) {
                Ok(fw) => mw.add(fw),
                Err(_) => {
                    // avoid recursion by not emitting here
                }
            }
        }
    }

    // If no sinks enabled, use a noop writer so journal can still be used uniformly.
    if mw.is_empty() {
        rt().journal.lock().set_writer(NoopWriter);
    } else {
        rt().journal.lock().set_writer(mw);
    }
}

/// Writer used when output is fully disabled.
///
/// Note: this does not affect recording; the journal still stores all records.
struct NoopWriter;

impl crate::journal::Writer for NoopWriter {
    fn write_record(
        &mut self,
        _record: &crate::core::Record,
        _scope: Option<&crate::core::Scope>,
    ) -> std::io::Result<()> {
        Ok(())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
