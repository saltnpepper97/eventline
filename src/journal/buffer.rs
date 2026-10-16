use crate::core::{ExitMessages, Record, RecordId, Scope, ScopeId};
use parking_lot::RwLock;
use std::collections::{BTreeSet, HashMap, VecDeque};
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

// Positions are logical offsets, so popping from the front never requires
// updating every retained ID. Duplicate IDs supplied through the public Buffer
// API retain the old "first retained occurrence" lookup behavior.
struct Position {
    first: usize,
    count: usize,
}

#[derive(Default)]
struct Records {
    entries: VecDeque<Record>,
    // Most applications record without querying individual record IDs. Build
    // this index on the first lookup, then maintain it on subsequent updates.
    by_id: Option<HashMap<RecordId, Position>>,
    base: usize,
}

impl Records {
    fn push(&mut self, record: Record) {
        if self
            .base
            .checked_add(self.entries.len())
            .is_none_or(|key| key == usize::MAX)
        {
            self.reindex();
        }
        let key = self.base + self.entries.len();
        if let Some(index) = &mut self.by_id {
            index
                .entry(record.id)
                .and_modify(|position| position.count += 1)
                .or_insert(Position {
                    first: key,
                    count: 1,
                });
        }
        self.entries.push_back(record);
    }

    fn reindex(&mut self) {
        self.base = 0;
        if self.by_id.is_some() {
            self.build_index();
        }
    }

    fn build_index(&mut self) {
        let index = self
            .by_id
            .get_or_insert_with(|| HashMap::with_capacity(self.entries.len()));
        index.clear();
        for (key, record) in self.entries.iter().enumerate() {
            index
                .entry(record.id)
                .and_modify(|position| position.count += 1)
                .or_insert(Position {
                    first: self.base + key,
                    count: 1,
                });
        }
    }

    fn trim(&mut self, max: usize) {
        while self.entries.len() > max {
            let record = self.entries.pop_front().unwrap();
            self.base += 1;
            if let Some(index) = &mut self.by_id {
                let position = index.get_mut(&record.id).unwrap();
                if position.count == 1 {
                    index.remove(&record.id);
                } else {
                    position.count -= 1;
                    position.first =
                        self.base + self.entries.iter().position(|r| r.id == record.id).unwrap();
                }
            }
        }
        if self.entries.is_empty() {
            self.base = 0;
        }
    }
}

#[derive(Default)]
struct Scopes {
    // Vacant slots let us evict old closed scopes even when older scopes remain
    // open. Periodic compaction amortizes the cost of moving retained scopes.
    entries: VecDeque<Option<BufferedScope>>,
    by_id: HashMap<ScopeId, Position>,
    exited: BTreeSet<usize>,
    base: usize,
    len: usize,
}

impl Scopes {
    fn push(&mut self, entry: BufferedScope) {
        if self
            .base
            .checked_add(self.entries.len())
            .is_none_or(|key| key == usize::MAX)
        {
            self.compact();
        }
        let key = self.base + self.entries.len();
        self.by_id
            .entry(entry.scope.id)
            .and_modify(|position| position.count += 1)
            .or_insert(Position {
                first: key,
                count: 1,
            });
        if entry.scope.exited_at.is_some() {
            self.exited.insert(key);
        }
        self.entries.push_back(Some(entry));
        self.len += 1;
    }

    fn get(&self, id: ScopeId) -> Option<&BufferedScope> {
        let key = self.by_id.get(&id)?.first;
        self.entries.get(key - self.base)?.as_ref()
    }

    fn get_mut(&mut self, id: ScopeId) -> Option<&mut BufferedScope> {
        let key = self.by_id.get(&id)?.first;
        self.entries.get_mut(key - self.base)?.as_mut()
    }

    fn iter(&self) -> impl Iterator<Item = &BufferedScope> {
        self.entries.iter().filter_map(Option::as_ref)
    }

    fn compact(&mut self) {
        self.entries.retain(Option::is_some);
        self.base = 0;
        self.by_id.clear();
        self.exited.clear();
        for (key, entry) in self.entries.iter().enumerate() {
            let scope = &entry.as_ref().unwrap().scope;
            self.by_id
                .entry(scope.id)
                .and_modify(|position| position.count += 1)
                .or_insert(Position {
                    first: key,
                    count: 1,
                });
            if scope.exited_at.is_some() {
                self.exited.insert(key);
            }
        }
    }

