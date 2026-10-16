use parking_lot::Mutex;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};

/// In-memory writer health, retained until explicitly cleared.
///
/// Counts are cumulative across runtime writer replacements. A successful
/// write, flush, or reconfiguration does not erase an earlier failure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriterDiagnostics {
    /// Output entries (records or scope metadata) discarded because the queue
    /// was full or unavailable.
    pub dropped_records: usize,
    /// Failed setup, write, flush, or worker/queue operations.
    /// This counts operations, not the number of lost records or failed sinks.
    pub failed_operations: usize,
    /// Most recently observed failure, including its operation context.
    pub last_error: Option<String>,
}

#[derive(Default)]
pub(crate) struct WriterState {
    dropped: AtomicUsize,
    failed: AtomicUsize,
    last_error: Mutex<Option<String>>,
}

impl WriterState {
    pub(crate) fn record_error(&self, operation: &str, error: &io::Error) {
        self.failed.fetch_add(1, Ordering::Relaxed);
        *self.last_error.lock() = Some(format!("{operation}: {error}"));
    }

    pub(crate) fn drop_record(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn dropped_count(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }

    pub(crate) fn snapshot(&self) -> WriterDiagnostics {
        WriterDiagnostics {
            dropped_records: self.dropped_count(),
            failed_operations: self.failed.load(Ordering::Relaxed),
            last_error: self.last_error.lock().clone(),
        }
    }

    pub(crate) fn clear(&self) {
        self.dropped.store(0, Ordering::Relaxed);
        self.failed.store(0, Ordering::Relaxed);
        *self.last_error.lock() = None;
    }
}
