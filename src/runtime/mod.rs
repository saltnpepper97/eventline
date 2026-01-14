//! Global runtime for fire-and-forget logging.
//!
//! This module provides a process-wide facade over the pure [`Journal`].
//! It enables:
//! - Fire-and-forget event recording from anywhere in the codebase
//! - Automatic scope tracking per async task
//! - No &mut Journal required at call sites
//! - Safe concurrent access via internal synchronization
//! - Optional dual output: journal + immediate console printing
//!
//! # Example
//!
//! ```rust,ignore
//! use eventline::runtime;
//! use eventline::{event_info, scoped_eventline};
//!
//! #[tokio::main]
//! async fn main() {
//!     // Initialize once at startup
//!     runtime::init().await;
//!
//!     // Enable dual output (optional)
//!     runtime::enable_console_output(true);
//!
//!     // Record single events
//!     event_info!("Application started");
//!
//!     // Create scoped contexts
//!     scoped_eventline!("DatabaseMigration", {
//!         runtime::info("Starting migration").await;
//!         runtime::info("Migration complete").await;
//!     });
//! }
//! ```

pub mod event;
pub mod live_log;
pub mod log_level;
pub mod scope;
pub mod tests;

pub use event::{record, info, warn, error, debug};
pub use live_log::{append, enable};
pub use scope::{current_scope, current_scope_sync, scoped_in_place, try_scoped_unnamed, try_scoped_unnamed_async, scoped_unnamed};
pub use log_level::*;

// Note: Other scope functions (scoped, scoped_async, try_scoped, etc.) are kept
// in the scope module for internal use and tests, but not re-exported publicly.
// Users should use the scoped_eventline! macro instead.

use std::io::Write;
use std::sync::{Arc, LazyLock};
use tokio::sync::{Mutex, RwLock};

use crate::render;
use crate::Filter;
use crate::ScopeId;
use crate::Journal;
use crate::render::console;

/// Global runtime singleton.
static RUNTIME: LazyLock<RwLock<Option<Arc<Runtime>>>> =
    LazyLock::new(|| RwLock::new(None));

// Task-local scope tracking for async code
tokio::task_local! {
    static CURRENT_SCOPE: Option<ScopeId>;
}

// Thread-local fallback for synchronous scoped(...) closures.
// This allows a synchronous closure to see the scope immediately.
//
// It's only used by the sync `scoped` function; async code continues to
// use the tokio task-local CURRENT_SCOPE.
thread_local! {
    static THREAD_SCOPE: std::cell::RefCell<Option<ScopeId>> = std::cell::RefCell::new(None);
}

/// The global runtime state.
struct Runtime {
    /// The underlying journal, protected by a mutex for safe concurrent access.
    journal: Arc<Mutex<Journal>>,
}

/// Access the global runtime (panics if not initialized).
async fn get_runtime() -> Arc<Runtime> {
    let runtime_guard = RUNTIME.read().await;
    runtime_guard
        .as_ref()
        .expect("eventline runtime not initialized - call runtime::init() first")
        .clone()
}

/// Initializes the global Eventline runtime.
///
/// Must be called once before using any logging or scoped operations.  
/// If called multiple times, it will reset the previous runtime state.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// runtime::init().await;
/// runtime::info("Application started").await;
/// # });
/// ```
pub async fn init() {
    let mut guard = RUNTIME.write().await;
    *guard = Some(Arc::new(Runtime {
        journal: Arc::new(Mutex::new(Journal::new())),
    }));
}

/// Resets the global runtime to an uninitialized state.
///
/// Primarily useful for testing scenarios where a clean runtime state is needed
/// between test runs.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// runtime::init().await;
/// runtime::info("Test event").await;
/// runtime::reset().await; // resets runtime for next test
/// # });
/// ```
pub async fn reset() {
    let mut guard = RUNTIME.write().await;
    *guard = None;
}

/// Returns whether the global runtime has been initialized.
pub async fn is_initialized() -> bool {
    RUNTIME.read().await.is_some()
}

/// Enable or disable automatic console output for events.
///
/// When enabled, events are printed to the console immediately as they're recorded,
/// in addition to being stored in the journal. This provides "dual output" mode.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// runtime::init().await;
/// runtime::enable_console_output(true);
/// runtime::info("Server started on port 8080").await;
/// # });
/// ```
pub fn enable_console_output(enable: bool) {
    console::enable_console_output(enable);
}

/// Check if console output is currently enabled.
pub fn is_console_enabled() -> bool {
    console::is_console_enabled()
}

/// Enable or disable color output for console events.
pub fn enable_console_color(enable: bool) {
    console::enable_console_color(enable);
}

/// Check if console color is currently enabled.
pub fn is_console_color_enabled() -> bool {
    console::is_console_color_enabled()
}

