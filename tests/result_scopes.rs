use eventline::core::{Outcome, RecordKind};
use eventline::{ScopeContext, ScopeId, scope_async_result, scope_guard, scope_result};
use std::future::{Future, pending, poll_fn};
use std::pin::Pin;
use std::sync::{Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

static RUNTIME: Mutex<()> = Mutex::new(());

fn reset() -> MutexGuard<'static, ()> {
    let lock = RUNTIME.lock().unwrap();
    eventline::runtime::clear();
    eventline::disable_all_output();
    lock
}

fn scope_id(name: &str) -> ScopeId {
    eventline::scopes()
        .iter()
        .find(|scope| scope.name.as_deref() == Some(name))
        .expect("scope was recorded")
        .id
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

fn event_scope(message: &str) -> Option<ScopeId> {
    eventline::records()
        .into_iter()
        .find(|record| matches!(&record.kind, RecordKind::Event { name, .. } if name == message))
        .expect("event was recorded")
        .scope
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

#[test]
fn sync_scopes_preserve_owned_values_without_clone_or_error_trait_bounds() {
    struct Value(Box<u32>);
    struct Error(Box<u32>);

    let _lock = reset();
    let value = Value(Box::new(42));
    let address = &*value.0 as *const u32;
    let result = scope_result("success", || Ok::<_, Error>(value));
    let Ok(value) = result else {
        panic!("expected Ok")
    };
    assert_eq!(&*value.0 as *const u32, address);
    assert_eq!(outcomes(scope_id("success")), [Outcome::Success]);

    let error = Error(Box::new(7));
    let address = &*error.0 as *const u32;
    let result = scope_result("failure", || Err::<Value, _>(error));
    let Err(error) = result else {
        panic!("expected Err")
    };
    assert_eq!(&*error.0 as *const u32, address);
    assert_eq!(outcomes(scope_id("failure")), [Outcome::Failure]);
}

#[test]
fn macro_records_failure_when_question_mark_returns_early() {
    fn read_config() -> Result<(), &'static str> {
        Err("missing config")
    }
    fn load_config() -> Result<(), &'static str> {
        eventline::scope_result!("config", {
            read_config()?;
            panic!("must not continue after Err");
        })
    }

    let _lock = reset();
    let caller = scope_guard("caller");
    assert_eq!(load_config(), Err("missing config"));
    eventline::info!("after config");
    assert_eq!(outcomes(scope_id("config")), [Outcome::Failure]);
    assert_eq!(event_scope("after config"), Some(caller.id()));
}

#[test]
fn macro_return_and_question_mark_stop_at_the_documented_closure_boundary() {
    let _lock = reset();
    let result: Result<(), &str> = eventline::scope_result!("returned error", {
        return Err("explicit return");
    });
    // Reaching this assertion proves `return` did not exit the test function.
    assert_eq!(result, Err("explicit return"));
    assert_eq!(outcomes(scope_id("returned error")), [Outcome::Failure]);

    let result: Result<(), &str> = eventline::scope_result!("parent", {
        eventline::scope_result!("child", { Err("child failed") })?;
        panic!("must not continue after child failure");
    });
    assert_eq!(result, Err("child failed"));
    assert_eq!(outcomes(scope_id("parent")), [Outcome::Failure]);
    assert_eq!(outcomes(scope_id("child")), [Outcome::Failure]);
    let child = eventline::scopes()
        .into_iter()
        .find(|s| s.id == scope_id("child"))
        .unwrap();
    assert_eq!(child.parent, Some(scope_id("parent")));
}

#[test]
fn question_mark_converts_errors_and_returns_borrowed_success_values() {
    #[derive(Debug, PartialEq)]
    struct Error(&'static str);
    impl From<&'static str> for Error {
        fn from(value: &'static str) -> Self {
            Self(value)
        }
    }

    let _lock = reset();
    let result = eventline::scope_result!("converted", {
        Err::<(), _>("read failed")?;
        Ok::<_, Error>(())
    });
    assert_eq!(result, Err(Error("read failed")));
    assert_eq!(outcomes(scope_id("converted")), [Outcome::Failure]);

    let text = String::from("config contents");
    let result = scope_result("borrowed", || Ok::<_, Error>(text.as_str()));
    assert_eq!(result, Ok(text.as_str()));
    assert_eq!(outcomes(scope_id("borrowed")), [Outcome::Success]);
}

