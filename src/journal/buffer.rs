use crate::core::{ExitMessages, Record, RecordId, Scope, ScopeId};
use parking_lot::RwLock;
use std::sync::Arc;

/// Thread-safe buffer for journal records and scopes.
///
/// Design notes:
/// - Records are append-only up to the configured cap; oldest are evicted.
/// - Scopes are created once (enter) and later *finalized* exactly once (exit)
///   by filling `exited_at`. This does not violate append-only record history; it
///   completes scope metadata needed for duration/outcome analysis and replay.
#[derive(Clone)]
pub struct Buffer {
    records: Arc<RwLock<Vec<Record>>>,
    scopes: Arc<RwLock<Vec<Scope>>>,
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            records: Arc::new(RwLock::new(Vec::new())),
            scopes: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn with_capacity(records: usize, scopes: usize) -> Self {
        Self {
            records: Arc::new(RwLock::new(Vec::with_capacity(records))),
            scopes: Arc::new(RwLock::new(Vec::with_capacity(scopes))),
        }
    }

    pub fn push_record(&self, record: Record) {
        self.records.write().push(record);
    }

    pub fn push_scope(&self, scope: Scope) {
        self.scopes.write().push(scope);
    }

    pub fn get_record(&self, idx: usize) -> Option<Record> {
        self.records.read().get(idx).cloned()
    }

    pub fn get_scope(&self, idx: usize) -> Option<Scope> {
        self.scopes.read().get(idx).cloned()
    }

    /// Lookup a record by its stable ID without cloning the whole buffer.
    pub fn get_record_by_id(&self, id: RecordId) -> Option<Record> {
        self.records
            .read()
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    /// Lookup a scope by its stable ID without cloning the whole buffer.
    pub fn get_scope_by_id(&self, id: ScopeId) -> Option<Scope> {
        self.scopes
            .read()
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }

    /// Set per-outcome exit messages for an existing scope.
    ///
    /// Returns `true` if the scope existed and was updated, `false` otherwise.
    ///
    /// Notes:
    /// - This is safe to call any time after `enter_scope` (even after exit),
    ///   but typically you set it right after entering.
    pub fn set_scope_exit_messages(&self, id: ScopeId, msgs: ExitMessages) -> bool {
        let mut scopes = self.scopes.write();
        if let Some(scope) = scopes.iter_mut().find(|s| s.id == id) {
            if msgs.success.is_some() {
                scope.exit_messages.success = msgs.success;
            }
            if msgs.failure.is_some() {
                scope.exit_messages.failure = msgs.failure;
            }
            if msgs.aborted.is_some() {
                scope.exit_messages.aborted = msgs.aborted;
            }
            true
        } else {
            false
        }
    }

    /// Finalize (close) a scope by setting `exited_at` if it has not already been set.
    ///
    /// Returns the updated scope snapshot, or `None` if:
    /// - scope does not exist, or
    /// - scope was already finalized (idempotence / safety).
    pub fn finalize_scope_exit(&self, id: ScopeId, exited_at_millis: u64) -> Option<Scope> {
        let mut scopes = self.scopes.write();
        let scope = scopes.iter_mut().find(|s| s.id == id)?;

        if scope.exited_at.is_some() {
            return None;
        }

        scope.exited_at = Some(exited_at_millis);
        Some(scope.clone())
    }

    pub fn records_len(&self) -> usize {
        self.records.read().len()
    }

    pub fn scopes_len(&self) -> usize {
        self.scopes.read().len()
    }

    /// Get a snapshot of all records (bulk export/debug).
    pub fn records_snapshot(&self) -> Vec<Record> {
        self.records.read().clone()
    }

    /// Get a snapshot of all scopes (bulk export/debug).
    pub fn scopes_snapshot(&self) -> Vec<Scope> {
        self.scopes.read().clone()
    }

    /// Drop the oldest records, keeping at most `max` entries.
    ///
    /// Scopes are left untouched: a scope may still be open (no `exited_at`)
    /// when its early records are trimmed, and removing it would orphan the
    /// exit event. Scope count is bounded by the number of concurrent/recent
    /// scopes rather than total lifetime record count, so it does not grow
    /// unboundedly the same way.
    pub fn trim_records(&self, max: usize) {
        let mut records = self.records.write();
        let len = records.len();
        if len > max {
            records.drain(..len - max);
        }
    }

    pub fn clear(&self) {
        self.records.write().clear();
        self.scopes.write().clear();
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}
