use std::{
    cell::Cell,
    collections::BTreeMap,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangePermission, ChangeSet, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, MapMode, Precedence, StateEffectType, StateField, Transaction,
    TransactionExtension, change_filter, transaction_extender, transaction_filter,
    view::{
        EditorView, ViewPlugin, ViewPluginType, ViewUpdate, Viewport,
        input::{
            InputProposal, InputStep, headless_input_handler, input_completion_handler,
            input_handler, plan_input,
        },
        update_listener,
    },
};
use serde_json::{Value, json};

static MARKER: LazyLock<StateEffectType<usize>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|pos, changes| {
        changes.map_pos(*pos, Assoc::After, MapMode::Simple)
    })
});
static EXTENSION_EVENT: LazyLock<StateEffectType<String>> = LazyLock::new(StateEffectType::new);

type Log = Arc<Mutex<BTreeMap<&'static str, Vec<Value>>>>;

fn push(log: &Log, key: &'static str, value: Value) {
    log.lock().unwrap().entry(key).or_default().push(value);
}

fn snapshot(state: &EditorState) -> Value {
    json!({"doc": state.text().as_str(), "head": state.selection().main().head()})
}

fn step(step: &InputStep) -> Value {
    let tr = &step.transaction;
    json!({
        "start": snapshot(&step.start_state), "end": snapshot(&step.state),
        "changes": tr.changes().changes().map(|(r, text)| json!([r.start, r.end, text])).collect::<Vec<_>>(),
        "event": tr.user_event(),
        "markers": tr.effects().iter().filter_map(|effect| effect.get(&MARKER)).collect::<Vec<_>>(),
        "extensions": tr.effects().iter().filter_map(|effect| effect.get(&EXTENSION_EVENT)).collect::<Vec<_>>(),
    })
}

fn observe(log: &Log, source: &str, update: &ViewUpdate<'_>) {
    if update.input_steps.is_empty() {
        return;
    }
    let changes: Vec<_> = update
        .changes
        .changed_ranges()
        .map(|(old, new)| json!([old.start, old.end, update.state.text().slice(new).unwrap()]))
        .collect();
    let steps = update.input_steps.iter().map(step).collect::<Vec<_>>();
    push(
        log,
        "observers",
        json!({
            "source": source, "start": snapshot(update.start_state), "end": snapshot(update.state),
            "changes": changes, "selection": update.selection_set, "steps": steps,
        }),
    );
    if source == "listener" {
        push(log, "dispatches", json!(steps));
        let markers = update
            .effects
            .iter()
            .filter_map(|effect| effect.get(&MARKER))
            .copied()
            .collect::<Vec<_>>();
        let expected: &[usize] = match update.input_steps.len() {
            1 => &[4],
            2 if update.state.text().as_str().starts_with('!') => &[6, 5],
            2 => &[4, 4],
            _ => unreachable!(),
        };
        assert_eq!(markers, expected);
    }
}

struct Observer(Log);

impl ViewPlugin for Observer {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        observe(&self.0, "plugin", update);
    }
}

fn single(range: std::ops::Range<usize>, text: &str) -> ChangeSet {
    ChangeSet::single(Change::new(range, text)).unwrap()
}

fn proposed_text(state: &EditorState, transaction: &Transaction) -> String {
    let mut text = state.text().as_str().to_owned();
    let edits = transaction.changes().changes().collect::<Vec<_>>();
    for (range, insert) in edits.into_iter().rev() {
        text.replace_range(range.clone(), insert);
    }
    text
}

fn input_event(tr: &Transaction) -> Option<&str> {
    tr.user_event()
        .filter(|event| matches!(*event, "input.type" | "input.complete"))
}