#[test]
fn macro_exit_messages_follow_the_selected_outcome_and_evaluate_once() {
    let _lock = reset();
    let evaluations = std::cell::Cell::new(0);
    let result: Result<u32, &str> = eventline::scope_result!(
        {
            evaluations.set(evaluations.get() + 1);
            "messages"
        },
        aborted = "cancelled",
        success = "loaded",
        failure = {
            evaluations.set(evaluations.get() + 1);
            "invalid"
        },
        { Err("invalid config") },
    );
    assert_eq!(result, Err("invalid config"));
    assert_eq!(evaluations.get(), 2);
    let scope = eventline::scopes().pop().unwrap();
    assert_eq!(scope.exit_message_for(Outcome::Failure), Some("invalid"));
    assert_eq!(scope.exit_message_for(Outcome::Success), Some("loaded"));
    assert_eq!(scope.exit_message_for(Outcome::Aborted), Some("cancelled"));
    assert_eq!(outcomes(scope.id), [Outcome::Failure]);

    let result =
        eventline::scope_result!("success message", success = "done", { Ok::<_, &str>(42) });
    assert_eq!(result, Ok(42));
    assert_eq!(outcomes(scope_id("success message")), [Outcome::Success]);
}

#[test]
fn panicking_sync_operation_records_failure_and_restores_its_parent() {
    let _lock = reset();
    let parent = scope_guard("parent");
    let result = std::panic::catch_unwind(|| {
        scope_result("panic", || -> Result<(), &str> { panic!("test panic") })
    });
    assert!(result.is_err());
    eventline::info!("after panic");
    assert_eq!(outcomes(scope_id("panic")), [Outcome::Failure]);
    assert_eq!(event_scope("after panic"), Some(parent.id()));
}

#[test]
fn guard_result_exit_preserves_explicit_abortion_and_is_idempotent() {
    let _lock = reset();
    let mut success = scope_guard("success");
    success.exit_result(&Ok::<_, &str>(42));
    success.exit_result(&Err::<(), _>("later failure"));
    drop(success);
    assert_eq!(outcomes(scope_id("success")), [Outcome::Success]);

    let mut failure = scope_guard("failure");
    failure.exit_result(&Err::<(), _>("failed"));
    failure.exit_result(&Ok::<_, &str>(()));
    drop(failure);
    assert_eq!(outcomes(scope_id("failure")), [Outcome::Failure]);

    let mut aborted = scope_guard("aborted");
    aborted.exit(Outcome::Aborted);
    aborted.exit_result(&Err::<(), _>("cancelled"));
    drop(aborted);
    assert_eq!(outcomes(scope_id("aborted")), [Outcome::Aborted]);
}

#[test]
fn journal_guard_supports_result_outcomes_and_explicit_abortion() {
    let mut journal = eventline::Journal::new();
    {
        let mut guard = journal.scope_guard("failure");
        guard.exit_result(&Err::<(), _>("failed"));
        guard.exit_result(&Ok::<_, &str>(()));
    }
    {
        let mut guard = journal.scope_guard("aborted");
        guard.exit(Outcome::Aborted);
        guard.exit_result(&Ok::<_, &str>(()));
    }
    let outcomes: Vec<_> = journal
        .records()
        .into_iter()
        .map(|r| match r.kind {
            RecordKind::ScopeExit { outcome, .. } => outcome,
            _ => panic!("expected scope exit"),
        })
        .collect();
    assert_eq!(outcomes, [Outcome::Failure, Outcome::Aborted]);
}

#[test]
fn ordinary_scope_macro_keeps_its_existing_general_purpose_behavior() {
    let _lock = reset();
    let result = eventline::scope!("plain", { Err::<(), _>("not inspected") });
    assert_eq!(result, Err("not inspected"));
    assert_eq!(outcomes(scope_id("plain")), [Outcome::Success]);
}

#[test]
fn interleaved_async_results_record_independent_success_and_failure() {
    let _lock = reset();
    let mut success = Box::pin(scope_async_result("success", async {
        eventline::info!("success before");
        yield_once().await;
        eventline::info!("success after");
        Ok::<_, &str>(42)
    }));
    let mut failure = Box::pin(scope_async_result("failure", async {
        eventline::info!("failure before");
        yield_once().await;
        eventline::info!("failure after");
        Err::<(), _>("failed")?;
        Ok(())
    }));
    let parent = scope_guard("poller");
    assert!(poll_once(success.as_mut()).is_pending());
    assert!(poll_once(failure.as_mut()).is_pending());
    assert_eq!(poll_once(failure.as_mut()), Poll::Ready(Err("failed")));
    assert_eq!(poll_once(success.as_mut()), Poll::Ready(Ok(42)));
    eventline::info!("poller after");
    drop((success, failure));

    assert_eq!(outcomes(scope_id("success")), [Outcome::Success]);
    assert_eq!(outcomes(scope_id("failure")), [Outcome::Failure]);
    for name in ["success", "failure"] {
        for timing in ["before", "after"] {
            assert_eq!(
                event_scope(&format!("{name} {timing}")),
                Some(scope_id(name))
            );
        }
    }
    assert_eq!(event_scope("poller after"), Some(parent.id()));
}

