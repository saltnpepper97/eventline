/// Record an informational event within a named scope (single line).
///
/// This is a convenience macro that creates a scope and logs a single info message
/// asynchronously in the background. The logging happens in a detached task, so you
/// don't need to `.await` the macro call.
///
/// Useful for IPC handlers and triggers where you want scoped context without
/// wrapping a full block or blocking on the logging operation.
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
                $crate::runtime::scope::scoped_in_place(Some(scope_owned), async move {
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
                $crate::runtime::scope::scoped_in_place(Some(scope_owned), async move {
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
                $crate::runtime::scope::scoped_in_place(Some(scope_owned), async move {
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
                $crate::runtime::scope::scoped_in_place(Some(scope_owned), async move {
                    $crate::runtime::debug(message).await;
                }).await;
            });
        }
    };
}

/// Run an async block inside a **single structured eventline scope**.
///
/// This macro allows you to wrap multiple info/debug/error/warn calls
/// in a **single logical scope** in the eventline journal. All events logged
/// inside the block belong to the same scope.
///
/// Inside the block, you can use the runtime functions directly:
/// - `eventline::runtime::info(...).await`
/// - `eventline::runtime::debug(...).await`
/// - `eventline::runtime::warn(...).await`
/// - `eventline::runtime::error(...).await`
///
/// The scope will show all messages flowing through it in a structured format.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::scoped_eventline;
/// # use eventline::runtime;
/// # async fn example() {
/// scoped_eventline!("Config", {
///     runtime::info("Profile 'gaming' has no actions.").await;
///     runtime::info("Profiles loaded: gaming").await;
/// });
/// # }
/// ```
///
/// This will render as:
/// ```text
/// [2026-01-14 00:18:52] Scope Config (id=7) → Success (0ms)
///   • info      Profile 'gaming' has no actions.
///   • info      Profiles loaded: gaming
/// ```
#[macro_export]
macro_rules! scoped_eventline {
    ($scope_name:expr, $body:block) => {{
        $crate::runtime::scope::scoped_in_place(
            Some($scope_name.to_string()),
            async move { $body }
        )
        .await
    }};
}

/// Run an async block inside a scope with structured fields.
///
/// Like `scoped_eventline!`, but allows logging events with structured fields (`Fields`).
///
/// Inside the block, you can call:
/// - `eventline::runtime::event::info_fields(msg, fields).await`
/// - `eventline::runtime::event::debug_fields(msg, fields).await`
/// - etc.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::{scoped_eventline_fields, fields};
/// # async fn example() {
/// scoped_eventline_fields!("UserLogin", {
///     let f = fields!({
///         "user_id" => 12345,
///         "action" => "login",
///     });
///     eventline::runtime::event::info_fields("Login attempt started", f.clone()).await;
///     eventline::runtime::event::debug_fields("Login attempt finished", f).await;
/// });
/// # }
/// ```
#[macro_export]
macro_rules! scoped_eventline_fields {
    ($scope_name:expr, $body:block) => {{
        $crate::runtime::scope::scoped_in_place(
            Some($scope_name.to_string()),
            async move { $body }
        )
        .await
    }};
}
