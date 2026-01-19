use super::{EventKind, Outcome, RecordId, ScopeId};
use super::value::Value;
use crate::journal::fields::Fields;

#[derive(Debug, Clone)]
pub enum RecordKind {
    Event {
        kind: EventKind,
        name: String,
        fields: Fields,
    },
    ScopeExit {
        outcome: Outcome,
        duration_ns: u64,
    },
}

#[derive(Debug, Clone)]
pub struct Record {
    pub id: RecordId,
    pub scope: Option<ScopeId>,
    pub time_ns: u64,
    pub kind: RecordKind,
}

impl Record {
    pub fn fields(&self) -> Option<&Fields> {
        match &self.kind {
            RecordKind::Event { fields, .. } => Some(fields),
            _ => None,
        }
    }

    pub fn get_field(&self, name: &str) -> Option<&Value> {
        self.fields().and_then(|f| f.get(name))
    }
}
