/// Unique identifier for a scope in the journal.
/// Returned by `Journal::enter_scope` and used for recording events or exiting scopes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScopeId(pub(crate) u64);

/// Unique identifier for a record in the journal.
/// Returned by `Journal::record` and `Journal::exit_scope`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecordId(pub(crate) u64);