#[test]
fn nested_async_errors_propagate_failure_to_both_scopes() {
    let _lock = reset();
    let mut future = Box::pin(scope_async_result("parent", async {
        scope_async_result("child", async {
            yield_once().await;
            Err::<(), _>("read failed")
        })
        .await?;
        Ok(())
    }));
    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(poll_once(future.as_mut()), Poll::Ready(Err("read failed")));
    assert_eq!(outcomes(scope_id("parent")), [Outcome::Failure]);
    assert_eq!(outcomes(scope_id("child")), [Outcome::Failure]);
}

#[test]
fn dropping_pending_result_future_on_another_thread_records_aborted() {
    let _lock = reset();
    let mut future = Box::pin(scope_async_result(
        "cancelled",
        pending::<Result<(), &str>>(),
    ));
    assert!(poll_once(future.as_mut()).is_pending());
    std::thread::spawn(move || {
        let caller = scope_guard("canceller");
        drop(future);
        eventline::info!("after cancellation");
        assert_eq!(event_scope("after cancellation"), Some(caller.id()));
    })
    .join()
    .unwrap();
    assert_eq!(outcomes(scope_id("cancelled")), [Outcome::Aborted]);
}

#[test]
fn panicking_result_future_records_failure_once_and_restores_context() {
    let _lock = reset();
    let mut future = Box::pin(scope_async_result("panic", async {
        yield_once().await;
        panic!("test panic");
        #[allow(unreachable_code)]
        Ok::<(), &str>(())
    }));
    let caller = scope_guard("caller");
    assert!(poll_once(future.as_mut()).is_pending());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| poll_once(future.as_mut())))
            .is_err()
    );
    drop(future);
    eventline::info!("after async panic");
    assert_eq!(outcomes(scope_id("panic")), [Outcome::Failure]);
    assert_eq!(event_scope("after async panic"), Some(caller.id()));
}

#[test]
fn unpolled_result_future_does_not_enter_a_scope() {
    let _lock = reset();
    drop(scope_async_result(
        "unpolled",
        pending::<Result<(), &str>>(),
    ));
    assert!(eventline::scopes().is_empty());
    assert!(eventline::records().is_empty());
}

#[test]
fn async_result_scope_owns_borrowed_names_and_remains_spawnable() {
    fn require_spawnable<F: Future + Send + 'static>(future: F) -> F {
        future
    }

    let _lock = reset();
    let future = {
        let name = String::from("owned name");
        require_spawnable(scope_async_result(name.as_str(), async {
            Err::<(), _>("failed")
        }))
    };
    let mut future = Box::pin(future);
    assert_eq!(poll_once(future.as_mut()), Poll::Ready(Err("failed")));
    assert_eq!(outcomes(scope_id("owned name")), [Outcome::Failure]);
}

#[test]
fn async_result_scope_supports_borrowed_non_send_values() {
    let _lock = reset();
    let value = std::rc::Rc::new(String::from("local"));
    let mut future = Box::pin(scope_async_result("local", async {
        yield_once().await;
        Ok::<_, &str>(value.as_str())
    }));
    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(poll_once(future.as_mut()), Poll::Ready(Ok("local")));
    assert_eq!(outcomes(scope_id("local")), [Outcome::Success]);
}

#[test]
fn application_specific_async_cancellation_can_be_marked_aborted_explicitly() {
    let _lock = reset();
    let mut future = Box::pin(ScopeContext::new().wrap(async {
        let guard = scope_guard("cancelled explicitly");
        yield_once().await;
        guard.aborted();
        Err::<(), _>("cancellation requested")
    }));
    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(
        poll_once(future.as_mut()),
        Poll::Ready(Err("cancellation requested"))
    );
    assert_eq!(
        outcomes(scope_id("cancelled explicitly")),
        [Outcome::Aborted]
    );
}
