//! Records represent individual entries in the journal.
//!
//! A record can either be a user-defined event or a scope exit, capturing
//! timing and outcome information. Records are always append-only and immutable.

use std::time::Instant;

use crate::id::{RecordId, ScopeId};
use crate::outcome::Outcome;

/// The type of a journal record.
#[derive(Debug)]
pub enum RecordKind {
    /// A generic event with a free-form message.
    Event { message: String },
    /// Marks the exit of a scope, capturing its outcome and exit time.
    ScopeExit { outcome: Outcome, exited_at: Instant },
}

/// A single journal entry, either an event or a scope exit.
#[derive(Debug)]
pub struct Record {
    /// Unique identifier for this record.
    pub id: RecordId,
    /// Optional scope this record belongs to.
    pub scope: Option<ScopeId>,
    /// Timestamp when the record was created.
    pub time: Instant,
    /// The kind of record: Event or ScopeExit.
    pub kind: RecordKind,
}
