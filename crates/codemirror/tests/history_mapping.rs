use std::ops::Range;

use gpui_editor_codemirror::{
    Assoc, Change, ChangeDesc, ChangeSet, EditError, EditorSelection, EditorState, Extension,
    History, HistoryIsolation, MapMode, SelectionRange, StateEffectType, StateField, Transaction,
    inverted_effects, transaction_filter,
};
use serde_json::{Value, json};

fn edit(range: Range<usize>, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, insert)).unwrap())
}

fn local(range: Range<usize>, insert: &str) -> Transaction {
    edit(range, insert).isolate_history(HistoryIsolation::Full)
}

fn remote(range: Range<usize>, insert: &str) -> Transaction {
    edit(range, insert).with_add_to_history(false)
}

fn apply(history: &mut History<()>, transaction: Transaction) {
    history.update(transaction, (), |_| (None, ())).unwrap();
}

fn source<M: Clone>(history: &History<M>) -> &str {
    history.state().text().as_str()
}

#[test]
fn native_redo_keeps_the_host_selection_with_or_without_mapping() {
    for (after, insert_before, expected_text, expected_caret) in
        [(0, false, "aX", 0), (2, true, "aBX", 3)]
    {
        let mut history = History::new(EditorState::new("a"));
        history
            .set_selection(EditorSelection::single(1, 1))
            .unwrap();
        history
            .update(local(1..1, "X"), "before", |_| {
                (Some(EditorSelection::single(after, after)), "after")
            })
            .unwrap();
        if insert_before {
            history
                .update(remote(1..1, "B"), "", |_| (None, ""))
                .unwrap();
        }
        assert_eq!(history.undo(), Some("before"));
        assert_eq!(source(&history), if insert_before { "aB" } else { "a" });
        assert_eq!(history.state().selection(), &EditorSelection::single(1, 1));
        assert_eq!(history.redo(), Some("after"));
        assert_eq!(source(&history), expected_text);
        assert_eq!(
            history.state().selection(),
            &EditorSelection::single(expected_caret, expected_caret)
        );
    }
}

#[test]
fn undo_and_redo_branches_both_map_through_received_replacements() {
    let mut history = History::new(EditorState::new("abc"));
    history
        .set_selection(EditorSelection::single(1, 1))
        .unwrap();
    apply(
        &mut history,
        local(1..1, "X").with_selection(EditorSelection::single(2, 2)),
    );
    apply(&mut history, local(4..4, "Y"));
    assert_eq!(history.undo(), Some(()));
    assert_eq!(source(&history), "aXbc");
    apply(&mut history, remote(0..0, "P"));
    apply(&mut history, remote(3..4, "β"));
    assert_eq!(source(&history), "PaXβc");
    assert!(history.can_undo() && history.can_redo());

    assert_eq!(history.undo(), Some(()));
    assert_eq!(source(&history), "Paβc");
    assert_eq!(history.state().selection().main().head(), 2);
    assert_eq!(history.undo(), None);
    assert_eq!(history.redo(), Some(()));
    assert_eq!(source(&history), "PaXβc");
    assert_eq!(history.state().selection().main().head(), 3);
    assert_eq!(history.redo(), Some(()));
    assert_eq!(source(&history), "PaXβcY");
    assert_eq!(history.state().selection().main().head(), 3);
    assert_eq!(history.redo(), None);
}

