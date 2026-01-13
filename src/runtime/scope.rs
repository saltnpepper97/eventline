use std::future::Future;
use futures::FutureExt;

use super::{get_runtime, RUNTIME, CURRENT_SCOPE, THREAD_SCOPE};
use crate::journal::id::ScopeId;
use crate::Outcome;
use crate::scope::AsyncScopeGuard;

/// Get the current scope for this thread.
///
/// Returns `None` if no scope is active.
///
/// # Example
///
/// ```rust,ignore
/// // This example is async; mark as ignore for rustdoc tests or run inside #[tokio::test]
/// use eventline::runtime;
/// use tokio; // if using tokio runtime
///
/// #[tokio::main]
/// async fn main() {
///     runtime::init().await;
///
///     assert!(runtime::current_scope().await.is_none());
///
///     runtime::scoped(Some("test"), || async {
///         assert!(runtime::current_scope().await.is_some());
///     }).await;
/// }
/// ```
pub async fn current_scope() -> Option<ScopeId> {
    // Prefer the task-local (async) scope if available.
    if let Some(s) = CURRENT_SCOPE.try_with(|s| *s).ok().flatten() {
        return Some(s);
    }

    // Fallback to thread-local for synchronous scopes.
    THREAD_SCOPE.with(|c| *c.borrow())
}

pub fn current_scope_sync() -> Option<ScopeId> {
    // Prefer task-local
    if let Ok(Some(s)) = CURRENT_SCOPE.try_with(|s| *s) {
        return Some(s);
    }

    // Fallback to thread-local
    THREAD_SCOPE.with(|c| *c.borrow())
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
    let rt = get_runtime().await;

    let parent = CURRENT_SCOPE.try_with(|s| *s).ok().flatten();
    let scope_id = {
        let mut journal = rt.journal.lock().await;
        journal.enter_scope(parent, name)
    };

    // Set thread-local current scope for the synchronous closure path,
    // so events recorded inside `f()` immediately see the scope.
    //
    // Save/restore previous thread-local value.
    let result = THREAD_SCOPE.with(|c| {
        let mut w = c.borrow_mut();
        let prev = *w;
        *w = Some(scope_id);
        // Run the closure and capture result (catch unwind outside to restore).
        // We'll restore the thread-local after we know the result/panic state.
        // Return prev so the outer scope can restore it after journal exit too if needed.
        (prev, std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)))
    });

    // result is (prev_thread_scope, catch_unwind_result)
    let (prev_thread_scope, catch_result) = result;

    // restore thread-local now (we already executed the closure synchronously)
    THREAD_SCOPE.with(|c| {
        *c.borrow_mut() = prev_thread_scope;
    });

    // Exit scope in the journal
    let mut journal = rt.journal.lock().await;
    match catch_result {
        Ok(v) => {
            journal.exit_scope(scope_id, Outcome::Success);
            v
        }
        Err(panic) => {
            journal.exit_scope(scope_id, Outcome::Aborted);
            std::panic::resume_unwind(panic);
        }
    }
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
    let rt = get_runtime().await;

    let scope_id = {
        let mut journal = rt.journal.lock().await;
        let parent = CURRENT_SCOPE.try_with(|s| *s).ok().flatten();
        journal.enter_scope(parent, name)
    };

    let guard = AsyncScopeGuard::new(rt.journal.clone(), scope_id);

    CURRENT_SCOPE
        .scope(Some(scope_id), async move {
            let result = std::panic::AssertUnwindSafe(f())
                .catch_unwind()
                .await;

            match result {
                Ok(value) => {
                    guard.exit(Outcome::Success).await;
                    value
                }
                Err(panic) => {
                    guard.exit(Outcome::Aborted).await;
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
/// ```rust,ignore
/// use eventline::runtime;
/// use tokio; // for async runtime
///
/// #[tokio::main]
/// async fn main() {
///     // Works even without init()
///     let result = runtime::try_scoped_unnamed(|| 42).await;
///     assert_eq!(result, 42);
/// }
/// ```
pub async fn try_scoped_unnamed<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    try_scoped::<String, _, _>(None, f).await
}

/// Execute an async closure within a new scope, without panicking if the runtime is uninitialized.
///
/// This is the async variant of [`try_scoped`].  
/// If the runtime is initialized, it behaves like [`scoped_async`]; otherwise, the closure runs normally.
///
/// # Example
///
/// ```rust,ignore
/// use eventline::runtime;
///
/// // Use ignore because this example is async; run inside #[tokio::test] or async main
/// let result = runtime::try_scoped_async(Some("optional"), || async {
///     // Async work here
///     42
/// }).await;
/// assert_eq!(result, 42);
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

    let scope_id = {
        let mut journal = rt.journal.lock().await;
        let parent = CURRENT_SCOPE.try_with(|s| *s).ok().flatten();
        journal.enter_scope(parent, name)
    };

    let guard = AsyncScopeGuard::new(rt.journal.clone(), scope_id);

    CURRENT_SCOPE
        .scope(Some(scope_id), async move {
            let result = std::panic::AssertUnwindSafe(f()).catch_unwind().await;
            match result {
                Ok(v) => {
                    guard.exit(Outcome::Success).await;
                    v
                }
                Err(p) => {
                    // Aborted will be recorded by guard drop
                    std::panic::resume_unwind(p);
                }
            }
        })
        .await
}

/// Execute an async closure within a new unnamed scope, without panicking if the runtime is uninitialized.
///
/// This is a non-panicking async variant of [`scoped_unnamed`].  
/// If the runtime is initialized, it behaves like [`scoped_async`]; otherwise, the closure runs normally.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let result = runtime::try_scoped_unnamed_async(|| async {
///     // Async work here
///     42
/// }).await;
/// assert_eq!(result, 42);
/// # });
/// ```
pub async fn try_scoped_unnamed_async<F, Fut, R>(f: F) -> R
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    try_scoped_async::<String, _, _, _>(None, f).await
}
