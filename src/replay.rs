use crate::core::{EventKind, Outcome, Record, RecordId, RecordKind, Scope, ScopeId};
use std::collections::{HashMap, HashSet};

mod load;
pub use load::{
    ReplayIssue, ReplayIssueKind, SavedJournal, SavedRun, load_jsonl, load_jsonl_files, read_jsonl,
};

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

/// Reusable indexes over one immutable journal snapshot or saved run.
///
/// Building an index takes O(records + scopes) time and extra memory. Point
/// lookups are expected O(1); scope-record queries visit only matching records
/// and borrow them without cloning. Input order and duplicate/conflicting
/// records are preserved. ID lookups return the first occurrence of an ID.
/// IDs from independent runs must be indexed separately.
///
/// ```
/// let mut journal = eventline::Journal::new();
/// let id = journal.enter_scope("request");
/// journal.record(eventline::EventKind::Info, "received", Default::default());
/// let records = journal.records();
/// let scopes = journal.scopes();
/// let index = eventline::replay::ReplayIndex::new(&records, &scopes);
/// assert_eq!(index.scope(id).unwrap().name.as_deref(), Some("request"));
/// assert_eq!(index.records_for_scope(id).count(), 1);
/// ```
pub struct ReplayIndex<'a> {
    records: &'a [Record],
    scopes: HashMap<ScopeId, &'a Scope>,
    records_by_id: HashMap<RecordId, &'a Record>,
    records_by_scope: HashMap<ScopeId, Vec<&'a Record>>,
}

impl<'a> ReplayIndex<'a> {
    pub fn new(records: &'a [Record], scopes: &'a [Scope]) -> Self {
        let mut records_by_id = HashMap::with_capacity(records.len());
        let mut records_by_scope: HashMap<_, Vec<_>> = HashMap::new();
        for record in records {
            records_by_id.entry(record.id).or_insert(record);
            if let Some(scope) = record.scope {
                records_by_scope.entry(scope).or_default().push(record);
            }
        }
        Self {
            records,
            scopes: index_scopes(scopes),
            records_by_id,
            records_by_scope,
        }
    }

    pub fn scope(&self, id: ScopeId) -> Option<&'a Scope> {
        self.scopes.get(&id).copied()
    }

    pub fn record(&self, id: RecordId) -> Option<&'a Record> {
        self.records_by_id.get(&id).copied()
    }

    /// Matching records in their original order, including conflicting entries.
    pub fn records_for_scope(&self, id: ScopeId) -> impl Iterator<Item = &'a Record> + '_ {
        self.records_by_scope
            .get(&id)
            .into_iter()
            .flatten()
            .copied()
    }

    /// Outcomes in record order. Missing scope metadata is omitted, just as in
    /// [`scope_outcomes`]; contradictory exits remain separate observations.
    pub fn scope_outcomes(&self) -> Vec<ScopeOutcome> {
        collect_outcomes(self.records, &self.scopes)
    }
}

