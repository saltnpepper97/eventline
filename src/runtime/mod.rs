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
//! ```
//! use eventline::runtime;
//! use eventline::EventKind;
//!
//! // Initialize once at startup
//! runtime::init();
//!
//! // Enable dual output (optional)
//! runtime::enable_console_output(true);
//!
//! // Record events - they'll be both journaled and printed
//! runtime::record(EventKind::Info, "Application started");
//!
//! // Create scoped contexts
//! runtime::scoped(Some("DatabaseMigration"), || {
//!     runtime::record(EventKind::Info, "Applying migrations");
//!     runtime::record(EventKind::Info, "Migration complete");
//! });
//!
//! // Access journal for output
//! runtime::with_journal(|journal| {
//!     journal.write_to_file("eventline.log").unwrap();
//! });
//! ```

pub mod console;
pub mod log_level;
pub mod macros;
pub mod tests;

use std::sync::{Mutex, RwLock};

use crate::event_kind::EventKind;
use crate::id::ScopeId;
use crate::journal::Journal;
use crate::outcome::Outcome;

/// Global runtime singleton.
///
/// Uses RwLock to allow reset() for testing without blocking readers.
static RUNTIME: RwLock<Option<Runtime>> = RwLock::new(None);

// Thread-local scope tracking.
// Each thread maintains its own current scope context, allowing
// nested scopes to work naturally across thread boundaries.
thread_local! {
    static CURRENT_SCOPE: std::cell::Cell<Option<ScopeId>> = std::cell::Cell::new(None);
}

/// The global runtime state.
struct Runtime {
    /// The underlying journal, protected by a mutex for safe concurrent access.
    journal: Mutex<Journal>,
}

/// Initialize the global runtime.
///
/// This must be called once before using any runtime functions.
/// Calling this multiple times will reinitialize the runtime,
/// discarding any previous journal state.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// fn main() {
///     runtime::init();
///     
///     // Now you can use runtime::record, runtime::scoped, etc.
/// }
/// ```
pub fn init() {
    let mut runtime = RUNTIME.write().unwrap();
    *runtime = Some(Runtime {
        journal: Mutex::new(Journal::new()),
    });
}

/// Reset the runtime to uninitialized state.
///
/// This is primarily useful for testing, where you need a clean slate
/// between test runs. In production code, this should rarely be needed.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// #[test]
/// fn test_with_clean_state() {
///     runtime::init();
///     // ... test code ...
///     runtime::reset(); // Clean up for next test
/// }
/// ```
pub fn reset() {
    // Clear runtime state
    let mut runtime = RUNTIME.write().unwrap();
    *runtime = None;
    
    // Clear thread-local scope
    CURRENT_SCOPE.with(|s| s.set(None));
}

