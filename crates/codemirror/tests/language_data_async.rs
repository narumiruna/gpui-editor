use std::{
    future::{Future, poll_fn, ready},
    num::NonZeroUsize,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, TAB_SIZE,
    Transaction,
    commands::CommentTokens,
    language::{INDENT_UNIT, IndentOptions, IndentStyle, get_indentation},
    language_data::{
        CloseBracketData, LANGUAGES, LanguageDescription, NativeAsyncLanguageLoad, NativeLanguage,
        NativeLanguageSupport, attached_language_snapshot,
    },
};
use serde_json::{Value, json};

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
    // The factory runs at load(), before either waiter is polled.
    assert_eq!(calls.load(Ordering::SeqCst), 1);
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
    assert_eq!(calls.load(Ordering::SeqCst), 2);
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
fn synchronous_failure_and_eager_retry_match_the_pinned_factory() {
    let calls = AtomicUsize::new(0);
    let handle = NativeAsyncLanguageLoad::with_support(&LANGUAGES[0], "initial", |_| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("failed synchronously");
        }
        ready(Ok::<_, ()>("ready"))
    });
    let log = |stage| {
        serde_json::json!([
            stage,
            calls.load(Ordering::SeqCst),
            handle.support().as_deref()
        ])
    };
    let before = log("before");
    assert!(catch_unwind(AssertUnwindSafe(|| handle.load())).is_err());
    let failed = log("failed");
    let mut retry = handle.load();
    let requested = log("requested");
    assert!(retry.same_attempt(&handle.load()));
    let joined = log("joined");
    assert!(matches!(poll(&mut retry, Waker::noop()), Poll::Ready(Ok(value)) if *value == "ready"));
    assert!(retry.same_attempt(&handle.load()));
    let actual = serde_json::json!([before, failed, requested, joined, log("loaded")]);
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/language_load_factory.json")).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn a_waiter_can_join_before_the_factory_returns() {
    let entering = Barrier::new(2);
    let release = Barrier::new(2);
    let handle = NativeAsyncLanguageLoad::new(&LANGUAGES[0], |_| {
        entering.wait();
        release.wait();
        ready(Ok::<_, ()>("loaded"))
    });
    std::thread::scope(|scope| {
        let first = scope.spawn(|| handle.load());
        entering.wait();
        let mut waiter = handle.load();
        let wake = Arc::new(WakeCount::default());
        let waker = Waker::from(wake.clone());
        let waiting_before_factory_returns = poll(&mut waiter, &waker).is_pending();
        release.wait();
        assert!(first.join().unwrap().same_attempt(&waiter));
        assert!(waiting_before_factory_returns);
        assert!(wake.0.load(Ordering::SeqCst) > 0);
        assert!(matches!(poll(&mut waiter, &waker), Poll::Ready(Ok(value)) if *value == "loaded"));
    });
}

#[test]
fn a_panicking_factory_does_not_leave_old_waiters_pending_or_block_retry() {
    let entering = Barrier::new(2);
    let release = Barrier::new(2);
    let calls = AtomicUsize::new(0);
    let handle = NativeAsyncLanguageLoad::new(&LANGUAGES[0], |_| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            entering.wait();
            release.wait();
            panic!("factory failure");
        }
        ready(Ok::<_, ()>("retry"))
    });
    std::thread::scope(|scope| {
        let first = scope.spawn(|| catch_unwind(AssertUnwindSafe(|| handle.load())));
        entering.wait();
        let mut waiter = handle.load();
        let wake = Arc::new(WakeCount::default());
        let waker = Waker::from(wake.clone());
        let waiting_before_factory_returns = poll(&mut waiter, &waker).is_pending();
        release.wait();
        assert!(first.join().unwrap().is_err());
        assert!(waiting_before_factory_returns);
        assert!(wake.0.load(Ordering::SeqCst) > 0);
        assert!(catch_unwind(AssertUnwindSafe(|| poll(&mut waiter, &waker))).is_err());
        let mut retry = handle.load();
        assert!(!retry.same_attempt(&waiter));
        assert!(
            matches!(poll(&mut retry, Waker::noop()), Poll::Ready(Ok(value)) if *value == "retry")
        );
    });
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
fn stock_descriptions_share_the_pinned_async_load_identity() {
    let oracle: Value =
        serde_json::from_str(include_str!("fixtures/language_data_catalog_load.json")).unwrap();
    assert_eq!(json!(LANGUAGES.len()), oracle["count"]);
    let load = |future: &mut _, name| match poll(future, Waker::noop()) {
        Poll::Ready(Ok(support)) => support,
        _ => panic!("no native loader for {name}"),
    };
    let mut records = Vec::new();
    let mut distinct = Vec::new();
    for (id, entry) in LANGUAGES.iter().enumerate() {
        let by_name = LanguageDescription::match_name(LANGUAGES, entry.name, false).unwrap();
        let handle = entry.catalog_support_load().unwrap();
        let lookup = by_name.catalog_support_load().unwrap();
        let initial = handle.support().is_some();
        let mut pending = handle.load();
        let mut waiter = lookup.load();
        let support_while_pending = lookup.support().is_some();
        let support = load(&mut pending, entry.name);
        let waited = load(&mut waiter, entry.name);
        let cached = by_name.catalog_support_load().unwrap();
        assert_eq!(Some(support.language()), entry.built_in(), "{}", entry.name);
        assert_eq!(
            json!([
                initial,
                std::ptr::eq(handle.description(), by_name),
                pending.same_attempt(&waiter),
                support_while_pending,
                Arc::ptr_eq(&support, &waited),
                cached
                    .support()
                    .is_some_and(|value| Arc::ptr_eq(&support, &value)),
                cached.load().same_attempt(&pending),
            ]),
            oracle["perDescription"],
            "{}",
            entry.name,
        );
        for (left, (other_attempt, other_support)) in records.iter().enumerate() {
            if entry.source == LANGUAGES[left].source {
                distinct.push(json!([
                    left,
                    id,
                    !pending.same_attempt(other_attempt),
                    !Arc::ptr_eq(&support, other_support),
                ]));
            }
        }
        records.push((pending, support));
    }
    assert_eq!(json!(distinct), oracle["distinct"]);
    let supports: Vec<_> = records.into_iter().map(|(_, support)| support).collect();
    check_pinned_stock_attachments(&supports, &oracle["stockAttachments"]);
    check_pinned_stream_installation(&supports);
}

