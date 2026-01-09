//! The journal is append-only and forms the core of `eventline`.
//!
//! [`Journal`] records all scopes and events in a deterministic, immutable way.
//! Once a scope or record is created, it is never mutated or removed.
//! This invariant allows:
//! - Deterministic replay of program execution
//! - Inspection of events and outcomes
//! - Safe concurrency for multiple readers
//!
//! All events, including scope entries/exits and user-defined messages,
//! are captured as [`Record`]s. Scopes allow nesting and provide context for
//! each event. Outcomes for scopes are recorded via [`ScopeExit`](RecordKind::ScopeExit) records.

use std::fs::OpenOptions;
use super::buffer::JournalBuffer;
use super::utils::current_millis;
use super::writer::JournalWriter;

use crate::id::{RecordId, ScopeId};
use crate::outcome::Outcome;
use crate::record::{Record, RecordKind};
use crate::scope::Scope;

/// The main journal structure for recording scopes and events.
///
/// Journal is purely data - it stores scopes and records with no rendering
/// or I/O policy. Use [`JournalWriter`] to output journals to different sinks.
#[derive(Debug, Clone)]
pub struct Journal {
    scopes: Vec<Scope>,
    records: Vec<Record>,
}

impl Journal {
    /// Create a new, empty journal.
    ///
    /// # Example
    /// ```
    /// use eventline::journal::Journal;
    /// 
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
    /// use eventline::journal::Journal;
    /// 
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
    /// use eventline::journal::Journal;
    /// 
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
    /// Returns a [`ScopeId`] for recording events and exiting the scope later.
    ///
    /// # Example
    /// ```
    /// use eventline::journal::Journal;
    /// 
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

    /// Exit a scope with a specific [`Outcome`].
    ///
    /// Appends a [`ScopeExit`](RecordKind::ScopeExit) record preserving the append-only invariant.
    ///
    /// # Debug Assertions
    /// In debug builds, asserts that the scope exists in this journal.
    ///
    /// # Example
    /// ```
    /// use eventline::journal::Journal;
    /// use eventline::outcome::Outcome;
    /// 
    /// let mut journal = Journal::new();
    /// let scope_id = journal.enter_scope(None);
    /// journal.exit_scope(scope_id, Outcome::Success);
    /// ```
    pub fn exit_scope(&mut self, scope: ScopeId, outcome: Outcome) -> RecordId {
        // DEBUG-ONLY: Validate scope exists in this journal
        debug_assert!(
            (scope.0 as usize) < self.scopes.len(),
            "Attempted to exit non-existent scope {:?}",
            scope
        );

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
    /// # Debug Assertions
    /// In debug builds, asserts that the scope exists in this journal.
    /// This catches logic errors while maintaining zero-cost in release builds.
    ///
    /// # Example
    /// ```
    /// use eventline::journal::Journal;
    /// 
    /// let mut journal = Journal::new();
    /// let scope_id = journal.enter_scope(None);
    /// journal.record(Some(scope_id), "Starting migration");
    /// journal.record(None, "Global startup event");
    /// ```
    pub fn record(&mut self, scope: Option<ScopeId>, message: impl Into<String>) -> RecordId {
        // DEBUG-ONLY: Validate scope exists in this journal
        debug_assert!(
            scope.map_or(true, |s| (s.0 as usize) < self.scopes.len()),
            "Attempted to record event in non-existent scope {:?}",
            scope
        );

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
    /// use eventline::journal::Journal;
    /// 
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
    /// use eventline::journal::Journal;
    /// 
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
    /// **Note**: For more flexible output options, see [`JournalWriter`].
    ///
    /// # Example
    /// ```
    /// use eventline::journal::Journal;
    /// use eventline::outcome::Outcome;
    /// 
    /// let mut journal = Journal::new();
    /// let scope_id = journal.enter_scope(None);
    /// journal.record(Some(scope_id), "Test event");
    /// journal.exit_scope(scope_id, Outcome::Success);
    /// journal.write_to_file("eventline.log").unwrap();
    /// ```
    pub fn write_to_file(&self, path: &str) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        JournalWriter::new().write_to(&mut file, self)
    }
}

impl Default for Journal {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    #[test]
    #[should_panic(expected = "non-existent scope")]
    #[cfg(debug_assertions)]
    fn test_record_invalid_scope_panics_in_debug() {
        let mut journal = Journal::new();
        let fake_scope = ScopeId(999);
        journal.record(Some(fake_scope), "This should panic in debug");
    }

    #[test]
    #[should_panic(expected = "non-existent scope")]
    #[cfg(debug_assertions)]
    fn test_exit_invalid_scope_panics_in_debug() {
        let mut journal = Journal::new();
        let fake_scope = ScopeId(999);
        journal.exit_scope(fake_scope, Outcome::Success);
    }

    #[test]
    fn test_valid_scope_operations() {
        let mut journal = Journal::new();
        let scope = journal.enter_scope(None);
        
        // These should work fine
        journal.record(Some(scope), "valid event");
        journal.exit_scope(scope, Outcome::Success);
    }
}
