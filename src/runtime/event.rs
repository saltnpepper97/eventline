use super::{console, RUNTIME, live_log, log_level};

use crate::core::event_kind::EventKind;

/// Record an event with the specified kind and message.
///
/// This is the central fire-and-forget logging function. It does the following:
/// 1. Checks the current log level; skips if the event kind is below the threshold.
/// 2. Records the event in the journal (thread-safe, recovers poisoned mutexes).
/// 3. Prints the event to the console if console output is enabled (simple format).
/// 4. Buffers the event - it will be written to live log when the scope exits.
///
/// The event is automatically associated with the current thread's active scope, if any.
///
/// **Console output uses simple format** (no scope headers, no bullets) because:
/// - Most events come from temporary single-event scopes (`event_info_scoped!`)
/// - Printing full scope headers for each event would be too verbose
/// - Live log file gets the full canonical format with scope headers
///
/// **Live log output is buffered per scope** - events are written only when their
/// scope exits, so the scope header can show the final outcome and duration.
///
/// # Examples
///
/// ```
/// use eventline::runtime;
/// use eventline::core::EventKind;
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

    // Grab a snapshot of runtime
    let rt_opt = RUNTIME.read().await.clone();

    if let Some(rt) = rt_opt {
        // --- Journal ---
        let scope = crate::runtime::scope::current_scope_sync();

        let mut journal = rt.journal.lock().await;
        journal.record_with_kind(scope, kind, &message);

        // --- Live log buffering ---
        // Events are NOT written here - they're buffered in the journal
        // and will be written by runtime/scope.rs when the scope exits
        
        // --- Console output (simple format - no scope headers) ---
        // Console stays simple and immediate
        if console::is_console_enabled() {
            let _ = std::panic::catch_unwind(|| {
                console::print_event(kind, &message);
            });
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
