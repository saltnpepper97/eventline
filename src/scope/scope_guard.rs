//! RAII guard for automatically exiting a scope in the journal.
//!
//! A `ScopeGuard` ensures that every scope entered via `Journal::enter_scope`
//! is eventually exited, even if the code panics or returns early. This preserves
//! the append-only invariant and guarantees that aborted scopes are recorded.

use std::ops::Drop;
use crate::{ScopeId, journal::Journal, Outcome};

/// RAII guard for a scope in the journal.
/// Automatically records an exit when dropped.
pub struct ScopeGuard<'a> {
    journal: &'a mut Journal,
    scope_id: ScopeId,
    exited: bool,
}

impl<'a> ScopeGuard<'a> {
    /// Create a new guard for the given scope.
    ///
    /// # Example
    ///
    /// ```
    /// use eventline::journal::Journal;
    /// use eventline::scope::ScopeGuard; // correct path to ScopeGuard
    /// use eventline::Outcome; // Outcome comes from the root of the crate
    ///
    /// let mut journal = Journal::new();
    /// let scope_id = journal.enter_scope_unnamed(None);
    /// let mut guard = ScopeGuard::new(&mut journal, scope_id);
    /// // perform work in the scope...
    /// guard.exit(Outcome::Success); // optional, will auto-abort if not called
    /// ```
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
            self.journal.exit_scope(self.scope_id, outcome);
            self.exited = true;
        }
    }
}

impl<'a> Drop for ScopeGuard<'a> {
    /// Automatically exits the scope as `Aborted` if not already exited.
    fn drop(&mut self) {
        if !self.exited {
            self.journal.exit_scope(self.scope_id, Outcome::Aborted);
            self.exited = true;
        }
    }
}
