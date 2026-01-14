#[cfg(test)]
mod validation_tests {
    use crate::journal::{EventKind, RecordKind, Journal, ScopeId, Outcome};

    #[test]
    #[should_panic(expected = "non-existent scope")]
    #[cfg(debug_assertions)]
    fn test_record_invalid_scope_panics_in_debug() {
        let mut journal = Journal::new();
        let fake_scope = ScopeId(999);

        // Default informational event
        journal.record(Some(fake_scope), "This should panic in debug");
    }

    #[test]
    #[should_panic(expected = "non-existent scope")]
    #[cfg(debug_assertions)]
    fn test_record_with_kind_invalid_scope_panics_in_debug() {
        let mut journal = Journal::new();
        let fake_scope = ScopeId(999);

        // Explicitly typed event
        journal.record_with_kind(
            Some(fake_scope),
            EventKind::Error,
            "This should also panic in debug",
        );
    }

    #[test]
    #[should_panic(expected = "non-existent scope")]
    #[cfg(debug_assertions)]
    fn test_exit_invalid_scope_panics_in_debug() {
        let mut journal = Journal::new();
        let fake_scope = ScopeId(999);

        journal.exit_scope(fake_scope, Outcome::Success);
    }

    #[test]
    fn test_valid_scope_operations() {
        let mut journal = Journal::new();
        let scope = journal.enter_scope_unnamed(None);

        // Informational event via default API
        journal.record(Some(scope), "valid info event");

        // Explicit semantic events
        journal.warn(Some(scope), "something unexpected happened");
        journal.error(Some(scope), "something went wrong");

        journal.exit_scope(scope, Outcome::Success);
    }

    #[test]
    fn test_event_kind_is_preserved() {
        let mut journal = Journal::new();
        let scope = journal.enter_scope_unnamed(None);
        journal.record_with_kind(
            Some(scope),
            EventKind::Warning,
            "this is a warning",
        );
        let record = journal.records().last().unwrap();
        match &record.kind {
            RecordKind::Event { kind, message, fields } => {
                assert_eq!(*kind, EventKind::Warning);
                assert_eq!(message, "this is a warning");
                assert!(fields.is_empty());
            }
            _ => panic!("Expected RecordKind::Event"),
        }
    }
}
