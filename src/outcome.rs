//! Represents the result of a scope or operation in `eventline`.
//!
//! Used by `Journal::exit_scope` and recorded as a `ScopeExit` record.

/// Outcome of a scope or operation.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The scope completed successfully.
    Success,
    /// The scope failed.
    Failure,
    /// The scope was aborted (e.g., dropped without explicit exit).
    Aborted,
}
