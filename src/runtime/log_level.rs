use std::sync::atomic::{AtomicU8, Ordering};

/// Defines the runtime log level.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warning = 2,
    Error = 3,
}

/// Global runtime log level.
static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

/// Set the current runtime log level.
///
/// Only events at or above this level will be recorded by macros.
pub fn set_log_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Get the current runtime log level.
pub fn get_log_level() -> LogLevel {
    match LOG_LEVEL.load(Ordering::Relaxed) {
        0 => LogLevel::Debug,
        1 => LogLevel::Info,
        2 => LogLevel::Warning,
        3 => LogLevel::Error,
        _ => LogLevel::Info,
    }
}

/// Check whether a given `EventKind` should be logged at the current level.
pub fn log_enabled(kind: crate::event_kind::EventKind) -> bool {
    let level = get_log_level();
    match kind {
        crate::event_kind::EventKind::Debug => level <= LogLevel::Debug,
        crate::event_kind::EventKind::Info => level <= LogLevel::Info,
        crate::event_kind::EventKind::Warning => level <= LogLevel::Warning,
        crate::event_kind::EventKind::Error => level <= LogLevel::Error,
    }
}
