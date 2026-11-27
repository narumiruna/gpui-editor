use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, History,
    HistoryIsolation, READ_ONLY, Transaction, transaction_filter,
    view::{EditorView, SCROLL_INTO_VIEW, ScrollTarget},
};
use serde_json::{Value, json};

type Log = Arc<Mutex<Vec<Value>>>;

fn number(value: &Value) -> Option<usize> {
    value.as_u64().map(|n| n.try_into().unwrap())
}

fn policy(values: &Value, log: &Log) -> Extension {
    Extension::group(values.as_array().unwrap().iter().map(|value| {
        let settings =
            EditorState::history_config(number(&value["depth"]), value["delay"].as_u64());
        if let Some(mode) = value["predicate"].as_str() {
            let mode = mode.to_owned();
            let log = log.clone();
            Extension::group([
                settings,
                EditorState::history_join_to_event(move |tx, adjacent| {
                    log.lock()
                        .unwrap()
                        .push(json!([mode, tx.user_event(), adjacent]));
                    match mode.as_str() {
                        "reject" => false,
                        "delete" => tx.is_user_event("delete"),
                        "adjacent" => adjacent,
                        _ => panic!("invalid predicate {mode}"),
                    }
                }),
            ])
        } else {
            settings
        }
    }))
}

fn apply(history: &mut History<()>, transaction: Transaction) {
    history.update(transaction, (), |_| (None, ())).unwrap();
}

fn changes(from: usize, to: usize, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(from..to, insert)).unwrap())
}

fn document_length(history: &History<()>) -> usize {
    history.state().text().len()
}

fn step(history: &mut History<()>, compartment: &Compartment, step: &Value, log: &Log) -> Value {
    let op = step["op"].as_str().unwrap();
    let transaction = match op {
        "edit" => {
            let from = number(&step["from"]).unwrap_or_else(|| document_length(history));
            let to = number(&step["to"]).unwrap_or(from);
            changes(from, to, step["insert"].as_str().unwrap())
        }
        "select" => {
            let pos = number(&step["pos"]).unwrap();
            Transaction::default().with_selection(EditorSelection::single(pos, pos))
        }
        "config" => Transaction::default()
            .with_effects([compartment.reconfigure(policy(&step["configs"], log))]),
        "fill" => {
            for index in 0..number(&step["count"]).unwrap() {
                let position = document_length(history);
                apply(
                    history,
                    changes(position, position, &(index % 10).to_string())
                        .with_user_event("input.paste")
                        .with_time(99999),
                );
            }
            return Value::Null;
        }
        _ => {
            let handled = (0..number(&step["count"]).unwrap())
                .filter(|_| {
                    match op {
                        "undo" => history.undo_command(),
                        "redo" => history.redo_command(),
                        "undoSelection" => history.undo_selection(()),
                        "redoSelection" => history.redo_selection(()),
                        _ => panic!("invalid operation {op}"),
                    }
                    .is_some()
                })
                .count();
            return json!(handled);
        }
    };
    let mut transaction = transaction.with_time(step["time"].as_u64().unwrap_or(99999));
    if let Some(event) = step["event"].as_str() {
        transaction = transaction.with_user_event(event);
    }
    if step["ignore"].as_bool() == Some(true) {
        transaction = transaction.with_add_to_history(false);
    }
    if let Some(isolation) = step["isolate"].as_str() {
        let isolation = match isolation {
            "before" => HistoryIsolation::Before,
            "after" => HistoryIsolation::After,
            "full" => HistoryIsolation::Full,
            _ => panic!("invalid isolation {isolation}"),
        };
        transaction =
            transaction.with_annotation(Transaction::history_isolation_annotation().of(isolation));
    }
    apply(history, transaction);
    Value::Null
}