fn check_pinned_stock_attachments(supports: &[Arc<NativeLanguageSupport>], expected: &Value) {
    let mut counts = [0; 4];
    for (entry, support) in LANGUAGES.iter().zip(supports) {
        let name = entry.name;
        let slot = Compartment::new();
        let extension = support
            .extension()
            .unwrap_or_else(|| panic!("{name}: no bundle"));
        let mut state = EditorState::with_extensions("x\n", slot.of(extension)).unwrap();
        let root =
            attached_language_snapshot(&state).unwrap_or_else(|| panic!("{name}: no native tree"));
        let direct = support.parse(state.text().clone()).unwrap();
        assert_eq!(
            std::mem::discriminant(root.syntax()),
            std::mem::discriminant(direct.syntax()),
            "{name}: wrong root grammar"
        );
        assert_eq!(root.text(), state.text(), "{name}: initial text");
        counts[0] += 1;
        state
            .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
            .unwrap();
        assert!(
            Arc::ptr_eq(&root, &attached_language_snapshot(&state).unwrap()),
            "{name}: selection replaced tree"
        );
        counts[1] += 1;
        let edit = ChangeSet::single(Change::new(0..1, "y")).unwrap();
        state.update(Transaction::new(edit)).unwrap();
        let edited = attached_language_snapshot(&state).unwrap();
        assert!(
            !Arc::ptr_eq(&root, &edited),
            "{name}: edit reused old snapshot"
        );
        assert_eq!(edited.text().as_str(), "y\n", "{name}: edited bytes");
        assert_eq!(root.text().as_str(), "x\n", "{name}: retained bytes");
        counts[2] += 1;
        state
            .update(Transaction::default().with_effects([slot.reconfigure(Extension::default())]))
            .unwrap();
        assert!(
            attached_language_snapshot(&state).is_none(),
            "{name}: detach kept active tree"
        );
        assert_eq!(
            edited.text().as_str(),
            "y\n",
            "{name}: detach changed retained tree"
        );
        counts[3] += 1;
    }
    assert_eq!(json!(counts), *expected);
}

