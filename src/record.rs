//! Records represent individual entries in the journal.
//!
//! A record can either be a user-defined event or a scope exit, capturing
//! timing and outcome information. Records are always append-only and immutable.

use crate::id::{RecordId, ScopeId};
use crate::outcome::Outcome;
use crate::event_kind::EventKind;

/// The type of a journal record.
#[derive(Debug, Clone)]
pub enum RecordKind {
    /// A generic event with semantic meaning.
    Event {
        kind: EventKind,
        message: String,
    },
    /// Marks the exit of a scope.
    ScopeExit {
        outcome: Outcome,
        exited_at: u64,
    },
}


/// A single journal entry, either an event or a scope exit.
#[derive(Debug, Clone)]
pub struct Record {
    /// Unique identifier for this record.
    pub id: RecordId,
    /// Optional scope this record belongs to.
    pub scope: Option<ScopeId>,
    /// Timestamp when the record was created.
    pub time: u64,
    /// The kind of record: Event or ScopeExit.
    pub kind: RecordKind,
}
