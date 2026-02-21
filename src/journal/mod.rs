pub mod buffer;
pub mod fields;
pub mod rotation;
pub mod utils;
pub mod writer;

use crate::core::*;
use buffer::Buffer;

pub use fields::Fields;
pub use rotation::LogPolicy;
pub use writer::{FileWriter, MultiWriter, RotatingFileWriter, StdoutWriter, SyncWriter, Writer};

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

/// The core journal — an append-only log of events and scopes.
///
/// ### Design
///
/// The `Journal` owns the in-memory `Buffer` and the optional direct `Writer`
/// (used when `Journal` is constructed standalone, outside of the global
/// `Runtime`).
///
/// When operated through the global `Runtime` the write path is intentionally
/// split into two halves:
///
/// 1. **`record_no_write` / `exit_scope_no_write`** — called *while* holding
///    the `Mutex<Journal>`.  These push the record to the buffer and return a
///    snapshot of the record + current scope so the caller can emit it.
/// 2. **Writer call** — performed by `runtime::emit` *after* releasing the
///    journal mutex, against a separately held writer lock.
///
/// This means the journal mutex is held only for the duration of a buffer push
/// (a `Vec::push` plus a couple of atomic increments), not for the potentially
/// slow file-system write.  Other threads can log concurrently instead of
/// queuing behind I/O.
///
/// ### Invariants
///
/// - Records are append-only and never rewritten.
/// - Scopes are created on enter and finalized exactly once on exit.
/// - The configured global log level gates *emission* only; every record is
///   always pushed to the buffer for post-mortem access.
pub struct Journal {
    pub(crate) buffer: Buffer,
    /// Writer used only when the journal is driven *directly* (i.e., not
    /// through `runtime::emit`).  `None` when the global runtime is in
    /// control.
    writer: Option<Box<dyn Writer>>,
    next_scope_id:  AtomicU64,
    next_record_id: AtomicU64,
    current_scope:  Option<ScopeId>,
}

impl Journal {
    pub fn new() -> Self {
        Self {
            buffer:         Buffer::new(),
            writer:         None,
            next_scope_id:  AtomicU64::new(0),
            next_record_id: AtomicU64::new(0),
            current_scope:  None,
        }
    }

    pub fn with_writer(writer: impl Writer + 'static) -> Self {
        Self {
            buffer:         Buffer::new(),
            writer:         Some(Box::new(writer)),
            next_scope_id:  AtomicU64::new(0),
            next_record_id: AtomicU64::new(0),
            current_scope:  None,
        }
    }

    pub fn set_writer(&mut self, writer: impl Writer + 'static) {
        self.writer = Some(Box::new(writer));
    }

    // -------------------------------------------------------------------------
    // Scope management
    // -------------------------------------------------------------------------

    pub fn enter_scope(&mut self, name: impl Into<String>) -> ScopeId {
        let id = ScopeId(self.next_scope_id.fetch_add(1, Ordering::Relaxed));
        let scope = Scope {
            id,
            parent:        self.current_scope,
            entered_at:    utils::current_millis(),
            name:          Some(name.into()),
            exited_at:     None,
            exit_messages: ExitMessages::default(),
        };
        self.buffer.push_scope(scope);
        self.current_scope = Some(id);
        id
    }

    pub fn enter_scope_unnamed(&mut self, parent: Option<ScopeId>) -> ScopeId {
        let id = ScopeId(self.next_scope_id.fetch_add(1, Ordering::Relaxed));
        let scope = Scope {
            id,
            parent:        parent.or(self.current_scope),
            entered_at:    utils::current_millis(),
            name:          None,
            exited_at:     None,
            exit_messages: ExitMessages::default(),
        };
        self.buffer.push_scope(scope);
        self.current_scope = Some(id);
        id
    }

    pub fn set_scope_exit_messages(&mut self, id: ScopeId, msgs: ExitMessages) -> bool {
        self.buffer.set_scope_exit_messages(id, msgs)
    }

