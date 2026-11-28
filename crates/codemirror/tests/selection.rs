//! Expected compatibility results are from the SHA-512-verified npm tarball
//! for @codemirror/state 6.7.6 in docs/codemirror-upstream.json. The external
//! Node oracle converted native byte offsets to UTF-16 and back for edits.

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Assoc, Change, ChangeSet, EditError, EditorSelection, EditorState,
    RangeEdit, SelectionRange, SerializationError, Transaction, TransactionCoordinates,
    collab::{CollabConfig, Update, collab, get_synced_version, receive_updates, sendable_updates},
};
use serde_json::{Value, json};

#[test]
fn selection_matches_the_pinned_oracle() {
    for line in include_str!("fixtures/selection_oracle.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        match case["op"].as_str().unwrap() {
            "range" => {
                let range = input_range(&case["input"]);
                assert_eq!(snapshot(range), case["range"], "{line}");
                assert_eq!(range.to_json(), case["wire"], "{line}");
                let mut wire = range.to_json();
                wire["assoc"] = json!(1);
                wire["bidiLevel"] = json!(2);
                wire["goalColumn"] = json!(24);
                wire["undirectional"] = json!(true);
                assert_eq!(
                    snapshot(SelectionRange::from_json(&wire).unwrap()),
                    case["decoded"],
                    "{line}"
                );
            }
            "eq" => {
                let a = input_range(&case["a"]);
                let b = input_range(&case["b"]);
                assert_eq!(json!(a == b), case["eq"], "{line}");
                assert_eq!(json!(a.eq_with_assoc(&b)), case["with_assoc"], "{line}");
                let wrap = |range| {
                    EditorSelection::new(vec![range, SelectionRange::new(9, 9)], 0).unwrap()
                };
                let (left, right) = (wrap(a), wrap(b));
                assert_eq!(json!(left == right), case["eq"], "{line}");
                assert_eq!(
                    json!(left.eq_with_assoc(&right)),
                    case["with_assoc"],
                    "{line}"
                );
                let other_main = EditorSelection::new(left.ranges().to_vec(), 1).unwrap();
                assert_ne!(left, other_main);
                assert!(!left.eq_with_assoc(&other_main));
                assert!(!left.eq_with_assoc(&left.as_single()));
            }
            "extend" => {
                let range = input_range(&case["a"]).extend(
                    position(&case["from"]),
                    position(&case["to"]),
                    association(&case["assoc"]),
                );
                assert_eq!(snapshot(range), case["range"], "{line}");
            }
            "normalize" => {
                let selection = input_selection(&case);
                assert_eq!(selection_snapshot(&selection), case["selection"], "{line}");
                assert_eq!(
                    selection_snapshot(&selection.as_single()),
                    case["single"],
                    "{line}"
                );
            }
            "update" => {
                let selection = input_selection(&case);
                let inserted = input_range(&case["inserted"]);
                let overlaps = input_range(&case["overlaps"]);
                assert_eq!(
                    selection_snapshot(&selection.add_range(inserted, true)),
                    case["new_main"]
                );
                assert_eq!(
                    selection_snapshot(&selection.add_range(inserted, false)),
                    case["kept_main"]
                );
                assert_eq!(
                    selection_snapshot(&selection.replace_range(1, overlaps).unwrap()),
                    case["replaced"]
                );
                assert_eq!(selection.to_json(), case["wire"]);
                let restored = EditorSelection::from_json(&selection.to_json()).unwrap();
                assert_eq!(selection_snapshot(&restored), case["decoded"]);
                assert_eq!(
                    selection.replace_range(2, overlaps),
                    Err(EditError::InvalidSelection)
                );
            }
            "state" => {
                let doc = case["doc"].as_str().unwrap();
                let mut state =
                    EditorState::with_extensions(doc, ALLOW_MULTIPLE_SELECTIONS.of(false)).unwrap();
                state
                    .update(Transaction::default().with_selection(input_selection(&case)))
                    .unwrap();
                assert_eq!(selection_snapshot(state.selection()), case["selection"]);
                assert_eq!(state.to_json(), case["wire"]);
                let restored = EditorState::from_json(&state.to_json()).unwrap();
                assert_eq!(restored.text().as_str(), doc);
                assert_eq!(selection_snapshot(restored.selection()), case["decoded"]);
            }
            "map" => check_map(&case),
            op => panic!("Unknown fixture operation {op}"),
        }
    }
}

