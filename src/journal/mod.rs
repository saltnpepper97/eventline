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

/// The core journal - an append-only log of events and scopes.
///
/// Invariants:
/// - Records are append-only and never rewritten.
/// - Scopes are created on enter and finalized exactly once on exit by setting `exited_at`.
/// - Replay should be deterministic using the structured records.
/// - Console/file output is optional and may be filtered (e.g., by global log level).
///
/// Log level design:
/// - The journal always records the full structured execution history.
/// - The configured global log level only gates *emission* to the configured writer.
///   This preserves complete post-mortem data while controlling output noise.
pub struct Journal {
    buffer: Buffer,
    writer: Option<Box<dyn Writer>>,
    next_scope_id: AtomicU64,
    next_record_id: AtomicU64,
    current_scope: Option<ScopeId>,
}

impl Journal {
    pub fn new() -> Self {
        Self {
            buffer: Buffer::new(),
            writer: None,
            next_scope_id: AtomicU64::new(0),
            next_record_id: AtomicU64::new(0),
            current_scope: None,
        }
    }

    pub fn with_writer(writer: impl Writer + 'static) -> Self {
        Self {
            buffer: Buffer::new(),
            writer: Some(Box::new(writer)),
            next_scope_id: AtomicU64::new(0),
            next_record_id: AtomicU64::new(0),
            current_scope: None,
        }
    }

    pub fn set_writer(&mut self, writer: impl Writer + 'static) {
        self.writer = Some(Box::new(writer));
    }

    pub fn enter_scope(&mut self, name: impl Into<String>) -> ScopeId {
        let id = ScopeId(self.next_scope_id.fetch_add(1, Ordering::SeqCst));
        let scope = Scope {
            id,
            parent: self.current_scope,
            entered_at: utils::current_millis(),
            name: Some(name.into()),
            exited_at: None,
            exit_messages: ExitMessages::default(),
        };

        self.buffer.push_scope(scope);
        self.current_scope = Some(id);
        id
    }

    pub fn enter_scope_unnamed(&mut self, parent: Option<ScopeId>) -> ScopeId {
        let id = ScopeId(self.next_scope_id.fetch_add(1, Ordering::SeqCst));
        let scope = Scope {
            id,
            parent: parent.or(self.current_scope),
            entered_at: utils::current_millis(),
            name: None,
            exited_at: None,
            exit_messages: ExitMessages::default(),
        };

        self.buffer.push_scope(scope);
        self.current_scope = Some(id);
        id
    }

    /// Drop the oldest exited scopes from the buffer, keeping at most `max` entries.
    /// Only scopes with a set `exited_at` are eligible; open scopes are never removed.
    pub fn trim_scopes(&mut self, max: usize) {
        self.buffer.trim_scopes(max);
    }

    /// Drop the oldest records from the buffer, keeping at most `max` entries.
    pub fn trim_records(&mut self, max: usize) {
        self.buffer.trim_records(max);
    }

    /// Update per-outcome exit messages for an existing scope.
    ///
    /// Returns true if the scope existed and was updated.
    pub fn set_scope_exit_messages(&mut self, id: ScopeId, msgs: ExitMessages) -> bool {
        self.buffer.set_scope_exit_messages(id, msgs)
    }

    pub fn record(&mut self, kind: EventKind, name: impl Into<String>, fields: Fields) -> RecordId {
        let id = RecordId(self.next_record_id.fetch_add(1, Ordering::SeqCst));
        let record = Record {
            id,
            scope: self.current_scope,
            time_ns: utils::current_nanos(),
            kind: RecordKind::Event {
                kind,
                name: name.into(),
                fields,
            },
        };

        let scope = self.current_scope.and_then(|sid| self.get_scope(sid));

        if let Some(writer) = &mut self.writer {
            if crate::runtime::log_level::enabled_for_record(&record) {
                let _ = writer.write_record(&record, scope.as_ref());
            }
        }

        self.buffer.push_record(record);
        id
    }

    pub fn exit_scope(&mut self, scope_id: ScopeId, outcome: Outcome) -> RecordId {
        let id = RecordId(self.next_record_id.fetch_add(1, Ordering::SeqCst));

        let exited_at = utils::current_millis();
        if let Some(scope) = self.buffer.finalize_scope_exit(scope_id, exited_at) {
            let duration_ns = scope.elapsed().as_nanos() as u64;

            let record = Record {
                id,
                scope: Some(scope_id),
                time_ns: utils::current_nanos(),
                kind: RecordKind::ScopeExit {
                    outcome,
                    duration_ns,
                },
            };

            if let Some(writer) = &mut self.writer {
                if crate::runtime::log_level::enabled_for_record(&record) {
                    let _ = writer.write_record(&record, Some(&scope));
                }
            }

            self.buffer.push_record(record);

            if Some(scope_id) == self.current_scope {
                self.current_scope = scope.parent;
            }
        }

        id
    }

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
