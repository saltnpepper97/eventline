//! Represents a scope in the `eventline` journal.
//!
//! Scopes allow grouping related events together and tracking their
//! lifetime and outcomes. Each scope has a unique `ScopeId` and may
//! be nested under a parent scope.

use super::Outcome;
use super::ScopeId;

/// Optional per-outcome exit messages for the scope.
///
/// These are used ONLY when rendering the `done:` (ScopeExit) line.
/// They do not generate separate event records.
#[derive(Debug, Clone, Default)]
pub struct ExitMessages {
    /// Optional suffix appended to `done:` line when outcome is Success.
    pub success: Option<String>,
    /// Optional suffix appended to `done:` line when outcome is Failure.
    pub failure: Option<String>,
    /// Optional suffix appended to `done:` line when outcome is Aborted.
    pub aborted: Option<String>,
}

/// A journal scope, representing a logical unit of work.
#[derive(Debug, Clone)]
pub struct Scope {
    /// Unique identifier for this scope.
    pub id: ScopeId,
    /// Optional parent scope, allowing nested scopes.
    pub parent: Option<ScopeId>,
    /// Timestamp when the scope was entered.
    pub entered_at: u64,
    /// Optional human-readable name for the scope.
    pub name: Option<String>,
    /// Timestamp when the scope was exited
    pub exited_at: Option<u64>,

    /// Optional per-outcome "done line" messages.
    pub exit_messages: ExitMessages,
}

impl Scope {
    pub fn elapsed(&self) -> std::time::Duration {
        let end = self
            .exited_at
            .unwrap_or_else(|| crate::journal::utils::current_millis());
        std::time::Duration::from_millis(end.saturating_sub(self.entered_at))
    }

    /// Convenience: set per-outcome exit messages (fluent).
    pub fn with_exit_messages(
        mut self,
        success: Option<String>,
        failure: Option<String>,
        aborted: Option<String>,
    ) -> Self {
        self.exit_messages.success = success;
        self.exit_messages.failure = failure;
        self.exit_messages.aborted = aborted;
        self
    }

    /// Pick the exit message for a given outcome, if configured.
    pub fn exit_message_for(&self, outcome: Outcome) -> Option<&str> {
        match outcome {
            Outcome::Success => self.exit_messages.success.as_deref(),
            Outcome::Failure => self.exit_messages.failure.as_deref(),
            Outcome::Aborted => self.exit_messages.aborted.as_deref(),
        }
    }
}

// -----------------------------------------------------------------------------
// Journal-level RAII guards (synchronous / direct Journal access)
// -----------------------------------------------------------------------------

use crate::journal::Journal;

/// RAII guard for a synchronous scope in the journal.
/// Automatically records an exit when dropped.
pub struct ScopeGuard<'a> {
    journal: &'a mut Journal,
    scope_id: ScopeId,
    exited: bool,
}

impl<'a> ScopeGuard<'a> {
    pub fn new(journal: &'a mut Journal, scope_id: ScopeId) -> Self {
        Self {
            journal,
            scope_id,
            exited: false,
        }
    }

    /// Explicitly mark the scope exit with a specific outcome.
    ///
    /// Safe to call multiple times; subsequent calls are ignored.
    pub fn exit(&mut self, outcome: Outcome) {
        if !self.exited {
            let _ = self.journal.exit_scope(self.scope_id, outcome);
            self.exited = true;
        }
    }
}

impl<'a> Drop for ScopeGuard<'a> {
    fn drop(&mut self) {
        if !self.exited {
            let _ = self.journal.exit_scope(self.scope_id, Outcome::Aborted);
            self.exited = true;
        }
    }
}

// -----------------------------------------------------------------------------
// Runtime-level RAII guard (async-friendly)
// -----------------------------------------------------------------------------

/// RAII guard for a scope managed by the global runtime.
///
/// - Enters a scope via `runtime::enter_scope`.
/// - Exits that scope on drop (default = Success).
pub struct RuntimeScopeGuard {
    id: ScopeId,
    exited: bool,
}

impl RuntimeScopeGuard {
    /// Enter a runtime scope and return a guard that will exit it on drop.
    pub fn enter(name: impl Into<String>) -> Self {
        let id = crate::runtime::enter_scope(name);
        Self { id, exited: false }
    }

    /// Get the underlying scope id (useful to attach exit messages right after enter).
    pub fn id(&self) -> ScopeId { self.id }

    /// Explicitly exit with a specific outcome. Safe to call multiple times.
    pub fn exit(&mut self, outcome: Outcome) {
        if !self.exited {
            crate::runtime::exit_scope(self.id, outcome);
            self.exited = true;
        }
    }

    pub fn success(mut self) { self.exit(Outcome::Success); }

    pub fn failure(mut self) { self.exit(Outcome::Failure); }

    pub fn aborted(mut self) { self.exit(Outcome::Aborted); }
}

impl Drop for RuntimeScopeGuard {
    fn drop(&mut self) {
        if !self.exited {
            crate::runtime::exit_scope(self.id, Outcome::Success);
            self.exited = true;
        }
    }
}