fn check_map(case: &Value) {
    let doc = case["doc"].as_str().unwrap();
    let selection = input_selection(case);
    let changes = fixture_changes(case);
    let desc = changes.desc(doc.len()).unwrap();
    let default = selection.map_desc(&desc).unwrap();
    let after = selection.map_desc_with_assoc(&desc, Assoc::After).unwrap();
    assert_eq!(selection_snapshot(&default), case["mapped"], "{case}");
    assert_eq!(selection_snapshot(&after), case["after"], "{case}");
    let mapped_ranges: Vec<_> = selection
        .ranges()
        .iter()
        .map(|range| snapshot(range.map(&desc).unwrap()))
        .collect();
    let after_ranges: Vec<_> = selection
        .ranges()
        .iter()
        .map(|range| snapshot(range.map_with_assoc(&desc, Assoc::After).unwrap()))
        .collect();
    assert_eq!(json!(mapped_ranges), case["mapped_ranges"], "{case}");
    assert_eq!(json!(after_ranges), case["after_ranges"], "{case}");

    let mut state = EditorState::with_extensions(doc, ALLOW_MULTIPLE_SELECTIONS.of(true)).unwrap();
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    state.update(Transaction::new(changes)).unwrap();
    assert_eq!(state.text().as_str(), case["next"].as_str().unwrap());
    assert_eq!(
        selection_snapshot(state.selection()),
        case["mapped"],
        "{case}"
    );
}

#[test]
fn crossing_matches_pinned_endpoints_and_keeps_native_selection_invariants() {
    for line in include_str!("fixtures/selection_crossing.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        match case["op"].as_str().unwrap() {
            "map" => check_crossing_map(&case),
            "remap" => check_remap(&case),
            "api" => check_transaction_mapping(&case),
            "collab" => check_remote_replacement(&case),
            "history" => (), // Covered by history_mapping.rs.
            op => panic!("Unknown fixture operation {op}"),
        }
    }
}

fn check_crossing_map(case: &Value) {
    let doc = case["doc"].as_str().unwrap();
    let changes = fixture_changes(case);
    let desc = changes.desc(doc.len()).unwrap();
    let selection = input_selection(case);
    for (range, (bare, after)) in selection
        .ranges()
        .iter()
        .zip(items(&case["bare"]).zip(items(&case["afterBare"])))
    {
        for (mapped, expected) in [
            (range.map(&desc).unwrap(), bare),
            (range.map_with_assoc(&desc, Assoc::After).unwrap(), after),
        ] {
            // Preserve mapped endpoint roles, but never expose reversed bounds.
            let (from, to) = (position(&expected[2]), position(&expected[3]));
            assert_eq!(mapped.range(), from.min(to)..from.max(to), "{case}");
            assert_eq!(snapshot(mapped), pinned_range(expected), "{case}");
        }
    }
    let expected = |key| {
        // CodeMirror normalizes by the raw bounds. Rust merges coincident
        // spans and the cursor at an upper bound. The main index follows the
        // merged span or shifts if the primary range is after it.
        let name = case["name"].as_str().unwrap();
        if name.starts_with("coincident-spans-") || name == "touching-span-and-cursor" {
            let first = if name == "touching-span-and-cursor" {
                2
            } else {
                1
            };
            let main = if case["main"] == 3 { 2 } else { 1 };
            return json!({"main": main, "ranges": [
                [0, first, -1, null, null, false],
                [2, 5, -1, null, null, false],
                [6, 7, -1, null, null, false]
            ]});
        }
        pinned_selection(&case[key])
    };
    assert_eq!(
        selection_snapshot(&selection.map_desc(&desc).unwrap()),
        expected("mapped"),
        "{case}"
    );
    assert_eq!(
        selection_snapshot(&selection.map_desc_with_assoc(&desc, Assoc::After).unwrap()),
        expected("after"),
        "{case}"
    );
    let mut state = EditorState::new(doc);
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    state.update(Transaction::new(changes)).unwrap();
    assert_eq!(
        state.text().as_str(),
        case["next"].as_str().unwrap(),
        "{case}"
    );
    assert_eq!(
        selection_snapshot(state.selection()),
        expected("mapped"),
        "{case}"
    );
}

