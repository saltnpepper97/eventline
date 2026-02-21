use crate::core::{ExitMessages, Record, RecordId, Scope, ScopeId};
use parking_lot::RwLock;
use std::sync::Arc;

/// Thread-safe buffer for journal records and scopes.
///
/// ### Design notes
///
/// - Records are append-only up to the configured cap; oldest are evicted on
///   trim.
/// - Scopes are created once (`push_scope`) and later *finalised* exactly once
///   (`finalize_scope_exit`) by setting `exited_at`.  This completes scope
///   metadata needed for duration/outcome analysis and does not violate the
///   append-only record contract.
/// - The inner `Arc<RwLock<Vec<_>>>` lets the `Buffer` be cheaply cloned (e.g.
///   for snapshot export) while still sharing the underlying allocation.
#[derive(Clone)]
pub struct Buffer {
    records: Arc<RwLock<Vec<Record>>>,
    scopes:  Arc<RwLock<Vec<Scope>>>,
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            records: Arc::new(RwLock::new(Vec::new())),
            scopes:  Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn with_capacity(records: usize, scopes: usize) -> Self {
        Self {
            records: Arc::new(RwLock::new(Vec::with_capacity(records))),
            scopes:  Arc::new(RwLock::new(Vec::with_capacity(scopes))),
        }
    }

    // -------------------------------------------------------------------------
    // Push
    // -------------------------------------------------------------------------

    #[inline]
    pub fn push_record(&self, record: Record) {
        self.records.write().push(record);
    }

    #[inline]
    pub fn push_scope(&self, scope: Scope) {
        self.scopes.write().push(scope);
    }

    // -------------------------------------------------------------------------
    // Point look-ups
    // -------------------------------------------------------------------------

    pub fn get_record(&self, idx: usize) -> Option<Record> {
        self.records.read().get(idx).cloned()
    }

    pub fn get_scope(&self, idx: usize) -> Option<Scope> {
        self.scopes.read().get(idx).cloned()
    }

    /// Look up a record by stable ID without cloning the whole buffer.
    pub fn get_record_by_id(&self, id: RecordId) -> Option<Record> {
        self.records.read().iter().find(|r| r.id == id).cloned()
    }

    /// Look up a scope by stable ID without cloning the whole buffer.
    pub fn get_scope_by_id(&self, id: ScopeId) -> Option<Scope> {
        self.scopes.read().iter().find(|s| s.id == id).cloned()
    }

    // -------------------------------------------------------------------------
    // Scope mutation
    // -------------------------------------------------------------------------

    /// Set per-outcome exit messages for an existing scope.
    ///
    /// Returns `true` if the scope was found and updated.
    pub fn set_scope_exit_messages(&self, id: ScopeId, msgs: ExitMessages) -> bool {
        let mut scopes = self.scopes.write();
        if let Some(scope) = scopes.iter_mut().find(|s| s.id == id) {
            if msgs.success.is_some()  { scope.exit_messages.success  = msgs.success;  }
            if msgs.failure.is_some()  { scope.exit_messages.failure  = msgs.failure;  }
            if msgs.aborted.is_some()  { scope.exit_messages.aborted  = msgs.aborted;  }
            true
        } else {
            false
        }
    }

    /// Finalise a scope by setting `exited_at`.
    ///
    /// Returns the updated scope snapshot, or `None` if the scope does not
    /// exist or was already finalised (idempotence).
    pub fn finalize_scope_exit(&self, id: ScopeId, exited_at_millis: u64) -> Option<Scope> {
        let mut scopes = self.scopes.write();
        let scope = scopes.iter_mut().find(|s| s.id == id)?;
        if scope.exited_at.is_some() {
            return None;
        }
        scope.exited_at = Some(exited_at_millis);
        Some(scope.clone())
    }

    // -------------------------------------------------------------------------
    // Lengths
    // -------------------------------------------------------------------------

    #[inline]
    pub fn records_len(&self) -> usize {
        self.records.read().len()
    }

    #[inline]
    pub fn scopes_len(&self) -> usize {
        self.scopes.read().len()
    }

    // -------------------------------------------------------------------------
    // Bulk snapshots (debug / export)
    // -------------------------------------------------------------------------

    pub fn records_snapshot(&self) -> Vec<Record> {
        self.records.read().clone()
    }

    pub fn scopes_snapshot(&self) -> Vec<Scope> {
        self.scopes.read().clone()
    }

    // -------------------------------------------------------------------------
    // Trim
    // -------------------------------------------------------------------------

    /// Drop the oldest records, keeping at most `max` entries.
    pub fn trim_records(&self, max: usize) {
        let mut records = self.records.write();
        let len = records.len();
        if len > max {
            records.drain(..len - max);
        }
    }

    /// Drop the oldest *exited* scopes, keeping at most `max` exited entries.
    ///
    /// Open scopes (`exited_at` is `None`) are always retained because they
    /// may still receive exit events or message updates.
    pub fn trim_scopes(&self, max: usize) {
        let mut scopes = self.scopes.write();
        let exited_count = scopes.iter().filter(|s| s.exited_at.is_some()).count();
        if exited_count <= max {
            return;
        }
        let to_drop = exited_count - max;
        let mut dropped = 0;
        scopes.retain(|s| {
            if dropped < to_drop && s.exited_at.is_some() {
                dropped += 1;
                false
            } else {
                true
            }
        });
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
