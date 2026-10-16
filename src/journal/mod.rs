pub mod buffer;
mod diagnostics;
pub mod fields;
pub mod rotation;
pub mod utils;
pub mod writer;

use crate::core::*;
use buffer::Buffer;

pub use diagnostics::WriterDiagnostics;
pub(crate) use diagnostics::WriterState;
pub use fields::Fields;
pub use rotation::LogPolicy;
pub use writer::{
    AsyncWriter, FileWriter, MultiWriter, RotatingFileWriter, StdoutWriter, WriteMode, Writer,
};

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct Journal {
    run_id: String,
    pub(crate) buffer: Buffer,
    writer: Option<Box<dyn Writer>>,
    next_scope_id: AtomicU64,
    next_record_id: AtomicU64,
    current_scope: Option<ScopeId>,
}

impl Journal {
    pub fn new() -> Self {
        Self {
            run_id: crate::format::new_run_id(),
            buffer: Buffer::new(),
            writer: None,
            next_scope_id: AtomicU64::new(0),
            next_record_id: AtomicU64::new(0),
            current_scope: None,
        }
    }

    pub fn with_writer(writer: impl Writer + 'static) -> Self {
        let mut journal = Self::new();
        journal.set_writer(writer);
        journal
    }

    pub fn set_writer(&mut self, mut writer: impl Writer + 'static) {
        writer.set_run_id(&self.run_id);
        if writer.wants_scope_metadata() {
            for scope in self.buffer.scopes_snapshot() {
                let _ = writer.write_scope(&scope);
            }
        }
        self.writer = Some(Box::new(writer));
    }

