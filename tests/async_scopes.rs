use eventline::core::{Outcome, RecordKind, Scope};
use eventline::{ScopeContext, ScopeId, scope_async, scope_guard};
use std::future::{Future, pending, poll_fn};
use std::pin::Pin;
use std::sync::{Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

// These tests exercise the public API against the global journal. Serialize
// resets within this integration-test process, independently of unit tests.
static RUNTIME: Mutex<()> = Mutex::new(());

fn reset() -> MutexGuard<'static, ()> {
    let lock = RUNTIME.lock().unwrap();
    eventline::runtime::clear();
    eventline::disable_all_output();
    lock
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

async fn yield_once() {
    let mut yielded = false;
    poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await
}

fn scope(name: &str) -> Scope {
    eventline::scopes()
        .into_iter()
        .find(|scope| scope.name.as_deref() == Some(name))
        .expect("scope was recorded")
}

fn event_scope(message: &str) -> Option<ScopeId> {
    eventline::records()
        .into_iter()
        .find(|record| matches!(&record.kind, RecordKind::Event { name, .. } if name == message))
        .expect("event was recorded")
        .scope
}

fn outcomes(id: ScopeId) -> Vec<Outcome> {
    eventline::records()
        .into_iter()
        .filter_map(|record| match record.kind {
            RecordKind::ScopeExit { outcome, .. } if record.scope == Some(id) => Some(outcome),
            _ => None,
        })
        .collect()
}

async fn operation(name: &'static str) {
    let _guard = scope_guard(format!("{name}-nested"));
    eventline::info!("{name}-before");
    yield_once().await;
    eventline::info!("{name}-after");
}

#[test]
fn interleaved_futures_do_not_share_scopes_or_inherit_the_polling_thread() {
    let _lock = reset();
    let mut first = Box::pin(scope_async("first", operation("first")));
    let mut second = Box::pin(scope_async("second", operation("second")));
    let caller = scope_guard("caller");

    assert!(poll_once(first.as_mut()).is_pending());
    assert!(poll_once(second.as_mut()).is_pending());
    eventline::info!("between polls");
    assert!(poll_once(first.as_mut()).is_ready());
    assert!(poll_once(second.as_mut()).is_ready());
    eventline::info!("after polls");

    for name in ["first", "second"] {
        let root = scope(name);
        let nested = scope(&format!("{name}-nested"));
        assert_eq!(root.parent, None);
        assert_eq!(nested.parent, Some(root.id));
        assert_eq!(event_scope(&format!("{name}-before")), Some(nested.id));
        assert_eq!(event_scope(&format!("{name}-after")), Some(nested.id));
        assert_eq!(outcomes(root.id), [Outcome::Success]);
    }
    assert_eq!(event_scope("between polls"), Some(caller.id()));
    assert_eq!(event_scope("after polls"), Some(caller.id()));
}

#[test]
fn scope_context_migrates_between_threads_and_restores_both_workers() {
    let _lock = reset();
    let future = Box::pin(scope_async("migrating", operation("migrating")));
    let future = std::thread::spawn(move || {
        let worker = scope_guard("worker-a");
        let mut future = future;
        assert!(poll_once(future.as_mut()).is_pending());
        eventline::info!("worker-a outside");
        assert_eq!(event_scope("worker-a outside"), Some(worker.id()));
        future
    })
    .join()
    .unwrap();

    std::thread::spawn(move || {
        let worker = scope_guard("worker-b");
        let mut future = future;
        assert!(poll_once(future.as_mut()).is_ready());
        eventline::info!("worker-b outside");
        assert_eq!(event_scope("worker-b outside"), Some(worker.id()));
    })
    .join()
    .unwrap();

    let nested = scope("migrating-nested");
    assert_eq!(nested.parent, Some(scope("migrating").id));
    assert_eq!(scope("migrating").parent, None);
    assert_eq!(event_scope("migrating-before"), Some(nested.id));
    assert_eq!(event_scope("migrating-after"), Some(nested.id));
    assert_eq!(outcomes(nested.id), [Outcome::Success]);
}

#[test]
fn nested_async_scopes_and_scope_macros_restore_their_parents() {
    let _lock = reset();
    let mut future = Box::pin(scope_async("parent", async {
        scope_async("child", async {
            eventline::scope!("macro", success = "finished", {
                yield_once().await;
                eventline::info!("inside macro");
            });
            eventline::info!("after macro");
        })
        .await;
        eventline::info!("after child");
        42
    }));
    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(poll_once(future.as_mut()), Poll::Ready(42));
    eventline::info!("outside");

    let parent = scope("parent");
    let child = scope("child");
    let macro_scope = scope("macro");
    assert_eq!(child.parent, Some(parent.id));
    assert_eq!(macro_scope.parent, Some(child.id));
    assert_eq!(
        macro_scope.exit_messages.success.as_deref(),
        Some("finished")
    );
    assert_eq!(event_scope("inside macro"), Some(macro_scope.id));
    assert_eq!(event_scope("after macro"), Some(child.id));
    assert_eq!(event_scope("after child"), Some(parent.id));
    assert_eq!(event_scope("outside"), None);
}

#[test]
fn captured_contexts_fork_independent_stacks_at_the_capture_site() {
    let _lock = reset();
    let parent = scope_guard("captured parent");
    let context = ScopeContext::capture();
    let mut first = Box::pin(context.clone().wrap(operation("first")));
    let mut second = Box::pin(context.wrap(operation("second")));
    let poller = scope_guard("poller");

    assert!(poll_once(first.as_mut()).is_pending());
    assert!(poll_once(second.as_mut()).is_pending());
    assert!(poll_once(first.as_mut()).is_ready());
    assert!(poll_once(second.as_mut()).is_ready());
    eventline::info!("poller event");

    for name in ["first", "second"] {
        let nested = scope(&format!("{name}-nested"));
        assert_eq!(nested.parent, Some(parent.id()));
        assert_eq!(event_scope(&format!("{name}-after")), Some(nested.id));
    }
    assert_eq!(event_scope("poller event"), Some(poller.id()));
}

#[test]
fn empty_context_does_not_inherit_ambient_scopes() {
    let _lock = reset();
    let caller = scope_guard("caller");
    let mut future = Box::pin(ScopeContext::new().wrap(async {
        eventline::info!("detached event");
        operation("detached").await;
    }));

    assert!(poll_once(future.as_mut()).is_pending());
    eventline::info!("caller between polls");
    assert!(poll_once(future.as_mut()).is_ready());

    assert_eq!(event_scope("detached event"), None);
    assert_eq!(scope("detached-nested").parent, None);
    assert_eq!(event_scope("caller between polls"), Some(caller.id()));
}

struct LogOnDrop(&'static str);

impl Drop for LogOnDrop {
    fn drop(&mut self) {
        eventline::info!("{}", self.0);
    }
}

#[test]
fn cancellation_on_another_thread_uses_the_future_context_and_exits_once() {
    let _lock = reset();
    let mut future = Box::pin(scope_async("cancelled", async {
        let _guard = scope_guard("nested");
        let _on_drop = LogOnDrop("cancel cleanup");
        pending::<()>().await;
    }));
    assert!(poll_once(future.as_mut()).is_pending());

    std::thread::spawn(move || {
        let caller = scope_guard("canceller");
        drop(future);
        eventline::info!("after cancellation");
        assert_eq!(event_scope("after cancellation"), Some(caller.id()));
    })
    .join()
    .unwrap();

    assert_eq!(event_scope("cancel cleanup"), Some(scope("nested").id));
    assert_eq!(outcomes(scope("cancelled").id), [Outcome::Aborted]);
    // The existing synchronous guard keeps its historical drop behavior.
    assert_eq!(outcomes(scope("nested").id), [Outcome::Success]);
}

#[test]
fn panicking_poll_restores_context_and_records_failure_once() {
    let _lock = reset();
    let mut future = Box::pin(scope_async("panicking", async {
        let _guard = scope_guard("nested");
        yield_once().await;
        panic!("test panic");
    }));
    let caller = scope_guard("caller");
    assert!(poll_once(future.as_mut()).is_pending());
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| poll_once(future.as_mut())));
    assert!(result.is_err());
    drop(future);
    eventline::info!("after panic");

    assert_eq!(event_scope("after panic"), Some(caller.id()));
    assert_eq!(outcomes(scope("panicking").id), [Outcome::Failure]);
    assert_eq!(outcomes(scope("nested").id), [Outcome::Failure]);
}

