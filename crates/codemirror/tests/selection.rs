//! Expected compatibility results are from the SHA-512-verified npm tarball
//! for @codemirror/state 6.7.6 in docs/codemirror-upstream.json. The external
//! Node oracle converted native byte offsets to UTF-16 and back for edits.

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Assoc, Change, ChangeSet, EditError, EditorSelection, EditorState,
    SelectionRange, SerializationError, Transaction,
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
    let changes = ChangeSet::new(
        items(&case["changes"])
            .map(|edit| {
                Change::new(
                    position(&edit[0])..position(&edit[1]),
                    edit[2].as_str().unwrap(),
                )
            })
            .collect(),
    )
    .unwrap();
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
fn native_mapping_collapses_crossed_endpoints() {
    // The JS map reports crossed from/to for this edit. Native mapping has
    // always collapsed at the originally upper endpoint, mapped with Before.
    let doc = "aé🦊Ωz";
    let changes = ChangeSet::single(Change::new(1..9, "漢")).unwrap();
    let desc = changes.desc(doc.len()).unwrap();
    for (anchor, head) in [(3, 7), (7, 3)] {
        let range = EditorSelection::range(anchor, head, Some(9.5), Some(4), Some(Assoc::After));
        let expected = json!([1, 1, 1, 4, 9.5, false]);
        assert_eq!(snapshot(range.map(&desc).unwrap()), expected);
        let mut state = EditorState::new(doc);
        let selection = EditorSelection::new(vec![range], 0).unwrap();
        state
            .update(Transaction::default().with_selection(selection))
            .unwrap();
        state.update(Transaction::new(changes.clone())).unwrap();
        assert_eq!(state.text().as_str(), "a漢z");
        assert_eq!(snapshot(state.selection().main()), expected);
    }
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
