/// Log at INFO level.
///
/// The level check is performed *before* `format!()` so that when INFO is
/// filtered out the entire call compiles down to a single atomic load and a
/// branch.  No `String` is ever allocated for suppressed messages.
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {{
        if $crate::runtime::log_level::level_enabled($crate::core::EventKind::Info) {
            $crate::runtime::emit(
                $crate::core::EventKind::Info,
                format!($($arg)*),
                $crate::journal::Fields::new(),
            );
        }
    }};
}

/// Log at DEBUG level.
///
/// DEBUG is typically the noisiest level; skipping `format!()` here gives
/// the largest win when debug output is suppressed (the common case in prod).
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {{
        if $crate::runtime::log_level::level_enabled($crate::core::EventKind::Debug) {
            $crate::runtime::emit(
                $crate::core::EventKind::Debug,
                format!($($arg)*),
                $crate::journal::Fields::new(),
            );
        }
    }};
}

/// Log at WARN level.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {{
        if $crate::runtime::log_level::level_enabled($crate::core::EventKind::Warning) {
            $crate::runtime::emit(
                $crate::core::EventKind::Warning,
                format!($($arg)*),
                $crate::journal::Fields::new(),
            );
        }
    }};
}

/// Log at ERROR level.
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {{
        if $crate::runtime::log_level::level_enabled($crate::core::EventKind::Error) {
            $crate::runtime::emit(
                $crate::core::EventKind::Error,
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
