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
use std::time::{SystemTime, UNIX_EPOCH};

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
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        let bullet = if cfg!(windows) { "*" } else { "•" };

        for scope in &self.scopes {
            // Find exit record for outcome and duration
            let exit = self.records.iter()
                .find(|r| matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(scope.id));

            let outcome = if let Some(r) = exit {
                if let RecordKind::ScopeExit { outcome, .. } = r.kind { outcome } else { Outcome::Aborted }
            } else {
                Outcome::Aborted
            };

            let duration_ms = if let Some(r) = exit { r.time.saturating_sub(scope.entered_at) } else { 0 };
            let duration_s = duration_ms as f64 / 1000.0;

            let ts = millis_to_local(scope.entered_at);

            writeln!(file, "[{}] Scope {} ({:?}) [{:.3}s]", ts, scope.id.0, outcome, duration_s)?;

            for record in self.records.iter().filter(|r| r.scope == Some(scope.id)) {
                if let RecordKind::Event { message } = &record.kind {
                    writeln!(file, "  {} {}", bullet, message)?;
                }
            }
        }

        Ok(())
    }
}

/// Get current system time in milliseconds since UNIX epoch
fn current_millis() -> u64 {
    let now = SystemTime::now();
    now.duration_since(UNIX_EPOCH)
        .expect("SystemTime before UNIX_EPOCH")
        .as_millis() as u64
}

/// Convert milliseconds since UNIX epoch to a human-readable local timestamp
fn millis_to_local(ms: u64) -> String {
    use chrono::{Local, TimeZone};
    let dt = Local.timestamp_millis_opt(ms as i64).single()
        .unwrap_or_else(|| Local.timestamp_millis_opt(0).single().unwrap());
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

