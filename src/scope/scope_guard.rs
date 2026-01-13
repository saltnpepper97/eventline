//! RAII guards for automatically exiting a scope in the journal.
//!
//! A `ScopeGuard` or `AsyncScopeGuard` ensures that every scope entered via `Journal::enter_scope`
//! is eventually exited, even if the code panics or returns early. This preserves
//! the append-only invariant and guarantees that aborted scopes are recorded.

use std::sync::atomic::{AtomicBool, Ordering};
use std::ops::Drop;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::{ScopeId, journal::Journal, Outcome};

/// RAII guard for a synchronous scope in the journal.
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
    /// use eventline::scope::ScopeGuard;
    /// use eventline::Outcome;
    ///
    /// let mut journal = Journal::new();
    /// let scope_id = journal.enter_scope_unnamed(None);
    /// let mut guard = ScopeGuard::new(&mut journal, scope_id);
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

/// Async RAII guard for a journal scope.
/// Automatically records an exit when dropped or when `.exit()` is called.
pub struct AsyncScopeGuard {
    journal: Arc<Mutex<Journal>>,
    scope_id: ScopeId,
    exited: Arc<AtomicBool>,
}


impl AsyncScopeGuard {
    /// Create a new async guard for the given scope.
    ///
    /// # Arguments
    /// * `journal` - An `Arc<Mutex<Journal>>` to allow async-safe access.
    /// * `scope_id` - The ID of the scope returned by `Journal::enter_scope`.
    ///
    /// # Example
    ///
    /// ```
    /// use std::sync::Arc;
    /// use tokio::sync::Mutex;
    /// use eventline::journal::Journal;
    /// use eventline::scope::AsyncScopeGuard;
    /// use eventline::Outcome;
    ///
    /// let journal = Arc::new(Mutex::new(Journal::new()));
    /// let scope_id = journal.lock().await.enter_scope(None, Some("startup"));
    /// let mut guard = AsyncScopeGuard::new(journal.clone(), scope_id);
    /// guard.exit(Outcome::Success).await; // optional
    /// ```
    pub fn new(journal: Arc<Mutex<Journal>>, scope_id: ScopeId) -> Self {
        Self {
            journal,
            scope_id,
            exited: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Explicitly mark the scope exit
    pub async fn exit(&self, outcome: Outcome) {
        // only first exit takes effect
        if !self.exited.swap(true, Ordering::SeqCst) {
            let mut j = self.journal.lock().await;
            j.exit_scope(self.scope_id, outcome);
        }
    }

    pub fn is_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

}

impl Drop for AsyncScopeGuard {
    fn drop(&mut self) {
        if !self.exited.load(Ordering::SeqCst) {
            let journal = self.journal.clone();
            let scope_id = self.scope_id;
            let exited_flag = self.exited.clone();

            tokio::spawn(async move {
                if !exited_flag.swap(true, Ordering::SeqCst) {
                    let mut j = journal.lock().await;
                    j.exit_scope(scope_id, Outcome::Aborted);
                }
            });
        }
    }
}