#[test]
fn unpolled_async_scope_never_enters_the_journal() {
    let _lock = reset();
    drop(scope_async("unpolled", async {
        panic!("must not run");
    }));
    assert!(eventline::scopes().is_empty());
    assert!(eventline::records().is_empty());
}

#[test]
fn completion_preserves_result_outputs_and_does_not_infer_outcomes() {
    let _lock = reset();
    let mut future = Box::pin(scope_async("result", async { Err::<(), _>("expected") }));
    assert_eq!(poll_once(future.as_mut()), Poll::Ready(Err("expected")));
    drop(future);
    assert_eq!(outcomes(scope("result").id), [Outcome::Success]);
}

struct ReadyWithGuard(Option<eventline::RuntimeScopeGuard>);

impl Future for ReadyWithGuard {
    type Output = u32;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<u32> {
        self.0 = Some(scope_guard("custom future"));
        Poll::Ready(42)
    }
}

impl Drop for ReadyWithGuard {
    fn drop(&mut self) {
        eventline::info!("completed future cleanup");
    }
}

#[test]
fn completed_custom_futures_are_dropped_in_context_before_returning_ready() {
    let _lock = reset();
    let mut future = Box::pin(ScopeContext::new().wrap(ReadyWithGuard(None)));
    let caller = scope_guard("caller");
    assert_eq!(poll_once(future.as_mut()), Poll::Ready(42));
    assert!(scope("custom future").exited_at.is_some());
    eventline::info!("after ready");
    drop(future);

    assert_eq!(
        event_scope("completed future cleanup"),
        Some(scope("custom future").id)
    );
    assert_eq!(outcomes(scope("custom future").id), [Outcome::Success]);
    assert_eq!(event_scope("after ready"), Some(caller.id()));
}

