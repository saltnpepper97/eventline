#[cfg(test)]
mod tests {
    // Import only what we need from the runtime module
    use crate::runtime::{
        init, reset, record, info, warn, error, debug,
        scoped, try_scoped, try_scoped_unnamed,
        current_scope, with_journal,
    };

    use crate::EventKind;
    use crate::outcome::Outcome;

    /// Helper to ensure clean state between tests.
    fn with_clean_runtime<F>(f: F)
    where
        F: FnOnce(),
    {
        reset();
        init();
        f();
        reset();
    }

    #[test]
    fn test_init() {
        reset();
        assert!(!crate::runtime::is_initialized());
        init();
        assert!(crate::runtime::is_initialized());
        reset();
    }

    #[test]
    fn test_reset_clears_state() {
        with_clean_runtime(|| {
            info("test event");

            with_journal(|journal| {
                assert_eq!(journal.records().len(), 1);
            });

            reset();
            assert!(!crate::runtime::is_initialized());
        });
    }

    #[test]
    fn test_record_without_init_is_noop() {
        reset();
        record(EventKind::Info, "test");
        reset();
    }

    #[test]
    fn test_basic_recording() {
        with_clean_runtime(|| {
            info("test info");
            warn("test warning");
            error("test error");
            debug("test debug");

            with_journal(|journal| {
                assert_eq!(journal.records().len(), 4);
            });
        });
    }

    #[test]
    fn test_scoped_execution() {
        with_clean_runtime(|| {
            scoped(Some("test_scope"), || {
                info("inside scope");
            });

            with_journal(|journal| {
                assert_eq!(journal.scopes().len(), 1);
                assert_eq!(journal.records().len(), 2);
            });
        });
    }

    #[test]
    fn test_nested_scopes() {
        with_clean_runtime(|| {
            scoped(Some("outer"), || {
                info("outer event");

                scoped(Some("inner"), || {
                    info("inner event");
                });
            });

            with_journal(|journal| {
                assert_eq!(journal.scopes().len(), 2);

                let inner_scope = &journal.scopes()[1];
                assert_eq!(inner_scope.parent, Some(journal.scopes()[0].id));
            });
        });
    }

    #[test]
    fn test_panic_handling() {
        reset();
        init();

        let result = std::panic::catch_unwind(|| {
            scoped(Some("panic_scope"), || {
                info("before panic");
                panic!("test panic");
            });
        });

        assert!(result.is_err());

        with_journal(|journal| {
            let exit_record = journal.records().last().unwrap();
            match &exit_record.kind {
                crate::record::RecordKind::ScopeExit { outcome, .. } => {
                    // Outcome is Copy, no deref needed
                    assert_eq!(*outcome, Outcome::Aborted);
                }
                _ => panic!("Expected ScopeExit record"),
            }
        });

        reset();
    }

    #[test]
    fn test_current_scope_tracking() {
        with_clean_runtime(|| {
            assert!(current_scope().is_none());

            scoped(Some("test"), || {
                let scope_inside = current_scope();
                assert!(scope_inside.is_some());

                scoped(Some("nested"), || {
                    let nested_scope = current_scope();
                    assert!(nested_scope.is_some());
                    assert_ne!(scope_inside, nested_scope);
                });

                assert_eq!(current_scope(), scope_inside);
            });

            assert!(current_scope().is_none());
        });
    }

    #[test]
    fn test_multiple_tests_have_clean_state() {
        with_clean_runtime(|| {
            info("test 1");
            with_journal(|journal| {
                assert_eq!(journal.records().len(), 1);
            });
        });

        with_clean_runtime(|| {
            info("test 2");
            with_journal(|journal| {
                assert_eq!(journal.records().len(), 1);
            });
        });
    }

    #[test]
    fn test_try_scoped_without_init() {
        reset();

        let result = try_scoped(Some("test"), || 42);

        assert_eq!(result, 42);
        reset();
    }

    #[test]
    fn test_try_scoped_with_init() {
        with_clean_runtime(|| {
            let result = try_scoped(Some("test"), || {
                info("inside try_scoped");
                42
            });

            assert_eq!(result, 42);

            with_journal(|journal| {
                assert_eq!(journal.scopes().len(), 1);
                assert_eq!(journal.scopes()[0].name.as_deref(), Some("test"));
            });
        });
    }

    #[test]
    fn test_try_scoped_unnamed_without_init() {
        reset();

        let result = try_scoped_unnamed(|| "no panic");

        assert_eq!(result, "no panic");
        reset();
    }

    #[test]
    fn test_try_scoped_unnamed_with_init() {
        with_clean_runtime(|| {
            try_scoped_unnamed(|| {
                info("in unnamed scope");
            });

            with_journal(|journal| {
                assert_eq!(journal.scopes().len(), 1);
                assert!(journal.scopes()[0].name.is_none());
            });
        });
    }
}
