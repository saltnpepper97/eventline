use std::future::Future;
use futures::FutureExt;

use super::{get_runtime, RUNTIME, CURRENT_SCOPE};
use crate::journal::id::ScopeId;
use crate::Outcome;

/// Get the current scope for this thread.
///
/// Returns `None` if no scope is active.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
///
/// assert!(runtime::current_scope().is_none());
///
/// runtime::scoped(Some("test"), || {
///     assert!(runtime::current_scope().is_some());
/// });
/// ```
pub async fn current_scope() -> Option<ScopeId> {
    CURRENT_SCOPE.try_with(|s| *s).ok().flatten()
}

/// Execute a closure within a new scope.
///
/// This automatically:
/// - Enters a scope before executing the closure
/// - Sets it as the current scope for this thread
/// - Records all events from the closure in that scope
/// - Exits the scope after the closure completes
/// - Restores the previous scope context
/// - Handles panics by marking the scope as [`Outcome::Aborted`]
///
/// # Panics
///
/// Panics if the runtime is not initialized.  
/// Panics inside the closure are propagated after the scope is marked `Aborted`.
///
/// # Note
///
/// If a panic occurs, the journal mutex is safely unpoisoned to allow further logging.
pub async fn scoped<S, F, R>(name: Option<S>, f: F) -> R
where
    S: Into<String>,
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    // Get runtime
    let rt = get_runtime().await;

    // Enter scope in journal
    let scope_id = {
        let mut journal = rt.journal.lock().await;
        let parent = CURRENT_SCOPE.try_with(|s| *s).ok().flatten();
        journal.enter_scope(parent, name)
    };

    // Run the closure inside the CURRENT_SCOPE
    CURRENT_SCOPE
        .scope(Some(scope_id), async move {
            // Execute closure, catching panics
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

            // Exit scope
            let mut journal = rt.journal.lock().await;
            match result {
                Ok(value) => {
                    journal.exit_scope(scope_id, Outcome::Success);
                    value
                }
                Err(panic) => {
                    journal.exit_scope(scope_id, Outcome::Aborted);
                    std::panic::resume_unwind(panic);
                }
            }
        })
        .await
}

/// Execute a closure within a new unnamed scope.
///
/// This is a convenience wrapper for `scoped(None, f)`.
///
/// # Panics
///
/// Panics if the runtime is not initialized. For a non-panicking variant,
/// use [`try_scoped_unnamed`].
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
///
/// runtime::scoped_unnamed(|| {
///     runtime::info("Anonymous task");
/// });
/// ```
pub async fn scoped_unnamed<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    scoped::<String, _, _>(None, f).await
}

/// Async version of `scoped`.
///
/// Wraps the async closure in a scope, marking success/aborted automatically.
pub async fn scoped_async<S, F, Fut, R>(name: Option<S>, f: F) -> R
where
    S: Into<String>,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = R> + Send,
    R: Send + 'static,
{
    // Get runtime
    let rt = get_runtime().await;

    // Enter scope in journal
    let scope_id = {
        let mut journal = rt.journal.lock().await;
        let parent = CURRENT_SCOPE.try_with(|s| *s).ok().flatten();
        journal.enter_scope(parent, name)
    };

    // Run the async closure inside a new CURRENT_SCOPE
    CURRENT_SCOPE
        .scope(Some(scope_id), async move {
            let fut = async {
                f().await
            };

            let result = std::panic::AssertUnwindSafe(fut).catch_unwind().await;

            // exit scope
            let mut journal = rt.journal.lock().await;
            match result {
                Ok(value) => {
                    journal.exit_scope(scope_id, Outcome::Success);
                    value
                }
                Err(panic) => {
                    journal.exit_scope(scope_id, Outcome::Aborted);
                    std::panic::resume_unwind(panic);
                }
            }
        })
        .await
}

/// Execute a closure within a new scope, without panicking if runtime is uninitialized.
///
/// This is a non-panicking variant of [`scoped`]. If the runtime is not initialized,
/// the closure is executed normally without logging. If the runtime is initialized,
/// it behaves identically to [`scoped`].
///
/// # Note
///
/// If a panic occurs, the journal mutex is safely unpoisoned to allow further logging.
pub async fn try_scoped<S, F, R>(name: Option<S>, f: F) -> R
where
    S: Into<String>,
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    // Take a snapshot of the runtime
    let rt_opt = RUNTIME.read().await.clone();
    let Some(rt) = rt_opt else {
        // Runtime not initialized, just run the closure
        return f();
    };

    // Enter scope in the journal
    let scope_id = {
        let mut journal = rt.journal.lock().await;
        let parent = CURRENT_SCOPE.try_with(|s| *s).ok().flatten();
        journal.enter_scope(parent, name)
    };

    // Run the closure inside CURRENT_SCOPE
    CURRENT_SCOPE
        .scope(Some(scope_id), async move {
            // Execute closure, catching panics
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

            // Exit scope in journal
            let mut journal = rt.journal.lock().await;
            match result {
                Ok(value) => {
                    journal.exit_scope(scope_id, Outcome::Success);
                    value
                }
                Err(panic) => {
                    journal.exit_scope(scope_id, Outcome::Aborted);
                    std::panic::resume_unwind(panic);
                }
            }
        })
        .await
}

/// Execute a closure within a new unnamed scope, without panicking if runtime is uninitialized.
///
/// This is a non-panicking variant of [`scoped_unnamed`].
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// // Works even without init()
/// let result = runtime::try_scoped_unnamed(|| {
///     42
/// });
/// assert_eq!(result, 42);
/// ```
pub async fn try_scoped_unnamed<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    try_scoped::<String, _, _>(None, f).await
}

/// Execute an async closure within a new scope, without panicking if runtime is uninitialized.
///
/// This is the async variant of [`try_scoped`]. If the runtime is not initialized,
/// the closure is executed normally without logging. If the runtime is initialized,
/// it behaves identically to [`scoped_async`].
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::try_scoped_async(Some("optional"), || async {
///     // Async work here
///     42
/// }).await;
/// ```
pub async fn try_scoped_async<S, F, Fut, R>(name: Option<S>, f: F) -> R
where
    S: Into<String>,
    F: FnOnce() -> Fut,
    Fut: Future<Output = R>,
{
    let rt = match &*RUNTIME.read().await {
        Some(rt) => rt.clone(),
        None => return f().await,
    };

    // Enter scope
    let scope_id = {
        let mut journal = rt.journal.lock().await;
        let parent = CURRENT_SCOPE.try_with(|s| *s).ok().flatten();
        journal.enter_scope(parent, name)
    };

    // Run async closure inside the new scope
    CURRENT_SCOPE
        .scope(Some(scope_id), async move {
            let fut = std::panic::AssertUnwindSafe(f()).catch_unwind();
            let result = fut.await;

            // Exit scope
            let mut journal = rt.journal.lock().await;
            match result {
                Ok(value) => {
                    journal.exit_scope(scope_id, Outcome::Success);
                    value
                }
                Err(panic) => {
                    journal.exit_scope(scope_id, Outcome::Aborted);
                    std::panic::resume_unwind(panic);
                }
            }
        })
        .await
}

/// Execute an async closure within a new unnamed scope, without panicking if runtime is uninitialized.
///
/// This is a non-panicking async variant of [`scoped_unnamed`].
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// // Works even without init()
/// let result = runtime::try_scoped_unnamed_async(|| async {
///     42
/// }).await;
/// assert_eq!(result, 42);
/// ```
pub async fn try_scoped_unnamed_async<F, Fut, R>(f: F) -> R
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    try_scoped_async::<String, _, _, _>(None, f).await
}