#[test]
fn configured_history_matches_the_pinned_commands_package() {
    // Captured from the npm tarballs for commands 6.11.2 and state 6.7.6.
    // Each step records the document, cursor, depths, selection availability,
    // number of successful commands, raw branch sizes and join predicate calls.
    for row in include_str!("fixtures/history_config.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let log = Log::default();
        let compartment = Compartment::new();
        let state = EditorState::with_extensions(
            row["doc"].as_str().unwrap(),
            Extension::group([
                compartment.of(policy(&row["configs"], &log)),
                READ_ONLY.of(row["readonly"].as_bool().unwrap_or(false)),
            ]),
        )
        .unwrap();
        let mut history = History::new(state);
        let steps = row["steps"].as_array().unwrap();
        let results = row["result"].as_array().unwrap();
        assert_eq!(steps.len(), results.len());
        for (step_number, (operation, expected)) in steps.iter().zip(results).enumerate() {
            let handled = step(&mut history, &compartment, operation, &log);
            let branch = |events: &Value| {
                let events = events.as_array().unwrap();
                let base = events
                    .first()
                    .filter(|first| first.get("changes").is_none())
                    .map(|selection| selection["selectionsAfter"].as_array().unwrap().len());
                json!([events.len(), base])
            };
            let stored = history.to_json().unwrap();
            let log = std::mem::take(&mut *log.lock().unwrap());
            let actual = json!([
                history.state().text().as_str(),
                history.state().selection().main().head(),
                history.undo_depth(),
                history.redo_depth(),
                history.can_undo_selection(),
                history.can_redo_selection(),
                handled,
                branch(&stored["done"]),
                branch(&stored["undone"]),
                log,
            ]);
            assert_eq!(&actual, expected, "{} at step {step_number}", row["name"]);
        }
    }
}

#[test]
fn view_policy_uses_filtered_changes_and_keeps_native_selection_and_scroll_reports() {
    let target = ScrollTarget::at(7);
    let filter = transaction_filter(move |_, tx| {
        if tx.is_user_event("replace-me") {
            Ok(Transaction::new(
                ChangeSet::new(vec![Change::new(1..1, "字"), Change::new(3..4, "d")]).unwrap(),
            )
            .with_user_event("input.type.filtered")
            .with_time(1001)
            .with_scroll_into_view()
            .with_effects([SCROLL_INTO_VIEW.of(target)]))
        } else {
            Ok(tx)
        }
    });
    let join = EditorState::history_join_to_event(move |tx, adjacent| {
        assert!(!adjacent);
        assert_eq!(tx.user_event(), Some("input.type.filtered"));
        assert_eq!(
            tx.changes().changes().collect::<Vec<_>>(),
            [(&(1..1), "字")]
        );
        assert!(tx.scroll_into_view());
        assert_eq!(tx.effects()[0].get(&SCROLL_INTO_VIEW), Some(&target));
        true
    });
    let state = EditorState::with_extensions("abcd", Extension::group([filter, join])).unwrap();
    let mut view = EditorView::new(state);
    view.update(changes(4, 4, "X").with_time(1000), "before", |_| {
        (None, "first")
    })
    .unwrap();
    let selection = EditorSelection::new(
        vec![EditorSelection::cursor(
            4,
            Some(Assoc::After),
            Some(2),
            Some(8.0),
        )],
        0,
    )
    .unwrap();
    view.update(
        Transaction::default().with_user_event("replace-me"),
        "first",
        |_| (Some(selection), "second"),
    )
    .unwrap();
    assert_eq!(view.undo_depth(), 1);
    assert_eq!(view.take_scroll_target(), Some(target));
    assert_eq!(view.undo(), Some("before"));
    assert_eq!(view.state().text().as_str(), "abcd");
    assert_eq!(view.take_scroll_target(), Some(ScrollTarget::at(0)));
    assert_eq!(view.redo(), Some("second"));
    assert_eq!(view.state().text().as_str(), "a字bcdX");
    let replayed = view.state().selection().main();
    assert_eq!(replayed.assoc(), Some(Assoc::After));
    assert_eq!(replayed.bidi_level(), Some(2));
    assert_eq!(replayed.goal_column(), Some(8.0));
    assert_eq!(view.take_scroll_target().unwrap().range.head(), 4);
}

#[test]
fn raw_read_only_replay_still_works_and_the_native_clock_never_reads_wall_time() {
    let state = EditorState::with_extensions("abc", READ_ONLY.of(true)).unwrap();
    let mut history = History::new(state);
    apply(&mut history, changes(1, 1, "x"));
    assert_eq!(history.undo_command(), None);
    assert_eq!(history.undo(), Some(()));
    assert_eq!(history.redo_command(), None);
    assert_eq!(history.redo(), Some(()));

    let state =
        EditorState::with_extensions("abc", EditorState::history_config(None, Some(500))).unwrap();
    let mut history = History::new(state);
    apply(&mut history, changes(1, 1, "x"));
    apply(&mut history, changes(2, 2, "y"));
    assert_eq!(history.undo_depth(), 2);
}