#[test]
fn covering_replacements_map_saved_edit_and_selection_history() {
    let bytes = |value: &Value| usize::try_from(value.as_u64().unwrap()).unwrap();
    let selection = |pair: &Value| EditorSelection::single(bytes(&pair[0]), bytes(&pair[1]));
    for line in include_str!("fixtures/selection_crossing.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        if case["op"] != "history" {
            continue;
        }
        let mut history = History::new(EditorState::new(case["source"].as_str().unwrap()));
        history.set_selection(selection(&case["initial"])).unwrap();
        for (index, (step, expected)) in case["steps"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["snapshots"].as_array().unwrap())
            .enumerate()
        {
            let op = step[0].as_str().unwrap();
            match op {
                "local" | "remote" => {
                    let change = &step[1][0];
                    let range = bytes(&change[0])..bytes(&change[1]);
                    let text = change[2].as_str().unwrap();
                    let tx = if op == "local" {
                        local(range, text)
                    } else {
                        remote(range, text)
                    };
                    apply(&mut history, tx);
                }
                "select" => apply(
                    &mut history,
                    Transaction::default()
                        .with_selection(selection(&step[1]))
                        .with_user_event("select.pointer")
                        .isolate_history(HistoryIsolation::Full),
                ),
                "undo" => assert_eq!(history.undo(), Some(())),
                "redo" => assert_eq!(history.redo(), Some(())),
                "undoSelection" => assert_eq!(history.undo_selection(()), Some(())),
                "redoSelection" => assert_eq!(history.redo_selection(()), Some(())),
                _ => panic!("Unknown history step {op}"),
            }
            assert_eq!(
                source(&history),
                expected["doc"].as_str().unwrap(),
                "{line}, step {index}"
            );
            assert_eq!(
                history.undo_depth(),
                bytes(&expected["undo"]),
                "{line}, step {index}"
            );
            assert_eq!(
                history.redo_depth(),
                bytes(&expected["redo"]),
                "{line}, step {index}"
            );
            let pinned = &expected["selection"]["ranges"][0];
            let expected_selection = if case["name"] == "local-crossed-undo-redo" && index == 3 {
                // The second upstream undo infers (9, 1) from its reversed
                // bounds. Native history keeps the original saved selection.
                selection(&case["initial"])
            } else {
                selection(pinned)
            };
            let actual = history.state().selection();
            assert_eq!(actual, &expected_selection, "{line}, step {index}");
            let assoc = match actual.main().assoc() {
                Some(Assoc::Before) => -1,
                Some(Assoc::After) => 1,
                None => 0,
            };
            assert_eq!(json!(assoc), pinned[4], "{line}, step {index}");
        }
    }
}

#[test]
fn overlapping_unrecorded_deletions_can_drop_multiple_undo_entries() {
    let mut history = History::new(EditorState::new(""));
    apply(&mut history, local(0..0, "hi"));
    apply(&mut history, local(2..2, "hello"));
    apply(&mut history, remote(0..7, ""));
    assert_eq!(source(&history), "");
    assert!(!history.can_undo());
    assert_eq!(history.undo(), None);
    assert_eq!(history.redo(), None);

    apply(&mut history, local(0..0, "old"));
    apply(&mut history, local(3..3, "new"));
    apply(&mut history, remote(3..6, "NEW"));
    assert_eq!(history.undo(), Some(()));
    assert_eq!(source(&history), "NEW");
    assert_eq!(history.undo(), None);
    assert_eq!(history.redo(), Some(()));
    assert_eq!(source(&history), "oldNEW");
    assert_eq!(history.redo(), None);
}

#[test]
fn overlapping_unrecorded_replacement_can_drop_the_redo_entry() {
    let mut history = History::new(EditorState::new("abc"));
    apply(&mut history, local(0..3, ""));
    assert_eq!(history.undo(), Some(()));
    assert_eq!(source(&history), "abc");
    apply(&mut history, remote(0..3, "XYZ"));
    assert_eq!(source(&history), "XYZ");
    assert!(!history.can_redo());
    assert_eq!(history.redo(), None);
}

#[test]
fn undo_and_redo_keep_an_insertion_between_adjacent_tracked_replacements() {
    let mut history = History::new(EditorState::new("123"));
    let changes = ChangeSet::new(vec![
        Change::new(0..1, "ab"),
        Change::new(1..2, "cd"),
        Change::new(2..3, "ef"),
    ])
    .unwrap();
    apply(&mut history, Transaction::new(changes));
    apply(&mut history, remote(2..2, "!!!!!!!!"));
    for _ in 0..3 {
        assert_eq!(history.undo(), Some(()));
        assert_eq!(source(&history), "1!!!!!!!!23");
        assert_eq!(history.redo(), Some(()));
        assert_eq!(source(&history), "ab!!!!!!!!cdef");
    }
}

