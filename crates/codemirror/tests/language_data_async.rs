use std::{
    future::{Future, poll_fn, ready},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use gpui_editor_codemirror::language_data::{
    LANGUAGES, LanguageDescription, NativeAsyncLanguageLoad,
};

#[derive(Default)]
struct Gate {
    result: Option<Result<&'static str, &'static str>>,
    wake: Option<Waker>,
}

impl Gate {
    fn finish(gate: &Mutex<Self>, result: Result<&'static str, &'static str>) {
        let wake = {
            let mut gate = gate.lock().unwrap();
            gate.result = Some(result);
            gate.wake.take().unwrap()
        };
        wake.wake();
    }
}

#[derive(Default)]
struct WakeCount(AtomicUsize);

impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn poll<F: Future + Unpin>(future: &mut F, waker: &Waker) -> Poll<F::Output> {
    Pin::new(future).poll(&mut Context::from_waker(waker))
}

#[test]
fn pending_waiters_share_a_result_and_a_failed_attempt_can_retry() {
    check_shared_load(None);
}

#[test]
fn preloaded_support_survives_failure_until_a_shared_retry_succeeds() {
    // @codemirror/language 6.13.1 accepts both support and load in
    // LanguageDescription.of. See fixtures/language_data_oracle.mjs.
    check_shared_load(Some("initial"));
}

fn check_shared_load(initial_support: Option<&'static str>) {
    let calls = AtomicUsize::new(0);
    let first_gate = Mutex::new(Gate::default());
    let second_gate = Mutex::new(Gate::default());
    let loader = |_: &LanguageDescription| {
        let gate = if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            &first_gate
        } else {
            &second_gate
        };
        poll_fn(move |cx| {
            let mut gate = gate.lock().unwrap();
            if let Some(result) = gate.result.take() {
                Poll::Ready(result)
            } else {
                gate.wake = Some(cx.waker().clone());
                Poll::Pending
            }
        })
    };
    let handle = match initial_support {
        Some(support) => NativeAsyncLanguageLoad::with_support(&LANGUAGES[0], support, loader),
        None => NativeAsyncLanguageLoad::new(&LANGUAGES[0], loader),
    };
    let initial = handle.support();
    assert_eq!(initial.as_deref().copied(), initial_support);
    let check_initial_support = || match (&initial, handle.support()) {
        (Some(initial), Some(current)) => assert!(Arc::ptr_eq(initial, &current)),
        (None, None) => (),
        _ => panic!("support changed before a successful load"),
    };
    let mut first = handle.load();
    let mut old_waiter = handle.load();
    assert!(first.same_attempt(&old_waiter));
    check_initial_support();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let a = Arc::new(WakeCount::default());
    let b = Arc::new(WakeCount::default());
    let a_waker = Waker::from(a.clone());
    let b_waker = Waker::from(b.clone());
    assert!(poll(&mut first, &a_waker).is_pending());
    assert!(poll(&mut old_waiter, &b_waker).is_pending());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    check_initial_support();
    Gate::finish(&first_gate, Err("first"));
    assert!(a.0.load(Ordering::SeqCst) > 0);
    assert!(b.0.load(Ordering::SeqCst) > 0);
    let Poll::Ready(Err(failed)) = poll(&mut first, &a_waker) else {
        panic!("expected failure")
    };
    assert_eq!(*failed, "first");
    check_initial_support();

    let mut retry = handle.load();
    assert!(!retry.same_attempt(&first));
    assert!(poll(&mut retry, &a_waker).is_pending());
    check_initial_support();
    let Poll::Ready(Err(same_error)) = poll(&mut old_waiter, &b_waker) else {
        panic!("expected the original failure")
    };
    assert!(Arc::ptr_eq(&failed, &same_error));
    let mut late_waiter = handle.load();
    assert!(late_waiter.same_attempt(&retry));
    Gate::finish(&second_gate, Ok("ready"));
    let Poll::Ready(Ok(result)) = poll(&mut late_waiter, &b_waker) else {
        panic!("expected support")
    };
    let Poll::Ready(Ok(same_result)) = poll(&mut retry, &a_waker) else {
        panic!("expected same support")
    };
    assert_eq!(*result, "ready");
    assert!(Arc::ptr_eq(&result, &same_result));
    assert!(Arc::ptr_eq(&result, &handle.support().unwrap()));
    assert!(handle.load().same_attempt(&retry));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn preloaded_support_is_immediate_and_descriptions_are_independent() {
    let preloaded = NativeAsyncLanguageLoad::<_, ()>::from_support(&LANGUAGES[0], "ready");
    let loader = NativeAsyncLanguageLoad::new(&LANGUAGES[0], |_| ready(Ok::<_, ()>("ready")));
    let cached = preloaded.support().unwrap();
    assert!(loader.support().is_none());
    let mut a = preloaded.load();
    let mut b = loader.load();
    assert!(!a.same_attempt(&b));
    let Poll::Ready(Ok(a)) = poll(&mut a, Waker::noop()) else {
        panic!("expected preloaded support")
    };
    let Poll::Ready(Ok(b)) = poll(&mut b, Waker::noop()) else {
        panic!("expected loaded support")
    };
    assert!(Arc::ptr_eq(&cached, &a));
    assert!(!Arc::ptr_eq(&a, &b));
}

#[test]
fn every_pinned_catalog_entry_has_an_async_native_load() {
    assert_eq!(LANGUAGES.len(), 143);
    for entry in LANGUAGES {
        let handle = entry.native_support_load_async();
        assert!(std::ptr::eq(entry, handle.description()));
        let Poll::Ready(Ok(support)) = poll(&mut handle.load(), Waker::noop()) else {
            panic!("no native loader for {}", entry.name);
        };
        assert!(Arc::ptr_eq(&support, &handle.support().unwrap()));
    }
}
