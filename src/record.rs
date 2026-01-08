use std::time::Instant;

use crate::id::{RecordId, ScopeId};

#[derive(Debug)]
pub struct Record {
    pub id: RecordId,
    pub scope: Option<ScopeId>,
    pub time: Instant,
}
