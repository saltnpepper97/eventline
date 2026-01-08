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

use std::time::Instant;

use crate::id::{RecordId, ScopeId};
use crate::outcome::Outcome;
use crate::scope::Scope;
use crate::record::{Record, RecordKind};

#[derive(Debug)]
pub struct Journal {
    scopes: Vec<Scope>,
    records: Vec<Record>,
}

impl Journal {
    /// Create a new, empty journal.
    /// Initially, there are no scopes or records.
    pub fn new() -> Self {
        Self {
            scopes: Vec::new(),
            records: Vec::new(),
        }
    }

    /// Enter a new scope, optionally nested under a parent scope.
    ///
    /// Returns a `ScopeId` which can be used for:
    /// - Recording events within this scope
    /// - Exiting the scope later
    ///
    /// # Example
    /// ```
    /// let scope_id = journal.enter_scope(None);
    /// ```
    pub fn enter_scope(&mut self, parent: Option<ScopeId>) -> ScopeId {
        let id = ScopeId(self.scopes.len() as u64);

        self.scopes.push(Scope {
            id,
            parent,
            entered_at: Instant::now(),
        });

        id
    }

    /// Exit a scope with a specific `Outcome`.
    ///
    /// This appends a `ScopeExit` record to preserve the append-only invariant.
    /// The scope's duration and outcome are captured for summary or replay.
    ///
    /// # Example
    /// ```
    /// journal.exit_scope(scope_id, Outcome::Success);
    /// ```
    pub fn exit_scope(&mut self, scope: ScopeId, outcome: Outcome) -> RecordId {
        let id = RecordId(self.records.len() as u64);

        self.records.push(Record {
            id,
            scope: Some(scope),
            time: Instant::now(),
            kind: RecordKind::ScopeExit {
                outcome,
                exited_at: Instant::now(),
            },
        });

        id
    }

    /// Record a generic event within an optional scope.
    ///
    /// Events are free-form messages attached to a scope or the root.
    /// Each event gets a unique `RecordId` and timestamp.
    ///
    /// # Example
    /// ```
    /// journal.record(Some(scope_id), "Starting database migration");
    /// journal.record(None, "Global startup event");
    /// ```
    pub fn record(&mut self, scope: Option<ScopeId>, message: impl Into<String>) -> RecordId {
        let id = RecordId(self.records.len() as u64);

        self.records.push(Record {
            id,
            scope,
            time: Instant::now(),
            kind: RecordKind::Event { message: message.into() },
        });

        id
    }

    /// Immutable access to all scopes.
    ///
    /// Returns a slice of `Scope`s in the order they were entered.
    /// Use this for rendering, replay, or analysis.
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    /// Immutable access to all records.
    ///
    /// Returns a slice of `Record`s in the order they were appended.
    /// Includes both events and scope exits.
    pub fn records(&self) -> &[Record] {
        &self.records
    }
}