fn check_remap(case: &Value) {
    let replace = ChangeSet::single(Change::new(2..8, "XYZ"))
        .unwrap()
        .desc(10)
        .unwrap();
    let insert = ChangeSet::new(vec![Change::new(2..2, "<"), Change::new(5..5, ">")])
        .unwrap()
        .desc(7)
        .unwrap();
    let selection =
        EditorSelection::single(position(&case["initial"][0]), position(&case["initial"][1]))
            .map_desc(&replace)
            .unwrap();
    assert_eq!(selection.to_json(), case["json"]);
    // Subsequent edits map the ordered span inward, like CodeMirror after
    // JSON decoding. Native mapping keeps the stored association, unlike JSON.
    let decoded = EditorSelection::from_json(&case["json"]).unwrap();
    let mapped = selection.map_desc(&insert).unwrap();
    let ordered = decoded.map_desc(&insert).unwrap();
    assert_eq!(
        selection_snapshot(&ordered),
        pinned_selection(&case["ordered"])
    );
    assert_eq!(mapped, ordered);
    assert_eq!(mapped.main().assoc(), selection.main().assoc());
    assert_ne!(selection_snapshot(&mapped), pinned_selection(&case["raw"]));
}

fn check_transaction_mapping(case: &Value) {
    let mut state = EditorState::new("0123456789");
    let initial = EditorSelection::single(3, 6);
    state
        .update(Transaction::default().with_selection(initial.clone()))
        .unwrap();
    let changes = ChangeSet::single(Change::new(2..8, "XYZ")).unwrap();
    let transaction = match case["name"].as_str().unwrap() {
        "merged-explicit-selection" => Transaction::default()
            .with_selection(initial)
            .merge(
                Transaction::new(changes),
                10,
                TransactionCoordinates::Original,
            )
            .unwrap(),
        "same-spec-explicit-selection" => {
            Transaction::new(changes).with_selection(EditorSelection::single(1, 2))
        }
        "same-text-replacement" => {
            let changes = ChangeSet::single(Change::new(2..8, "234567")).unwrap();
            state.update(Transaction::new(changes)).unwrap();
            // Ordinary state updates still discard same-text replacements.
            assert_eq!(state.selection(), &initial);
            return;
        }
        "change-by-range" => {
            let two = EditorSelection::new(
                vec![SelectionRange::new(1, 1), SelectionRange::new(9, 9)],
                1,
            )
            .unwrap();
            state
                .update(Transaction::default().with_selection(two))
                .unwrap();
            state
                .change_by_range(|range| {
                    let (changes, selection) = if range.head() == 1 {
                        (ChangeSet::default(), SelectionRange::new(3, 6))
                    } else {
                        (changes.clone(), SelectionRange::new(7, 7))
                    };
                    Ok(RangeEdit {
                        changes,
                        selection,
                        effects: Vec::new(),
                    })
                })
                .unwrap()
        }
        name => panic!("Unknown transaction case {name}"),
    };
    state.update(transaction).unwrap();
    assert_eq!(
        selection_snapshot(state.selection()),
        pinned_selection(&case["selection"])
    );
}

