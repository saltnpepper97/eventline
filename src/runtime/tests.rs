#[cfg(test)]
mod tests {
    // Import only what we need from the runtime module
    use crate::runtime::{ init, reset, with_journal, is_initialized};
    use crate::runtime::event::{record, info, warn, error, debug};
    use crate::runtime::scope::{scoped, scoped_async, try_scoped, try_scoped_async,
        try_scoped_unnamed, try_scoped_unnamed_async, current_scope
    };

    use crate::core::EventKind;
    use crate::Outcome;
    use serial_test::serial;

    /// Helper to ensure clean state between tests.
    async fn with_clean_runtime<F, Fut>(f: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        reset().await;
        init().await;
        f().await;
        reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_init() {
        reset().await;
        assert!(!is_initialized().await);
        init().await;
        assert!(is_initialized().await);
        reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_reset_clears_state() {
        with_clean_runtime(|| async {
            info("test event").await;

            with_journal(|journal| {
                assert_eq!(journal.records().len(), 1);
            }).await;

            reset().await;
            assert!(!is_initialized().await);
        }).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_record_without_init_is_noop() {
        reset().await;
        record(EventKind::Info, "test").await;
        reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_basic_recording() {
        with_clean_runtime(|| async {
            info("test info").await;
            warn("test warning").await;
            error("test error").await;
            debug("test debug").await;

            with_journal(|journal| {
                assert_eq!(journal.records().len(), 4);
            }).await;
        }).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_scoped_execution() {
        with_clean_runtime(|| async {
            scoped_async(Some("test_scope"), || async {
                info("inside scope").await;
            }).await;

            with_journal(|journal| {
                assert_eq!(journal.scopes().len(), 1);
                // Records: info, scope exit = 2
                assert_eq!(journal.records().len(), 2);
            }).await;
        }).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_scoped_async_execution() {
        with_clean_runtime(|| async {
            scoped_async(Some("test_scope_async"), || async {
                info("inside async scope").await;
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                info("after delay").await;
            }).await;

            with_journal(|journal| {
                assert_eq!(journal.scopes().len(), 1);
                // Records: info, info, scope exit = 3
                assert_eq!(journal.records().len(), 3);
            }).await;
        }).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_nested_scopes() {
        with_clean_runtime(|| async {
            scoped_async(Some("outer"), || async {
                info("outer event").await;

                scoped_async(Some("inner"), || async {
                    info("inner event").await;
                }).await;
            }).await;

            with_journal(|journal| {
                assert_eq!(journal.scopes().len(), 2);

                let inner_scope = &journal.scopes()[1];
                assert_eq!(inner_scope.parent, Some(journal.scopes()[0].id));
            }).await;
        }).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_nested_async_scopes() {
        with_clean_runtime(|| async {
            scoped_async(Some("outer"), || async {
                info("outer event").await;

                scoped_async(Some("inner"), || async {
                    info("inner event").await;
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }).await;

                info("back in outer").await;
            }).await;

            with_journal(|journal| {
                assert_eq!(journal.scopes().len(), 2);
                assert_eq!(journal.records().len(), 5); // enter/inner enter/info/info/inner exit/back info/exit
            }).await;
        }).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_panic_handling() {
        reset().await;
        init().await;

        let result = tokio::task::spawn(async {
            scoped_async(Some("panic_scope"), || async {
                info("before panic").await;
                panic!("test panic");
            }).await;
        }).await;

        assert!(result.is_err());

        with_journal(|journal| {
            let exit_record = journal.records().last().unwrap();
            match &exit_record.kind {
                crate::RecordKind::ScopeExit { outcome, .. } => {
                    assert_eq!(*outcome, Outcome::Aborted);
                }
                _ => panic!("Expected ScopeExit record"),
            }
        }).await;

        reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_current_scope_tracking() {
        with_clean_runtime(|| async {
            assert!(current_scope().await.is_none());

            scoped_async(Some("test"), || async {
                let scope_inside = current_scope().await;
                assert!(scope_inside.is_some());
     
                scoped_async(Some("nested"), move || async move {
                    let nested_scope = current_scope().await;
                    assert!(nested_scope.is_some());
                    assert_ne!(scope_inside, nested_scope);
                }).await;

                assert_eq!(current_scope().await, scope_inside);
            }).await;

            assert!(current_scope().await.is_none());
        }).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_multiple_tests_have_clean_state() {
        with_clean_runtime(|| async {
            info("test 1").await;
            with_journal(|journal| {
                assert_eq!(journal.records().len(), 1);
            }).await;
        }).await;

        with_clean_runtime(|| async {
            info("test 2").await;
            with_journal(|journal| {
                assert_eq!(journal.records().len(), 1);
            }).await;
        }).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_try_scoped_without_init() {
        reset().await;

        let result = try_scoped(Some("test"), || 42).await;

        assert_eq!(result, 42);
        reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_try_scoped_with_init() {
        with_clean_runtime(|| async {
            let result = try_scoped_async(Some("test"), || async {
                info("inside try_scoped").await;
                42
            }).await;

            assert_eq!(result, 42);

            with_journal(|journal| {
                assert_eq!(journal.scopes().len(), 1);
                assert_eq!(journal.scopes()[0].name.as_deref(), Some("test"));
            }).await;
        }).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_try_scoped_unnamed_without_init() {
        reset().await;

        let result = try_scoped_unnamed(|| "no panic").await;

        assert_eq!(result, "no panic");
        reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_try_scoped_unnamed_with_init() {
        with_clean_runtime(|| async {
            try_scoped_unnamed_async(|| async {
                info("in unnamed scope").await;
            }).await;

            with_journal(|journal| {
                assert_eq!(journal.scopes().len(), 1);
                assert!(journal.scopes()[0].name.is_none());
            }).await;
        }).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_sync_closure_in_async_scope() {
        with_clean_runtime(|| async {
            scoped(Some("sync_in_async"), || {
                // Sync closure can return sync value
                42
            }).await;

            with_journal(|journal| {
                assert_eq!(journal.scopes().len(), 1);
            }).await;
        }).await;
    }
}
