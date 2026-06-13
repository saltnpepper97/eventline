#[doc(hidden)]
#[macro_export]
macro_rules! __eventline_fields {
    ($($key:ident = $value:expr),* $(,)?) => {{
        let mut fields = $crate::journal::Fields::new();
        $(
            fields.insert(stringify!($key), $value);
        )*
        fields
    }};
}

/// Log at INFO level.
///
/// Sink filtering does not prevent recording. Use `set_record_level` to raise
/// the journal's recording threshold if you need to avoid formatting work.
#[macro_export]
macro_rules! info {
    ($message:expr, $($key:ident = $value:expr),+ $(,)?) => {{
        let kind = $crate::core::EventKind::Info;
        if $crate::runtime::log_level::recording_enabled(kind) {
            let fields = $crate::__eventline_fields!($($key = $value),+);
            $crate::runtime::emit(kind, $message.to_string(), fields);
        }
    }};

    ($($arg:tt)*) => {{
        let kind = $crate::core::EventKind::Info;
        if $crate::runtime::log_level::recording_enabled(kind) {
            $crate::runtime::emit(
                kind,
                format!($($arg)*),
                $crate::journal::Fields::new(),
            );
        }
    }};
}

/// Log at DEBUG level.
///
/// DEBUG is recorded by default even when it is hidden from all outputs.
#[macro_export]
macro_rules! debug {
    ($message:expr, $($key:ident = $value:expr),+ $(,)?) => {{
        let kind = $crate::core::EventKind::Debug;
        if $crate::runtime::log_level::recording_enabled(kind) {
            let fields = $crate::__eventline_fields!($($key = $value),+);
            $crate::runtime::emit(kind, $message.to_string(), fields);
        }
    }};

    ($($arg:tt)*) => {{
        let kind = $crate::core::EventKind::Debug;
        if $crate::runtime::log_level::recording_enabled(kind) {
            $crate::runtime::emit(
                kind,
                format!($($arg)*),
                $crate::journal::Fields::new(),
            );
        }
    }};
}

/// Log at WARN level.
#[macro_export]
macro_rules! warn {
    ($message:expr, $($key:ident = $value:expr),+ $(,)?) => {{
        let kind = $crate::core::EventKind::Warning;
        if $crate::runtime::log_level::recording_enabled(kind) {
            let fields = $crate::__eventline_fields!($($key = $value),+);
            $crate::runtime::emit(kind, $message.to_string(), fields);
        }
    }};

    ($($arg:tt)*) => {{
        let kind = $crate::core::EventKind::Warning;
        if $crate::runtime::log_level::recording_enabled(kind) {
            $crate::runtime::emit(
                kind,
                format!($($arg)*),
                $crate::journal::Fields::new(),
            );
        }
    }};
}

/// Log at ERROR level.
#[macro_export]
macro_rules! error {
    ($message:expr, $($key:ident = $value:expr),+ $(,)?) => {{
        let kind = $crate::core::EventKind::Error;
        if $crate::runtime::log_level::recording_enabled(kind) {
            let fields = $crate::__eventline_fields!($($key = $value),+);
            $crate::runtime::emit(kind, $message.to_string(), fields);
        }
    }};

    ($($arg:tt)*) => {{
        let kind = $crate::core::EventKind::Error;
        if $crate::runtime::log_level::recording_enabled(kind) {
            $crate::runtime::emit(
                kind,
                format!($($arg)*),
                $crate::journal::Fields::new(),
            );
        }
    }};
}

/// Run a block inside a named scope.
///
/// Forms:
/// - `scope!("config", { ... })`
/// - `scope!("config", success="loaded", { ... })`
/// - `scope!("config", success="loaded", failure="failed", { ... })`
/// - `scope!("config", success="loaded", failure="failed", aborted="aborted", { ... })`
///
/// Exit messages only appear on the scope exit ("done:") line.
#[macro_export]
macro_rules! scope {
    // -------------------------------------------------------------------------
    // Base form — no messages, zero overhead beyond entering/exiting the scope.
    // -------------------------------------------------------------------------
    ($name:expr, $block:block) => {{
        let _guard = $crate::core::RuntimeScopeGuard::enter($name);
        $block
    }};

    // -------------------------------------------------------------------------
    // success-only
    // -------------------------------------------------------------------------
    ($name:expr, success=$success:expr, $block:block) => {{
        let _guard = $crate::core::RuntimeScopeGuard::enter($name);
        $crate::runtime::set_scope_exit_messages(
            _guard.id(),
            $crate::core::ExitMessages {
                success: Some($success.to_string()),
                failure: None,
                aborted: None,
            },
        );
        $block
    }};

    // -------------------------------------------------------------------------
    // success + failure
    // -------------------------------------------------------------------------
    ($name:expr, success=$success:expr, failure=$failure:expr, $block:block) => {{
        let _guard = $crate::core::RuntimeScopeGuard::enter($name);
        $crate::runtime::set_scope_exit_messages(
            _guard.id(),
            $crate::core::ExitMessages {
                success: Some($success.to_string()),
                failure: Some($failure.to_string()),
                aborted: None,
            },
        );
        $block
    }};

    // -------------------------------------------------------------------------
    // success + failure + aborted
    // -------------------------------------------------------------------------
    (
        $name:expr,
        success=$success:expr,
        failure=$failure:expr,
        aborted=$aborted:expr,
        $block:block
    ) => {{
        let _guard = $crate::core::RuntimeScopeGuard::enter($name);
        $crate::runtime::set_scope_exit_messages(
            _guard.id(),
            $crate::core::ExitMessages {
                success: Some($success.to_string()),
                failure: Some($failure.to_string()),
                aborted: Some($aborted.to_string()),
            },
        );
        $block
    }};
}
