use super::{console, RUNTIME, log_level};
use crate::EventKind;
use crate::core::value::{Fields, IntoFields};

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
/// # Examples
///
/// ```no_run
/// use eventline::runtime;
/// use eventline::EventKind;
///
/// #[tokio::main]
/// async fn main() {
///     runtime::init().await;
///     runtime::enable_console_output(true);
///
///     eventline::runtime::event::record(
///         EventKind::Info,
///         "Application started",
///     ).await;
/// }
/// ```
pub async fn record(kind: EventKind, message: impl Into<String>) {
    record_with_fields(kind, message, Fields::new()).await;
}

/// Record an event with structured fields.
///
/// This is the core logging function that accepts structured data.
/// Other logging functions are convenience wrappers around this.
///
/// # Examples
///
/// ```no_run
/// use eventline::runtime;
/// use eventline::{EventKind, Fields, Value};
///
/// #[tokio::main]
/// async fn main() {
///     runtime::init().await;
///
///     let mut fields = Fields::new();
///     fields.insert("user_id".into(), Value::from(12345));
///     fields.insert("action".into(), Value::from("login"));
///
///     eventline::runtime::event::record_with_fields(
///         EventKind::Info,
///         "User logged in",
///         fields,
///     ).await;
/// }
/// ```
pub async fn record_with_fields(
    kind: EventKind,
    message: impl Into<String>,
    fields: impl IntoFields,
) {
    if !log_level::log_enabled(kind) {
        return;
    }

    let message = message.into();
    let fields = fields.into_fields();

    // Grab a snapshot of runtime
    let rt_opt = RUNTIME.read().await.clone();
    if let Some(rt) = rt_opt {
        // --- Journal ---
        let scope = crate::runtime::scope::current_scope_sync();
        let mut journal = rt.journal.lock().await;
        journal.record_event(scope, kind, &message, fields.clone());

        // --- Console output (simple format - no scope headers) ---
        if console::is_console_enabled() {
            let _ = std::panic::catch_unwind(|| {
                console::print_event_with_fields(kind, &message, &fields);
            });
        }
    }
}

/// Record an informational event.
pub async fn info(message: impl Into<String>) {
    record(EventKind::Info, message).await;
}

/// Record an informational event with structured fields.
///
/// # Example
///
/// ```no_run
/// use eventline::runtime;
/// use eventline::runtime::event::info_fields;
///
/// #[tokio::main]
/// async fn main() {
///     runtime::init().await;
///
///     info_fields(
///         "User logged in",
///         vec![
///             ("user_id".into(), 12345.into()),
///             ("ip".into(), "192.168.1.1".into()),
///         ],
///     ).await;
/// }
/// ```
pub async fn info_fields(message: impl Into<String>, fields: impl IntoFields) {
    record_with_fields(EventKind::Info, message, fields).await;
}

/// Record a warning event.
pub async fn warn(message: impl Into<String>) {
    record(EventKind::Warning, message).await;
}

/// Record a warning event with structured fields.
pub async fn warn_fields(message: impl Into<String>, fields: impl IntoFields) {
    record_with_fields(EventKind::Warning, message, fields).await;
}

/// Record an error event.
pub async fn error(message: impl Into<String>) {
    record(EventKind::Error, message).await;
}

/// Record an error event with structured fields.
pub async fn error_fields(message: impl Into<String>, fields: impl IntoFields) {
    record_with_fields(EventKind::Error, message, fields).await;
}

/// Record a debug event.
pub async fn debug(message: impl Into<String>) {
    record(EventKind::Debug, message).await;
}

/// Record a debug event with structured fields.
pub async fn debug_fields(message: impl Into<String>, fields: impl IntoFields) {
    record_with_fields(EventKind::Debug, message, fields).await;
}
