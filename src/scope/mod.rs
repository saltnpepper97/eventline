//! Represents a scope in the `eventline` journal.
//!
//! Scopes allow grouping related events together and tracking their
//! lifetime and outcomes. Each scope has a unique `ScopeId` and may
//! be nested under a parent scope.

pub mod scope_guard;

pub use scope_guard::ScopeGuard;

use crate::id::ScopeId;

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
}
