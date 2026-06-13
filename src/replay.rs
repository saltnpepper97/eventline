use crate::core::{EventKind, Outcome, Record, RecordKind, Scope, ScopeId};

#[derive(Debug, Clone)]
pub struct ScopeNode {
    pub scope: Scope,
    pub children: Vec<ScopeNode>,
}

#[derive(Debug, Clone)]
pub struct ScopeOutcome {
    pub scope: Scope,
    pub outcome: Outcome,
    pub duration_ns: u64,
}

pub fn records_for_scope(records: &[Record], scope_id: ScopeId) -> Vec<Record> {
    records
        .iter()
        .filter(|record| record.scope == Some(scope_id))
        .cloned()
        .collect()
}

pub fn records_at_level(records: &[Record], level: EventKind) -> Vec<Record> {
    records
        .iter()
        .filter(|record| matches!(&record.kind, RecordKind::Event { kind, .. } if *kind == level))
        .cloned()
        .collect()
}

pub fn scope_outcomes(records: &[Record], scopes: &[Scope]) -> Vec<ScopeOutcome> {
    records
        .iter()
        .filter_map(|record| match &record.kind {
            RecordKind::ScopeExit {
                outcome,
                duration_ns,
            } => {
                let scope_id = record.scope?;
                let scope = scopes.iter().find(|scope| scope.id == scope_id)?.clone();
                Some(ScopeOutcome {
                    scope,
                    outcome: *outcome,
                    duration_ns: *duration_ns,
                })
            }
            RecordKind::Event { .. } => None,
        })
        .collect()
}

pub fn failed_scopes(records: &[Record], scopes: &[Scope]) -> Vec<ScopeOutcome> {
    scope_outcomes(records, scopes)
        .into_iter()
        .filter(|summary| summary.outcome == Outcome::Failure)
        .collect()
}

pub fn slowest_scopes(records: &[Record], scopes: &[Scope], limit: usize) -> Vec<ScopeOutcome> {
    let mut summaries = scope_outcomes(records, scopes);
    summaries.sort_by(|a, b| b.duration_ns.cmp(&a.duration_ns));
    summaries.truncate(limit);
    summaries
}

pub fn scope_tree(scopes: &[Scope]) -> Vec<ScopeNode> {
    fn build_node(scope: &Scope, scopes: &[Scope]) -> ScopeNode {
        let children = scopes
            .iter()
            .filter(|child| child.parent == Some(scope.id))
            .map(|child| build_node(child, scopes))
            .collect();

        ScopeNode {
            scope: scope.clone(),
            children,
        }
    }

    scopes
        .iter()
        .filter(|scope| scope.parent.is_none())
        .map(|scope| build_node(scope, scopes))
        .collect()
}

pub fn render_scope_tree(scopes: &[Scope]) -> String {
    fn render_node(out: &mut String, node: &ScopeNode, depth: usize) {
        for _ in 0..depth {
            out.push_str("  ");
        }

        let name = node.scope.name.as_deref().unwrap_or("unnamed");
        out.push_str(name);
        out.push('#');
        out.push_str(&node.scope.id.0.to_string());
        out.push('\n');

        for child in &node.children {
            render_node(out, child, depth + 1);
        }
    }

    let mut out = String::new();
    for node in scope_tree(scopes) {
        render_node(&mut out, &node, 0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{failed_scopes, render_scope_tree, scope_tree, slowest_scopes};
    use crate::core::{Outcome, Record, RecordId, RecordKind, Scope, ScopeId};

    fn scope(id: u64, parent: Option<u64>, name: &str) -> Scope {
        Scope {
            id: ScopeId(id),
            parent: parent.map(ScopeId),
            entered_at: 0,
            name: Some(name.to_string()),
            exited_at: Some(1),
            exit_messages: Default::default(),
        }
    }

    fn exit_record(id: u64, scope: u64, outcome: Outcome, duration_ns: u64) -> Record {
        Record {
            id: RecordId(id),
            scope: Some(ScopeId(scope)),
            time_ns: id,
            kind: RecordKind::ScopeExit {
                outcome,
                duration_ns,
            },
        }
    }

    #[test]
    fn scope_tree_preserves_parent_child_relationships() {
        let scopes = vec![scope(1, None, "root"), scope(2, Some(1), "child")];
        let tree = scope_tree(&scopes);

        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].scope.id, ScopeId(2));
        assert_eq!(render_scope_tree(&scopes), "root#1\n  child#2\n");
    }

    #[test]
    fn failed_and_slowest_scope_helpers_use_exit_records() {
        let scopes = vec![scope(1, None, "fast"), scope(2, None, "slow")];
        let records = vec![
            exit_record(1, 1, Outcome::Success, 10),
            exit_record(2, 2, Outcome::Failure, 20),
        ];

        let failed = failed_scopes(&records, &scopes);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].scope.id, ScopeId(2));

        let slowest = slowest_scopes(&records, &scopes, 1);
        assert_eq!(slowest.len(), 1);
        assert_eq!(slowest[0].scope.id, ScopeId(2));
    }
}