#[test]
fn deferred_mappings_reach_lower_entries_across_repeated_replay() {
    let mut history = History::new(EditorState::new(""));
    apply(&mut history, edit(0..0, "one").with_time(1000));
    apply(&mut history, edit(3..3, " two").with_time(1001));
    apply(&mut history, remote(7..7, "xxx"));
    apply(&mut history, local(10..10, " three"));
    apply(&mut history, local(0..0, "zero "));
    apply(&mut history, edit(0..0, "\n\n").with_time(1002));
    apply(&mut history, edit(0..0, "top").with_time(1003));
    apply(&mut history, remote(0..0, "yyy"));
    for _ in 0..3 {
        for expected in [
            "yyyzero one twoxxx three",
            "yyyone twoxxx three",
            "yyyone twoxxx",
            "yyyxxx",
        ] {
            assert_eq!(history.undo(), Some(()));
            assert_eq!(source(&history), expected);
        }
        assert_eq!(history.undo(), None);
        for expected in [
            "yyyone twoxxx",
            "yyyone twoxxx three",
            "yyyzero one twoxxx three",
            "yyytop\n\nzero one twoxxx three",
        ] {
            assert_eq!(history.redo(), Some(()));
            assert_eq!(source(&history), expected);
        }
        assert_eq!(history.redo(), None);
    }
}

#[test]
fn maps_old_multiple_cursors_and_keeps_the_primary_range() {
    let mut history = History::new(EditorState::new("abc"));
    history
        .set_selection(
            EditorSelection::new((0..3).map(|i| SelectionRange::new(i, i)).collect(), 1).unwrap(),
        )
        .unwrap();
    apply(&mut history, local(0..3, "d"));
    let changes = ChangeSet::new(vec![Change::new(0..0, "x"), Change::new(1..1, "y")]).unwrap();
    apply(
        &mut history,
        Transaction::new(changes).with_add_to_history(false),
    );
    assert_eq!(history.undo(), Some(()));
    assert_eq!(source(&history), "xabcy");
    let selection = history.state().selection();
    assert_eq!(selection.main_index(), 1);
    assert_eq!(
        selection
            .ranges()
            .iter()
            .map(|range| range.head())
            .collect::<Vec<_>>(),
        [0, 2, 3]
    );
}

#[test]
fn maps_utf8_selection_snapshots_without_replacing_host_metadata() {
    let mut history = History::new(EditorState::new("hi"));
    history
        .set_selection(EditorSelection::single(2, 0))
        .unwrap();
    assert_eq!(
        history.update(local(0..0, "hello"), "before", |_| {
            (Some(EditorSelection::single(7, 5)), "after")
        }),
        Ok("after")
    );
    assert_eq!(
        history.update(remote(0..0, "🦊"), "remote-before", |_| (
            None,
            "remote-after"
        )),
        Ok("remote-after")
    );
    assert_eq!(history.undo(), Some("before"));
    assert_eq!(source(&history), "🦊hi");
    assert_eq!(history.state().selection(), &EditorSelection::single(6, 4));
    assert_eq!(history.redo(), Some("after"));
    assert_eq!(source(&history), "🦊hellohi");
    assert_eq!(history.state().selection(), &EditorSelection::single(11, 9));
}

#[test]
fn mapping_uses_the_accepted_transaction_and_invalid_finishes_leave_both_branches_intact() {
    let filter = transaction_filter(|_, transaction| {
        Ok(if transaction.is_user_event("receive") {
            remote(0..0, "🦊")
        } else {
            transaction
        })
    });
    let mut history = History::new(EditorState::with_extensions("", filter).unwrap());
    apply(&mut history, local(0..0, "a"));
    apply(&mut history, local(1..1, "b"));
    assert_eq!(history.undo(), Some(()));
    assert_eq!(
        history.update(
            Transaction::default().with_user_event("receive"),
            (),
            |_| { (Some(EditorSelection::single(1, 1)), ()) }
        ),
        Err(EditError::InvalidSelection)
    );
    assert_eq!(source(&history), "a");
    apply(
        &mut history,
        Transaction::default().with_user_event("receive"),
    );
    assert_eq!(source(&history), "🦊a");
    assert_eq!(history.undo(), Some(()));
    assert_eq!(source(&history), "🦊");
    assert_eq!(history.redo(), Some(()));
    assert_eq!(history.redo(), Some(()));
    assert_eq!(source(&history), "🦊ab");
}

fn map_range(range: &Range<usize>, mapping: &ChangeDesc) -> Option<Range<usize>> {
    let from = mapping.map_pos(range.start, Assoc::After, MapMode::Simple)?;
    let to = mapping.map_pos(range.end, Assoc::Before, MapMode::Simple)?;
    (from < to).then_some(from..to)
}

