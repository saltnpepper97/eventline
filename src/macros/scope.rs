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

/// Run an async block inside a **single structured eventline scope**.
///
/// This macro allows you to wrap multiple info/debug/error/warn calls
/// in a **single logical scope** in the eventline journal. It ensures that
/// all events logged inside the block belong to the same scope, avoiding
/// excessive scope creation and lifetime issues when using `&mut self` or
/// other borrowed data.
///
/// Unlike the single-line macros like [`event_info_scoped!`] or
/// [`event_debug_scoped!`], this macro executes your block **in-place**
/// and allows `.await` inside. This is perfect for multi-step operations
/// where you want all log messages to be grouped under the same header.
///
/// # Note
///
/// - The `$scope_name` must be convertible to `String` (usually `&str` or `String`).
/// - Inside the block, you can use:
///   - `eventline::runtime::info(...)`
///   - `eventline::runtime::debug(...)`
///   - `eventline::runtime::warn(...)`
///   - `eventline::runtime::error(...)`
///   - `.await` normally on async calls
/// - This macro does **not spawn a detached task**; it runs the block inline.
/// - Recommended for operations that manipulate borrowed data (`&mut self`) or need
///   guaranteed order of log messages.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::scoped_eventline;
/// # use eventline::runtime;
/// # async fn example() {
/// // Wrap the entire process in one journal scope
/// scoped_eventline!("TriggerInstantActions", {
///     // Info message at the start
///     eventline::runtime::info("Triggering instant actions...").await;
///
///     // Imagine a list of "actions"
///     let instant_actions = vec!["action1", "action2"];
///
///     for action in &instant_actions {
///         // Placeholder for action run
///         // async fn run_action(action: &str) { ... } could be called here
///         // run_action(action).await;
///
///         // Log inside the same scope
///         eventline::runtime::debug(&format!("Ran instant action: {}", action)).await;
///     }
///
///     // Placeholder for marking state
///     let index = instant_actions.len();
///
///     eventline::runtime::info(&format!("Instant actions complete, starting at index {}", index)).await;
/// });
/// # }
/// ```
///
/// This pattern replaces multiple detached scopes like [`event_info_scoped!`]
/// or [`event_debug_scoped!`] with **one unified scope**, keeping the journal cleaner
/// and avoiding lifetime/`'static` issues.
///
/// [`event_info_scoped!`]: crate::event_info_scoped
/// [`event_debug_scoped!`]: crate::event_debug_scoped
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

// Keep the scope macros unchanged - they return futures that should be awaited
// for proper scope lifetime management

/// Execute code within a named scope (async version).
///
/// Automatically enters/exits the scope, records events, and restores previous scope.
/// Panics if runtime not initialized. Use [`try_scope!`] to avoid panicking.
///
/// Note: This returns a Future that must be awaited.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::{event_scope, event_info};
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// # async fn run_migrations() {}
/// event_scope!("DatabaseMigration", {
///     event_info!("Starting migration");
///     run_migrations().await;
///     event_info!("Migration complete");
/// }).await;
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_scope {
    ($name:expr, $body:block) => {
        $crate::runtime::scope::scoped_async::<String, _, _, _>(Some($name.to_string()), || async move $body)
    };
}

/// Execute async code within a named scope.
///
/// This is the async version of `event_scope!`. It properly handles async closures
/// and futures, ensuring scope tracking works correctly across await points.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::{event_scope_async, event_info};
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_scope_async!("AsyncTask", {
///     event_info!("Starting async work");
///     tokio::time::sleep(std::time::Duration::from_millis(10)).await;
///     event_info!("Async work complete");
/// }).await;
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_scope_async {
    ($name:expr, $body:block) => {
        {
            // wrap in async move to take ownership of outer vars
            let fut = async move $body;
            $crate::runtime::scope::scoped_async(Some($name.to_string()), || fut)
        }
    };
}

/// Execute code within a named scope, without panicking if runtime is uninitialized.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::try_scope;
/// # use eventline::runtime;
/// # async fn example() {
/// let result = try_scope!("OptionalLogging", {
///     42
/// }).await;
/// assert_eq!(result, 42);
/// # }
/// ```
#[macro_export]
macro_rules! try_scope {
    ($name:expr, async $body:block) => {
        async {
            if $crate::runtime::is_initialized().await {
                $crate::runtime::scope::scoped_async(Some($name), || async move $body).await
            } else {
                async move $body.await
            }
        }
    };
    ($name:expr, $body:block) => {
        async {
            $crate::runtime::try_scoped(Some($name), || $body).await
        }
    };
}

/// Execute async code within a named scope, without panicking if runtime is uninitialized.
///
/// This is the async version of `try_scope!`.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::try_scope_async;
/// # use eventline::runtime;
/// # async fn example() {
/// let result = try_scope_async!("OptionalLogging", {
///     tokio::time::sleep(std::time::Duration::from_millis(10)).await;
///     42
/// }).await;
/// assert_eq!(result, 42);
/// # }
/// ```
#[macro_export]
macro_rules! try_scope_async {
    ($name:expr, $body:block) => {
        async move {
            if $crate::runtime::is_initialized().await {
                $crate::runtime::scope::scoped_async(Some($name), move || async move $body).await
            } else {
                $body
            }
        }
    };
}

/// Execute code within an unnamed scope.
///
/// Useful for scope structure without naming overhead. Panics if runtime not initialized.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::{event_scope_unnamed, event_info};
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_scope_unnamed!(async {
///     event_info!("Anonymous work");
/// }).await;
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_scope_unnamed {
    (async $body:block) => {
        $crate::runtime::scope::scoped_async::<String, _, _, _>(None, || async move $body)
    };
}

/// Execute async code within an unnamed scope.
///
/// This is the async version of `event_scope_unnamed!`.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::{event_scope_unnamed_async, event_info};
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_scope_unnamed_async!({
///     event_info!("Anonymous async work");
/// }).await;
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_scope_unnamed_async {
    ($body:block) => {
        $crate::runtime::scope::scoped_async::<String, _, _, _>(None, || async move $body)
    };
}

/// Execute code within an unnamed scope, without panicking if runtime is uninitialized.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::try_scope_unnamed;
/// # use eventline::runtime;
/// # async fn example() {
/// let result = try_scope_unnamed!(async {
///     42
/// }).await;
/// assert_eq!(result, 42);
/// # }
/// ```
#[macro_export]
macro_rules! try_scope_unnamed {
    (async $body:block) => {
        async {
            if $crate::runtime::is_initialized().await {
                $crate::runtime::scope::scoped_async::<String, _, _, _>(None, || async move $body).await
            } else {
                async move $body.await
            }
        }
    };
    ($body:block) => {
        async {
            $crate::runtime::try_scoped_unnamed(|| $body).await
        }
    };
}

/// Execute async code within an unnamed scope, without panicking if runtime is uninitialized.
///
/// This is the async version of `try_scope_unnamed!`.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::try_scope_unnamed_async;
/// # use eventline::runtime;
/// # async fn example() {
/// let result = try_scope_unnamed_async!({
///     tokio::time::sleep(std::time::Duration::from_millis(10)).await;
///     42
/// }).await;
/// assert_eq!(result, 42);
/// # }
/// ```
#[macro_export]
macro_rules! try_scope_unnamed_async {
    ($body:block) => {
        $crate::runtime::scope::try_scoped_unnamed_async(|| async move $body)
    };
}
