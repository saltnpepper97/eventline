/// Helper macro to convert a literal map into `Fields`.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::fields;
/// let f = fields!({
///     "user_id" => 12345,
///     "action" => "login",
///     "success" => true,
/// });
/// ```
#[macro_export]
macro_rules! fields {
    ({ $($k:expr => $v:expr),* $(,)? }) => {{
        let mut f = $crate::core::Fields::new();
        $(f.insert($k.into(), $crate::core::Value::from($v));)*
        f
    }};
}

/// Fire-and-forget informational event with structured fields.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::{event_info_fields, fields};
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_info_fields!(
///     "User logged in",
///     fields!({
///         "user_id" => 12345,
///         "ip" => "192.168.1.1",
///     })
/// );
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_info_fields {
    ($msg:expr, $fields:expr) => {{
        let msg = $msg.to_string();
        let f = $fields;
        $crate::runtime::spawn_detached(async move {
            $crate::runtime::event::info_fields(msg, f).await;
        });
    }};
}

/// Fire-and-forget warning event with structured fields.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::{event_warn_fields, fields};
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_warn_fields!(
///     "Cache at capacity",
///     fields!({ "percent" => 95 })
/// );
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_warn_fields {
    ($msg:expr, $fields:expr) => {{
        let msg = $msg.to_string();
        let f = $fields;
        $crate::runtime::spawn_detached(async move {
            $crate::runtime::event::warn_fields(msg, f).await;
        });
    }};
}

/// Fire-and-forget error event with structured fields.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::{event_error_fields, fields};
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_error_fields!(
///     "Connection failed",
///     fields!({ "host" => "localhost", "port" => 5432 })
/// );
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_error_fields {
    ($msg:expr, $fields:expr) => {{
        let msg = $msg.to_string();
        let f = $fields;
        $crate::runtime::spawn_detached(async move {
            $crate::runtime::event::error_fields(msg, f).await;
        });
    }};
}

/// Fire-and-forget debug event with structured fields.
///
/// # Example
///
/// ```rust,no_run
/// # use eventline::{event_debug_fields, fields};
/// # use eventline::runtime;
/// # async fn example() {
/// # runtime::init().await;
/// event_debug_fields!(
///     "Request processed",
///     fields!({ "duration_ms" => 42, "status" => 200 })
/// );
/// # runtime::reset().await;
/// # }
/// ```
#[macro_export]
macro_rules! event_debug_fields {
    ($msg:expr, $fields:expr) => {{
        let msg = $msg.to_string();
        let f = $fields;
        $crate::runtime::spawn_detached(async move {
            $crate::runtime::event::debug_fields(msg, f).await;
        });
    }};
}
