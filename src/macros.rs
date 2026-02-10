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
/// - `scope!("config", failure="bad config", { ... })`
/// - `scope!("config", aborted="skipped", { ... })`
/// - `scope!("config", success="loaded", failure="bad config", { ... })`
/// - `scope!("config", success="loaded", failure="bad config", aborted="skipped", { ... })`
#[macro_export]
macro_rules! scope {
    // Base form
    ($name:expr, $block:block) => {{
        let _guard = $crate::core::RuntimeScopeGuard::enter($name);
        $block
    }};

    // Key/value form(s) then a block:
    ($name:expr, $($rest:tt)+) => {{
        let _guard = $crate::core::RuntimeScopeGuard::enter($name);

        // Collect optional exit messages.
        let mut _msgs = $crate::core::ExitMessages {
            success: None,
            failure: None,
            aborted: None,
        };

        // Parse pairs into _msgs, then run the block.
        $crate::scope!(@parse _msgs, $($rest)+);

        // Apply messages once. (If none were set, this is a no-op you can keep or remove.)
        $crate::runtime::set_scope_exit_messages(_guard.id(), _msgs);

        $crate::scope!(@block $($rest)+)
    }};

    // ----- Parser -----
    (@parse $msgs:ident, success = $val:expr, $($rest:tt)+) => {{
        $msgs.success = Some($val.to_string());
        $crate::scope!(@parse $msgs, $($rest)+);
    }};
    (@parse $msgs:ident, failure = $val:expr, $($rest:tt)+) => {{
        $msgs.failure = Some($val.to_string());
        $crate::scope!(@parse $msgs, $($rest)+);
    }};
    (@parse $msgs:ident, aborted = $val:expr, $($rest:tt)+) => {{
        $msgs.aborted = Some($val.to_string());
        $crate::scope!(@parse $msgs, $($rest)+);
    }};

    // Allow trailing comma before the block
    (@parse $msgs:ident, , $($rest:tt)+) => {{
        $crate::scope!(@parse $msgs, $($rest)+);
    }};

    // Stop parsing when we hit the block
    (@parse $msgs:ident, $block:block) => {{
        // done
        let _ = &$msgs;
        let _ = &$block;
    }};

    // If someone passes junk before the block, fail loudly
    (@parse $msgs:ident, $($bad:tt)+) => {{
        compile_error!("scope!: expected success=..., failure=..., aborted=..., then a block");
    }};

    // ----- Extract the block to execute -----
    (@block $block:block) => { $block };
    (@block $k:ident = $v:expr, $($rest:tt)+) => { $crate::scope!(@block $($rest)+) };
    (@block , $($rest:tt)+) => { $crate::scope!(@block $($rest)+) };
}

