//! Global runtime for fire-and-forget logging.
//!
//! This module provides a process-wide facade over the pure [`Journal`].
//! It enables:
//! - Fire-and-forget event recording from anywhere in the codebase
//! - Automatic scope tracking per thread
//! - No &mut Journal required at call sites
//! - Safe concurrent access via internal synchronization
//! - Optional dual output: journal + immediate console printing
//!
//! The runtime is optional - [`Journal`] remains usable standalone.
//!
//! # Architecture
//!
//! ```text
//! ┌────────────┐
//! │  Macros    │  event_info!(...), event_scope!(...)
//! └─────┬──────┘
//!       ↓
//! ┌────────────┐
//! │  Runtime   │  (global, thread-safe)
//! └─────┬──────┘
//!       ↓
//! ┌────────────┐  ┌─────────────┐
//! │  Journal   │  │  Console    │
//! └────────────┘  └─────────────┘
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use eventline::runtime;
//! use eventline::EventKind;
//! use eventline::journal::JournalWriter;
//! use std::fs::File;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Initialize once at startup
//!     runtime::init().await;
//!
//!     // Enable dual output (optional)
//!     runtime::enable_console_output(true);
//!
//!     // Record events - they'll be both journaled and printed
//!     runtime::record(EventKind::Info, "Application started");
//!
//!     // Create scoped contexts
//!     runtime::scoped(Some("DatabaseMigration"), || {
//!         runtime::record(EventKind::Info, "Applying migrations");
//!         runtime::record(EventKind::Info, "Migration complete");
//!     }).await;
//!
//!     // Access journal for output
//!     runtime::with_journal(|journal| {
//!         let mut file = File::create("eventline.log").unwrap();
//!         JournalWriter::new().write_to(&mut file, journal).unwrap();
//!     }).await;
//! }
//! ```

pub mod console;
pub mod event;
pub mod live_log;
pub mod log_level;
pub mod scope;
pub mod tests;

pub use console::print_event;
pub use event::{record, info, warn, error, debug};
pub use live_log::{append, enable};
pub use scope::{
    // sync scopes
    current_scope,
    current_scope_sync,
    scoped,
    scoped_unnamed,
    try_scoped,
    try_scoped_unnamed,

    // async scopes
    scoped_async,
    try_scoped_async,
    try_scoped_unnamed_async,
};

use std::sync::{Arc, LazyLock};
use std::collections::HashSet;
use tokio::sync::{Mutex, RwLock};

use crate::Outcome;
use crate::journal::id::ScopeId;
use crate::journal::Journal;

/// Global runtime singleton.
///
/// Uses RwLock to allow reset() for testing without blocking readers.
static RUNTIME: LazyLock<RwLock<Option<Arc<Runtime>>>> =
    LazyLock::new(|| RwLock::new(None));

// Thread-local scope tracking.
// Each thread maintains its own current scope context, allowing
// nested scopes to work naturally across thread boundaries.
tokio::task_local! {
    static CURRENT_SCOPE: Option<ScopeId>;
}


tokio::task_local! {
    pub static PENDING_OUTCOME: std::sync::Arc<tokio::sync::Mutex<Option<Outcome>>>;
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
    written_headers: Mutex<HashSet<ScopeId>>
}

/// Access the global journal for advanced operations.
///
/// Provides a way to interact with the journal directly via a closure.
/// For general logging, prefer the high-level API: [`record`], [`info`], [`warn`], [`error`], [`debug`].
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// runtime::init().await;
///
/// // Access the journal safely
/// runtime::with_journal_mut(|journal| {
///     journal.record(None, "Direct log entry");
/// });
/// # });
/// ```
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
/// After initialization, functions like [`record`], [`scoped`], and console logging are available.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
/// runtime::info("Application started");
/// ```
pub async fn init() {
    let mut guard = RUNTIME.write().await;
    *guard = Some(Arc::new(Runtime {
        journal: Arc::new(Mutex::new(Journal::new())),
        written_headers: Mutex::new(HashSet::new()),
    }));
}

/// Resets the global runtime to an uninitialized state.
///
/// Primarily useful for testing scenarios where a clean runtime state is needed
/// between test runs. Also clears the thread-local current scope.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
/// runtime::info("Test event");
/// runtime::reset(); // resets runtime for next test
/// ```
pub async fn reset() {
    let mut guard = RUNTIME.write().await;
    *guard = None;
}

/// Returns whether the global runtime has been initialized.
///
/// # Returns
///
/// - `true` if the runtime is initialized (i.e., [`init`] has been called)
/// - `false` if the runtime is uninitialized
///
/// # Example
///
/// ```rust,ignore
/// use eventline::runtime;
///
/// #[tokio::main]
/// async fn main() {
///     // Before init, runtime is not initialized
///     assert!(!runtime::is_initialized().await);
///
///     // Initialize the runtime
///     runtime::init().await;
///
///     // Now it is initialized
///     assert!(runtime::is_initialized().await);
/// }
/// ```
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
/// runtime::init();
/// runtime::enable_console_output(true); // Enable real-time console output
///
/// // This will both record in journal AND print to console
/// runtime::info("Server started on port 8080");
/// ```
pub fn enable_console_output(enable: bool) {
    console::enable_console_output(enable);
}

/// Check if console output is currently enabled.
pub fn is_console_enabled() -> bool {
    console::is_console_enabled()
}

/// Enable or disable color output for console events.
///
/// This controls whether ANSI color codes are used when printing events to the console.
/// This only has effect if:
/// 1. The `color` feature is enabled at compile time, AND
/// 2. Console output is enabled via `enable_console_output(true)`
///
/// Without the `color` feature, output is always plain regardless of this setting.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
/// runtime::enable_console_output(true);
/// runtime::enable_console_color(true); // Enable colored output
///
/// runtime::error("This will be red");
/// runtime::warn("This will be yellow");
/// ```
pub fn enable_console_color(enable: bool) {
    console::enable_console_color(enable);
}

/// Check if console color is currently enabled.
pub fn is_console_color_enabled() -> bool {
    console::is_console_color_enabled()
}

/// Access the journal with a read-only closure.
///
/// This allows inspecting the journal state without exposing mutable access.
///
/// Use this to access the journal for rendering, writing to files, or
/// any other read-only operations.
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
/// runtime::info("test event");
///
/// // Access the journal for read-only operations
/// runtime::with_journal(|journal| {
///     println!("Total events: {}", journal.records().len());
///
///     // Use JournalWriter to write to a file
///     let mut file = File::create("eventline.log").unwrap();
///     JournalWriter::new().write_to(&mut file, journal).unwrap();
/// });
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
/// This provides full mutable access to the underlying journal.
/// Use with caution - prefer the high-level API when possible.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
///
/// runtime::with_journal_mut(|journal| {
///     // Direct journal manipulation
///     journal.record(None, "Direct journal access");
///     
///     // Flush buffers
///     let buffer = journal.create_buffer();
///     journal.flush_buffer(buffer);
/// });
/// ```
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
