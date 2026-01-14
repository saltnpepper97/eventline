/// Record an informational event.
///
/// This macro logs an info message asynchronously in the background. You don't
/// need to `.await` the macro call as it spawns a detached task to handle the logging.
///
/// Accepts format string syntax like `println!`. Respects runtime log level.
///
/// # Note
///
/// Logging is fire-and-forget - events are recorded asynchronously and may be
/// reordered relative to surrounding code. For critical paths where you need
/// guaranteed ordering, use `runtime::info()` directly with `.await`.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_info;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// # let count = 3;
/// event_info!("Application started");
/// event_info!("Loaded {} configuration files", count);
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_info {
    ($($arg:tt)*) => {
        {
            let message = format!($($arg)*);
            $crate::runtime::spawn_detached(async move {
                $crate::runtime::info(message).await;
            });
        }
    };
}

/// Record a warning event.
///
/// This macro logs a warning message asynchronously in the background. You don't
/// need to `.await` the macro call as it spawns a detached task to handle the logging.
///
/// Warnings indicate something unexpected or suboptimal happened,
/// but execution can continue. Respects runtime log level.
///
/// # Note
///
/// Logging is fire-and-forget - events are recorded asynchronously and may be
/// reordered relative to surrounding code. For critical paths where you need
/// guaranteed ordering, use `runtime::warn()` directly with `.await`.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_warn;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// # let attempt = 1;
/// event_warn!("Retry attempt {} failed", attempt);
/// event_warn!("Deprecated configuration option used");
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_warn {
    ($($arg:tt)*) => {
        {
            let message = format!($($arg)*);
            $crate::runtime::spawn_detached(async move {
                $crate::runtime::warn(message).await;
            });
        }
    };
}

/// Record an error event.
///
/// This macro logs an error message asynchronously in the background. You don't
/// need to `.await` the macro call as it spawns a detached task to handle the logging.
///
/// Errors indicate something went wrong. Does **not** fail the current scope automatically.
///
/// # Note
///
/// Logging is fire-and-forget - events are recorded asynchronously and may be
/// reordered relative to surrounding code. For critical error paths where you need
/// guaranteed ordering before shutdown or panic, use `runtime::error()` directly
/// with `.await` or ensure proper cleanup/flush mechanisms.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_error;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// # let err = "DB connection failed";
/// event_error!("Database connection failed: {}", err);
/// event_error!("Invalid input received");
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_error {
    ($($arg:tt)*) => {
        {
            let message = format!($($arg)*);
            $crate::runtime::spawn_detached(async move {
                $crate::runtime::error(message).await;
            });
        }
    };
}

/// Record a debug event.
///
/// This macro logs a debug message asynchronously in the background. You don't
/// need to `.await` the macro call as it spawns a detached task to handle the logging.
///
/// Debug events are typically used for verbose diagnostic information.
///
/// # Note
///
/// Logging is fire-and-forget - events are recorded asynchronously and may be
/// reordered relative to surrounding code. For critical paths where you need
/// guaranteed ordering, use `runtime::debug()` directly with `.await`.
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
/// event_debug!("Cache hit for key: {}", key);
/// event_debug!("State transition: {:?} -> {:?}", old, new);
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_debug {
    ($($arg:tt)*) => {
        {
            let message = format!($($arg)*);
            $crate::runtime::spawn_detached(async move {
                $crate::runtime::debug(message).await;
            });
        }
    };
}
