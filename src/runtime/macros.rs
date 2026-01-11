//! Ergonomic macros for fire-and-forget event logging.
//!
//! These macros provide a clean, zero-overhead interface to the runtime.
//! They only allocate if the runtime is initialized and match Rust's
//! standard logging conventions. Each macro respects the runtime's
//! log level setting, so you can globally control which events are recorded.
//!
//! # Example
//!
//! ```rust
//! # use eventline::runtime;
//! # use eventline::{event_info, event_warn, event_error, event_debug, event_scope};
//! # runtime::init();
//! # let cache_size = 256;
//! # let err = "connection failed";
//! # let id = 42;
//! event_info!("Server started on port {}", 8080);
//! event_warn!("Cache size: {} MB", cache_size);
//! event_error!("Failed to connect: {}", err);
//! event_debug!("Processing request {}", id);
//!
//! event_scope!("RequestHandler", {
//!     event_info!("Handling request");
//!     // ... work happens here ...
//! });
//! # runtime::reset();
//! ```

/// Record an informational event.
///
/// Accepts format string syntax like `println!`. Respects runtime log level.
///
/// # Example
///
/// ```rust
/// # #[doc(hidden)] use eventline::event_info;
/// # use eventline::runtime;
/// # runtime::init();
/// # let count = 3;
/// event_info!("Application started");
/// event_info!("Loaded {} configuration files", count);
/// # runtime::reset();
/// ```
#[macro_export]
macro_rules! event_info {
    ($($arg:tt)*) => {
        $crate::runtime::info(format!($($arg)*))
    };
}

/// Record a warning event.
///
/// Warnings indicate something unexpected or suboptimal happened,
/// but execution can continue. Respects runtime log level.
///
/// # Example
///
/// ```rust
/// # #[doc(hidden)] use eventline::event_warn;
/// # use eventline::runtime;
/// # runtime::init();
/// # let attempt = 1;
/// event_warn!("Retry attempt {} failed", attempt);
/// event_warn!("Deprecated configuration option used");
/// # runtime::reset();
/// ```
#[macro_export]
macro_rules! event_warn {
    ($($arg:tt)*) => {
        $crate::runtime::warn(format!($($arg)*))
    };
}

/// Record an error event.
///
/// Errors indicate something went wrong. Does **not** fail the current scope automatically.
///
/// # Example
///
/// ```rust
/// # #[doc(hidden)] use eventline::event_error;
/// # use eventline::runtime;
/// # runtime::init();
/// # let err = "DB connection failed";
/// event_error!("Database connection failed: {}", err);
/// event_error!("Invalid input received");
/// # runtime::reset();
/// ```
#[macro_export]
macro_rules! event_error {
    ($($arg:tt)*) => {
        $crate::runtime::error(format!($($arg)*))
    };
}

/// Record a debug event.
///
/// Debug events are typically used for verbose diagnostic information.
///
/// # Example
///
/// ```rust
/// # #[doc(hidden)] use eventline::event_debug;
/// # use eventline::runtime;
/// # runtime::init();
/// # let key = "user_id";
/// # let old = 0;
/// # let new = 1;
/// event_debug!("Cache hit for key: {}", key);
/// event_debug!("State transition: {:?} -> {:?}", old, new);
/// # runtime::reset();
/// ```
#[macro_export]
macro_rules! event_debug {
    ($($arg:tt)*) => {
        $crate::runtime::debug(format!($($arg)*))
    };
}

/// Execute code within a named scope.
///
/// Automatically enters/exits the scope, records events, and restores previous scope.
/// Panics if runtime not initialized. Use [`try_scope!`] to avoid panicking.
///
/// # Example
///
/// ```rust
/// # #[doc(hidden)] use eventline::{event_scope, event_info};
/// # use eventline::runtime;
/// # runtime::init();
/// # fn run_migrations() {}
/// event_scope!("DatabaseMigration", {
///     event_info!("Starting migration");
///     run_migrations();
///     event_info!("Migration complete");
/// });
/// # runtime::reset();
/// ```
#[macro_export]
macro_rules! event_scope {
    ($name:expr, $body:block) => {
        $crate::runtime::scoped(Some($name), || $body)
    };
}

