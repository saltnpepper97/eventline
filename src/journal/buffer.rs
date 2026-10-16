use crate::core::{ExitMessages, Record, RecordId, Scope, ScopeId};
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct BufferedScope {
    scope: Scope,
    // Kept outside the public/serialized Scope so existing struct literals and
    // exported metadata remain compatible. Trimming a scope also drops its clock.
    clock: Option<ScopeClock>,
}

struct ScopeClock {
    started_at: Instant,
    finished: Option<Duration>,
}

impl BufferedScope {
    fn elapsed(&self) -> Duration {
        match &self.clock {
            Some(clock) => clock.finished.unwrap_or_else(|| clock.started_at.elapsed()),
            None => self.scope.elapsed(),
        }
    }
}

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
    scopes: Arc<RwLock<Vec<BufferedScope>>>,
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

    // -------------------------------------------------------------------------
    // Push
    // -------------------------------------------------------------------------

    #[inline]
    pub fn push_record(&self, record: Record) {
        self.records.write().push(record);
    }

    #[inline]
    /// Store portable scope metadata without assuming its monotonic start time.
    /// Use a live [`crate::Journal`] to create scopes with monotonic timing.
    pub fn push_scope(&self, scope: Scope) {
        // Externally supplied metadata has no known monotonic start time.
        self.scopes
            .write()
            .push(BufferedScope { scope, clock: None });
    }

    pub(crate) fn push_scope_at(&self, scope: Scope, started_at: Instant) {
        self.scopes.write().push(BufferedScope {
            scope,
            clock: Some(ScopeClock {
                started_at,
                finished: None,
            }),
        });
    }

    // -------------------------------------------------------------------------
    // Point look-ups
    // -------------------------------------------------------------------------

    pub fn get_record(&self, idx: usize) -> Option<Record> {
        self.records.read().get(idx).cloned()
    }

    pub fn get_scope(&self, idx: usize) -> Option<Scope> {
        self.scopes.read().get(idx).map(|entry| entry.scope.clone())
    }

    /// Look up a record by stable ID without cloning the whole buffer.
    pub fn get_record_by_id(&self, id: RecordId) -> Option<Record> {
        self.records.read().iter().find(|r| r.id == id).cloned()
    }

    /// Look up a scope by stable ID without cloning the whole buffer.
    pub fn get_scope_by_id(&self, id: ScopeId) -> Option<Scope> {
        self.scopes
            .read()
            .iter()
            .find(|s| s.scope.id == id)
            .map(|s| s.scope.clone())
    }

    /// Elapsed time for a scope created by a live journal, frozen on exit.
    /// Scopes inserted with [`Self::push_scope`] contain only portable metadata
    /// and fall back to the wall-clock estimate in [`Scope::elapsed`].
    pub fn get_scope_elapsed(&self, id: ScopeId) -> Option<Duration> {
        self.scopes
            .read()
            .iter()
            .find(|s| s.scope.id == id)
            .map(BufferedScope::elapsed)
    }

    // -------------------------------------------------------------------------
    // Scope mutation
    // -------------------------------------------------------------------------

    /// Set per-outcome exit messages for an existing scope.
    ///
    /// Returns `true` if the scope was found and updated.
    pub fn set_scope_exit_messages(&self, id: ScopeId, msgs: ExitMessages) -> bool {
        let mut scopes = self.scopes.write();
        if let Some(entry) = scopes.iter_mut().find(|s| s.scope.id == id) {
            let scope = &mut entry.scope;
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

    /// Finalise a scope by setting `exited_at`.
    ///
    /// Returns the updated scope snapshot, or `None` if the scope does not
    /// exist or was already finalised (idempotence).
    pub fn finalize_scope_exit(&self, id: ScopeId, exited_at_millis: u64) -> Option<Scope> {
        self.finalize_scope_exit_at(id, exited_at_millis, Instant::now())
            .map(|(scope, _)| scope)
    }

    pub(crate) fn finalize_scope_exit_at(
        &self,
        id: ScopeId,
        exited_at_millis: u64,
        exited_at: Instant,
    ) -> Option<(Scope, Duration)> {
        let mut scopes = self.scopes.write();
        let entry = scopes.iter_mut().find(|s| s.scope.id == id)?;
        let scope = &mut entry.scope;
        if scope.exited_at.is_some() {
            return None;
        }
        scope.exited_at = Some(exited_at_millis);
        let elapsed = match &mut entry.clock {
            Some(clock) => {
                let duration = exited_at.saturating_duration_since(clock.started_at);
                clock.finished = Some(duration);
                duration
            }
            None => scope.elapsed(),
        };
        Some((scope.clone(), elapsed))
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
        self.scopes
            .read()
            .iter()
            .map(|entry| entry.scope.clone())
            .collect()
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
        let exited_count = scopes
            .iter()
            .filter(|s| s.scope.exited_at.is_some())
            .count();
        if exited_count <= max {
            return;
        }
        let to_drop = exited_count - max;
        let mut dropped = 0;
        scopes.retain(|s| {
            if dropped < to_drop && s.scope.exited_at.is_some() {
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

#[cfg(test)]
mod tests {
    use super::Buffer;
    use crate::Journal;
    use crate::core::{ExitMessages, Outcome, RecordKind, Scope, ScopeId};
    use std::time::{Duration, Instant};

    fn scope(id: u64, entered_at: u64) -> Scope {
        Scope {
            id: ScopeId(id),
            parent: None,
            entered_at,
            name: Some("timed".into()),
            exited_at: None,
            exit_messages: ExitMessages::default(),
        }
    }

    #[test]
    fn finalization_ignores_forward_and_backward_wall_clock_changes() {
        let elapsed = Duration::from_nanos(1_234_567);
        for wall_exit in [1, 10_000, u64::MAX] {
            let buffer = Buffer::new();
            let start = Instant::now();
            buffer.push_scope_at(scope(1, 10_000), start);
            let (scope, duration) = buffer
                .finalize_scope_exit_at(ScopeId(1), wall_exit, start + elapsed)
                .unwrap();
            assert_eq!(duration, elapsed);
            assert_eq!(buffer.get_scope_elapsed(ScopeId(1)), Some(elapsed));
            assert_eq!(scope.entered_at, 10_000);
            assert_eq!(scope.exited_at, Some(wall_exit));
        }
    }

    #[test]
    fn finalized_duration_is_shared_by_buffer_clones_and_not_recomputed() {
        let buffer = Buffer::new();
        let cloned = buffer.clone();
        let start = Instant::now();
        let elapsed = Duration::from_nanos(987_654);
        buffer.push_scope_at(scope(1, 5), start);
        cloned
            .finalize_scope_exit_at(ScopeId(1), 6, start + elapsed)
            .unwrap();
        assert_eq!(buffer.get_scope_elapsed(ScopeId(1)), Some(elapsed));
        assert!(
            buffer
                .finalize_scope_exit_at(ScopeId(1), 100, start + Duration::from_secs(10))
                .is_none()
        );
        assert_eq!(cloned.get_scope_elapsed(ScopeId(1)), Some(elapsed));
        assert_eq!(
            buffer.get_scope_by_id(ScopeId(1)).unwrap().exited_at,
            Some(6)
        );
    }

    #[test]
    fn journal_exit_records_use_monotonic_timing_for_named_and_unnamed_scopes() {
        for wall_entry in [0, u64::MAX] {
            for unnamed in [false, true] {
                let mut journal = Journal::new();
                let id = if unnamed {
                    journal.enter_scope_unnamed(None)
                } else {
                    journal.enter_scope("scope")
                };
                let start = Instant::now() - Duration::from_secs(5);
                {
                    // Change only the stored wall clock, simulating a large
                    // adjustment without changing the host's system time.
                    let mut scopes = journal.buffer.scopes.write();
                    let entry = scopes
                        .iter_mut()
                        .find(|entry| entry.scope.id == id)
                        .unwrap();
                    entry.scope.entered_at = wall_entry;
                    entry.clock.as_mut().unwrap().started_at = start;
                }
                let before = start.elapsed();
                let record_id = journal.exit_scope(id, Outcome::Success);
                let after = start.elapsed();
                let record = journal.get_record(record_id).unwrap();
                let RecordKind::ScopeExit { duration_ns, .. } = record.kind else {
                    panic!("expected scope exit")
                };
                let elapsed = Duration::from_nanos(duration_ns);
                assert!(elapsed >= before);
                assert!(elapsed <= after);
                assert_eq!(journal.scope_elapsed(id), Some(elapsed));
                assert_eq!(journal.get_scope(id).unwrap().entered_at, wall_entry);
                assert!(journal.get_scope(id).unwrap().exited_at.is_some());
                let exported = journal
                    .records_jsonl()
                    .iter()
                    .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                    .find(|line| line["type"] == "scope_exit")
                    .unwrap();
                assert_eq!(exported["type"], "scope_exit");
                assert_eq!(exported["duration_ns"], duration_ns);
                assert_eq!(exported["time_ns"], record.time_ns);
                assert_eq!(exported["scope"]["entered_at"], wall_entry);
                let summary = crate::replay::scope_outcomes(&journal.records(), &journal.scopes())
                    .pop()
                    .unwrap();
                assert_eq!(summary.duration_ns, duration_ns);
                journal.exit_scope(id, Outcome::Failure);
                assert_eq!(journal.scope_elapsed(id), Some(elapsed));
                assert_eq!(journal.records().len(), 1);
            }
        }
    }

    #[test]
    fn live_queries_use_the_monotonic_start_instead_of_wall_metadata() {
        let mut journal = Journal::new();
        let id = journal.enter_scope("open");
        let start = Instant::now() - Duration::from_secs(5);
        {
            let mut scopes = journal.buffer.scopes.write();
            let entry = &mut scopes[0];
            entry.scope.entered_at = u64::MAX;
            entry.clock.as_mut().unwrap().started_at = start;
        }
        let before = start.elapsed();
        let elapsed = journal.scope_elapsed(id).unwrap();
        let after = start.elapsed();
        assert!(elapsed >= before);
        assert!(elapsed <= after);
        assert!(journal.get_scope(id).unwrap().exited_at.is_none());
    }

    #[test]
    fn trimming_and_clearing_remove_timing_state_with_the_scope() {
        let buffer = Buffer::with_capacity(0, 3);
        let start = Instant::now();
        for id in 1..=3 {
            buffer.push_scope_at(scope(id, 0), start);
        }
        buffer
            .finalize_scope_exit_at(ScopeId(1), 1, start + Duration::from_nanos(100))
            .unwrap();
        buffer
            .finalize_scope_exit_at(ScopeId(2), 1, start + Duration::from_nanos(200))
            .unwrap();
        buffer.trim_scopes(1);
        assert_eq!(buffer.scopes_len(), 2);
        assert_eq!(buffer.get_scope_elapsed(ScopeId(1)), None);
        assert_eq!(
            buffer.get_scope_elapsed(ScopeId(2)),
            Some(Duration::from_nanos(200))
        );
        assert!(buffer.get_scope_elapsed(ScopeId(3)).is_some());
        let cloned = buffer.clone();
        buffer.clear();
        assert_eq!(cloned.scopes_len(), 0);
        assert_eq!(cloned.get_scope_elapsed(ScopeId(2)), None);
        assert_eq!(cloned.get_scope_elapsed(ScopeId(3)), None);
    }

    #[test]
    fn portable_scope_metadata_keeps_its_legacy_shape_and_wall_clock_estimate() {
        let buffer = Buffer::new();
        // Preserve external struct-literal construction as well as JSON data.
        let mut original = scope(1, 10);
        original.exited_at = Some(14);
        let json = serde_json::to_value(&original).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 6);
        assert_eq!(json["entered_at"], 10);
        assert_eq!(json["exited_at"], 14);
        let restored: Scope = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(restored.elapsed(), Duration::from_millis(4));
        buffer.push_scope(restored);
        assert_eq!(
            buffer.get_scope_elapsed(ScopeId(1)),
            Some(Duration::from_millis(4))
        );
        assert_eq!(
            serde_json::to_value(buffer.get_scope(0).unwrap()).unwrap(),
            json
        );
    }
}
