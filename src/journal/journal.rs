//! The journal is append-only and forms the core of `eventline`.
//!
//! `Journal` records all scopes and events in a deterministic, immutable way.
//! Once a scope or record is created, it is never mutated or removed.
//! This invariant allows:
//! - Deterministic replay of program execution
//! - Inspection of events and outcomes
//! - Safe concurrency for multiple readers
//!
//! All events, including scope entries/exits and user-defined messages,
//! are captured as `Record`s. Scopes allow nesting and provide context for
//! each event. Outcomes for scopes are recorded via `ScopeExit` records.

use std::fs::OpenOptions;
use std::io::Write;
use super::buffer::JournalBuffer;
use super::utils::{current_millis, millis_to_local};

use crate::id::{RecordId, ScopeId};
use crate::outcome::Outcome;
use crate::record::{Record, RecordKind};
use crate::scope::Scope;

#[derive(Debug)]
pub struct Journal {
    scopes: Vec<Scope>,
    records: Vec<Record>,
}

impl Journal {
    /// Create a new, empty journal.
    ///
    /// # Example
    /// ```
    /// let journal = Journal::new();
    /// assert!(journal.scopes().is_empty());
    /// assert!(journal.records().is_empty());
    /// ```
    pub fn new() -> Self {
        Self {
            scopes: Vec::new(),
            records: Vec::new(),
        }
    }

    /// Create a new buffer for batched logging.
    ///
    /// The buffer uses local IDs starting from 0. These are rebased to
    /// global IDs when the buffer is flushed.
    ///
    /// # Example
    /// ```
    /// let mut journal = Journal::new();
    /// let buffer = journal.create_buffer();
    /// ```
    pub fn create_buffer(&self) -> JournalBuffer {
        JournalBuffer::new()
    }

    /// Flush a buffer's contents into this journal.
    ///
    /// All buffered scopes and records have their IDs rebased to global IDs
    /// based on the current journal state. The buffer is consumed in this process.
    ///
    /// **IMPORTANT**: Flush order matters. The order in which buffers are flushed
    /// determines the final global ID ordering, which affects deterministic replay.
    ///
    /// # Example
    /// ```
    /// let mut journal = Journal::new();
    /// let mut buffer = journal.create_buffer();
    /// buffer.record(None, "Buffered event");
    /// journal.flush_buffer(buffer);
    /// assert_eq!(journal.records().len(), 1);
    /// ```
    pub fn flush_buffer(&mut self, mut buffer: JournalBuffer) {
        // DEBUG-ONLY SAFETY CHECKS
        debug_assert!(
            buffer.scopes().iter().all(|s| {
                s.parent.map_or(true, |p| p.0 < buffer.scopes().len() as u64)
            }),
            "JournalBuffer contains a scope whose parent is not in the same buffer"
        );

        debug_assert!(
            buffer.records().iter().all(|r| {
                r.scope.map_or(true, |s| s.0 < buffer.scopes().len() as u64)
            }),
            "JournalBuffer record references a scope not owned by this buffer"
        );

        let scope_base = self.scopes.len() as u64;
        let record_base = self.records.len() as u64;

        // Rebase scope IDs and parent references
        for scope in &mut buffer.scopes {
            scope.id.0 += scope_base;
            if let Some(parent) = &mut scope.parent {
                parent.0 += scope_base;
            }
        }

        // Rebase record IDs and scope references
        for record in &mut buffer.records {
            record.id.0 += record_base;
            if let Some(scope) = &mut record.scope {
                scope.0 += scope_base;
            }
        }

        self.scopes.extend(buffer.scopes);
        self.records.extend(buffer.records);
    }

    /// Enter a new scope, optionally nested under a parent scope.
    ///
    /// Returns a `ScopeId` for recording events and exiting the scope later.
    ///
    /// # Example
    /// ```
    /// let mut journal = Journal::new();
    /// let scope_id = journal.enter_scope(None);
    /// ```
    pub fn enter_scope(&mut self, parent: Option<ScopeId>) -> ScopeId {
        let id = ScopeId(self.scopes.len() as u64);

        self.scopes.push(Scope {
            id,
            parent,
            entered_at: current_millis(),
        });

        id
    }