    /// Identity used by JSONL exports and writers for this journal's lifetime.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn enter_scope(&mut self, name: impl Into<String>) -> ScopeId {
        let id = self.enter_scope_with_parent(name, self.current_scope);
        self.current_scope = Some(id);
        id
    }

    pub fn enter_scope_with_parent(
        &mut self,
        name: impl Into<String>,
        parent: Option<ScopeId>,
    ) -> ScopeId {
        let started_at = Instant::now();
        let id = ScopeId(self.next_scope_id.fetch_add(1, Ordering::Relaxed));
        let scope = Scope {
            id,
            parent,
            entered_at: utils::current_millis(),
            name: Some(name.into()),
            exited_at: None,
            exit_messages: ExitMessages::default(),
        };
        self.buffer.push_scope_at(scope, started_at);
        self.write_scope_metadata(id);
        id
    }

    pub fn enter_scope_unnamed(&mut self, parent: Option<ScopeId>) -> ScopeId {
        let started_at = Instant::now();
        let id = ScopeId(self.next_scope_id.fetch_add(1, Ordering::Relaxed));
        let scope = Scope {
            id,
            parent: parent.or(self.current_scope),
            entered_at: utils::current_millis(),
            name: None,
            exited_at: None,
            exit_messages: ExitMessages::default(),
        };
        self.buffer.push_scope_at(scope, started_at);
        self.write_scope_metadata(id);
        self.current_scope = Some(id);
        id
    }

    fn write_scope_metadata(&mut self, id: ScopeId) {
        if let Some(writer) = &mut self.writer
            && writer.wants_scope_metadata()
        {
            let scope = self
                .buffer
                .get_scope_by_id(id)
                .expect("new scope is retained");
            let _ = writer.write_scope(&scope);
        }
    }

    pub fn set_scope_exit_messages(&mut self, id: ScopeId, msgs: ExitMessages) -> bool {
        self.buffer.set_scope_exit_messages(id, msgs)
    }

    #[inline]
    pub fn record_no_write(
        &mut self,
        kind: EventKind,
        name: impl Into<String>,
        fields: Fields,
    ) -> (RecordId, Record, Option<Scope>) {
        self.record_no_write_in_scope(kind, name, fields, self.current_scope)
    }

    #[inline]
    pub fn record_no_write_in_scope(
        &mut self,
        kind: EventKind,
        name: impl Into<String>,
        fields: Fields,
        scope_id: Option<ScopeId>,
    ) -> (RecordId, Record, Option<Scope>) {
        let id = RecordId(self.next_record_id.fetch_add(1, Ordering::Relaxed));
        let record = Record {
            id,
            scope: scope_id,
            time_ns: utils::current_nanos(),
            kind: RecordKind::Event {
                kind,
                name: name.into(),
                fields,
            },
        };
        let scope = scope_id.and_then(|sid| self.buffer.get_scope_by_id(sid));
        self.buffer.push_record(record.clone());
        (id, record, scope)
    }

    #[inline]
    pub fn exit_scope_no_write(
        &mut self,
        scope_id: ScopeId,
        outcome: Outcome,
    ) -> (RecordId, Option<(Record, Scope)>) {
        let finished_at = Instant::now();
        let id = RecordId(self.next_record_id.fetch_add(1, Ordering::Relaxed));
        let exited_at = utils::current_millis();

        if let Some((scope, elapsed)) =
            self.buffer
                .finalize_scope_exit_at(scope_id, exited_at, finished_at)
        {
            let duration_ns = utils::duration_nanos(elapsed);
            let record = Record {
                id,
                scope: Some(scope_id),
                time_ns: utils::current_nanos(),
                kind: RecordKind::ScopeExit {
                    outcome,
                    duration_ns,
                },
            };
            self.buffer.push_record(record.clone());
            if Some(scope_id) == self.current_scope {
                self.current_scope = scope.parent;
            }
            (id, Some((record, scope)))
        } else {
            (id, None)
        }
    }

    pub fn record(&mut self, kind: EventKind, name: impl Into<String>, fields: Fields) -> RecordId {
        let (id, record, scope) = self.record_no_write(kind, name, fields);
        if let Some(writer) = &mut self.writer {
            let _ = writer.write_record(&record, scope.as_ref());
        }
        id
    }

    pub fn exit_scope(&mut self, scope_id: ScopeId, outcome: Outcome) -> RecordId {
        let (id, payload) = self.exit_scope_no_write(scope_id, outcome);
        if let Some((record, scope)) = payload
            && let Some(writer) = &mut self.writer
        {
            let _ = writer.write_record(&record, Some(&scope));
        }
        id
    }

    pub fn trim_records(&mut self, max: usize) {
        self.buffer.trim_records(max);
    }

    pub fn trim_scopes(&mut self, max: usize) {
        self.buffer.trim_scopes(max);
    }

    pub fn get_scope(&self, id: ScopeId) -> Option<Scope> {
        self.buffer.get_scope_by_id(id)
    }

    /// Monotonic elapsed time for an open scope, or its frozen duration on exit.
    /// Returns `None` if the scope was cleared or trimmed from this journal.
    /// This is independent of the scope's wall-clock timestamps.
    ///
    /// ```
    /// let mut journal = eventline::Journal::new();
    /// let id = journal.enter_scope("request");
    /// assert!(journal.scope_elapsed(id).is_some());
    /// journal.exit_scope(id, eventline::core::Outcome::Success);
    /// let finished = journal.scope_elapsed(id).unwrap();
    /// assert_eq!(journal.scope_elapsed(id), Some(finished));
    /// ```
    pub fn scope_elapsed(&self, id: ScopeId) -> Option<Duration> {
        self.buffer.get_scope_elapsed(id)
    }

    /// Look up a retained record by ID. The first query builds an O(n) index;
    /// subsequent lookups are expected O(1), with the index kept up to date.
    pub fn get_record(&self, id: RecordId) -> Option<Record> {
        self.buffer.get_record_by_id(id)
    }

    pub fn records(&self) -> Vec<Record> {
        self.buffer.records_snapshot()
    }

    pub fn records_jsonl(&self) -> Vec<String> {
        let records = self.buffer.records_snapshot();
        let scopes = self.buffer.scopes_snapshot();
        let scopes_by_id = crate::replay::index_scopes(&scopes);

        let mut lines: Vec<_> = scopes
            .iter()
            .map(|scope| crate::render::render_scope_jsonl(scope, &self.run_id))
            .collect();
        lines.extend(records.iter().map(|record| {
            let scope = record.scope.and_then(|id| scopes_by_id.get(&id).copied());
            crate::render::render_jsonl_in_run(record, scope, &self.run_id)
        }));
        lines
    }

    pub fn scopes(&self) -> Vec<Scope> {
        self.buffer.scopes_snapshot()
    }

    pub fn flush(&mut self) -> io::Result<()> {
        if let Some(writer) = &mut self.writer {
            writer.flush()?;
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.current_scope = None;
    }

    pub fn scope_guard(&mut self, name: impl Into<String>) -> ScopeGuard<'_> {
        let id = self.enter_scope(name);
        ScopeGuard::new(self, id)
    }
}

impl Default for Journal {
    fn default() -> Self {
        Self::new()
    }
}
