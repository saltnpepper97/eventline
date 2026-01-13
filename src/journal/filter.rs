//! Filtering system for journals, scopes, and events.
//!
//! This module provides flexible criteria-based filtering to control which
//! scopes and events are included in rendered output or written to sinks.
//!
//! Filters can be combined using logical operations (`and`, `or`, `not`) to
//! create complex filtering rules.
//!
//! # Examples
//!
//! ```
//! use eventline::{Filter, ScopeFilter, EventFilter};
//! use eventline::Outcome;
//! use eventline::EventKind;
//!
//! // Filter for failed scopes
//! let failed_only = Filter::scope(ScopeFilter::Outcome(Outcome::Failure));
//!
//! // Filter for warnings and errors only
//! let important = Filter::event(
//!     EventFilter::Kind(EventKind::Warning)
//!         .or(EventFilter::Kind(EventKind::Error))
//! );
//!
//! // Combine filters
//! let combined = failed_only.and(important);
//! ```

use super::EventKind;
use crate::Journal;
use super::Outcome;
use super::ScopeId;
use super::{Record, RecordKind};
use crate::Scope;

/// A filter for journal scopes.
#[derive(Debug, Clone)]
pub enum ScopeFilter {
    /// Match scopes with a specific outcome.
    Outcome(Outcome),

    /// Match scopes at or below a specific depth.
    ///
    /// Depth is calculated from root scopes (depth 0).
    MaxDepth(usize),

    /// Match scopes at or above a specific depth.
    MinDepth(usize),

    /// Match scopes whose name contains a substring (case-insensitive).
    NameContains(String),

    /// Match scopes whose name matches exactly.
    NameEquals(String),

    /// Match unnamed scopes only.
    Unnamed,

    /// Match named scopes only.
    Named,

    /// Match scopes with a specific parent.
    HasParent(ScopeId),

    /// Match root scopes (no parent).
    IsRoot,

    /// Match scopes whose duration exceeds a threshold (in milliseconds).
    DurationAbove(u64),

    /// Match scopes whose duration is below a threshold (in milliseconds).
    DurationBelow(u64),

    /// Logical AND of two scope filters.
    And(Box<ScopeFilter>, Box<ScopeFilter>),

    /// Logical OR of two scope filters.
    Or(Box<ScopeFilter>, Box<ScopeFilter>),

    /// Logical NOT of a scope filter.
    Not(Box<ScopeFilter>),

    /// Always matches.
    Any,

    /// Never matches.
    None,
}

impl ScopeFilter {
    /// Combine this filter with another using logical AND.
    ///
    /// # Example
    /// ```
    /// use eventline::ScopeFilter;
    /// use eventline::Outcome;
    ///
    /// let filter = ScopeFilter::Outcome(Outcome::Failure)
    ///     .and(ScopeFilter::MinDepth(1));
    /// ```
    pub fn and(self, other: ScopeFilter) -> Self {
        ScopeFilter::And(Box::new(self), Box::new(other))
    }

    /// Combine this filter with another using logical OR.
    pub fn or(self, other: ScopeFilter) -> Self {
        ScopeFilter::Or(Box::new(self), Box::new(other))
    }

    /// Negate this filter using logical NOT.
    pub fn not(self) -> Self {
        ScopeFilter::Not(Box::new(self))
    }

    /// Check if a scope matches this filter.
    ///
    /// The journal is required to compute derived properties like outcome and duration.
    pub fn matches(&self, scope: &Scope, journal: &Journal) -> bool {
        match self {
            ScopeFilter::Outcome(expected_outcome) => {
                let outcome = get_scope_outcome(scope, journal);
                outcome == *expected_outcome
            }

            ScopeFilter::MaxDepth(max) => {
                let depth = compute_scope_depth(scope, journal);
                depth <= *max
            }

            ScopeFilter::MinDepth(min) => {
                let depth = compute_scope_depth(scope, journal);
                depth >= *min
            }

            ScopeFilter::NameContains(substring) => scope
                .name
                .as_ref()
                .map_or(false, |name| name.to_lowercase().contains(&substring.to_lowercase())),

            ScopeFilter::NameEquals(expected) => scope
                .name
                .as_ref()
                .map_or(false, |name| name == expected),

            ScopeFilter::Unnamed => scope.name.is_none(),

            ScopeFilter::Named => scope.name.is_some(),

            ScopeFilter::HasParent(parent_id) => scope.parent == Some(*parent_id),

            ScopeFilter::IsRoot => scope.parent.is_none(),

            ScopeFilter::DurationAbove(threshold) => {
                let duration = get_scope_duration(scope, journal);
                duration > *threshold
            }

            ScopeFilter::DurationBelow(threshold) => {
                let duration = get_scope_duration(scope, journal);
                duration < *threshold
            }

            ScopeFilter::And(left, right) => {
                left.matches(scope, journal) && right.matches(scope, journal)
            }

            ScopeFilter::Or(left, right) => {
                left.matches(scope, journal) || right.matches(scope, journal)
            }

            ScopeFilter::Not(inner) => !inner.matches(scope, journal),

            ScopeFilter::Any => true,

            ScopeFilter::None => false,
        }
    }
}

