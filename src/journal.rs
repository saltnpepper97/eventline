//! the journal is append-only.
//!
//! Once a scope or record is created, it is never mutated
//! or removed. This invariant enables deterministic replay,
//! inspection, and safe concurrency later.

use std::time::Instant;

use crate::id::{RecordId, ScopeId};
use crate::scope::Scope;
use crate::record::Record;

#[derive(Debug)]
pub struct Journal {
    scopes: Vec<Scope>,
    records: Vec<Record>,
}

impl Journal {
    pub fn new() -> Self {
        Self {
            scopes: Vec::new(),
            records: Vec::new(),
        }
    }

    pub fn enter_scope(&mut self, parent: Option<ScopeId>) -> ScopeId {
        let id = ScopeId(self.scopes.len() as u64);

        self.scopes.push(Scope {
            id,
            parent,
            entered_at: Instant::now(),
        });

        id
    }

    pub fn record(&mut self, scope: Option<ScopeId>) -> RecordId {
        let id = RecordId(self.records.len() as u64);

        self.records.push(Record {
            id,
            scope,
            time: Instant::now()
        });

        id
    }

    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    pub fn records(&self) -> &[Record] {
        &self.records
    }
}
