use super::{console, RUNTIME, live_log, log_level};

use crate::journal::event_kind::EventKind;
use crate::journal::outcome::Outcome;

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

    // Grab a snapshot of runtime
    let rt_opt = RUNTIME.read().await.clone();

    if let Some(rt) = rt_opt {
        // --- Journal ---
        let scope = crate::runtime::scope::current_scope_sync();

        let mut journal = rt.journal.lock().await;
        journal.record_with_kind(scope, kind, &message);

        // --- Live log ---
        if let Some(current_scope_id) = journal.scopes().iter().rev()
            .find(|s| s.exited_at.is_none())
            .map(|s| s.id)
        {
            let mut headers = rt.written_headers.lock().await;
            if !headers.contains(&current_scope_id) {
                if let Some(scope) = journal.get_scope(current_scope_id) {
                    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                    let outcome = journal.scope_outcome(Some(current_scope_id))
                        .unwrap_or(Outcome::Success);
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

            let scope_prefix = {
                let path_len = journal.scope_path(Some(current_scope_id)).len();
                "  ".repeat(path_len.saturating_sub(1))
            };

            let bullet_line = format!("• {}: {}", kind.as_str().to_lowercase(), message);
            live_log::append(&format!("{}{}", scope_prefix, bullet_line));
        }

        // --- Console output ---
        if console::is_console_enabled() {
            let _ = std::panic::catch_unwind(|| console::print_event(kind, &message));
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