#[test]
fn inverts_and_maps_effects_across_remote_edits_and_replay() {
    let add = StateEffectType::with_mapping(map_range);
    let remove = StateEffectType::with_mapping(map_range);
    let ranges = StateField::<Vec<Range<usize>>>::new(|_| Ok(Vec::new()), {
        let (add, remove) = (add.clone(), remove.clone());
        move |value, update| {
            let mut ranges = value
                .iter()
                .filter_map(|range| map_range(range, update.changes()))
                .collect::<Vec<_>>();
            for effect in update.transaction().effects() {
                if let Some(range) = effect.get(&add) {
                    ranges.push(range.clone());
                } else if let Some(range) = effect.get(&remove) {
                    ranges.retain(|existing| existing != range);
                }
            }
            ranges.sort_by_key(|range| range.start);
            Ok(ranges)
        }
    });
    let invert = inverted_effects({
        let (add, remove, ranges) = (add.clone(), remove.clone(), ranges.clone());
        move |start, transaction, changes| {
            let inverse = changes.inverted();
            let mut effects = transaction
                .effects()
                .iter()
                .filter_map(|effect| {
                    if let Some(range) = effect.get(&add) {
                        map_range(range, &inverse).map(|range| remove.of(range))
                    } else {
                        map_range(effect.get(&remove)?, &inverse).map(|range| add.of(range))
                    }
                })
                .collect::<Vec<_>>();
            for range in &*start.field(&ranges).unwrap().unwrap() {
                if map_range(range, changes).is_none() {
                    effects.push(add.of(range.clone()));
                }
            }
            effects
        }
    });
    let assert_ranges = |history: &History<()>, expected: &[Range<usize>]| {
        assert_eq!(
            &**history
                .state()
                .context()
                .field(&ranges)
                .unwrap()
                .as_ref()
                .unwrap(),
            expected
        );
    };
    let extensions = Extension::group([ranges.extension(), invert]);
    let mut history =
        History::new(EditorState::with_extensions("one two foo", extensions.clone()).unwrap());
    apply(
        &mut history,
        Transaction::default().with_effects([add.of(0..3)]),
    );
    apply(
        &mut history,
        local(3..4, "---").with_effects([add.of(6..9)]),
    );
    apply(&mut history, remote(0..0, "---"));
    assert_ranges(&history, &[3..6, 9..12]);
    assert_eq!(history.undo(), Some(()));
    assert_eq!(source(&history), "---one two foo");
    assert_ranges(&history, std::slice::from_ref(&(3..6)));
    assert_eq!(history.undo(), Some(()));
    assert_ranges(&history, &[]);
    assert_eq!(history.redo(), Some(()));
    assert_ranges(&history, std::slice::from_ref(&(3..6)));
    assert_eq!(history.redo(), Some(()));
    assert_eq!(source(&history), "---one---two foo");
    assert_ranges(&history, &[3..6, 9..12]);

    let mut history =
        History::new(EditorState::with_extensions("123456", extensions.clone()).unwrap());
    apply(
        &mut history,
        Transaction::default().with_effects([add.of(3..5)]),
    );
    apply(&mut history, local(2..6, ""));
    assert_ranges(&history, &[]);
    assert_eq!(history.undo(), Some(()));
    assert_ranges(&history, std::slice::from_ref(&(3..5)));
    assert_eq!(history.redo(), Some(()));
    assert_ranges(&history, &[]);
    assert_eq!(history.undo(), Some(()));
    apply(&mut history, remote(2..6, ""));
    assert_ranges(&history, &[]);
    assert!(!history.can_undo() && !history.can_redo());

    let mut history = History::new(EditorState::with_extensions("abcdef", extensions).unwrap());
    apply(
        &mut history,
        edit(0..0, "x").with_time(1000).with_effects([add.of(3..4)]),
    );
    apply(
        &mut history,
        edit(0..0, "y").with_time(1001).with_effects([add.of(5..6)]),
    );
    apply(&mut history, remote(0..0, "!"));
    assert_ranges(&history, &[5..6, 6..7]);
    assert_eq!(history.undo(), Some(()));
    assert_eq!(source(&history), "!abcdef");
    assert_ranges(&history, &[]);
    assert_eq!(history.undo(), None);
    assert_eq!(history.redo(), Some(()));
    assert_eq!(source(&history), "!yxabcdef");
    assert_ranges(&history, &[5..6, 6..7]);
}
