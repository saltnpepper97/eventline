use super::{console, RUNTIME, live_log, log_level};

use crate::journal::event_kind::EventKind;
use crate::render::canonical::{render_scope_header, render_event, RenderConfig};

/// Record an event with the specified kind and message.
///
/// This is the central fire-and-forget logging function. It does the following:
/// 1. Checks the current log level; skips if the event kind is below the threshold.
/// 2. Records the event in the journal (thread-safe, recovers poisoned mutexes).
/// 3. Prints the event to the console if console output is enabled (simple format).
/// 4. Appends the event to the live log file if live logging is enabled (using canonical format).
///
/// The event is automatically associated with the current thread's active scope, if any.
///
/// **Console output uses simple format** (no scope headers, no bullets) because:
/// - Most events come from temporary single-event scopes (`event_info_scoped!`)
/// - Printing full scope headers for each event would be too verbose
/// - Live log file gets the full canonical format with scope headers
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

        // --- Live log (using canonical format) ---
        if let Some(current_scope_id) = journal.scopes().iter().rev()
            .find(|s| s.exited_at.is_none())
            .map(|s| s.id)
        {
            let mut headers = rt.written_headers.lock().await;
            
            // Write scope header if not already written
            if !headers.contains(&current_scope_id) {
                if let Some(scope_ref) = journal.scopes().iter().find(|s| s.id == current_scope_id) {
                    // Use canonical rendering for scope header
                    let config = RenderConfig {
                        color: false,  // Live log files should not have color codes
                        timestamps: true,
                        bullet: if cfg!(windows) { "*".to_string() } else { "•".to_string() },
                        indent_size: 2,
                    };
                    
                    let rendered = render_scope_header(&journal, scope_ref, &config);
                    live_log::append(&rendered.header);
                    headers.insert(current_scope_id);
                }
            }

            // Render event using canonical format
            let scope_depth = journal.scope_path(Some(current_scope_id)).len();
            let indent_level = scope_depth.saturating_sub(1) + 1; // +1 for event indent within scope
            
            // Create a Record for rendering
            if let Some(last_record) = journal.records().iter().rev().next() {
                let config = RenderConfig {
                    color: false,
                    timestamps: false,
                    bullet: if cfg!(windows) { "*".to_string() } else { "•".to_string() },
                    indent_size: 2,
                };
                
                if let Some(rendered) = render_event(last_record, &config, indent_level) {
                    live_log::append(&rendered.main);
                    
                    // Add detail line if present (arrow rule: only if it adds information)
                    if let Some(detail) = rendered.detail {
                        live_log::append(&detail);
                    }
                }
            }
        }

        // --- Console output (simple format - no scope headers) ---
        // Console stays simple because most scopes are single-event temporary scopes
        // from event_*_scoped! macros. Full canonical format goes to live log.
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