#[test]
fn panicking_destructor_also_restores_the_callers_context() {
    struct PanicOnDrop;
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            eventline::info!("panicking cleanup");
            panic!("test destructor panic");
        }
    }

    let _lock = reset();
    let mut future = Box::pin(scope_async("drop panic", async {
        let _on_drop = PanicOnDrop;
        pending::<()>().await;
    }));
    let caller = scope_guard("caller");
    assert!(poll_once(future.as_mut()).is_pending());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(future)));
    assert!(result.is_err());
    eventline::info!("after drop panic");

    assert_eq!(
        event_scope("panicking cleanup"),
        Some(scope("drop panic").id)
    );
    assert_eq!(event_scope("after drop panic"), Some(caller.id()));
    assert_eq!(outcomes(scope("drop panic").id), [Outcome::Failure]);
}

#[test]
fn contexts_support_borrowed_and_non_send_futures() {
    let _lock = reset();
    let shared = std::rc::Rc::new(std::cell::Cell::new(0));
    let mut output = 0;
    let mut future = Box::pin(scope_async("local operation", async {
        let _guard = scope_guard("local future");
        yield_once().await;
        shared.set(42);
        output = shared.get();
    }));

    assert!(poll_once(future.as_mut()).is_pending());
    assert!(poll_once(future.as_mut()).is_ready());
    drop(future);
    assert_eq!(output, 42);
    assert_eq!(outcomes(scope("local future").id), [Outcome::Success]);
}

#[test]
fn async_scope_owns_its_name_before_returning_the_future() {
    fn require_spawnable<F: Future + Send + 'static>(future: F) -> F {
        future
    }

    let _lock = reset();
    let future = {
        let name = String::from("owned name");
        scope_async(name.as_str(), async { 42 })
    };
    let mut future = Box::pin(require_spawnable(future));
    assert_eq!(poll_once(future.as_mut()), Poll::Ready(42));
    assert_eq!(outcomes(scope("owned name").id), [Outcome::Success]);
}

#[test]
fn exiting_an_already_finalized_scope_cleans_up_other_context_snapshots() {
    let _lock = reset();
    let id = eventline::enter_scope("shared scope");
    let mut future = Box::pin(ScopeContext::capture().wrap(async move {
        eventline::exit_scope(id, Outcome::Success);
        eventline::info!("after first exit");
    }));
    assert!(poll_once(future.as_mut()).is_ready());
    eventline::exit_scope(id, Outcome::Success);
    eventline::info!("after second exit");

    assert_eq!(event_scope("after first exit"), None);
    assert_eq!(event_scope("after second exit"), None);
    assert_eq!(outcomes(id), [Outcome::Success]);
}