/// Check if the runtime has been initialized.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// assert!(!runtime::is_initialized());
/// runtime::init();
/// assert!(runtime::is_initialized());
/// ```
pub fn is_initialized() -> bool {
    RUNTIME.read().unwrap().is_some()
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

/// Record an event with the specified kind and message.
///
/// The event is associated with the current thread's active scope, if any.
/// If console output is enabled, the event is also printed immediately.
///
/// If the runtime is not initialized, this is a no-op.
///
/// Handles poisoned mutexes safely.
pub fn record(kind: EventKind, message: impl Into<String>) {
    // Skip events below current log level
    if !log_level::log_enabled(kind) {
        return;
    }

    let message = message.into();

    // Record in journal
    let runtime_guard = RUNTIME.read().unwrap();
    if let Some(rt) = &*runtime_guard {
        let scope = CURRENT_SCOPE.with(|s| s.get());

        // SAFELY lock the journal, even if it was poisoned
        let mut journal = match rt.journal.lock() {
            Ok(j) => j,
            Err(poisoned) => {
                eprintln!("Warning: journal mutex was poisoned, recovering");
                poisoned.into_inner()
            }
        };

        // Record the event
        journal.record_with_kind(scope, kind, &message);
    }

    // Print to console if enabled
    if console::is_console_enabled() {
        console::print_event(kind, &message);
    }
}

/// Record an informational event.
///
/// This is a convenience wrapper around [`record`].
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
/// runtime::info("Request processed successfully");
/// ```
pub fn info(message: impl Into<String>) {
    record(EventKind::Info, message);
}

/// Record a warning event.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
/// runtime::warn("Cache size approaching limit");
/// ```
pub fn warn(message: impl Into<String>) {
    record(EventKind::Warning, message);
}

/// Record an error event.
///
/// Note: This does not automatically fail the current scope.
/// Scope outcomes must be set explicitly via scope exit.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
/// runtime::error("Failed to connect to database");
/// ```
pub fn error(message: impl Into<String>) {
    record(EventKind::Error, message);
}

/// Record a debug event.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
/// runtime::debug("Cache hit for key: user_123");
/// ```
pub fn debug(message: impl Into<String>) {
    record(EventKind::Debug, message);
}

/// Execute a closure within a new scope.
///
/// This automatically:
/// - Enters a scope before executing the closure
/// - Sets it as the current scope for this thread
/// - Records all events from the closure in that scope
/// - Exits the scope after the closure completes
/// - Restores the previous scope context
/// - Handles panics by marking the scope as [`Outcome::Aborted`]
///
/// # Panics
///
/// Panics if the runtime is not initialized.  
/// Panics inside the closure are propagated after the scope is marked `Aborted`.
///
/// # Note
///
/// If a panic occurs, the journal mutex is safely unpoisoned to allow further logging.
pub fn scoped<S, F, R>(name: Option<S>, f: F) -> R
where
    S: Into<String>,
    F: FnOnce() -> R,
{
    let runtime_guard = RUNTIME.read().unwrap();
    let rt = runtime_guard
        .as_ref()
        .expect("eventline runtime not initialized - call runtime::init() first");

    // Enter scope in the journal
    let scope_id = {
        let mut journal = rt.journal.lock().unwrap_or_else(|e| e.into_inner());
        let parent = CURRENT_SCOPE.with(|s| s.get());
        journal.enter_scope(parent, name)
    };

    // Save previous scope and set new current scope
    let prev_scope = CURRENT_SCOPE.with(|s| {
        let old = s.get();
        s.set(Some(scope_id));
        old
    });

    // Execute the closure, catching panics
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

    // Restore previous scope
    CURRENT_SCOPE.with(|s| s.set(prev_scope));

    // Exit scope with appropriate outcome
    let mut journal = rt.journal.lock().unwrap_or_else(|e| e.into_inner());
    match result {
        Ok(value) => {
            journal.exit_scope(scope_id, Outcome::Success);
            value
        }
        Err(panic) => {
            journal.exit_scope(scope_id, Outcome::Aborted);
            std::panic::resume_unwind(panic);
        }
    }
}

/// Execute a closure within a new scope, without panicking if runtime is uninitialized.
///
/// This is a non-panicking variant of [`scoped`]. If the runtime is not initialized,
/// the closure is executed normally without logging. If the runtime is initialized,
/// it behaves identically to [`scoped`].
///
/// # Note
///
/// If a panic occurs, the journal mutex is safely unpoisoned to allow further logging.
pub fn try_scoped<S, F, R>(name: Option<S>, f: F) -> R
where
    S: Into<String>,
    F: FnOnce() -> R,
{
    let runtime_guard = RUNTIME.read().unwrap();
    
    // If runtime not initialized, just run the closure
    let Some(rt) = runtime_guard.as_ref() else {
        return f();
    };

    // Enter scope in the journal
    let scope_id = {
        let mut journal = rt.journal.lock().unwrap_or_else(|e| e.into_inner());
        let parent = CURRENT_SCOPE.with(|s| s.get());
        journal.enter_scope(parent, name)
    };

    // Save previous scope and set new current scope
    let prev_scope = CURRENT_SCOPE.with(|s| {
        let old = s.get();
        s.set(Some(scope_id));
        old
    });

    // Execute the closure, catching panics
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

    // Restore previous scope
    CURRENT_SCOPE.with(|s| s.set(prev_scope));

    // Exit scope with appropriate outcome
    let mut journal = rt.journal.lock().unwrap_or_else(|e| e.into_inner());
    match result {
        Ok(value) => {
            journal.exit_scope(scope_id, Outcome::Success);
            value
        }
        Err(panic) => {
            journal.exit_scope(scope_id, Outcome::Aborted);
            std::panic::resume_unwind(panic);
        }
    }
}

/// Execute a closure within a new unnamed scope.
///
/// This is a convenience wrapper for `scoped(None, f)`.
///
/// # Panics
///
/// Panics if the runtime is not initialized. For a non-panicking variant,
/// use [`try_scoped_unnamed`].
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
///
/// runtime::scoped_unnamed(|| {
///     runtime::info("Anonymous task");
/// });
/// ```
pub fn scoped_unnamed<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    scoped::<String, _, _>(None, f)
}

/// Execute a closure within a new unnamed scope, without panicking if runtime is uninitialized.
///
/// This is a non-panicking variant of [`scoped_unnamed`].
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// // Works even without init()
/// let result = runtime::try_scoped_unnamed(|| {
///     42
/// });
/// assert_eq!(result, 42);
/// ```
pub fn try_scoped_unnamed<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    try_scoped::<String, _, _>(None, f)
}

/// Get the current scope for this thread.
///
/// Returns `None` if no scope is active.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
///
/// assert!(runtime::current_scope().is_none());
///
/// runtime::scoped(Some("test"), || {
///     assert!(runtime::current_scope().is_some());
/// });
/// ```
pub fn current_scope() -> Option<ScopeId> {
    CURRENT_SCOPE.with(|s| s.get())
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
///
/// runtime::init();
/// runtime::info("test event");
///
/// runtime::with_journal(|journal| {
///     println!("Total events: {}", journal.records().len());
///     
///     // Write to file using journal's API
///     journal.write_to_file("eventline.log").unwrap();
/// });
/// ```
pub fn with_journal<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&Journal) -> R,
{
    let runtime_guard = RUNTIME.read().unwrap();
    runtime_guard.as_ref().map(|rt| {
        let journal = rt.journal.lock().unwrap_or_else(|e| e.into_inner());
        f(&*journal)
    })
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
pub fn with_journal_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut Journal) -> R,
{
    let runtime_guard = RUNTIME.read().unwrap();
    runtime_guard.as_ref().map(|rt| {
        let mut journal = rt.journal.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut *journal)
    })
}
