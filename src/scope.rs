use std::time::Instant;

use crate::id::ScopeId;

#[derive(Debug)]
pub struct Scope {
    pub id: ScopeId,
    pub parent: Option<ScopeId>,
    pub entered_at: Instant,
}