pub(crate) fn index_scopes(scopes: &[Scope]) -> HashMap<ScopeId, &Scope> {
    let mut index = HashMap::with_capacity(scopes.len());
    for scope in scopes {
        index.entry(scope.id).or_insert(scope);
    }
    index
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

/// Reconstruct observed exits in record order using an O(records + scopes)
/// scope index. Records without scope metadata are omitted. Duplicate scope IDs
/// use the first metadata entry; conflicting exits remain separate summaries.
pub fn scope_outcomes(records: &[Record], scopes: &[Scope]) -> Vec<ScopeOutcome> {
    collect_outcomes(records, &index_scopes(scopes))
}

fn collect_outcomes(records: &[Record], scopes: &HashMap<ScopeId, &Scope>) -> Vec<ScopeOutcome> {
    records
        .iter()
        .filter_map(|record| match &record.kind {
            RecordKind::ScopeExit {
                outcome,
                duration_ns,
            } => {
                let scope_id = record.scope?;
                let scope = (*scopes.get(&scope_id)?).clone();
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
    summaries.sort_by_key(|summary| std::cmp::Reverse(summary.duration_ns));
    summaries.truncate(limit);
    summaries
}

/// Build a scope forest in input root/sibling order, indexing children once.
/// Scopes with unknown parents are omitted; saved-file recovery supplies marked
/// placeholder ancestors. Reachable repeated-ID cycles are skipped.
pub fn scope_tree(scopes: &[Scope]) -> Vec<ScopeNode> {
    struct Frame<'a> {
        scope: &'a Scope,
        next_child: usize,
        children: Vec<ScopeNode>,
    }
    let children = index_children(scopes);
    let mut roots = Vec::new();
    let mut path = HashSet::new();
    for root in scopes.iter().filter(|scope| scope.parent.is_none()) {
        path.insert(root.id);
        let mut stack = vec![Frame {
            scope: root,
            next_child: 0,
            children: Vec::new(),
        }];
        while let Some(frame) = stack.last_mut() {
            if let Some(child) = children
                .get(&frame.scope.id)
                .and_then(|children| children.get(frame.next_child))
            {
                frame.next_child += 1;
                // Malformed direct inputs with duplicate IDs can otherwise
                // introduce reachable cycles. Saved-file loading repairs them.
                if path.insert(child.id) {
                    stack.push(Frame {
                        scope: child,
                        next_child: 0,
                        children: Vec::new(),
                    });
                }
            } else {
                let frame = stack.pop().unwrap();
                path.remove(&frame.scope.id);
                let node = ScopeNode {
                    scope: frame.scope.clone(),
                    children: frame.children,
                };
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    roots.push(node);
                }
            }
        }
    }
    roots
}

fn index_children(scopes: &[Scope]) -> HashMap<ScopeId, Vec<&Scope>> {
    let mut children: HashMap<_, Vec<_>> = HashMap::new();
    for scope in scopes {
        if let Some(parent) = scope.parent {
            children.entry(parent).or_default().push(scope);
        }
    }
    children
}

pub fn render_scope_tree(scopes: &[Scope]) -> String {
    let children = index_children(scopes);
    // Render directly without cloning a recursive ScopeNode tree. Explicit
    // enter/leave frames also avoid recursive traversal for deeply nested input.
    let mut stack: Vec<_> = scopes
        .iter()
        .rev()
        .filter(|scope| scope.parent.is_none())
        .map(|scope| (scope, 0, false))
        .collect();
    let mut path = HashSet::new();
    let mut out = String::new();
    while let Some((scope, depth, leaving)) = stack.pop() {
        if leaving {
            path.remove(&scope.id);
            continue;
        }
        if !path.insert(scope.id) {
            continue;
        }
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push_str(scope.name.as_deref().unwrap_or("unnamed"));
        out.push('#');
        out.push_str(&scope.id.0.to_string());
        out.push('\n');
        stack.push((scope, depth, true));
        if let Some(children) = children.get(&scope.id) {
            stack.extend(
                children
                    .iter()
                    .rev()
                    .map(|child| (*child, depth + 1, false)),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{ReplayIndex, failed_scopes, render_scope_tree, scope_tree, slowest_scopes};
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

    #[test]
    fn index_preserves_duplicate_conflicting_and_missing_metadata_records() {
        let scopes = vec![
            scope(9, None, "first"),
            scope(4, None, "second"),
            scope(9, None, "duplicate"),
        ];
        let records = vec![
            exit_record(10, 9, Outcome::Success, 20),
            exit_record(3, 4, Outcome::Aborted, 15),
            exit_record(10, 9, Outcome::Failure, 25),
            exit_record(12, 99, Outcome::Failure, 50),
            Record {
                scope: None,
                ..exit_record(15, 9, Outcome::Success, 7)
            },
        ];
        let index = ReplayIndex::new(&records, &scopes);
        assert!(std::ptr::eq(index.scope(ScopeId(9)).unwrap(), &scopes[0]));
        assert!(std::ptr::eq(
            index.record(RecordId(10)).unwrap(),
            &records[0]
        ));
        assert!(index.scope(ScopeId(99)).is_none());
        assert!(index.record(RecordId(99)).is_none());
        let grouped: Vec<_> = index.records_for_scope(ScopeId(9)).collect();
        assert_eq!(grouped.len(), 2);
        assert!(std::ptr::eq(grouped[0], &records[0]));
        assert!(std::ptr::eq(grouped[1], &records[2]));
        assert_eq!(index.records_for_scope(ScopeId(99)).count(), 1);
        assert_eq!(index.records_for_scope(ScopeId(42)).count(), 0);
        let expected = [
            (9, Outcome::Success, 20),
            (4, Outcome::Aborted, 15),
            (9, Outcome::Failure, 25),
        ];
        for outcomes in [
            index.scope_outcomes(),
            super::scope_outcomes(&records, &scopes),
        ] {
            let observed: Vec<_> = outcomes
                .iter()
                .map(|o| (o.scope.id.0, o.outcome, o.duration_ns))
                .collect();
            assert_eq!(observed, expected);
            assert_eq!(outcomes[0].scope.name.as_deref(), Some("first"));
            assert_eq!(outcomes[2].scope.name.as_deref(), Some("first"));
        }
        assert!(ReplayIndex::new(&[], &[]).scope_outcomes().is_empty());
    }

    #[test]
    fn indexed_queries_match_one_off_helpers_for_large_scrambled_histories() {
        let scopes: Vec<_> = (0..2000).map(|id| scope(id, None, "work")).collect();
        let records: Vec<_> = (0..10_000)
            .map(|id| exit_record(id, id * 8191 % 2000, Outcome::Failure, id))
            .collect();
        let index = ReplayIndex::new(&records, &scopes);
        for id in (0..2000).step_by(17) {
            let expected: Vec<_> = super::records_for_scope(&records, ScopeId(id))
                .into_iter()
                .map(|r| r.id)
                .collect();
            let actual: Vec<_> = index.records_for_scope(ScopeId(id)).map(|r| r.id).collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn tree_index_keeps_root_and_sibling_input_order_and_omits_orphans() {
        let scopes = vec![
            scope(8, Some(3), "child-b"),
            scope(3, None, "root-b"),
            scope(7, Some(3), "child-a"),
            scope(2, None, "root-a"),
            scope(6, Some(77), "orphan"),
            scope(9, Some(8), "grandchild"),
        ];
        assert_eq!(
            render_scope_tree(&scopes),
            "root-b#3\n  child-b#8\n    grandchild#9\n  child-a#7\nroot-a#2\n"
        );
        let tree = scope_tree(&scopes);
        assert_eq!(
            tree.iter().map(|n| n.scope.id).collect::<Vec<_>>(),
            [ScopeId(3), ScopeId(2)]
        );
        assert_eq!(
            tree[0]
                .children
                .iter()
                .map(|n| n.scope.id)
                .collect::<Vec<_>>(),
            [ScopeId(8), ScopeId(7)]
        );
    }

    #[test]
    fn deep_tree_traversal_and_duplicate_id_cycles_do_not_recurse() {
        let scopes: Vec<_> = (0..2000)
            .map(|id| scope(id, id.checked_sub(1), "nested"))
            .collect();
        let tree = scope_tree(&scopes);
        let mut node = &tree[0];
        let mut count = 1;
        while let Some(child) = node.children.first() {
            count += 1;
            node = child;
        }
        assert_eq!(count, 2000);
        assert_eq!(render_scope_tree(&scopes).lines().count(), 2000);
        // The returned public tree is recursive; release this stress fixture
        // iteratively as well, independent of the test thread's stack size.
        let mut pending = tree;
        while let Some(mut node) = pending.pop() {
            pending.append(&mut node.children);
        }
        let cyclic = vec![
            scope(1, None, "root"),
            scope(2, Some(1), "child"),
            scope(1, Some(2), "cycle"),
        ];
        assert_eq!(render_scope_tree(&cyclic), "root#1\n  child#2\n");
        assert_eq!(scope_tree(&cyclic)[0].children.len(), 1);
    }

    #[test]
    fn equal_duration_summaries_keep_record_order() {
        let scopes = vec![scope(1, None, "one"), scope(2, None, "two")];
        let records = vec![
            exit_record(9, 2, Outcome::Success, 10),
            exit_record(3, 1, Outcome::Success, 10),
        ];
        assert_eq!(
            slowest_scopes(&records, &scopes, 2)
                .iter()
                .map(|o| o.scope.id)
                .collect::<Vec<_>>(),
            [ScopeId(2), ScopeId(1)]
        );
    }
}
