//! ## Buffered Logging
//!
//! `JournalBuffer` provides buffered logging for high-throughput scenarios.
//! It accumulates scopes and records in memory using **local IDs** (starting from 0),
//! then rebases them to global IDs when flushed to the main journal.
//!
//! This design ensures:
//! - **Deterministic ordering**: Flush order determines final ID assignment
//! - **No ID collisions**: IDs are assigned only at flush time by the Journal
//! - **Concurrent safety**: Multiple buffers can exist without coordination
//! - **Simplicity**: No complex ID reservation or offset tracking

use super::journal::Journal;
use super::utils::current_millis;
use crate::id::{RecordId, ScopeId};
use crate::outcome::Outcome;
use crate::scope::Scope;
use crate::record::{Record, RecordKind};

/// A buffered journal for batching writes before flushing to the main `Journal`.
///
/// **Design**: Uses local IDs (starting from 0) that are rebased when flushed.
/// This ensures deterministic ID assignment while allowing concurrent buffering.
///
/// Invariant:
/// - All ScopeId values used in this buffer must originate from this buffer
///
/// ## Usage
///
/// ```rust
/// let mut journal = Journal::new();
/// let mut buffer = journal.create_buffer();
/// 
/// let scope = buffer.enter_scope(None);
/// buffer.record(Some(scope), "Buffered event");
/// buffer.exit_scope(scope, Outcome::Success);
///
/// journal.flush_buffer(buffer);
/// ```
#[derive(Debug)]
pub struct JournalBuffer {
    pub(super) scopes: Vec<Scope>,
    pub(super) records: Vec<Record>,
}


impl JournalBuffer {
    /// Create a new, empty buffer with local IDs.
    ///
    /// # Example
    /// ```
    /// let buffer = JournalBuffer::new();
    /// assert!(buffer.is_empty());
    /// ```
    pub fn new() -> Self {
        Self {
            scopes: Vec::new(),
            records: Vec::new(),
        }
    }

    /// Enter a new scope in the buffer, optionally nested under a parent scope.
    ///
    /// Returns a `ScopeId` using **local numbering** (starting from 0).
    /// This ID will be rebased to a global ID when the buffer is flushed.
    ///
    /// # Example
    /// ```
    /// let mut buffer = JournalBuffer::new();
    /// let scope_id = buffer.enter_scope(None);
    /// assert_eq!(scope_id.0, 0); // Local ID
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
    /// Appends a `ScopeExit` record to the buffer using local IDs.
    ///
    /// # Example
    /// ```
    /// let mut buffer = JournalBuffer::new();
    /// let scope_id = buffer.enter_scope(None);
    /// buffer.exit_scope(scope_id, Outcome::Success);
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
    /// Uses local IDs that will be rebased on flush.
    ///
    /// # Example
    /// ```
    /// let mut buffer = JournalBuffer::new();
    /// let scope_id = buffer.enter_scope(None);
    /// buffer.record(Some(scope_id), "Buffered event");
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

    /// Flush buffered contents to a journal.
    ///
    /// This consumes the buffer and rebases all local IDs to global IDs.
    ///
    /// # Example
    /// ```
    /// let mut journal = Journal::new();
    /// let mut buffer = JournalBuffer::new();
    /// buffer.record(None, "Event");
    /// buffer.flush_to(&mut journal);
    /// ```
    pub fn flush_to(self, journal: &mut Journal) {
        journal.flush_buffer(self);
    }

    /// Get the number of buffered records.
    ///
    /// # Example
    /// ```
    /// let buffer = JournalBuffer::new();
    /// assert_eq!(buffer.len(), 0);
    /// ```
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Check if the buffer is empty.
    ///
    /// # Example
    /// ```
    /// let buffer = JournalBuffer::new();
    /// assert!(buffer.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.records.is_empty() && self.scopes.is_empty()
    }

    /// Immutable access to buffered scopes (with local IDs).
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    /// Immutable access to buffered records (with local IDs).
    pub fn records(&self) -> &[Record] {
        &self.records
    }
}

impl Default for JournalBuffer {
    fn default() -> Self {
        Self::new()
    }
}
