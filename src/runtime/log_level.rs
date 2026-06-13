use crate::core::{EventKind, Record, RecordKind};
use std::sync::atomic::{AtomicU8, Ordering};

/// Logging verbosity level.
///
/// Lower values are more verbose:
/// - `Debug` allows everything
/// - `Info` hides debug
/// - `Warning` shows only warnings/errors
/// - `Error` shows only errors
/// - `Off` disables all event emission to outputs
///
/// The explicit discriminants let us store and compare the level as a plain
/// `u8` inside an atomic without any locking.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warning = 2,
    Error = 3,
    /// Suppress all output (records still land in the in-memory journal).
    Off = 4,
}

impl LogLevel {
    #[inline]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Debug,
            1 => Self::Info,
            2 => Self::Warning,
            3 => Self::Error,
            _ => Self::Off,
        }
    }
}

#[inline]
pub fn level_for_event_kind(kind: EventKind) -> LogLevel {
    match kind {
        EventKind::Debug => LogLevel::Debug,
        EventKind::Info => LogLevel::Info,
        EventKind::Warning => LogLevel::Warning,
        EventKind::Error => LogLevel::Error,
    }
}

/// Global output fast-path threshold maintained from the enabled sinks.
///
/// This is **not** a per-sink level. It is the effective "emitted anywhere"
/// threshold, typically computed as the most verbose level required by any
/// active output sink.
///
/// Example:
/// - console = Info
/// - file    = Debug
///
/// Recording is controlled separately by [`set_record_level`]. Sink filtering
/// must not decide whether an event lands in the journal.
///
/// `Ordering::Relaxed` is correct here: a stale read only means an occasional
/// extra or missing log line during reconfiguration, which is acceptable and
/// avoids paying for stronger synchronization on every log call.
static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

/// Global recording threshold used by logging macros before `format!()` runs.
///
/// Defaults to `Debug`, meaning every built-in level is recorded even when no
/// output sink would currently render it. This is eventline's core contract:
/// recording and emission are separate concerns.
static RECORD_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Debug as u8);

/// Set the global fast-path threshold.
///
/// In multi-sink configurations, this should usually be the most verbose level
/// required by any enabled sink, not an individual sink's local threshold.
pub fn set_log_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Read the current global fast-path threshold.
pub fn get_log_level() -> LogLevel {
    LogLevel::from_u8(LOG_LEVEL.load(Ordering::Relaxed))
}

/// Set the global recording threshold.
///
/// This is the explicit performance escape hatch. Raising it prevents lower
/// priority events from being formatted or recorded at all, independent of
/// console/file sink levels.
pub fn set_record_level(level: LogLevel) {
    RECORD_LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Read the current global recording threshold.
pub fn get_record_level() -> LogLevel {
    LogLevel::from_u8(RECORD_LEVEL.load(Ordering::Relaxed))
}

/// HOT PATH — called from log macros before `format!()` is evaluated.
///
/// Returns true when an event of this kind is enabled by the current global
/// fast-path threshold, meaning that at least one active sink may want it.
#[inline(always)]
pub fn level_enabled(kind: EventKind) -> bool {
    let threshold = LOG_LEVEL.load(Ordering::Relaxed);
    (level_for_event_kind(kind) as u8) >= threshold
}

/// HOT PATH — called from log macros before `format!()` is evaluated.
///
/// Returns true when an event of this kind should be recorded in the journal.
#[inline(always)]
pub fn recording_enabled(kind: EventKind) -> bool {
    let threshold = RECORD_LEVEL.load(Ordering::Relaxed);
    (level_for_event_kind(kind) as u8) >= threshold
}

/// Convenience helper for already-built records.
///
/// Important: this only answers whether the record is enabled by the global
/// output threshold. Final per-sink filtering still happens later in the writer
/// layer.
///
/// Scope-exit records are always considered enabled because they are
/// structural/logical records rather than user-verbosity records.
#[inline]
pub fn enabled_for_record(record: &Record) -> bool {
    match &record.kind {
        RecordKind::Event { kind, .. } => level_enabled(*kind),
        RecordKind::ScopeExit { .. } => true,
    }
}