fn check_remote_replacement(case: &Value) {
    let mut state =
        EditorState::with_extensions("0123456789", collab(CollabConfig::new("local", 0))).unwrap();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(3, 6)))
        .unwrap();
    let changes = ChangeSet::single(Change::new(2..8, case["insert"].as_str().unwrap())).unwrap();
    let remote = Update::new("peer", changes, 10).unwrap();
    state
        .update(receive_updates(&state, 0, &[remote]).unwrap())
        .unwrap();
    assert_eq!(state.text().as_str(), case["doc"].as_str().unwrap());
    assert_eq!(
        selection_snapshot(state.selection()),
        pinned_selection(&case["selection"])
    );
    assert_eq!(
        get_synced_version(&state).unwrap(),
        position(&case["version"])
    );
    assert_eq!(
        sendable_updates(&state).unwrap().len(),
        position(&case["queued"])
    );
}

fn pinned_range(range: &Value) -> Value {
    json!([range[0], range[1], range[4], range[5], range[6], range[7]])
}

fn pinned_selection(selection: &Value) -> Value {
    json!({"main": selection["main"], "ranges": items(&selection["ranges"]).map(pinned_range).collect::<Vec<_>>()})
}

#[test]
fn native_goal_columns_and_range_json_validation() {
    for column in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let range = EditorSelection::cursor(0, None, None, Some(column));
        assert_eq!(range.goal_column(), None);
        assert_eq!(range, SelectionRange::new(0, 0));
    }
    let zeros = [0.0, -0.0].map(|goal| EditorSelection::cursor(0, None, None, Some(goal)));
    assert_eq!(zeros[0], zeros[1]);
    for invalid in [
        json!(null),
        json!({"anchor": 0}),
        json!({"anchor": -1, "head": 0}),
        json!({"anchor": 0, "head": 1.25}),
    ] {
        assert_eq!(
            SelectionRange::from_json(&invalid),
            Err(SerializationError::InvalidFormat("selection range"))
        );
    }
}

fn fixture_changes(case: &Value) -> ChangeSet {
    ChangeSet::new(
        items(&case["changes"])
            .map(|edit| {
                Change::new(
                    position(&edit[0])..position(&edit[1]),
                    edit[2].as_str().unwrap(),
                )
            })
            .collect(),
    )
    .unwrap()
}

fn items(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().unwrap().iter()
}

fn position(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn association(value: &Value) -> Option<Assoc> {
    match value.as_i64().unwrap() {
        -1 => Some(Assoc::Before),
        0 => None,
        1 => Some(Assoc::After),
        value => panic!("Unexpected association {value}"),
    }
}

fn input_range(input: &Value) -> SelectionRange {
    let (anchor, head) = (position(&input[0]), position(&input[1]));
    let assoc = association(&input[2]);
    let bidi = input[3].as_u64().map(|value| u8::try_from(value).unwrap());
    let goal = input[4].as_f64();
    if input[5].as_bool().unwrap() {
        EditorSelection::undirectional_range(anchor, head)
    } else if anchor == head {
        EditorSelection::cursor(head, assoc, bidi, goal)
    } else {
        EditorSelection::range(anchor, head, goal, bidi, assoc)
    }
}

fn input_selection(case: &Value) -> EditorSelection {
    EditorSelection::new(
        items(&case["ranges"]).map(input_range).collect(),
        position(&case["main"]),
    )
    .unwrap()
}

fn snapshot(range: SelectionRange) -> Value {
    let assoc = match range.assoc() {
        Some(Assoc::Before) => -1,
        None => 0,
        Some(Assoc::After) => 1,
    };
    json!([
        range.anchor(),
        range.head(),
        assoc,
        range.bidi_level(),
        range.goal_column(),
        range.undirectional()
    ])
}

fn selection_snapshot(selection: &EditorSelection) -> Value {
    json!({"main": selection.main_index(), "ranges": selection.ranges().iter().copied().map(snapshot).collect::<Vec<_>>()})
}
