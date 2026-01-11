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
pub mod live_log;
pub mod log_level;
pub mod macros;
pub mod tests;

use std::sync::{Arc, LazyLock};
use tokio::sync::{Mutex, RwLock};
use std::collections::HashSet;
use futures::FutureExt;

use crate::event_kind::EventKind;
use crate::id::ScopeId;
use crate::journal::Journal;
use crate::outcome::Outcome;

/// Global runtime singleton.
///
/// Uses RwLock to allow reset() for testing without blocking readers.
static RUNTIME: LazyLock<RwLock<Option<Arc<Runtime>>>> =
    LazyLock::new(|| RwLock::new(None));

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
    written_headers: Mutex<HashSet<ScopeId>>
}

/// Returns a clone of the global runtime [`Arc<Runtime>`].
///
/// Panics if the runtime is not initialized.
///
/// This is useful for advanced operations such as async scopes or direct journal access.
/// For general logging, use the high-level API: [`record`], [`info`], [`warn`], [`error`], [`debug`].
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
///
/// let rt = runtime::get_runtime(); // Arc<Runtime>
/// let mut journal = rt.journal.lock().unwrap();
/// journal.record(None, "Direct log entry");
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
        journal: Mutex::new(Journal::new()),
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

    CURRENT_SCOPE.with(|s| s.set(None));
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
/// ```
/// use eventline::runtime;
///
/// assert!(!runtime::is_initialized());
/// runtime::init();
/// assert!(runtime::is_initialized());
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

/// Record an event with the specified kind and message.
///
/// This is the central fire-and-forget logging function. It does the following:
/// 1. Checks the current log level; skips if the event kind is below the threshold.
/// 2. Records the event in the journal (thread-safe, recovers poisoned mutexes).
/// 3. Prints the event to the console if console output is enabled.
/// 4. Appends the event to the live log file if live logging is enabled.
///
/// The event is automatically associated with the current thread's active scope, if any.
///
/// # Examples
///
/// ```
/// use eventline::runtime;
/// use eventline::EventKind;
///
/// runtime::init();
/// runtime::enable_console_output(true);
///
/// runtime::record(EventKind::Info, "Application started");
/// ```
pub async fn record(kind: EventKind, message: impl Into<String>) {
    if !log_level::log_enabled(kind) {
        return;
    }

    let message = message.into();

    // --- Journal ---
    if let Some(rt) = RUNTIME.read().await.as_ref() {
        let scope = CURRENT_SCOPE.with(|s| s.get());

        let mut journal = rt.journal.lock().await;
        journal.record_with_kind(scope, kind, &message);
    }

    // --- Console output ---
    if console::is_console_enabled() {
        let _ = std::panic::catch_unwind(|| console::print_event(kind, &message));
    }

    // --- Live log ---
    if let Some(rt) = RUNTIME.read().await.as_ref() {
        let journal = rt.journal.lock().await;

        // Find the current scope
        if let Some(current_scope_id) = journal.scopes().iter().rev()
            .find(|s| s.exited_at.is_none())
            .map(|s| s.id)
        {
            // --- Use written_headers set instead of Journal methods ---
            let mut headers = rt.written_headers.lock().await;
            if !headers.contains(&current_scope_id) {
                if let Some(scope) = journal.get_scope(current_scope_id) {
                    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                    let outcome = journal.scope_outcome(Some(current_scope_id)).unwrap_or(Outcome::Success);
                    let elapsed_secs = scope.elapsed().as_secs_f64();

                    let header = format!(
                        "[{}] Scope {} ({:?}) [{:.3}s]",
                        timestamp,
                        scope.id.0,
                        outcome,
                        elapsed_secs
                    );

                    live_log::append(&header);
                    headers.insert(current_scope_id);
                }
            }

            // Compute prefix based on scope depth
            let scope_prefix = {
                let path_len = journal.scope_path(Some(current_scope_id)).len();
                "  ".repeat(path_len.saturating_sub(1))
            };

            let bullet_line = format!("• {}: {}", kind.as_str().to_lowercase(), message);
            live_log::append(&format!("{}{}", scope_prefix, bullet_line));
        }
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
pub async fn info(message: impl Into<String>) {
    record(EventKind::Info, message).await;
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
pub async fn warn(message: impl Into<String>) {
    record(EventKind::Warning, message).await;
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
pub async fn error(message: impl Into<String>) {
    record(EventKind::Error, message).await;
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
pub async fn debug(message: impl Into<String>) {
    record(EventKind::Debug, message).await;
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
pub async fn scoped<S, F, R>(name: Option<S>, f: F) -> R
where
    S: Into<String>,
    F: FnOnce() -> R,
{
    let runtime_guard = RUNTIME.read().await;
    let rt = runtime_guard
        .as_ref()
        .expect("eventline runtime not initialized - call runtime::init() first");

    // Enter scope in the journal
    let scope_id = {
        let mut journal = rt.journal.lock().await;
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
    let mut journal = rt.journal.lock().await;
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
/// Async version of `scoped`.
///
/// Wraps the async closure in a scope, marking success/aborted automatically.
pub async fn scoped_async<S, F, Fut, R>(name: Option<S>, f: F) -> R
where
    S: Into<String>,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = R>,
{
    let rt = get_runtime().await; // Arc<Runtime> clone
    let scope_id = {
        let mut journal = rt.journal.lock().await;
        let parent = CURRENT_SCOPE.with(|s| s.get());
        journal.enter_scope(parent, name)
    };

    // Set thread-local scope
    let prev_scope = CURRENT_SCOPE.with(|s| {
        let old = s.get();
        s.set(Some(scope_id));
        old
    });

    let result = std::panic::AssertUnwindSafe(f()).catch_unwind().await;

    // Restore previous scope
    CURRENT_SCOPE.with(|s| s.set(prev_scope));

    // Exit scope
    {
        let mut journal = rt.journal.lock().await;
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
pub async fn try_scoped<S, F, R>(name: Option<S>, f: F) -> R
where
    S: Into<String>,
    F: FnOnce() -> R,
{
    let runtime_guard = RUNTIME.read().await;
    
    // If runtime not initialized, just run the closure
    let Some(rt) = runtime_guard.as_ref() else {
        return f();
    };

    // Enter scope in the journal
    let scope_id = {
        let mut journal = rt.journal.lock().await;
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
    let mut journal = rt.journal.lock().await;
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
pub async fn scoped_unnamed<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    scoped::<String, _, _>(None, f).await
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
pub async fn try_scoped_unnamed<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    try_scoped::<String, _, _>(None, f).await
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