#[test]
fn headless_input_matches_pinned_chromium_transport() {
    for row in include_str!("fixtures/view_input_completion.jsonl").lines() {
        let expected: Value = serde_json::from_str(row).unwrap();
        let pass = expected["mode"] == "pass";
        let filtered = expected["mode"] == "filtered";
        let trace = Log::default();
        let identity = Arc::new(AtomicUsize::new(0));
        let log = trace.clone();
        let saved = identity.clone();
        let first = input_completion_handler(move |change, default| {
            let range = change.range();
            push(
                &log,
                "handlers",
                json!(["peek", range.start, range.end, change.text()]),
            );
            let step = default()?;
            assert!(std::ptr::eq(step, default()?));
            assert_eq!(step.state.text().as_str(), "aXYZb");
            saved.store(std::ptr::from_ref(step).addr(), Ordering::Relaxed);
            Ok(None)
        })
        .with_precedence(Precedence::High);
        let log = trace.clone();
        let second = input_completion_handler(move |change, default| {
            let range = change.range();
            push(
                &log,
                "handlers",
                json!(["complete", range.start, range.end, change.text()]),
            );
            let step = default()?;
            assert_eq!(
                identity.load(Ordering::Relaxed),
                std::ptr::from_ref(step).addr()
            );
            if pass {
                return Ok(None);
            }
            let at = step.state.selection().main().head();
            let changes = ChangeSet::new(vec![Change::new(0..0, "!"), Change::new(at..at, ")")])?;
            Ok(Some(
                Transaction::new(changes)
                    .with_selection(EditorSelection::single(at + 1, at + 1))
                    .with_effects([MARKER.of(at + 1)])
                    .with_time(2000),
            ))
        });
        let replacement_calls = Arc::new(AtomicUsize::new(0));
        let calls = replacement_calls.clone();
        let replacement = headless_input_handler(move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(None)
        });
        let log = trace.clone();
        let last = input_completion_handler(move |change, _| {
            assert!(pass, "handler ran after completion");
            let range = change.range();
            push(
                &log,
                "handlers",
                json!(["last", range.start, range.end, change.text()]),
            );
            Ok(None)
        })
        .with_precedence(Precedence::Low);
        let filter_changes = change_filter(move |_, tr| {
            if filtered && tr.is_user_event("input.complete") {
                ChangePermission::Reject
            } else {
                ChangePermission::Allow
            }
        });
        let log = trace.clone();
        let filter_transactions = transaction_filter(move |start, tr| {
            let Some(event) = input_event(&tr) else {
                return Ok(tr);
            };
            push(
                &log,
                "filters",
                json!([event, start.text().as_str(), proposed_text(start, &tr)]),
            );
            if event != "input.type" {
                return Ok(tr);
            }
            let from = start.selection().main().head();
            Ok(Transaction::new(single(from..from, "XYZ"))
                .with_selection(EditorSelection::single(from + 3, from + 3))
                .with_user_event(event)
                .with_time(tr.time().unwrap())
                .with_scroll_into_view()
                .with_effects([MARKER.of(from + 3)]))
        });
        let log = trace.clone();
        let extender = transaction_extender(move |start, tr| {
            let Some(event) = input_event(tr) else {
                return TransactionExtension::default();
            };
            push(
                &log,
                "extenders",
                json!([event, start.text().as_str(), proposed_text(start, tr)]),
            );
            TransactionExtension {
                effects: vec![EXTENSION_EVENT.of(event.to_owned())],
                ..Default::default()
            }
        });
        let log = trace.clone();
        let field = StateField::new(
            |_| Ok(0),
            move |value, update| {
                if let Some(event) = input_event(update.transaction()) {
                    push(
                        &log,
                        "fields",
                        json!([
                            event,
                            update.start().text().as_str(),
                            update.state().text().as_str()
                        ]),
                    );
                }
                Ok(*value + 1)
            },
        );
        let log = trace.clone();
        let listener = update_listener(move |update| {
            observe(&log, "listener", update);
            Ok(())
        });
        let plugin = ViewPluginType::define(|_, trace: &Log| Ok(Observer(trace.clone())));
        let state = EditorState::create(
            "ab",
            Some(EditorSelection::single(1, 1)),
            Extension::group([
                EditorState::history_config(None, None),
                second,
                first,
                replacement,
                last,
                filter_changes,
                filter_transactions,
                extender,
                field.extension(),
                plugin.of(trace.clone()),
                listener,
            ]),
        )
        .unwrap();
        let mut view = EditorView::new(state);
        let builds = Cell::new(0);
        view.apply_headless_input(
            1..1,
            "x",
            |state| {
                builds.set(builds.get() + 1);
                Ok(state
                    .replace_selection("x")?
                    .with_user_event("input.type")
                    .with_time(2000)
                    .with_scroll_into_view())
            },
            (),
        )
        .unwrap();
        assert_eq!(builds.get(), 1);
        assert_eq!(replacement_calls.load(Ordering::Relaxed), usize::from(pass));
        assert_eq!(snapshot(view.state()), expected["end"]);
        assert_eq!(json!(view.undo_depth()), expected["undoDepth"]);
        assert_eq!(
            json!(*view.state().context().field(&field).unwrap().unwrap()),
            json!(expected["fields"].as_array().unwrap().len())
        );
        assert_eq!(view.revision(), 1);
        assert_eq!(view.viewport().interval(), 0..view.state().text().len());
        let actual = trace.lock().unwrap();
        for key in [
            "handlers",
            "filters",
            "extenders",
            "fields",
            "dispatches",
            "observers",
        ] {
            assert_eq!(
                json!(actual[key]),
                expected[key],
                "{} {key}",
                expected["mode"]
            );
        }
        drop(actual);
        let mut undo = vec![];
        while view.undo().is_some() {
            let mut next = snapshot(view.state());
            next["depth"] = json!(view.undo_depth());
            undo.push(next);
        }
        assert_eq!(json!(undo), expected["undo"]);
        let mut redo = vec![];
        while view.redo().is_some() {
            let mut next = snapshot(view.state());
            next["depth"] = json!(view.redo_depth());
            redo.push(next);
        }
        assert_eq!(json!(redo), expected["redo"]);
    }
}

