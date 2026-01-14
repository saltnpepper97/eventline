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

pub mod buffer;
pub mod filter;
pub mod tests;
pub mod utils;
pub mod writer;

use self::buffer::JournalBuffer;
use self::utils::current_millis;

// Re-export JournalWriter so it's accessible as journal::JournalWriter
pub use self::writer::JournalWriter;

use crate::Outcome;
use crate::{Record, RecordKind};
use crate::EventKind;
use crate::{RecordId, ScopeId};
use crate::Scope;
use crate::core::value::Fields; // NEW

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
    /// use eventline::Journal;
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
    /// use eventline::Journal;
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
    /// use eventline::Journal;
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
    
    /// Execute a closure within a new scope, automatically recording scope
    /// entry and exit.
    ///
    /// This method provides a borrow-checker–friendly alternative to RAII
    /// guards by explicitly delimiting the lifetime of a scope using a closure.
    /// The scope is entered before the closure is executed and exited
    /// immediately after the closure returns.
    ///
    /// All events recorded inside the closure may freely borrow the journal
    /// mutably without conflicting with scope management.
    ///
    /// ## Outcomes
    ///
    /// * If the closure returns normally, the scope is exited with
    ///   [`Outcome::Success`].
    /// * If the closure panics, the scope is exited with
    ///   [`Outcome::Aborted`] before unwinding continues.
    ///
    /// This guarantees that every entered scope is eventually exited,
    /// preserving the journal's append-only and deterministic invariants.
    ///
    /// ## Determinism
    ///
    /// Scope entry, all records produced by the closure, and scope exit are
    /// recorded in strict sequential order. No records are reordered or
    /// removed, even in the presence of panics.
    ///
    /// ## Example
    ///
    /// ```
    /// use eventline::Journal;
    ///
    /// let mut journal = Journal::new();
    ///
    /// journal.scoped(None, None::<String>, |journal, scope| {
    ///     journal.record(Some(scope), "Starting Stasis daemon...");
    ///     journal.record(Some(scope), "Loading profiles...");
    /// });
    ///
    /// assert_eq!(journal.scopes().len(), 1);
    /// ```
    pub fn scoped<F, R, S>(
        &mut self,
        parent: Option<ScopeId>,
        name: Option<S>,
        f: F,
    ) -> R
    where
        F: FnOnce(&mut Journal, ScopeId) -> R,
        S: Into<String>,
    {
        let name_string = name.map(|s| s.into());
        let scope = self.enter_scope(parent, name_string);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self, scope)));

        match result {
            Ok(value) => {
                self.exit_scope(scope, Outcome::Success);
                value
            }
            Err(panic) => {
                self.exit_scope(scope, Outcome::Aborted);
                std::panic::resume_unwind(panic);
            }
        }
    }

    /// Enter a new scope, optionally nested under a parent scope.
    ///
    /// Returns a [`ScopeId`] for recording events and exiting the scope later.
    ///
    /// # Example
    /// ```
    /// use eventline::Journal;
    /// 
    /// let mut journal = Journal::new();
    /// let scope_id = journal.enter_scope(None, Some("my-scope"));
    /// ```
    pub fn enter_scope<S>(&mut self, parent: Option<ScopeId>, name: Option<S>) -> ScopeId
    where
        S: Into<String>,
    {
        let id = ScopeId(self.scopes.len() as u64);

        self.scopes.push(Scope {
            id,
            parent,
            entered_at: current_millis(),
            name: name.map(|s| s.into()),
            exited_at: None,
        });

        id
    }

    /// Enter a new unnamed scope.
    ///
    /// This avoids the type inference problem when you don't want to name a scope.
    pub fn enter_scope_unnamed(&mut self, parent: Option<ScopeId>) -> ScopeId {
        self.enter_scope::<String>(parent, None)
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
    /// use eventline::Journal;
    /// use eventline::Outcome;
    /// 
    /// let mut journal = Journal::new();
    /// let scope_id = journal.enter_scope_unnamed(None);
    /// journal.exit_scope(scope_id, Outcome::Success);
    /// ```   
    pub fn exit_scope(&mut self, scope: ScopeId, outcome: Outcome) -> RecordId {
        // 1. Validate scope exists
        debug_assert!(
            (scope.0 as usize) < self.scopes.len(),
            "Attempted to exit non-existent scope {:?}",
            scope
        );

        if let Some(s) = self.scopes.get(scope.0 as usize) {
            if s.exited_at.is_some() {
                // Already exited; return last exit record ID if you want
                return self.records.iter()
                    .rev()
                    .find(|r| matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(scope))
                    .map(|r| r.id)
                    .unwrap_or(RecordId(0));
            }
        }

        let id = RecordId(self.records.len() as u64);
        let now = current_millis();

        // 3. Update the scope metadata (optional but recommended for performance)
        if let Some(s) = self.scopes.get_mut(scope.0 as usize) {
            s.exited_at = Some(now);
        }

        // 4. Record the exit
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

    /// Get the full scope path from root to the given scope.
    ///
    /// Example: ["Root", "DatabaseMigration"]
    pub fn scope_path(&self, scope: Option<ScopeId>) -> Vec<String> {
        let mut path = Vec::new();
        let mut current = scope;
        while let Some(id) = current {
            if let Some(scope) = self.scopes.get(id.0 as usize) {
                path.push(scope.name.clone().unwrap_or_else(|| "<unnamed>".to_string()));
                current = scope.parent;
            } else {
                break;
            }
        }
        path.reverse();
        path
    }

    /// Returns the elapsed time of a scope in milliseconds.
    ///
    /// This measures the difference between when the scope was entered (`Scope.entered_at`)
    /// and either the current time (if still active) or the recorded exit time (if exited).
    ///
    /// Returns `None` if the scope ID does not exist.
    ///
    /// # Example
    ///
    /// ```
    /// use eventline::Journal;
    /// let mut journal = Journal::new();
    /// let scope = journal.enter_scope_unnamed(None);
    /// assert!(journal.scope_elapsed(Some(scope)).is_some());
    /// ```
    pub fn scope_elapsed(&self, scope: Option<ScopeId>) -> Option<std::time::Duration> {
        scope.and_then(|id| self.scopes.get(id.0 as usize).map(|s| s.elapsed()))
    }

    /// Returns the outcome of a scope (`Success`, `Aborted`, etc.) if it has exited.
    ///
    /// Returns `None` if the scope has not exited or the ID is invalid.
    ///
    /// # Example
    ///
    /// ```
    /// use eventline::Journal;
    /// use eventline::Outcome;
    ///
    /// let mut journal = Journal::new();
    /// let scope = journal.enter_scope_unnamed(None);
    /// journal.exit_scope(scope, Outcome::Success);
    /// assert_eq!(journal.scope_outcome(Some(scope)), Some(Outcome::Success));
    /// ```
    pub fn scope_outcome(&self, scope: Option<ScopeId>) -> Option<Outcome> {
        scope.and_then(|id| {
            // Scan records in reverse for the scope exit record
            self.records
                .iter()
                .rev()
                .find_map(|r| match &r.kind {
                    RecordKind::ScopeExit { outcome, .. } if r.scope == Some(id) => {
                        Some(*outcome)
                    }
                    _ => None,
                })
        })
    }

    /// Get a reference to a scope by `ScopeId`.
    ///
    /// Returns `Some(&Scope)` if the scope exists, `None` otherwise.
    pub fn get_scope(&self, id: ScopeId) -> Option<&Scope> {
        self.scopes.get(id.0 as usize)
    }

    /// Returns `true` if the given scope is still active (i.e., has not been exited).
    ///
    /// Returns `false` if the scope has been exited or the scope ID is invalid.
    ///
    /// # Example
    ///
    /// ```
    /// use eventline::Journal;
    /// use eventline::Outcome;
    ///
    /// let mut journal = Journal::new();
    /// let scope = journal.enter_scope_unnamed(None);
    /// assert!(journal.is_scope_active(scope));
    ///
    /// journal.exit_scope(scope, Outcome::Success);
    /// assert!(!journal.is_scope_active(scope));
    /// ```
    pub fn is_scope_active(&self, scope: ScopeId) -> bool {
        // If scope ID is invalid, consider it inactive
        if (scope.0 as usize) >= self.scopes.len() {
            return false;
        }

        // Scan records in reverse for the first exit of this scope
        !self.records.iter().rev().any(|r| matches!(
            r.kind,
            RecordKind::ScopeExit { .. } if r.scope == Some(scope)
        ))
    }

    /// Record an informational event within an optional scope.
    ///
    /// This is a convenience wrapper around [`record_with_kind`] that records
    /// the event as [`EventKind::Info`].
    ///
    /// Use this method for routine progress messages and status updates.
    /// For semantically meaningful events (warnings, errors, debugging output),
    /// prefer the more explicit helper methods such as [`warn`] or [`error`].
    ///
    /// ## Debug Assertions
    ///
    /// In debug builds, asserts that the referenced scope exists in this journal.
    ///
    /// ## Example
    ///
    /// ```
    /// use eventline::Journal;
    ///
    /// let mut journal = Journal::new();
    /// journal.record(None, "Application starting");
    /// ```
    pub fn record(
        &mut self,
        scope: Option<ScopeId>,
        message: impl Into<String>,
    ) -> RecordId {
        self.record_with_kind(scope, EventKind::Info, message)
    }

    /// Record an event with structured fields.
    ///
    /// This is the most flexible event-recording method, accepting both a message
    /// and structured key-value fields.
    ///
    /// # Example
    ///
    /// ```
    /// use eventline::Journal;
    /// use eventline::core::{EventKind, Fields, Value};
    ///
    /// let mut journal = Journal::new();
    /// let mut fields = Fields::new();
    /// fields.insert("user_id".into(), Value::from(12345));
    /// fields.insert("action".into(), Value::from("login"));
    ///
    /// journal.record_event(None, EventKind::Info, "User logged in", fields);
    /// ```
    pub fn record_event(
        &mut self,
        scope: Option<ScopeId>,
        kind: EventKind,
        message: impl Into<String>,
        fields: Fields,
    ) -> RecordId {
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
            kind: RecordKind::Event {
                kind,
                message: message.into(),
                fields,
            },
        });

        id
    }

    /// Record an event with an explicit [`EventKind`], optionally associated
    /// with a scope.
    ///
    /// This is the most general event-recording API in the journal.
    /// All other convenience methods (such as [`record`], [`info`], [`warn`],
    /// and [`error`]) ultimately delegate to this function.
    ///
    /// ## Semantics
    ///
    /// * The event is appended to the journal and is never mutated or removed.
    /// * The [`EventKind`] expresses *what kind of thing happened* (informational,
    ///   warning, error, etc.), but **does not by itself affect scope outcomes**.
    /// * Scope outcomes are determined exclusively by explicit
    ///   [`ScopeExit`](RecordKind::ScopeExit) records.
    ///
    /// This separation allows tooling to answer questions such as:
    /// * "Did anything concerning happen during this task?"
    /// * "Did the task ultimately succeed or fail?"
    ///
    /// without conflating the two concepts.
    ///
    /// ## Debug Assertions
    ///
    /// In debug builds, asserts that the referenced scope exists in this journal.
    /// This catches logic errors during development while remaining zero-cost
    /// in release builds.
    ///
    /// ## Example
    ///
    /// ```
    /// use eventline::Journal;
    /// use eventline::EventKind;
    ///
    /// let mut journal = Journal::new();
    /// let scope = journal.enter_scope_unnamed(None);
    ///
    /// journal.record_with_kind(Some(scope), EventKind::Warning, "Low disk space");
    /// journal.record_with_kind(Some(scope), EventKind::Error, "Failed to open file");
    /// ```
    pub fn record_with_kind(
        &mut self,
        scope: Option<ScopeId>,
        kind: EventKind,
        message: impl Into<String>,
    ) -> RecordId {
        self.record_event(scope, kind, message, Fields::new())
    }

    /// Immutable access to all scopes.
    ///
    /// # Example
    /// ```
    /// use eventline::Journal;
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
    /// use eventline::Journal;
    /// 
    /// let journal = Journal::new();
    /// let records = journal.records();
    /// assert!(records.is_empty());
    /// ```
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Record a warning event within an optional scope.
    ///
    /// Warnings indicate something unexpected or suboptimal happened,
    /// but execution may still continue normally.
    pub fn warn(
        &mut self,
        scope: Option<ScopeId>,
        message: impl Into<String>,
    ) -> RecordId {
        self.record_with_kind(scope, EventKind::Warning, message)
    }

    /// Record an error event within an optional scope.
    ///
    /// Errors indicate something went wrong, but **do not automatically
    /// fail a scope**. Scope outcomes must still be set explicitly via
    /// [`exit_scope`].
    pub fn error(
        &mut self,
        scope: Option<ScopeId>,
        message: impl Into<String>,
    ) -> RecordId {
        self.record_with_kind(scope, EventKind::Error, message)
    }
}

impl Default for Journal {
    fn default() -> Self {
        Self::new()
    }
}