/// A filter for journal events (records).
#[derive(Debug, Clone)]
pub enum EventFilter {
    /// Match events with a specific kind.
    Kind(EventKind),

    /// Match events whose message contains a substring (case-insensitive).
    MessageContains(String),

    /// Match events whose message matches a substring (case-sensitive).
    MessageContainsCaseSensitive(String),

    /// Match events in a specific scope.
    InScope(ScopeId),

    /// Match events that are not in any scope.
    Unscoped,

    /// Match events in any scope.
    Scoped,

    /// Logical AND of two event filters.
    And(Box<EventFilter>, Box<EventFilter>),

    /// Logical OR of two event filters.
    Or(Box<EventFilter>, Box<EventFilter>),

    /// Logical NOT of an event filter.
    Not(Box<EventFilter>),

    /// Always matches.
    Any,

    /// Never matches.
    None,
}

impl EventFilter {
    /// Combine this filter with another using logical AND.
    pub fn and(self, other: EventFilter) -> Self {
        EventFilter::And(Box::new(self), Box::new(other))
    }

    /// Combine this filter with another using logical OR.
    pub fn or(self, other: EventFilter) -> Self {
        EventFilter::Or(Box::new(self), Box::new(other))
    }

    /// Negate this filter using logical NOT.
    pub fn not(self) -> Self {
        EventFilter::Not(Box::new(self))
    }

    /// Check if a record matches this event filter.
    ///
    /// Returns `false` for non-event records (e.g., `ScopeExit`).
    pub fn matches(&self, record: &Record) -> bool {
        // Only match Event records
        let (kind, message) = match &record.kind {
            RecordKind::Event { kind, message } => (kind, message),
            RecordKind::ScopeExit { .. } => return false,
        };

        match self {
            EventFilter::Kind(expected_kind) => kind == expected_kind,

            EventFilter::MessageContains(substring) => {
                message.to_lowercase().contains(&substring.to_lowercase())
            }

            EventFilter::MessageContainsCaseSensitive(substring) => {
                message.contains(substring)
            }

            EventFilter::InScope(scope_id) => record.scope == Some(*scope_id),

            EventFilter::Unscoped => record.scope.is_none(),

            EventFilter::Scoped => record.scope.is_some(),

            EventFilter::And(left, right) => {
                left.matches(record) && right.matches(record)
            }

            EventFilter::Or(left, right) => {
                left.matches(record) || right.matches(record)
            }

            EventFilter::Not(inner) => !inner.matches(record),

            EventFilter::Any => true,

            EventFilter::None => false,
        }
    }
}

/// Combined filter for both scopes and events.
///
/// This is the primary filter type used by renderers and writers.
#[derive(Debug, Clone)]
pub struct Filter {
    pub scope_filter: ScopeFilter,
    pub event_filter: EventFilter,
}

impl Filter {
    /// Create a new filter with the given scope and event filters.
    pub fn new(scope_filter: ScopeFilter, event_filter: EventFilter) -> Self {
        Self {
            scope_filter,
            event_filter,
        }
    }

    /// Create a filter that only filters scopes (all events pass).
    pub fn scope(scope_filter: ScopeFilter) -> Self {
        Self {
            scope_filter,
            event_filter: EventFilter::Any,
        }
    }

    /// Create a filter that only filters events (all scopes pass).
    pub fn event(event_filter: EventFilter) -> Self {
        Self {
            scope_filter: ScopeFilter::Any,
            event_filter,
        }
    }

    /// Create a filter that matches everything.
    pub fn any() -> Self {
        Self {
            scope_filter: ScopeFilter::Any,
            event_filter: EventFilter::Any,
        }
    }

    /// Create a filter that matches nothing.
    pub fn none() -> Self {
        Self {
            scope_filter: ScopeFilter::None,
            event_filter: EventFilter::None,
        }
    }

    /// Combine this filter with another using logical AND.
    ///
    /// Both scope and event filters are combined independently.
    pub fn and(self, other: Filter) -> Self {
        Self {
            scope_filter: self.scope_filter.and(other.scope_filter),
            event_filter: self.event_filter.and(other.event_filter),
        }
    }

    /// Combine this filter with another using logical OR.
    pub fn or(self, other: Filter) -> Self {
        Self {
            scope_filter: self.scope_filter.or(other.scope_filter),
            event_filter: self.event_filter.or(other.event_filter),
        }
    }

    /// Check if a scope matches the scope filter.
    pub fn matches_scope(&self, scope: &Scope, journal: &Journal) -> bool {
        self.scope_filter.matches(scope, journal)
    }