    fn trim(&mut self, max: usize) {
        while self.exited.len() > max {
            // Eviction follows insertion order, not the order scopes exited.
            let key = self.exited.pop_first().unwrap();
            let entry = self.entries[key - self.base].take().unwrap();
            self.len -= 1;
            let position = self.by_id.get_mut(&entry.scope.id).unwrap();
            if position.count == 1 {
                self.by_id.remove(&entry.scope.id);
            } else {
                position.count -= 1;
                if position.first == key {
                    position.first = self.base
                        + self
                            .entries
                            .iter()
                            .position(|candidate| {
                                candidate
                                    .as_ref()
                                    .is_some_and(|s| s.scope.id == entry.scope.id)
                            })
                            .unwrap();
                }
            }
        }
        while self.entries.front().is_some_and(Option::is_none) {
            self.entries.pop_front();
            self.base += 1;
        }
        if self.entries.is_empty() {
            self.base = 0;
        } else if self.entries.len() > 64 && self.entries.len() - self.len > self.len {
            // A long-lived open scope must not pin unbounded vacant slots.
            self.compact();
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
/// - ID indexes and entries share a lock, so cloned buffers observe consistent
///   updates. Snapshots preserve insertion order.
/// - Records use a deque for front eviction; scope storage also permits removal
///   of closed entries behind long-lived open scopes without shifting on every
///   trim. Vacant slots are periodically compacted.
#[derive(Clone)]
pub struct Buffer {
    records: Arc<RwLock<Records>>,
    scopes: Arc<RwLock<Scopes>>,
}

impl Buffer {
    pub fn new() -> Self {
        Self::with_capacity(0, 0)
    }

    pub fn with_capacity(records: usize, scopes: usize) -> Self {
        Self {
            records: Arc::new(RwLock::new(Records {
                entries: VecDeque::with_capacity(records),
                ..Records::default()
            })),
            scopes: Arc::new(RwLock::new(Scopes {
                entries: VecDeque::with_capacity(scopes),
                by_id: HashMap::with_capacity(scopes),
                ..Scopes::default()
            })),
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
        self.records.read().entries.get(idx).cloned()
    }

    /// Positional access in insertion order. May scan vacant slots after trims;
    /// use [`Self::get_scope_by_id`] for indexed access.
    pub fn get_scope(&self, idx: usize) -> Option<Scope> {
        let scopes = self.scopes.read();
        let entry = if scopes.len == scopes.entries.len() {
            scopes.entries.get(idx).and_then(Option::as_ref)
        } else {
            scopes.iter().nth(idx)
        };
        entry.map(|entry| entry.scope.clone())
    }

    /// Look up a record by stable ID without cloning the whole buffer.
    /// The first query builds an O(n) index; subsequent lookups are expected
    /// O(1). Recording-only users avoid the record index's memory/update cost.
    pub fn get_record_by_id(&self, id: RecordId) -> Option<Record> {
        let records = self.records.read();
        if let Some(index) = &records.by_id {
            let key = index.get(&id)?.first;
            return records.entries.get(key - records.base).cloned();
        }
        drop(records);
        let mut records = self.records.write();
        if records.by_id.is_none() {
            records.build_index();
        }
        let key = records.by_id.as_ref().unwrap().get(&id)?.first;
        records.entries.get(key - records.base).cloned()
    }

    /// Look up a scope by stable ID without cloning the whole buffer.
    pub fn get_scope_by_id(&self, id: ScopeId) -> Option<Scope> {
        self.scopes.read().get(id).map(|s| s.scope.clone())
    }

    /// Elapsed time for a scope created by a live journal, frozen on exit.
    /// Scopes inserted with [`Self::push_scope`] contain only portable metadata
    /// and fall back to the wall-clock estimate in [`Scope::elapsed`].
    pub fn get_scope_elapsed(&self, id: ScopeId) -> Option<Duration> {
        self.scopes.read().get(id).map(BufferedScope::elapsed)
    }

    // -------------------------------------------------------------------------
    // Scope mutation
    // -------------------------------------------------------------------------

    /// Set per-outcome exit messages for an existing scope.
    ///
    /// Returns `true` if the scope was found and updated.
    pub fn set_scope_exit_messages(&self, id: ScopeId, msgs: ExitMessages) -> bool {
        let mut scopes = self.scopes.write();
        if let Some(entry) = scopes.get_mut(id) {
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
        let entry = scopes.get_mut(id)?;
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
        let result = (scope.clone(), elapsed);
        let key = scopes.by_id[&id].first;
        scopes.exited.insert(key);
        Some(result)
    }

    // -------------------------------------------------------------------------
    // Lengths
    // -------------------------------------------------------------------------

    #[inline]
    pub fn records_len(&self) -> usize {
        self.records.read().entries.len()
    }

    #[inline]
    pub fn scopes_len(&self) -> usize {
        self.scopes.read().len
    }

    // -------------------------------------------------------------------------
    // Bulk snapshots (debug / export)
    // -------------------------------------------------------------------------

    pub fn records_snapshot(&self) -> Vec<Record> {
        self.records.read().entries.iter().cloned().collect()
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
        self.records.write().trim(max);
    }

    /// Drop the oldest *exited* scopes, keeping at most `max` exited entries.
    ///
    /// Open scopes (`exited_at` is `None`) are always retained because they
    /// may still receive exit events or message updates.
    pub fn trim_scopes(&self, max: usize) {
        self.scopes.write().trim(max);
    }

    pub fn clear(&self) {
        let mut records = self.records.write();
        records.entries.clear();
        if let Some(index) = &mut records.by_id {
            index.clear();
        }
        records.base = 0;
        let mut scopes = self.scopes.write();
        scopes.entries.clear();
        scopes.by_id.clear();
        scopes.exited.clear();
        scopes.base = 0;
        scopes.len = 0;
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
    use crate::core::{ExitMessages, Outcome, Record, RecordId, RecordKind, Scope, ScopeId};
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

    fn record(id: u64, tag: u64) -> Record {
        Record {
            id: RecordId(id),
            scope: None,
            time_ns: tag,
            kind: RecordKind::ScopeExit {
                outcome: Outcome::Success,
                duration_ns: tag,
            },
        }
    }

    #[test]
    fn indexed_storage_matches_a_vector_model_through_mixed_updates() {
        let buffer = Buffer::with_capacity(8, 8);
        let mut records: Vec<Record> = Vec::new();
        let mut scopes: Vec<Scope> = Vec::new();
        let mut random = 0x73ab_c81d_u64;
        for step in 0..4000 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let id = (random >> 32) % 37;
            match (random >> 24) % 9 {
                0 | 1 => {
                    let entry = record(id, step);
                    buffer.push_record(entry.clone());
                    records.push(entry);
                }
                2 | 3 => {
                    let mut entry = scope(id, step);
                    if random & 1 == 0 {
                        entry.exited_at = Some(step + 1);
                    }
                    buffer.push_scope(entry.clone());
                    scopes.push(entry);
                }
                4 => {
                    let expected = scopes
                        .iter_mut()
                        .find(|s| s.id == ScopeId(id))
                        .and_then(|s| {
                            if s.exited_at.is_some() {
                                None
                            } else {
                                s.exited_at = Some(step);
                                Some(s.clone())
                            }
                        });
                    assert_eq!(
                        serde_json::to_value(buffer.finalize_scope_exit(ScopeId(id), step))
                            .unwrap(),
                        serde_json::to_value(expected).unwrap()
                    );
                }
                5 => {
                    let max = id as usize;
                    records.drain(..records.len().saturating_sub(max));
                    buffer.trim_records(max);
                }
                6 => {
                    let mut remaining = scopes
                        .iter()
                        .filter(|s| s.exited_at.is_some())
                        .count()
                        .saturating_sub(id as usize);
                    scopes.retain(|s| {
                        if s.exited_at.is_some() && remaining > 0 {
                            remaining -= 1;
                            false
                        } else {
                            true
                        }
                    });
                    buffer.trim_scopes(id as usize);
                }
                7 => {
                    let messages = ExitMessages {
                        success: Some(step.to_string()),
                        ..ExitMessages::default()
                    };
                    let expected = scopes.iter_mut().find(|s| s.id == ScopeId(id));
                    let found = expected.is_some();
                    if let Some(scope) = expected {
                        scope.exit_messages.success.clone_from(&messages.success);
                    }
                    assert_eq!(buffer.set_scope_exit_messages(ScopeId(id), messages), found);
                }
                _ if random & 31 == 0 => {
                    buffer.clone().clear();
                    records.clear();
                    scopes.clear();
                }
                _ => {}
            }
            assert_eq!(buffer.records_len(), records.len());
            assert_eq!(buffer.scopes_len(), scopes.len());
            assert_eq!(
                serde_json::to_value(buffer.records_snapshot()).unwrap(),
                serde_json::to_value(&records).unwrap()
            );
            assert_eq!(
                serde_json::to_value(buffer.scopes_snapshot()).unwrap(),
                serde_json::to_value(&scopes).unwrap()
            );
            let expected = records.iter().find(|r| r.id == RecordId(id));
            assert_eq!(
                buffer.get_record_by_id(RecordId(id)).map(|r| r.time_ns),
                expected.map(|r| r.time_ns)
            );
            let expected = scopes.iter().find(|s| s.id == ScopeId(id));
            assert_eq!(
                serde_json::to_value(buffer.get_scope_by_id(ScopeId(id))).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
            let idx = id as usize;
            assert_eq!(
                buffer.get_record(idx).map(|r| r.time_ns),
                records.get(idx).map(|r| r.time_ns)
            );
            assert_eq!(
                serde_json::to_value(buffer.get_scope(idx)).unwrap(),
                serde_json::to_value(scopes.get(idx)).unwrap()
            );
        }
    }

    #[test]
    fn long_lived_open_scopes_do_not_pin_unbounded_vacant_slots() {
        let buffer = Buffer::new();
        let start = Instant::now();
        buffer.push_scope_at(scope(0, 0), start);
        for id in 1..=10_000 {
            buffer.push_scope_at(scope(id, 0), start);
            buffer.finalize_scope_exit(ScopeId(id), 1).unwrap();
            buffer.trim_scopes(16);
            assert_eq!(buffer.scopes_len(), 1 + (id as usize).min(16));
            assert!(buffer.scopes.read().entries.len() <= 64);
        }
        assert!(buffer.get_scope_elapsed(ScopeId(0)).is_some());
        for id in 9985..=10_000 {
            assert_eq!(
                buffer.get_scope_by_id(ScopeId(id)).unwrap().exited_at,
                Some(1)
            );
        }
        assert!(buffer.get_scope_by_id(ScopeId(9984)).is_none());
        let scopes = buffer.scopes_snapshot();
        assert_eq!(scopes[0].id, ScopeId(0));
        assert_eq!(scopes[1].id, ScopeId(9985));
        buffer.trim_scopes(0);
        assert_eq!(buffer.scopes_snapshot().len(), 1);
        assert!(buffer.finalize_scope_exit(ScopeId(0), 2).is_some());
        buffer.trim_scopes(0);
        assert_eq!(buffer.scopes_len(), 0);
        assert!(buffer.scopes.read().entries.is_empty());
    }

    #[test]
    fn trimming_closed_scopes_uses_entry_order_even_when_exits_are_reversed() {
        let buffer = Buffer::new();
        for id in 0..200 {
            buffer.push_scope(scope(id, 0));
        }
        for id in (0..200).rev().filter(|id| id % 3 != 0) {
            buffer.finalize_scope_exit(ScopeId(id), 1).unwrap();
        }
        buffer.trim_scopes(7);
        let scopes = buffer.scopes_snapshot();
        let closed: Vec<_> = scopes
            .iter()
            .filter(|s| s.exited_at.is_some())
            .map(|s| s.id)
            .collect();
        let mut expected: Vec<_> = (0..200).filter(|id| id % 3 != 0).map(ScopeId).collect();
        expected.drain(..expected.len() - 7);
        assert_eq!(closed, expected);
        assert_eq!(scopes.iter().filter(|s| s.exited_at.is_none()).count(), 67);
        for scope in &scopes {
            assert!(buffer.get_scope_by_id(scope.id).is_some());
        }
    }

    #[test]
    fn logical_positions_are_rebased_before_overflow() {
        let buffer = Buffer::new();
        buffer.push_record(record(3, 1));
        assert!(buffer.get_record_by_id(RecordId(3)).is_some());
        {
            let mut records = buffer.records.write();
            records.base = usize::MAX - 2;
            records
                .by_id
                .as_mut()
                .unwrap()
                .get_mut(&RecordId(3))
                .unwrap()
                .first = records.base;
        }
        buffer.push_record(record(7, 2));
        buffer.push_record(record(9, 3));
        buffer.trim_records(2);
        assert_eq!(buffer.get_record_by_id(RecordId(7)).unwrap().time_ns, 2);
        assert!(buffer.get_record_by_id(RecordId(3)).is_none());

        buffer.push_scope(scope(3, 1));
        {
            let mut scopes = buffer.scopes.write();
            scopes.base = usize::MAX - 2;
            scopes.by_id.get_mut(&ScopeId(3)).unwrap().first = scopes.base;
        }
        buffer.push_scope(scope(7, 2));
        buffer.finalize_scope_exit(ScopeId(7), 3).unwrap();
        buffer.push_scope(scope(9, 3)); // Rebuilds closed-scope and ID indexes.
        buffer.trim_scopes(0);
        assert!(buffer.get_scope_by_id(ScopeId(7)).is_none());
        assert_eq!(buffer.get_scope(0).unwrap().id, ScopeId(3));
        assert_eq!(buffer.get_scope(1).unwrap().id, ScopeId(9));
    }

    #[test]
    fn record_index_is_lazy_and_starts_from_the_retained_deque() {
        let buffer = Buffer::new();
        for id in 0..300 {
            buffer.push_record(record(id, id));
        }
        buffer.trim_records(8);
        assert!(buffer.records.read().by_id.is_none());
        assert!(buffer.get_record_by_id(RecordId(0)).is_none());
        assert_eq!(buffer.records.read().by_id.as_ref().unwrap().len(), 8);
        assert_eq!(buffer.get_record_by_id(RecordId(292)).unwrap().time_ns, 292);
        for id in 300..600 {
            buffer.push_record(record(id, id));
            buffer.trim_records(8);
        }
        assert!(buffer.get_record_by_id(RecordId(292)).is_none());
        assert_eq!(buffer.get_record_by_id(RecordId(599)).unwrap().time_ns, 599);
        assert_eq!(buffer.records.read().by_id.as_ref().unwrap().len(), 8);
        buffer.clear();
        buffer.push_record(record(599, 42));
        assert_eq!(buffer.get_record_by_id(RecordId(599)).unwrap().time_ns, 42);
    }

    #[test]
    fn cloned_buffers_support_concurrent_insert_finalize_trim_and_query() {
        let buffer = Buffer::new();
        std::thread::scope(|workers| {
            for worker in 0..4 {
                let buffer = buffer.clone();
                workers.spawn(move || {
                    for offset in 0..2000 {
                        let id = worker * 10_000 + offset;
                        buffer.push_scope(scope(id, 0));
                        buffer.push_record(record(id, offset));
                        assert!(buffer.get_scope_by_id(ScopeId(id)).is_some());
                        buffer.finalize_scope_exit(ScopeId(id), 1).unwrap();
                        buffer.trim_records(128);
                        buffer.trim_scopes(64);
                        for snapshot in buffer.scopes_snapshot() {
                            if let Some(current) = buffer.get_scope_by_id(snapshot.id) {
                                assert_eq!(current.id, snapshot.id);
                            }
                        }
                    }
                });
            }
        });
        assert_eq!(buffer.records_len(), 128);
        assert_eq!(buffer.scopes_len(), 64);
        for record in buffer.records_snapshot() {
            assert_eq!(
                buffer.get_record_by_id(record.id).unwrap().time_ns,
                record.time_ns
            );
        }
        for scope in buffer.scopes_snapshot() {
            assert_eq!(buffer.get_scope_by_id(scope.id).unwrap().exited_at, Some(1));
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
                    let entry = scopes.get_mut(id).unwrap();
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
            let entry = scopes.get_mut(id).unwrap();
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