#[test]
fn adjacent_headless_types_join_without_changing_saved_native_replay() {
    let mut view = EditorView::<()>::new(EditorState::new("a"));
    view.set_selection(EditorSelection::single(1, 1)).unwrap();
    for (at, text) in [(1, "b"), (2, "c")] {
        view.apply_headless_input(
            at..at,
            text,
            |state| {
                Ok(state
                    .replace_selection(text)?
                    .with_user_event("input.type")
                    .with_time(1000 + at as u64))
            },
            (),
        )
        .unwrap();
    }
    assert_eq!(view.undo_depth(), 1);
    assert_eq!(snapshot(view.state()), json!({"doc": "abc", "head": 3}));

    let insert = ChangeSet::single(Change::new(3..3, ")")).unwrap();
    view.update(
        Transaction::new(insert)
            .with_user_event("input.type")
            .with_time(1003)
            .with_selection(EditorSelection::single(3, 3)),
        (),
        |_| (None, ()),
    )
    .unwrap();
    assert_eq!(view.undo_depth(), 2);
    assert_eq!(view.undo(), Some(()));
    assert_eq!(snapshot(view.state()), json!({"doc": "abc", "head": 3}));
    assert_eq!(view.undo(), Some(()));
    assert_eq!(snapshot(view.state()), json!({"doc": "a", "head": 1}));
    assert_eq!(view.redo(), Some(()));
    assert_eq!(snapshot(view.state()), json!({"doc": "abc", "head": 3}));
    assert_eq!(view.redo(), Some(()));
    assert_eq!(snapshot(view.state()), json!({"doc": "abc)", "head": 3}));
}

