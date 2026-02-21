use crate::core::{EventKind, Record, RecordKind};
use std::sync::atomic::{AtomicU8, Ordering};

/// Logging verbosity level.
///
/// Variants are assigned explicit discriminants so we can store and compare
/// them as a plain `u8` in an atomic without any locking.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug   = 0,
    Info    = 1,
    Warning = 2,
    Error   = 3,
    /// Suppress all output (records still land in the in-memory journal).
    Off     = 4,
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

/// Single atomic byte — the only state needed for level checks.
/// `Ordering::Relaxed` is correct here: a stale read just means one extra
/// log line leaks through; that is far better than paying for sequentially-
/// consistent ordering (a full memory fence) on every log call.
static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

pub fn set_log_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn get_log_level() -> LogLevel {
    LogLevel::from_u8(LOG_LEVEL.load(Ordering::Relaxed))
}

/// HOT PATH — called from every log macro before `format!()` is evaluated.
///
/// Inlined so the compiler can constant-fold / hoist the check when the level
/// is known at compile time.
#[inline(always)]
pub fn level_enabled(kind: EventKind) -> bool {
    let threshold = LOG_LEVEL.load(Ordering::Relaxed);
    (kind as u8) >= threshold
}

/// Used inside the journal when the writer is deciding whether to emit a
/// record that has already been pushed to the buffer.
#[inline]
pub fn enabled_for_record(record: &Record) -> bool {
    match &record.kind {
        RecordKind::Event { kind, .. } => level_enabled(*kind),
        // Scope-exit lines are always emitted — they are structural, not
        // verbosity-gated, and carry duration / outcome metadata.
        RecordKind::ScopeExit { .. } => true,
    }
}
