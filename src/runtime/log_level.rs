use std::sync::atomic::{AtomicU8, Ordering};

use crate::core::{EventKind, Outcome, Record, RecordKind};

/// Global log-level threshold for *emission* (writers/console).
///
/// Important: this does NOT affect journaling/recording.
/// Eventline always keeps the full structured record in-memory (and/or persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Debug   = 10,
    Info    = 20,
    Warning = 30,
    Error   = 40,
}

static GLOBAL_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

/// Set the global log level used for writer emission.
pub fn set_log_level(level: LogLevel) {
    GLOBAL_LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Read the current global log level.
pub fn get_log_level() -> LogLevel {
    match GLOBAL_LEVEL.load(Ordering::Relaxed) {
        x if x == LogLevel::Debug as u8   => LogLevel::Debug,
        x if x == LogLevel::Warning as u8 => LogLevel::Warning,
        x if x == LogLevel::Error as u8   => LogLevel::Error,
        _ => LogLevel::Info,
    }
}

/// Decide whether an *event kind* is enabled given the current global log level.
///
/// This is a **cheap, allocation-free fast path** intended for use *before*
/// constructing Records or acquiring the journal lock.
///
/// Important:
/// - This only gates *whether we should bother recording at all*.
/// - It does NOT affect scope tracking or scope exit semantics.
/// - When Debug is disabled, Debug events are completely elided.
///
/// This exists to keep hot paths (render loops, polling, transitions) fast.
#[inline]
pub fn enabled_for_event_kind(kind: EventKind) -> bool {
    event_kind_level(kind) >= get_log_level()
}

/// Decide whether a fully-constructed record should be emitted to writers
/// given the current log level.
///
/// Rules (solid defaults):
/// - Event records: map EventKind -> LogLevel threshold.
/// - ScopeExit records: emit at Info for success, Warn/Error for non-success
///   (keeps failures visible even at higher log levels).
pub fn enabled_for_record(record: &Record) -> bool {
    let threshold = get_log_level();
    let record_level = record_log_level(record);
    record_level >= threshold
}

/// Compute a record’s effective log level for emission.
pub fn record_log_level(record: &Record) -> LogLevel {
    match &record.kind {
        RecordKind::Event { kind, .. } => event_kind_level(*kind),
        RecordKind::ScopeExit { outcome, .. } => outcome_level(*outcome),
        // If you later add more record variants, pick sensible defaults here.
    }
}

fn event_kind_level(kind: EventKind) -> LogLevel {
    // Adjust if your EventKind differs, but this is the typical mapping.
    match kind {
        EventKind::Debug   => LogLevel::Debug,
        EventKind::Info    => LogLevel::Info,
        EventKind::Warning => LogLevel::Warning,
        EventKind::Error   => LogLevel::Error,
    }
}

fn outcome_level(outcome: Outcome) -> LogLevel {
    match outcome {
        Outcome::Success => LogLevel::Debug,
        Outcome::Aborted => LogLevel::Warning,
        Outcome::Failure => LogLevel::Error,
    }
}
