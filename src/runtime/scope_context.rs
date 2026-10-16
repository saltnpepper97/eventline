use super::SCOPE_STACK;
use crate::core::{Outcome, RuntimeScopeGuard, ScopeId};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A snapshot of the active runtime scopes, for explicit async propagation.
///
/// Each wrapped future owns an independent stack. Scopes entered by one future
/// cannot leak into another future between polls, even on the same thread.
/// Context follows the future when it moves to another worker thread.
///
/// Capture context before spawning a task; executors do not inherit it
/// automatically. A snapshot contains scope IDs, not ownership of those scopes,
/// so keep parent scopes open until their child operations finish.
#[derive(Clone, Debug, Default)]
pub struct ScopeContext {
    stack: Vec<ScopeId>,
}

impl ScopeContext {
    /// Create an empty context for work unrelated to the current scopes.
    pub const fn new() -> Self {
        Self { stack: Vec::new() }
    }

    /// Capture the active scope stack at the point this method is called.
    pub fn capture() -> Self {
        SCOPE_STACK.with(|stack| Self {
            stack: stack.borrow().clone(),
        })
    }

    /// Run a future with this context installed during every poll and on drop.
    ///
    /// The calling thread's context is restored when a poll returns or panics.
    /// Wrap each concurrently polled operation separately, including siblings
    /// in combinators such as `join!` or `select!`.
    ///
    /// The future is pinned in a box; no executor dependency is required.
    /// The wrapper is `Send` when the wrapped future is `Send`.
    ///
    /// ```
    /// # async fn example() {
    /// eventline::scope_async("request", async {
    ///     let work = eventline::ScopeContext::capture().wrap(async {
    ///         eventline::info!("still in request");
    ///     });
    ///     // `work` can also be passed to an executor's spawn function.
    ///     work.await;
    /// }).await;
    /// # }
    /// ```
    pub fn wrap<F: Future>(self, future: F) -> WithScopeContext<F> {
        WithScopeContext {
            context: self,
            future: Some(Box::pin(future)),
        }
    }
}

/// A future carrying its own runtime scope stack.
///
/// Construct with [`ScopeContext::wrap`]. Dropping a pending future installs its
/// context while its destructors run, so cancellation also cleans up scopes on
/// the correct stack. Existing scope guards retain their usual drop outcomes.
#[must_use = "futures do nothing unless polled or awaited"]
pub struct WithScopeContext<F> {
    context: ScopeContext,
    // Option lets Drop destroy the future before restoring the thread's stack.
    future: Option<Pin<Box<F>>>,
}

impl<F: Future> Future for WithScopeContext<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let _installed = InstalledContext::enter(&mut this.context.stack);
        let result = this
            .future
            .as_mut()
            .expect("scope-context future polled after completion")
            .as_mut()
            .poll(cx);
        if result.is_ready() {
            // Custom futures may keep guards in fields even after completion.
            // Destroy those fields while their own context is still installed.
            this.future.take();
        }
        result
    }
}

impl<F> Drop for WithScopeContext<F> {
    fn drop(&mut self) {
        let _installed = InstalledContext::enter(&mut self.context.stack);
        self.future.take();
    }
}

/// Run a future in a new named scope, inheriting context at this call site.
///
/// The scope is entered lazily on the first poll. Completion records Success,
/// cancellation records Aborted, and unwinding records Failure. Completion
/// does not inspect the output (including `Result` values). A future dropped
/// before its first poll does not enter a scope.
/// Use [`scope_async_result`] to derive success or failure from a `Result`.
///
/// This works with any executor. Wrap separately spawned or concurrently
/// polled operations individually; wrapping an outer task alone cannot isolate
/// unwrapped siblings inside it.
///
/// ```
/// # async fn example() {
/// let answer = eventline::scope_async("request", async {
///     eventline::info!("processing");
///     eventline::scope_async("lookup", async { 42 }).await
/// }).await;
/// assert_eq!(answer, 42);
/// # }
/// ```
// Tie the returned lifetime to F, allowing a borrowed name to be released
// immediately after it has been converted to an owned String below.
pub fn scope_async<'a, F: Future + 'a>(
    name: impl Into<String>,
    future: F,
) -> impl Future<Output = F::Output> + 'a {
    let context = ScopeContext::capture();
    let name = name.into();
    context.wrap(async move {
        let mut guard = AsyncScopeGuard(super::scope_guard(name));
        let output = future.await;
        guard.0.exit(Outcome::Success);
        output
    })
}

/// Run a future in a new scope whose outcome follows its returned `Result`.
///
/// `Ok` records Success, `Err` records Failure, cancellation after the first
/// poll records Aborted, and unwinding records Failure. The original result is
/// returned without cloning or formatting its contents. A future dropped
/// before its first poll creates no scope.
///
/// Context is captured at this call site, just like [`scope_async`]. Wrap each
/// spawned or concurrently polled operation separately. For explicit abortion,
/// drop the pending future; an error value representing cancellation still
/// counts as `Err` and records Failure. Use a guard inside [`ScopeContext::wrap`]
/// when application-specific errors need a different outcome.
///
/// ```
/// # async fn example() {
/// let result = eventline::scope_async_result("config", async {
///     Err::<(), _>("invalid config")?;
///     Ok(())
/// }).await;
/// assert_eq!(result, Err("invalid config"));
/// # }
/// ```
pub fn scope_async_result<'a, T, E, F>(
    name: impl Into<String>,
    future: F,
) -> impl Future<Output = Result<T, E>> + 'a
where
    F: Future<Output = Result<T, E>> + 'a,
{
    let context = ScopeContext::capture();
    let name = name.into();
    context.wrap(async move {
        let mut guard = AsyncScopeGuard(super::scope_guard(name));
        let result = future.await;
        guard.0.exit_result(&result);
        result
    })
}

struct AsyncScopeGuard(RuntimeScopeGuard);

impl Drop for AsyncScopeGuard {
    fn drop(&mut self) {
        self.0.exit(if std::thread::panicking() {
            Outcome::Failure
        } else {
            Outcome::Aborted
        });
    }
}

// Only the brief poll/drop call uses thread-local state. The owned future
// keeps its stack between calls, and this guard restores the previous stack
// even if user code panics. No RefCell borrow is held while running user code.
struct InstalledContext<'a> {
    previous: &'a mut Vec<ScopeId>,
}

impl<'a> InstalledContext<'a> {
    fn enter(stack: &'a mut Vec<ScopeId>) -> Self {
        SCOPE_STACK.with(|active| std::mem::swap(&mut *active.borrow_mut(), stack));
        Self { previous: stack }
    }
}

impl Drop for InstalledContext<'_> {
    fn drop(&mut self) {
        SCOPE_STACK.with(|active| std::mem::swap(&mut *active.borrow_mut(), self.previous));
    }
}
