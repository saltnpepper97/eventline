#[cfg(test)]
mod tests {
    use crate::runtime;
    use crate::runtime::log_level::{set_log_level, LogLevel};
    use serial_test::serial;
    use tokio::time::Duration;

    use crate::{
        event_info, 
        event_warn, 
        event_error, 
        event_debug, 
        event_scope,
        event_scope_async,
        try_scope_async,
        event_scope_unnamed,
        event_scope_unnamed_async,
        try_scope,
        try_scope_unnamed,
        try_scope_unnamed_async,
    };

    // Import the macros explicitly
   
    async fn safe_reset() {
        runtime::reset().await;
        runtime::init().await;
        set_log_level(LogLevel::Debug);
    }
  
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_event_macros() {
        safe_reset().await;

        try_scope_async!("test", {
            event_info!("test");
            event_warn!("test {}", 42);
            event_error!("test");
            event_debug!("test");
        }).await;

        tokio::time::sleep(Duration::from_millis(50)).await;

        runtime::with_journal(|journal| {
            assert_eq!(journal.records().len(), 5);
        }).await;

        runtime::reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_scope_macro() {
        safe_reset().await;
        event_scope!("test_scope", { 
            event_info!("inside"); 
        }).await;

        runtime::with_journal(|journal| {
            assert_eq!(journal.scopes().len(), 1);
            assert_eq!(journal.scopes()[0].name.as_deref(), Some("test_scope"));
        }).await;

        runtime::reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_scope_async_macro() {
        safe_reset().await;
        try_scope_async!("test_scope_async", {
            event_info!("inside async");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            event_info!("after delay");
        }).await;

        runtime::with_journal(|journal| {
            assert_eq!(journal.scopes().len(), 1);
            assert_eq!(journal.scopes()[0].name.as_deref(), Some("test_scope_async"));
            assert_eq!(journal.records().len(), 3); // enter, info, info, exit
        }).await;

        runtime::reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_unnamed_scope_macro() {
        safe_reset().await;
        event_scope_unnamed!(async { 
            event_info!("inside"); 
        }).await;

        runtime::with_journal(|journal| {
            assert_eq!(journal.scopes().len(), 1);
            assert!(journal.scopes()[0].name.is_none());
        }).await;

        runtime::reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_unnamed_scope_async_macro() {
        safe_reset().await;
        event_scope_unnamed_async!({
            event_info!("inside async unnamed");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }).await;

        runtime::with_journal(|journal| {
            assert_eq!(journal.scopes().len(), 1);
            assert!(journal.scopes()[0].name.is_none());
        }).await;

        runtime::reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_try_scope_macro_without_init() {
        runtime::reset().await;
        let result = try_scope!("test", async { 42 }).await;
        assert_eq!(result, 42);
        runtime::reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_try_scope_macro_with_init() {
        safe_reset().await;
        try_scope!("test", async { 
            event_info!("inside"); 
        }).await;

        runtime::with_journal(|journal| {
            assert_eq!(journal.scopes().len(), 1);
            assert_eq!(journal.scopes()[0].name.as_deref(), Some("test"));
        }).await;

        runtime::reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_try_scope_async_macro() {
        runtime::reset().await;
        let result = try_scope_async!("test_async", {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            42
        }).await;
        assert_eq!(result, 42);
        runtime::reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_try_scope_unnamed_macro() {
        runtime::reset().await;
        let result = try_scope_unnamed!(async { 42 }).await;
        assert_eq!(result, 42);
        runtime::reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_try_scope_unnamed_async_macro() {
        runtime::reset().await;
        let result = try_scope_unnamed_async!({
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            42
        }).await;
        assert_eq!(result, 42);
        runtime::reset().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial]
    async fn test_nested_async_scopes() {
        safe_reset().await;
        
        event_scope_async!("outer", {
            event_info!("outer scope");
            
            event_scope_async!("inner", {
                event_info!("inner scope");
            }).await;
            
            event_info!("back to outer");
        }).await;

        runtime::with_journal(|journal| {
            assert_eq!(journal.scopes().len(), 2);
            // Records: info(outer), info(inner), ScopeExit(inner), info(back outer), ScopeExit(outer) = 5
            assert_eq!(journal.records().len(), 5);
        }).await;

        runtime::reset().await;
    }
}
