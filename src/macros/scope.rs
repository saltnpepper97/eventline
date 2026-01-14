/// Record an informational event within a named scope (single line).
///
/// This is a convenience macro that creates a scope and logs a single info message
/// asynchronously in the background. The logging happens in a detached task, so you
/// don't need to `.await` the macro call.
///
/// Useful for IPC handlers and triggers where you want scoped context without
/// wrapping a full block or blocking on the logging operation.
///
/// # Note
///
/// Logging is fire-and-forget - events are recorded asynchronously and may be
/// reordered relative to surrounding code. For critical error paths where you
/// need guaranteed ordering, consider using the async scoped functions directly
/// with `.await`.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_info_scoped;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_info_scoped!("ProfileSwitch", "Switched to profile: {}", "work");
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_info_scoped {
    ($scope:expr, $($arg:tt)*) => {
        {
            let scope_owned = $scope.to_string();
            let message = format!($($arg)*);
            $crate::runtime::spawn_detached(async move {
                $crate::runtime::scoped_async(Some(scope_owned), move || async move {
                    $crate::runtime::info(message).await;
                }).await;
            });
        }
    };
}

/// Record a warning event within a named scope (single line).
///
/// This macro logs a warning message asynchronously in the background. You don't
/// need to `.await` the macro call as it spawns a detached task to handle the logging.
///
/// # Note
///
/// Logging is fire-and-forget - events are recorded asynchronously and may be
/// reordered relative to surrounding code. For critical error paths where you
/// need guaranteed ordering, consider using the async scoped functions directly
/// with `.await`.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_warn_scoped;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_warn_scoped!("CacheCheck", "Cache at {}% capacity", 95);
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_warn_scoped {
    ($scope:expr, $($arg:tt)*) => {
        {
            let scope_owned = $scope.to_string();
            let message = format!($($arg)*);
            $crate::runtime::spawn_detached(async move {
                $crate::runtime::scoped_async(Some(scope_owned), move || async move {
                    $crate::runtime::warn(message).await;
                }).await;
            });
        }
    };
}

/// Record an error event within a named scope (single line).
///
/// This macro logs an error message asynchronously in the background. You don't
/// need to `.await` the macro call as it spawns a detached task to handle the logging.
///
/// # Note
///
/// Logging is fire-and-forget - events are recorded asynchronously and may be
/// reordered relative to surrounding code. For critical error paths where you
/// need guaranteed ordering before shutdown or panic, consider using the async
/// scoped functions directly with `.await` or ensure proper cleanup/flush mechanisms.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_error_scoped;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_error_scoped!("IpcHandler", "Failed to process: {}", "invalid data");
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_error_scoped {
    ($scope:expr, $($arg:tt)*) => {
        {
            let scope_owned = $scope.to_string();
            let message = format!($($arg)*);
            $crate::runtime::spawn_detached(async move {
                $crate::runtime::scoped_async(Some(scope_owned), move || async move {
                    $crate::runtime::error(message).await;
                }).await;
            });
        }
    };
}

/// Record a debug event within a named scope (single line).
///
/// This macro logs a debug message asynchronously in the background. You don't
/// need to `.await` the macro call as it spawns a detached task to handle the logging.
///
/// # Note
///
/// Logging is fire-and-forget - events are recorded asynchronously and may be
/// reordered relative to surrounding code. For critical error paths where you
/// need guaranteed ordering, consider using the async scoped functions directly
/// with `.await`.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_debug_scoped;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_debug_scoped!("Trigger", "Window {} focused", 12345);
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_debug_scoped {
    ($scope:expr, $($arg:tt)*) => {
        {
            let scope_owned = $scope.to_string();
            let message = format!($($arg)*);
            $crate::runtime::spawn_detached(async move {
                $crate::runtime::scoped_async(Some(scope_owned), move || async move {
                    $crate::runtime::debug(message).await;
                }).await;
            });
        }
    };
}
