/// Log at INFO level.
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {{
        $crate::runtime::emit(
            $crate::core::EventKind::Info,
            format!($($arg)*),
            $crate::journal::Fields::default(),
        );
    }};
}

/// Log at DEBUG level.
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {{
        $crate::runtime::emit(
            $crate::core::EventKind::Debug,
            format!($($arg)*),
            $crate::journal::Fields::default(),
        );
    }};
}

/// Log at WARN level.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {{
        $crate::runtime::emit(
            $crate::core::EventKind::Warning,
            format!($($arg)*),
            $crate::journal::Fields::default(),
        );
    }};
}

/// Log at ERROR level.
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {{
        $crate::runtime::emit(
            $crate::core::EventKind::Error,
            format!($($arg)*),
            $crate::journal::Fields::default(),
        );
    }};
}

/// Run a block inside a named scope.
///
/// Forms:
/// - `scope!("config", { ... })`
/// - `scope!("config", success="loaded", { ... })`
/// - `scope!("config", success="loaded", failure="failed", aborted="aborted", { ... })`
///
/// Exit messages only appear on the scope exit ("done:") line.
#[macro_export]
macro_rules! scope {
    // ---------------------------------------------------------------------
    // Base form — FAST PATH (no parsing, no messages)
    // ---------------------------------------------------------------------
    ($name:expr, $block:block) => {{
        let _guard = $crate::core::RuntimeScopeGuard::enter($name);
        $block
    }};

    // ---------------------------------------------------------------------
    // success-only form — FAST PATH
    // ---------------------------------------------------------------------
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

    // ---------------------------------------------------------------------
    // success + failure
    // ---------------------------------------------------------------------
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

    // ---------------------------------------------------------------------
    // success + failure + aborted
    // ---------------------------------------------------------------------
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