/// Execute code within a named scope, without panicking if runtime is uninitialized.
///
/// # Example
///
/// ```rust
/// # #[doc(hidden)] use eventline::try_scope;
/// # use eventline::runtime;
/// let result = try_scope!("OptionalLogging", {
///     42
/// });
/// assert_eq!(result, 42);
/// ```
#[macro_export]
macro_rules! try_scope {
    ($name:expr, $body:block) => {
        $crate::runtime::try_scoped(Some($name), || $body)
    };
}

/// Execute code within an unnamed scope.
///
/// Useful for scope structure without naming overhead. Panics if runtime not initialized.
///
/// # Example
///
/// ```rust
/// # #[doc(hidden)] use eventline::{event_scope_unnamed, event_info};
/// # use eventline::runtime;
/// # runtime::init();
/// event_scope_unnamed!({
///     event_info!("Anonymous work");
/// });
/// # runtime::reset();
/// ```
#[macro_export]
macro_rules! event_scope_unnamed {
    ($body:block) => {
        $crate::runtime::scoped_unnamed(|| $body)
    };
}

/// Execute code within an unnamed scope, without panicking if runtime is uninitialized.
///
/// # Example
///
/// ```rust
/// # #[doc(hidden)] use eventline::try_scope_unnamed;
/// # use eventline::runtime;
/// let result = try_scope_unnamed!({
///     42
/// });
/// assert_eq!(result, 42);
/// ```
#[macro_export]
macro_rules! try_scope_unnamed {
    ($body:block) => {
        $crate::runtime::try_scoped_unnamed(|| $body)
    };
}

#[cfg(test)]
mod tests {
    use crate::runtime;
    use crate::runtime::log_level::{set_log_level, LogLevel};
  
    fn safe_reset() {
        let _ = std::panic::catch_unwind(|| runtime::reset());
        runtime::init();
        set_log_level(LogLevel::Debug); // ensure debug events are recorded
    }

    #[test]
    fn test_event_macros() {
        safe_reset();
        event_info!("test");
        event_warn!("test {}", 42);
        event_error!("test");
        event_debug!("test");

        runtime::with_journal(|journal| {
            assert_eq!(journal.records().len(), 4);
        });

        runtime::reset();
    }

    #[test]
    fn test_scope_macro() {
        safe_reset();
        event_scope!("test_scope", { event_info!("inside"); });

        runtime::with_journal(|journal| {
            assert_eq!(journal.scopes().len(), 1);
            assert_eq!(journal.scopes()[0].name.as_deref(), Some("test_scope"));
        });

        runtime::reset();
    }

    #[test]
    fn test_unnamed_scope_macro() {
        safe_reset();
        event_scope_unnamed!({ event_info!("inside"); });

        runtime::with_journal(|journal| {
            assert_eq!(journal.scopes().len(), 1);
            assert!(journal.scopes()[0].name.is_none());
        });

        runtime::reset();
    }

    #[test]
    fn test_try_scope_macro_without_init() {
        runtime::reset();
        let result = try_scope!("test", { 42 });
        assert_eq!(result, 42);
        runtime::reset();
    }

    #[test]
    fn test_try_scope_macro_with_init() {
        safe_reset();
        try_scope!("test", { event_info!("inside"); });

        runtime::with_journal(|journal| {
            assert_eq!(journal.scopes().len(), 1);
            assert_eq!(journal.scopes()[0].name.as_deref(), Some("test"));
        });

        runtime::reset();
    }

    #[test]
    fn test_try_scope_unnamed_macro() {
        runtime::reset();
        let result = try_scope_unnamed!({ 42 });
        assert_eq!(result, 42);
        runtime::reset();
    }
}
