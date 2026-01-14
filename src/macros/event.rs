/// Record an informational event.
///
/// This macro logs an info message asynchronously in the background. You don't
/// need to `.await` the macro call as it spawns a detached task to handle the logging.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_info;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_info!("Application started");
/// event_info!("Loaded {} configuration files", 3);
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
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_warn;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_warn!("Retry attempt {} failed", 1);
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
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_error;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_error!("Database connection failed: {}", "timeout");
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
/// # Example
///
/// ```rust,no_run
/// # use eventline::event_debug;
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_debug!("Cache hit for key: {}", "user_id");
/// event_debug!("State transition: {:?} -> {:?}", 0, 1);
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
