pub mod log_level;
pub mod run_header;

pub use log_level::{get_log_level, set_log_level, LogLevel};
pub use run_header::RunHeader;

use crate::journal::{rotation, FileWriter, Journal, LogPolicy, MultiWriter, RotatingFileWriter, StdoutWriter};
use crate::render::{ConsoleStyle, FileStyle};
use parking_lot::Mutex;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
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

    // File sink configuration
    file_enabled: Mutex<bool>,
    file_path: Mutex<Option<std::path::PathBuf>>,

    // Rotation policy; None means use plain FileWriter (no rotation).
    file_policy: Mutex<Option<LogPolicy>>,
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
        file_policy: Mutex::new(None),
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

/// Enable file output (append) using canonical detailed format, no rotation.
///
/// File output is intended for audit/post-mortem. It remains enabled across
/// console config changes (because file config is stored in `Runtime`).
pub fn enable_file_output(path: impl AsRef<Path>) -> io::Result<()> {
    *rt().file_enabled.lock() = true;
    *rt().file_path.lock() = Some(path.as_ref().to_path_buf());
    *rt().file_policy.lock() = None;
    rebuild_writers();
    Ok(())
}

/// Enable file output with automatic rotation.
///
/// - If the existing log file is at or over `policy.max_bytes` it is rotated
///   immediately before the writer opens.
/// - If `header` is `Some`, a decorated header line is written as raw bytes
///   into the (possibly fresh) file before structured records begin. A blank
///   separator line is inserted automatically when appending to an existing
///   non-empty file.
///
/// # Example
///
/// ```rust
/// eventline::enable_file_output_rotating(
///     "logs/app.log",
///     LogPolicy::default(),
///     Some(RunHeader::new("my-daemon")),
/// )?;
/// ```
pub fn enable_file_output_rotating(
    path: impl AsRef<Path>,
    policy: LogPolicy,
    header: Option<RunHeader>,
) -> io::Result<()> {
    let path = path.as_ref();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Determine whether we need to rotate and whether to insert a blank
    // separator before the header.
    let needs_separator = match fs::metadata(path) {
        Ok(m) if m.len() >= policy.max_bytes => {
            rotation::rotate(path, policy.keep_backups)?;
            false // fresh file after rotation — no separator needed
        }
        Ok(m) if m.len() > 0 => true, // existing content — insert blank line
        _ => false,
    };

    // Write the run header as raw bytes, bypassing the record pipeline.
    if let Some(hdr) = header {
        let mut f = OpenOptions::new().create(true).append(true).open(path)?;
        if needs_separator {
            f.write_all(b"\n")?;
        }
        f.write_all(hdr.render().as_bytes())?;
        f.write_all(b"\n")?;
        f.flush()?;
    }

    *rt().file_enabled.lock() = true;
    *rt().file_path.lock() = Some(path.to_path_buf());
    *rt().file_policy.lock() = Some(policy);
    rebuild_writers();
    Ok(())
}

/// Disable file output (still records in memory).
pub fn disable_file_output() {
    *rt().file_enabled.lock() = false;
    *rt().file_path.lock() = None;
    *rt().file_policy.lock() = None;
    rebuild_writers();
}

/// Disable all writers (still records in memory).
pub fn disable_all_output() {
    *rt().console_enabled.lock() = false;
    *rt().file_enabled.lock() = false;
    *rt().file_path.lock() = None;
    *rt().file_policy.lock() = None;
    rebuild_writers();
}

/// Emit an event into the journal.
/// This is the core primitive macros call.
pub fn emit(kind: crate::core::EventKind, message: String, fields: crate::journal::Fields) {
    let mut j = rt().journal.lock();
    let _ = j.record(kind, message, fields);
}

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
    let _ = rt().journal.lock().exit_scope(id, outcome);
}

/// Flush sinks.
pub fn flush() -> io::Result<()> {
    rt().journal.lock().flush()
}

/// Access snapshots (for saving/export).
pub fn records() -> Vec<crate::core::Record> {
    rt().journal.lock().records()
}

pub fn scopes() -> Vec<crate::core::Scope> {
    rt().journal.lock().scopes()
}

// ---------------------------------------------------------------------------
// Internal writer wiring
// ---------------------------------------------------------------------------

fn rebuild_writers() {
    let console_enabled = *rt().console_enabled.lock();
    let console_color = *rt().console_color.lock();
    let console_duration = *rt().console_duration.lock();
    let console_timestamp = *rt().console_timestamp.lock();

    let file_enabled = *rt().file_enabled.lock();
    let file_path = rt().file_path.lock().clone();
    let file_policy = rt().file_policy.lock().clone();

    let mut mw = MultiWriter::new();

    if console_enabled {
        mw.add(StdoutWriter::with_style(ConsoleStyle {
            color: console_color,
            show_scope: true,
            show_duration: console_duration,
            show_timestamp: console_timestamp,
        }));
    }

    if file_enabled {
        if let Some(path) = file_path {
            let file_style = FileStyle {
                show_timestamp: true,
                show_scope: true,
            };

            match file_policy {
                // Rotating writer — uses the stored LogPolicy.
                Some(policy) => {
                    match RotatingFileWriter::with_style(&path, file_style, policy) {
                        Ok(rfw) => mw.add(rfw),
                        Err(_) => {} // avoid recursion; failure is silent here
                    }
                }
                // Plain writer — no rotation.
                None => {
                    match FileWriter::with_style(&path, file_style) {
                        Ok(fw) => mw.add(fw),
                        Err(_) => {}
                    }
                }
            }
        }
    }

    if mw.is_empty() {
        rt().journal.lock().set_writer(NoopWriter);
    } else {
        rt().journal.lock().set_writer(mw);
    }
}

/// Writer used when output is fully disabled.
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