// Run after checking the untouched global handles, not in a separate test that
// could load the same catalog entries concurrently.
fn check_pinned_stream_installation(supports: &[Arc<NativeLanguageSupport>]) {
    let oracle: Value =
        serde_json::from_str(include_str!("fixtures/language_data_stream_extension.json")).unwrap();
    let index = |value: &Value| usize::try_from(value.as_u64().unwrap()).unwrap();
    let mut stream_entries = Vec::new();
    for (id, support) in supports.iter().enumerate() {
        if !matches!(
            support.language(),
            NativeLanguage::Legacy(_) | NativeLanguage::Asn1
        ) {
            continue;
        }
        let extension = support.extension().unwrap();
        stream_entries.push(id);
        let state = EditorState::with_extensions("x", extension).unwrap();
        let snapshot = attached_language_snapshot(&state)
            .unwrap_or_else(|| panic!("{} did not attach its parser", LANGUAGES[id].name));
        assert_eq!(snapshot.text(), state.text());
        assert_eq!(
            snapshot.language_at(0, Assoc::After),
            Some(support.language())
        );
    }
    assert_eq!(json!(stream_entries), oracle["streamEntries"]);

    let slot = Compartment::new();
    let extension = |ids: &Value| {
        Extension::group(
            ids.as_array()
                .unwrap()
                .iter()
                .map(|id| supports[index(id)].extension().unwrap()),
        )
    };
    let initial = &oracle["initial"];
    let mut state = EditorState::with_extensions(
        initial["source"].as_str().unwrap(),
        Extension::group([
            slot.of(extension(&initial["extensions"])),
            TAB_SIZE.of(NonZeroUsize::new(index(&initial["tabSize"])).unwrap()),
            INDENT_UNIT.of(IndentStyle::parse(initial["indent"].as_str().unwrap()).unwrap()),
        ]),
    )
    .unwrap();
    let retained = attached_language_snapshot(&state).unwrap();
    let mut previous = None;
    for (i, row) in oracle["records"].as_array().unwrap().iter().enumerate() {
        let step = &row["step"];
        if i != 0 {
            let mut transaction = Transaction::default();
            if let Some(edit) = row["edit"].as_array() {
                let range = index(&edit[0])..index(&edit[1]);
                transaction = Transaction::new(
                    ChangeSet::single(Change::new(range, edit[2].as_str().unwrap())).unwrap(),
                );
            }
            if !row["select"].is_null() {
                let at = index(&row["select"]);
                transaction = transaction.with_selection(EditorSelection::single(at, at));
            }
            if !row["extensions"].is_null() {
                transaction =
                    transaction.with_effects([slot.reconfigure(extension(&row["extensions"]))]);
            }
            state.update(transaction).unwrap();
        }
        let at = index(&row["at"]);
        assert_eq!(
            state.text().as_str(),
            row["source"].as_str().unwrap(),
            "{step}"
        );
        assert_eq!(
            state.selection().main().head(),
            index(&row["head"]),
            "{step}"
        );
        let snapshot = attached_language_snapshot(&state);
        let reused = (i != 0).then(|| match (&previous, &snapshot) {
            (Some(before), Some(after)) => Arc::ptr_eq(before, after),
            (None, None) => true,
            _ => false,
        });
        assert_eq!(json!(reused), row["reused"], "{step}");
        let language =
            (!row["active"].is_null()).then(|| supports[index(&row["active"])].language());
        assert_eq!(
            snapshot
                .as_ref()
                .and_then(|doc| doc.language_at(at, Assoc::Before)),
            language,
            "{step}"
        );
        let spans: Vec<_> = snapshot.as_ref().map_or_else(Vec::new, |doc| {
            assert_eq!(doc.text(), state.text());
            doc.highlights(0..state.text().len())
                .unwrap()
                .into_iter()
                .filter_map(|span| {
                    let tag =
                        span.tags.iter().map(|tag| tag.name()).find(|tag| {
                            matches!(*tag, "keyword" | "string" | "number" | "comment")
                        })?;
                    Some(json!([span.range.start, span.range.end, tag]))
                })
                .collect()
        });
        assert_eq!(json!(spans), row["spans"], "{step}");
        assert_eq!(
            json!(get_indentation(&state, at, IndentOptions::default(), None).unwrap()),
            row["indent"],
            "{step}"
        );
        let comments = state.language_data_at::<CommentTokens>("commentTokens", at, Assoc::Before);
        assert_eq!(
            json!(
                comments
                    .iter()
                    .map(|tokens| tokens.line)
                    .collect::<Vec<_>>()
            ),
            row["comments"],
            "{step}"
        );
        let brackets =
            state.language_data_at::<CloseBracketData>("closeBrackets", at, Assoc::Before);
        assert_eq!(
            json!(
                brackets
                    .iter()
                    .map(|data| data.brackets)
                    .collect::<Vec<_>>()
            ),
            row["brackets"],
            "{step}"
        );
        let words = state.language_data_at::<&[&str]>("autocomplete", at, Assoc::Before);
        assert_eq!(
            json!(
                words
                    .iter()
                    .map(|items| items.contains(&"defn"))
                    .collect::<Vec<_>>()
            ),
            row["hasDefn"],
            "{step}"
        );
        let triggers =
            state.language_data_at::<fn(&str) -> bool>("indentOnInput", at, Assoc::Before);
        assert_eq!(
            json!(
                triggers
                    .iter()
                    .map(|trigger| trigger("  }"))
                    .collect::<Vec<_>>()
            ),
            row["indentTrigger"],
            "{step}"
        );
        previous = snapshot;
    }
    assert_eq!(
        retained.text().as_str(),
        initial["source"].as_str().unwrap()
    );
}

#[test]
fn a_copy_of_a_stock_description_has_independent_load_handles() {
    let copied = *LanguageDescription::match_name(LANGUAGES, "Clojure", false).unwrap();
    assert!(copied.catalog_support_load().is_none());
    let first = copied.native_support_load_async();
    let second = copied.native_support_load_async();
    assert!(std::ptr::eq(first.description(), &copied));
    let mut pending = first.load();
    let mut other = second.load();
    assert!(!pending.same_attempt(&other));
    let Poll::Ready(Ok(support)) = poll(&mut pending, Waker::noop()) else {
        panic!("expected loaded support");
    };
    assert!(second.support().is_none());
    let Poll::Ready(Ok(other_support)) = poll(&mut other, Waker::noop()) else {
        panic!("expected loaded support");
    };
    assert!(!Arc::ptr_eq(&support, &other_support));
    assert!(support.extension().is_none());
    assert!(other_support.extension().is_none());
}