    /// Check if a record matches the event filter.
    pub fn matches_event(&self, record: &Record) -> bool {
        self.event_filter.matches(record)
    }
}

impl Default for Filter {
    fn default() -> Self {
        Self::any()
    }
}

// Helper functions

/// Get the outcome of a scope from the journal.
fn get_scope_outcome(scope: &Scope, journal: &Journal) -> Outcome {
    journal
        .records()
        .iter()
        .find(|r| matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(scope.id))
        .and_then(|r| {
            if let RecordKind::ScopeExit { outcome, .. } = r.kind {
                Some(outcome)
            } else {
                None
            }
        })
        .unwrap_or(Outcome::Aborted)
}

/// Get the duration of a scope in milliseconds.
fn get_scope_duration(scope: &Scope, journal: &Journal) -> u64 {
    journal
        .records()
        .iter()
        .find(|r| matches!(r.kind, RecordKind::ScopeExit { .. }) && r.scope == Some(scope.id))
        .map(|r| r.time.saturating_sub(scope.entered_at))
        .unwrap_or(0)
}

/// Compute the depth of a scope (0 for root scopes).
fn compute_scope_depth(scope: &Scope, journal: &Journal) -> usize {
    let mut depth = 0;
    let mut current = scope.parent;

    while let Some(parent_id) = current {
        depth += 1;
        current = journal
            .scopes()
            .iter()
            .find(|s| s.id == parent_id)
            .and_then(|s| s.parent);
    }

    depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::Journal;

    #[test]
    fn test_outcome_filter() {
        let mut journal = Journal::new();
        let scope = journal.enter_scope_unnamed(None);
        journal.exit_scope(scope, Outcome::Success);

        let filter = ScopeFilter::Outcome(Outcome::Success);
        let scope_obj = &journal.scopes()[0];
        assert!(filter.matches(scope_obj, &journal));

        let filter = ScopeFilter::Outcome(Outcome::Failure);
        assert!(!filter.matches(scope_obj, &journal));
    }

    #[test]
    fn test_event_kind_filter() {
        let mut journal = Journal::new();
        journal.record_with_kind(None, EventKind::Warning, "test warning");

        let filter = EventFilter::Kind(EventKind::Warning);
        let record = &journal.records()[0];
        assert!(filter.matches(record));

        let filter = EventFilter::Kind(EventKind::Error);
        assert!(!filter.matches(record));
    }

    #[test]
    fn test_combined_filters() {
        let mut journal = Journal::new();
        let scope = journal.enter_scope(None, Some("test"));
        journal.warn(Some(scope), "warning message");
        journal.exit_scope(scope, Outcome::Failure);

        let scope_obj = &journal.scopes()[0];
        let record = &journal.records()[0];

        // Test AND
        let filter = ScopeFilter::Outcome(Outcome::Failure)
            .and(ScopeFilter::NameEquals("test".to_string()));
        assert!(filter.matches(scope_obj, &journal));

        // Test OR
        let filter = EventFilter::Kind(EventKind::Warning)
            .or(EventFilter::Kind(EventKind::Error));
        assert!(filter.matches(record));

        // Test NOT
        let filter = ScopeFilter::Outcome(Outcome::Success).not();
        assert!(filter.matches(scope_obj, &journal));
    }

    #[test]
    fn test_depth_filter() {
        let mut journal = Journal::new();
        let root = journal.enter_scope_unnamed(None);
        let child = journal.enter_scope_unnamed(Some(root));
        let _grandchild = journal.enter_scope_unnamed(Some(child));

        let root_scope = &journal.scopes()[0];
        let child_scope = &journal.scopes()[1];
        let grandchild_scope = &journal.scopes()[2];

        let filter = ScopeFilter::MaxDepth(1);
        assert!(filter.matches(root_scope, &journal));
        assert!(filter.matches(child_scope, &journal));
        assert!(!filter.matches(grandchild_scope, &journal));

        let filter = ScopeFilter::MinDepth(1);
        assert!(!filter.matches(root_scope, &journal));
        assert!(filter.matches(child_scope, &journal));
        assert!(filter.matches(grandchild_scope, &journal));
    }

    #[test]
    fn test_message_filter() {
        let mut journal = Journal::new();
        journal.record(None, "This is a test MESSAGE");

        let record = &journal.records()[0];

        let filter = EventFilter::MessageContains("test".to_string());
        assert!(filter.matches(record));

        let filter = EventFilter::MessageContains("TEST".to_string());
        assert!(filter.matches(record)); // Case-insensitive

        let filter = EventFilter::MessageContainsCaseSensitive("MESSAGE".to_string());
        assert!(filter.matches(record));

        let filter = EventFilter::MessageContainsCaseSensitive("message".to_string());
        assert!(!filter.matches(record)); // Case-sensitive
    }
}
