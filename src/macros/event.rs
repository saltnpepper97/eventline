/// Record an informational event.
///
/// Accepts format string syntax like `println!`. Respects runtime log level.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_info;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// # let count = 3;
/// event_info!("Application started").await;
/// event_info!("Loaded {} configuration files", count).await;
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_info {
    ($($arg:tt)*) => {
        $crate::runtime::event::info(format!($($arg)*))
    };
}

/// Record a warning event.
///
/// Warnings indicate something unexpected or suboptimal happened,
/// but execution can continue. Respects runtime log level.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_warn;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// # let attempt = 1;
/// event_warn!("Retry attempt {} failed", attempt).await;
/// event_warn!("Deprecated configuration option used").await;
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_warn {
    ($($arg:tt)*) => {
        $crate::runtime::event::warn(format!($($arg)*))
    };
}

/// Record an error event.
///
/// Errors indicate something went wrong. Does **not** fail the current scope automatically.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_error;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// # let err = "DB connection failed";
/// event_error!("Database connection failed: {}", err).await;
/// event_error!("Invalid input received").await;
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_error {
    ($($arg:tt)*) => {
        $crate::runtime::event::error(format!($($arg)*))
    };
}

/// Record a debug event.
///
/// Debug events are typically used for verbose diagnostic information.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_debug;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// # let key = "user_id";
/// # let old = 0;
/// # let new = 1;
/// event_debug!("Cache hit for key: {}", key).await;
/// event_debug!("State transition: {:?} -> {:?}", old, new).await;
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_debug {
    ($($arg:tt)*) => {
        $crate::runtime::event::debug(format!($($arg)*))
    };
}

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
///     event_info!("Starting migration").await;
///     run_migrations().await;
///     event_info!("Migration complete").await;
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
///     event_info!("Starting async work").await;
///     tokio::time::sleep(std::time::Duration::from_millis(10)).await;
///     event_info!("Async work complete").await;
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
///     event_info!("Anonymous work").await;
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
///     event_info!("Anonymous async work").await;
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
