/// Record an informational event within a named scope (single line).
///
/// This is a convenience macro that creates a scope and logs a single info message.
/// Useful for IPC handlers and triggers where you want scoped context without
/// wrapping a full block.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_info_scoped;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_info_scoped!("ProfileSwitch", "Switched to profile: {}", "work").await;
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_info_scoped {
    ($scope:expr, $($arg:tt)*) => {
        $crate::runtime::scoped_async(Some($scope), || async {
            $crate::runtime::info(format!($($arg)*)).await;
        })
    };
}

/// Record a warning event within a named scope (single line).
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_warn_scoped;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_warn_scoped!("CacheCheck", "Cache at {}% capacity", 95).await;
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_warn_scoped {
    ($scope:expr, $($arg:tt)*) => {
        $crate::runtime::scoped_async(Some($scope), || async {
            $crate::runtime::warn(format!($($arg)*)).await;
        })
    };
}

/// Record an error event within a named scope (single line).
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_error_scoped;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_error_scoped!("IpcHandler", "Failed to process: {}", "invalid data").await;
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_error_scoped {
    ($scope:expr, $($arg:tt)*) => {
        $crate::runtime::scoped_async(Some($scope), || async {
            $crate::runtime::error(format!($($arg)*)).await;
        })
    };
}

/// Record a debug event within a named scope (single line).
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_debug_scoped;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_debug_scoped!("Trigger", "Window {} focused", 12345).await;
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_debug_scoped {
    ($scope:expr, $($arg:tt)*) => {
        $crate::runtime::scoped_async(Some($scope), || async {
            $crate::runtime::debug(format!($($arg)*)).await;
        })
    };
}