    // -------------------------------------------------------------------------
    // Record + scope-exit — SPLIT API
    //
    // `*_no_write` variants push to the buffer and return the data the caller
    // needs to drive the writer *outside* this struct's lock region.
    //
    // `record` / `exit_scope` are kept for direct (non-runtime) use and call
    // the internal writer inline (the original behaviour).
    // -------------------------------------------------------------------------

    /// Push an event to the buffer without calling any writer.
    ///
    /// Returns `(id, record_clone, scope_snapshot)` so the caller can write
    /// after releasing the lock that guards this `Journal`.
    #[inline]
    pub fn record_no_write(
        &mut self,
        kind:   EventKind,
        name:   impl Into<String>,
        fields: Fields,
    ) -> (RecordId, Record, Option<Scope>) {
        let id = RecordId(self.next_record_id.fetch_add(1, Ordering::Relaxed));
        let record = Record {
            id,
            scope:   self.current_scope,
            time_ns: utils::current_nanos(),
            kind:    RecordKind::Event {
                kind,
                name:   name.into(),
                fields,
            },
        };
        let scope = self.current_scope.and_then(|sid| self.buffer.get_scope_by_id(sid));
        self.buffer.push_record(record.clone());
        (id, record, scope)
    }

    /// Exit a scope, push the exit record, but do not write anywhere.
    ///
    /// Returns `(id, Some((record, scope)))` if the scope existed and had not
    /// already been exited, or `(id, None)` otherwise.
    #[inline]
    pub fn exit_scope_no_write(
        &mut self,
        scope_id: ScopeId,
        outcome:  Outcome,
    ) -> (RecordId, Option<(Record, Scope)>) {
        let id        = RecordId(self.next_record_id.fetch_add(1, Ordering::Relaxed));
        let exited_at = utils::current_millis();

        if let Some(scope) = self.buffer.finalize_scope_exit(scope_id, exited_at) {
            let duration_ns = scope.elapsed().as_nanos() as u64;
            let record = Record {
                id,
                scope:   Some(scope_id),
                time_ns: utils::current_nanos(),
                kind:    RecordKind::ScopeExit { outcome, duration_ns },
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

    // -------------------------------------------------------------------------
    // Original inline-write variants (used when Journal is driven directly).
    // -------------------------------------------------------------------------

    pub fn record(&mut self, kind: EventKind, name: impl Into<String>, fields: Fields) -> RecordId {
        let (id, record, scope) = self.record_no_write(kind, name, fields);
        if let Some(writer) = &mut self.writer {
            if crate::runtime::log_level::enabled_for_record(&record) {
                let _ = writer.write_record(&record, scope.as_ref());
            }
        }
        id
    }

    pub fn exit_scope(&mut self, scope_id: ScopeId, outcome: Outcome) -> RecordId {
        let (id, payload) = self.exit_scope_no_write(scope_id, outcome);
        if let Some((record, scope)) = payload {
            if let Some(writer) = &mut self.writer {
                if crate::runtime::log_level::enabled_for_record(&record) {
                    let _ = writer.write_record(&record, Some(&scope));
                }
            }
        }
        id
    }

    // -------------------------------------------------------------------------
    // Buffer maintenance
    // -------------------------------------------------------------------------

    /// Drop oldest records, keeping at most `max`.
    pub fn trim_records(&mut self, max: usize) {
        self.buffer.trim_records(max);
    }

    /// Drop oldest *exited* scopes, keeping at most `max`.  Open scopes are
    /// always retained.
    pub fn trim_scopes(&mut self, max: usize) {
        self.buffer.trim_scopes(max);
    }

    // -------------------------------------------------------------------------
    // Accessors
    // -------------------------------------------------------------------------

    pub fn get_scope(&self, id: ScopeId) -> Option<Scope> {
        self.buffer.get_scope_by_id(id)
    }

    pub fn get_record(&self, id: RecordId) -> Option<Record> {
        self.buffer.get_record_by_id(id)
    }

    pub fn records(&self) -> Vec<Record> {
        self.buffer.records_snapshot()
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