#[test]
fn replacement_skips_the_default_even_when_filtered() {
    for reject in [false, true] {
        let replacement = headless_input_handler(|change| {
            assert_eq!(change.range(), 1..1);
            assert_eq!(change.text(), "x");
            assert_eq!(change.state().text().as_str(), "ab");
            Ok(Some(
                Transaction::new(single(change.range(), "yz"))
                    .with_user_event("input.replace")
                    .with_selection(EditorSelection::single(2, 2))
                    .with_scroll_into_view(),
            ))
        })
        .with_precedence(Precedence::High);
        let observed = Arc::new(AtomicUsize::new(0));
        let calls = observed.clone();
        let state = EditorState::create(
            "ab",
            Some(EditorSelection::single(1, 1)),
            Extension::group([
                input_completion_handler(|_, _| panic!("skipped completion")),
                replacement,
                change_filter(move |state, tr| {
                    if tr.is_user_event("input.replace") {
                        assert_eq!(state.text().as_str(), "ab");
                        assert_eq!(proposed_text(state, tr), "ayzb");
                        if reject {
                            return ChangePermission::Reject;
                        }
                    }
                    ChangePermission::Allow
                }),
                update_listener(move |update| {
                    if let [step] = update.input_steps {
                        assert_eq!(step.start_state.text().as_str(), "ab");
                        assert_eq!(step.transaction.user_event(), Some("input.replace"));
                        assert_eq!(step.transaction.changes().is_empty(), reject);
                        assert!(step.transaction.scroll_into_view());
                        assert_eq!(update.doc_changed(), !reject);
                        calls.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(())
                }),
            ]),
        )
        .unwrap();
        let planned = plan_input(&state, InputProposal::ReplaceSelection("x"))
            .unwrap()
            .unwrap();
        assert_eq!(proposed_text(&state, &planned), "axb");

        let mut view = EditorView::new(state);
        view.apply_headless_input(1..1, "x", |_| panic!("skipped default"), "replace")
            .unwrap();
        let head = if reject { 1 } else { 2 };
        assert_eq!(
            snapshot(view.state()),
            json!({"doc": if reject { "ab" } else { "ayzb" }, "head": head})
        );
        assert_eq!(view.take_scroll_target().unwrap().range.head(), head);
        assert_eq!(observed.load(Ordering::Relaxed), 1);
        assert_eq!(view.undo_depth(), usize::from(!reject));
        if !reject {
            assert_eq!(view.undo(), Some("replace"));
            assert_eq!(view.state().text().as_str(), "ab");
        }
    }
}

#[test]
fn replacement_uses_original_state_after_completion_peeks() {
    for (default_error, replacement_error) in [(false, false), (true, false), (false, true)] {
        let peek = input_completion_handler(move |_, default| {
            if default_error {
                assert_eq!(default().unwrap_err(), EditError::InvalidSelection);
                assert_eq!(default().unwrap_err(), EditError::InvalidSelection);
            } else {
                let step = default()?;
                assert_eq!(step.state.text().as_str(), "");
                assert_eq!(step.transaction.effects().len(), 1);
                assert!(std::ptr::eq(step, default()?));
            }
            Ok(None)
        })
        .with_precedence(Precedence::High);
        let replacement = headless_input_handler(move |change| {
            assert_eq!(change.state().text().as_str(), "aéb!");
            let range = if replacement_error { 2..2 } else { 4..5 };
            Ok(Some(Transaction::new(single(range, "z"))))
        });
        let observed = Arc::new(AtomicUsize::new(0));
        let calls = observed.clone();
        let state = EditorState::with_extensions(
            "aéb",
            Extension::group([
                replacement,
                peek,
                update_listener(move |update| {
                    if let [step] = update.input_steps {
                        assert_eq!(step.start_state.text().as_str(), "aéb!");
                        assert_eq!(step.state.text().as_str(), "aébz");
                        assert_eq!(step.transaction.user_event(), None);
                        assert!(update.effects.is_empty());
                        calls.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(())
                }),
            ]),
        )
        .unwrap();
        let mut view = EditorView::new(state);
        view.update(Transaction::new(single(4..4, "!")), "older", |_| {
            (None, "before")
        })
        .unwrap();
        view.update(Transaction::new(single(5..5, "?")), "before", |_| {
            (None, "after")
        })
        .unwrap();
        assert_eq!(view.undo(), Some("before"));
        let revision = view.revision();
        let builds = Cell::new(0);
        let result = view.apply_headless_input(
            4..4,
            "x",
            |_| {
                builds.set(builds.get() + 1);
                if default_error {
                    return Err(EditError::InvalidSelection);
                }
                Ok(Transaction::new(single(0..5, "")).with_effects([MARKER.of(0)]))
            },
            "replace",
        );
        assert_eq!(builds.get(), 1);
        if replacement_error {
            assert_eq!(result, Err(EditError::InvalidRange));
            assert_eq!(observed.load(Ordering::Relaxed), 0);
            assert_eq!(view.revision(), revision);
            assert_eq!(view.state().text().as_str(), "aéb!");
            assert_eq!((view.undo_depth(), view.redo_depth()), (1, 1));
            assert_eq!(view.redo(), Some("after"));
            assert_eq!(view.state().text().as_str(), "aéb!?");
        } else {
            result.unwrap();
            assert_eq!(observed.load(Ordering::Relaxed), 1);
            assert_eq!(view.revision(), revision + 1);
            assert_eq!(view.state().text().as_str(), "aébz");
            assert_eq!((view.undo_depth(), view.redo_depth()), (2, 0));
            assert_eq!(view.undo(), Some("replace"));
            assert_eq!(view.state().text().as_str(), "aéb!");
        }
    }
}

#[test]
fn invalid_completion_does_not_publish_or_change_either_history_branch() {
    for eager_failure in [false, true] {
        let callbacks = Arc::new(AtomicUsize::new(0));
        let called = callbacks.clone();
        let field = StateField::new(
            |_| Ok(0),
            move |value, update| {
                if input_event(update.transaction()).is_some() {
                    called.fetch_add(1, Ordering::Relaxed);
                }
                if update.transaction().is_user_event("input.complete") {
                    return Err(ConfigurationError::InvalidFieldData);
                }
                Ok(*value + 1)
            },
        );
        let handler = input_completion_handler(move |_, default| {
            let base = default()?;
            assert_eq!(base.state.text().as_str(), "péx!");
            let at = if eager_failure { 3 } else { 2 };
            Ok(Some(Transaction::new(single(at..at, "!"))))
        });
        let observed = Arc::new(AtomicUsize::new(0));
        let notifications = observed.clone();
        let state = EditorState::create(
            "éx",
            Some(EditorSelection::single(2, 2)),
            Extension::group([
                handler,
                field.extension(),
                update_listener(move |_| {
                    notifications.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                }),
            ]),
        )
        .unwrap();
        let mut view = EditorView::new(state);
        view.update(Transaction::new(single(3..3, "!")), "older", |_| {
            (None, "before")
        })
        .unwrap();
        view.update(Transaction::new(single(4..4, "?")), "before", |_| {
            (None, "after")
        })
        .unwrap();
        assert_eq!(view.undo(), Some("before"));
        view.set_viewport_geometry(
            Viewport::new(view.state().text(), 0..2, std::iter::once(0..2)).unwrap(),
        )
        .unwrap();
        observed.store(0, Ordering::Relaxed);
        let revision = view.revision();
        let viewport = view.viewport().clone();
        let previous = snapshot(view.state());
        let field_value = view.state().context().field(&field).unwrap().unwrap();
        let result = view.apply_headless_input(
            0..0,
            "p",
            |_| Ok(Transaction::new(single(0..0, "p")).with_user_event("input.type")),
            "input",
        );
        assert_eq!(
            result,
            Err(if eager_failure {
                ConfigurationError::InvalidFieldData.into()
            } else {
                EditError::InvalidRange
            })
        );
        assert_eq!(
            callbacks.load(Ordering::Relaxed),
            if eager_failure { 2 } else { 1 }
        );
        assert_eq!(snapshot(view.state()), previous);
        assert_eq!(
            view.state().context().field(&field).unwrap().unwrap(),
            field_value
        );
        assert_eq!((view.undo_depth(), view.redo_depth()), (1, 1));
        assert_eq!(view.revision(), revision);
        assert_eq!(view.viewport(), &viewport);
        assert_eq!(view.take_scroll_target().unwrap().range.head(), 2);
        assert_eq!(observed.load(Ordering::Relaxed), 0);
        assert_eq!(view.redo(), Some("after"));
        assert_eq!(view.state().text().as_str(), "éx!?");
        assert_eq!(view.undo(), Some("before"));
        assert_eq!(view.undo(), Some("older"));
        assert_eq!(view.state().text().as_str(), "éx");
    }
}

#[test]
fn cancelling_steps_keep_the_original_geometry_and_map_the_default_scroll_target() {
    let handler = input_completion_handler(|_, _| {
        Ok(Some(
            Transaction::new(single(1..2, "")).with_selection(EditorSelection::single(0, 0)),
        ))
    });
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let state = EditorState::with_extensions(
        "ab",
        Extension::group([
            handler,
            update_listener(move |update| {
                assert_eq!(update.input_steps.len(), 2);
                assert!(!update.doc_changed());
                assert!(!update.viewport_changed);
                assert!(update.selection_set);
                observed.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }),
        ]),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    view.apply_headless_input(
        1..1,
        "x",
        |_| {
            Ok(Transaction::new(single(1..1, "x"))
                .with_selection(EditorSelection::single(2, 2))
                .with_scroll_into_view()
                .with_user_event("input.type"))
        },
        (),
    )
    .unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert_eq!(view.revision(), 0);
    assert_eq!(view.state().text().as_str(), "ab");
    assert_eq!(view.state().selection().main().head(), 0);
    assert_eq!(view.take_scroll_target().unwrap().range.head(), 1);
    assert_eq!(view.undo_depth(), 2);
}

#[test]
fn unfiltered_host_default_still_extends_on_the_no_handler_path() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let state = EditorState::with_extensions(
        "",
        Extension::group([
            input_handler(|_, _| panic!("single-edit handler")),
            transaction_filter(|_, _| panic!("bypassed filter")),
            transaction_extender(move |_, _| {
                counted.fetch_add(1, Ordering::Relaxed);
                TransactionExtension::default()
            }),
        ]),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    view.apply_headless_input(
        0..0,
        "x",
        |_| Ok(Transaction::new(single(0..0, "x")).without_filtering()),
        (),
    )
    .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(view.state().text().as_str(), "x");
    assert_eq!(view.undo_depth(), 1);
}