/// Access the journal with a read-only closure.
///
/// # Example
///
/// ```
/// use eventline::runtime;
/// use eventline::journal::JournalWriter;
/// use std::fs::File;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// runtime::init().await;
/// runtime::info("test event").await;
///
/// runtime::with_journal(|journal| {
///     let mut file = File::create("eventline.log").unwrap();
///     JournalWriter::new().write_to(&mut file, journal).unwrap();
/// }).await;
/// # });
/// ```
pub async fn with_journal<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&Journal) -> R,
{
    let runtime_guard = RUNTIME.read().await;
    if let Some(rt) = runtime_guard.as_ref() {
        let journal = rt.journal.lock().await;
        Some(f(&*journal))
    } else {
        None
    }
}

/// Access the journal with a mutable closure.
///
/// Use with caution - prefer the high-level API when possible.
pub async fn with_journal_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut Journal) -> R,
{
    let runtime_guard = RUNTIME.read().await;
    if let Some(rt) = runtime_guard.as_ref() {
        let mut journal = rt.journal.lock().await;
        Some(f(&mut *journal))
    } else {
        None
    }
}

/// Enable live logging to the given file path.
/// This will create directories if they don't exist.
pub fn enable_live_logging(path: impl Into<std::path::PathBuf>) {
    live_log::enable(path.into());
}

/// Render a journal summary using runtime's current journal.
///
/// Prints to console if console output is enabled, and writes to live log file if enabled.
pub async fn runtime_summary(color: bool, filter: Option<&Filter>, per_scope: bool) {
    let rt_opt = RUNTIME.read().await.clone();
    let Some(rt) = rt_opt else { return };

    let summary_data = {
        let journal = rt.journal.lock().await;
        let default_filter_storage = Filter::default();
        let default_filter = filter.unwrap_or(&default_filter_storage);

        let filtered_scopes: Vec<_> = journal
            .scopes()
            .iter()
            .filter(|s| default_filter.matches_scope(s, &journal))
            .cloned()
            .collect();

        let total_scopes = filtered_scopes.len();
        let total_events = journal
            .records()
            .iter()
            .filter(|r| matches!(r.kind, crate::RecordKind::Event { .. }))
            .filter(|r| default_filter.matches_event(r))
            .count();

        let mut success = 0;
        let mut failure = 0;
        let mut aborted = 0;

        for scope in &filtered_scopes {
            let outcome = journal.records().iter()
                .find(|r| matches!(r.kind, crate::RecordKind::ScopeExit { .. }) && r.scope == Some(scope.id))
                .map(|r| if let crate::RecordKind::ScopeExit { outcome, .. } = r.kind { outcome } else { crate::Outcome::Aborted })
                .unwrap_or(crate::Outcome::Aborted);

            match outcome {
                crate::Outcome::Success => success += 1,
                crate::Outcome::Failure => failure += 1,
                crate::Outcome::Aborted => aborted += 1,
            }
        }

        let total_duration_ms: u64 = filtered_scopes.iter().map(|s| {
            journal.records().iter()
                .find(|r| matches!(r.kind, crate::RecordKind::ScopeExit { .. }) && r.scope == Some(s.id))
                .and_then(|r| {
                    if let crate::RecordKind::ScopeExit { exited_at, .. } = r.kind {
                        Some(exited_at.saturating_sub(s.entered_at))
                    } else { None }
                })
                .unwrap_or(0)
        }).sum();

        (total_scopes, total_events, success, failure, aborted, total_duration_ms, filtered_scopes, journal.clone())
    };

    let (total_scopes, total_events, success, failure, aborted, total_duration_ms, filtered_scopes, journal_snapshot) = summary_data;

    // Console output
    if is_console_enabled() {
        render::render_summary(&journal_snapshot, color, filter, per_scope);
    }

    // Live log output
    let mut buffer = Vec::new();
    let config = render::canonical::RenderConfig::no_color();

    let _ = writeln!(buffer, "Session summary: {} scopes, {} events", total_scopes, total_events);
    let _ = writeln!(buffer, "  Successful scopes: {}", success);
    let _ = writeln!(buffer, "  Failed scopes: {}", failure);
    let _ = writeln!(buffer, "  Aborted scopes: {}", aborted);
    let _ = writeln!(buffer, "  Total duration: {}ms", total_duration_ms);

    if per_scope {
        let _ = writeln!(buffer, "\nPer-scope summary:");
        for scope in &filtered_scopes {
            let scope_header = render::canonical::render_scope_header(&journal_snapshot, scope, &config);
            let _ = writeln!(buffer, "  {}", scope_header.header);
        }
    }

    let text = String::from_utf8_lossy(&buffer);
    for line in text.lines() {
        live_log::append(line);
    }
}

/// Spawn a detached task for fire-and-forget operations like logging.
///
/// This allows logging macros to avoid `.await` by spawning background tasks.
pub fn spawn_detached<F>(future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(future);
}