    /// Exit a scope with a specific `Outcome`.
    ///
    /// Appends a `ScopeExit` record preserving the append-only invariant.
    ///
    /// # Example
    /// ```
    /// let mut journal = Journal::new();
    /// let scope_id = journal.enter_scope(None);
    /// journal.exit_scope(scope_id, Outcome::Success);
    /// ```
    pub fn exit_scope(&mut self, scope: ScopeId, outcome: Outcome) -> RecordId {
        let id = RecordId(self.records.len() as u64);

        let now = current_millis();
        self.records.push(Record {
            id,
            scope: Some(scope),
            time: now,
            kind: RecordKind::ScopeExit {
                outcome,
                exited_at: now,
            },
        });

        id
    }

    /// Record a generic event within an optional scope.
    ///
    /// # Example
    /// ```
    /// let mut journal = Journal::new();
    /// let scope_id = journal.enter_scope(None);
    /// journal.record(Some(scope_id), "Starting migration");
    /// journal.record(None, "Global startup event");
    /// ```
    pub fn record(&mut self, scope: Option<ScopeId>, message: impl Into<String>) -> RecordId {
        let id = RecordId(self.records.len() as u64);

        self.records.push(Record {
            id,
            scope,
            time: current_millis(),
            kind: RecordKind::Event { message: message.into() },
        });

        id
    }

    /// Immutable access to all scopes.
    ///
    /// # Example
    /// ```
    /// let journal = Journal::new();
    /// let scopes = journal.scopes();
    /// assert!(scopes.is_empty());
    /// ```
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    /// Immutable access to all records.
    ///
    /// # Example
    /// ```
    /// let journal = Journal::new();
    /// let records = journal.records();
    /// assert!(records.is_empty());
    /// ```
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Append the journal to a human-readable file.
    ///
    /// Each scope is shown with outcome and duration.
    /// Events are listed under each scope with bullets (`•`), fallback to `*` on Windows.
    ///
    /// # Example
    /// ```
    /// let mut journal = Journal::new();
    /// let scope_id = journal.enter_scope(None);
    /// journal.record(Some(scope_id), "Test event");
    /// journal.exit_scope(scope_id, Outcome::Success);
    /// journal.write_to_file("eventline.log").unwrap();
    /// ```
    pub fn write_to_file(&self, path: &str) -> std::io::Result<()> {
        use std::collections::HashMap;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        let bullet = if cfg!(windows) { "*" } else { "•" };

        let mut records_by_scope: HashMap<ScopeId, Vec<&Record>> = HashMap::new();
        let mut exits: HashMap<ScopeId, &Record> = HashMap::new();

        for record in &self.records {
            if let Some(scope) = record.scope {
                records_by_scope.entry(scope).or_default().push(record);

                if matches!(record.kind, RecordKind::ScopeExit { .. }) {
                    exits.insert(scope, record);
                }
            }
        }

        for scope in &self.scopes {
            let exit = exits.get(&scope.id);

            let outcome = exit
                .and_then(|r| {
                    if let RecordKind::ScopeExit { outcome, .. } = r.kind {
                        Some(outcome)
                    } else {
                        None
                    }
                })
                .unwrap_or(Outcome::Aborted);

            let duration_ms = exit
                .map(|r| r.time.saturating_sub(scope.entered_at))
                .unwrap_or(0);

            let duration_s = duration_ms as f64 / 1000.0;
            let ts = millis_to_local(scope.entered_at);

            writeln!(
                file,
                "[{}] Scope {} ({:?}) [{:.3}s]",
                ts,
                scope.id.0,
                outcome,
                duration_s
            )?;

            if let Some(records) = records_by_scope.get(&scope.id) {
                for record in records {
                    if let RecordKind::Event { message } = &record.kind {
                        writeln!(file, "  {} {}", bullet, message)?;
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for Journal {
    fn default() -> Self {
        Self::new()
    }
}
