use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, ChangeSpec, EditError, EditorSelection, EditorState, LINE_SEPARATOR,
    MapMode, SelectionRange, Text,
};
use serde_json::{Value, json};

#[test]
fn state_change_specs_match_pinned_application_ranges_and_mapping() {
    for row in include_str!("fixtures/state_change_specs.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        if row["inputs"].is_array() {
            continue;
        }
        let name = row["name"].as_str().unwrap();
        let state = EditorState::new(row["source"].as_str().unwrap());
        let changes = match name {
            "nested specs keep group order" => state.changes_from_specs(&[
                ChangeSpec::Group(&[
                    ChangeSpec::String(2..4, "X"),
                    ChangeSpec::Group(&[ChangeSpec::String(0..3, "Y")]),
                ]),
                ChangeSpec::Group(&[]),
                ChangeSpec::String(2..2, "Z"),
            ]),
            "prebuilt sets and literal text" => {
                let set = ChangeSet::single(Change::new(2..4, "X\rY")).unwrap();
                let literal = Text::new("M\rN");
                state.changes_from_specs(&[
                    ChangeSpec::String(1..3, "a\rb"),
                    ChangeSpec::Group(&[ChangeSpec::Set(&set)]),
                    ChangeSpec::Text(5..5, &literal),
                    ChangeSpec::String(0..2, "P"),
                ])
            }
            _ => {
                let specs: Vec<(usize, usize, String)> =
                    serde_json::from_value(row["specs"].clone()).unwrap();
                state.changes(specs.iter().map(|(from, to, insert)| (*from..*to, insert)))
            }
        }
        .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let mut result = state.text().clone();
        changes.apply_to(&mut result).unwrap();
        assert_eq!(json!(result.as_str()), row["doc"], "{name}: document");
        let inverse = changes.inverted(state.text()).unwrap();
        inverse.apply_to(&mut result).unwrap();
        assert_eq!(&result, state.text(), "{name}: inverse");
        let ranges: Vec<_> = changes
            .changes()
            .map(|(range, insert)| json!([range.start, range.end, insert]))
            .collect();
        assert_eq!(json!(ranges), row["edit"], "{name}: sections");
        let desc = changes.desc(state.text().len()).unwrap();
        let map: Vec<_> = row["map"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                let pos = entry[0].as_u64().unwrap() as usize;
                let mapped = [Assoc::Before, Assoc::After].map(|assoc| {
                    [
                        MapMode::Simple,
                        MapMode::TrackDel,
                        MapMode::TrackBefore,
                        MapMode::TrackAfter,
                    ]
                    .map(|mode| desc.map_pos(pos, assoc, mode))
                });
                json!([pos, mapped])
            })
            .collect();
        assert_eq!(json!(map), row["map"], "{name}: mapping");
    }
}

#[test]
fn parsed_multi_range_replacement_matches_pinned_separators_and_caret_association() {
    for row in include_str!("fixtures/state_change_specs.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let Some(inputs) = row["inputs"].as_array() else {
            continue;
        };
        let ranges = inputs.iter().map(|input| {
            let [anchor, head]: [usize; 2] = serde_json::from_value(input.clone()).unwrap();
            SelectionRange::new(anchor, head)
        });
        let mut state = EditorState::create(
            row["source"].as_str().unwrap(),
            Some(EditorSelection::new(ranges.collect(), 1).unwrap()),
            LINE_SEPARATOR.of(row["separator"].as_str().unwrap().to_owned()),
        )
        .unwrap();
        let insert = row["insert"].as_str().unwrap();
        let exact = state.replace_selection(insert).unwrap();
        assert!(
            exact
                .changes()
                .changes()
                .all(|(_, inserted)| inserted == insert)
        );
        let parsed = state.replace_selection_parsed(insert).unwrap();
        state.update(parsed).unwrap();
        assert_eq!(json!(state.text().as_str()), row["doc"]);
        let output: Vec<_> = state
            .selection()
            .ranges()
            .iter()
            .map(|range| {
                assert_eq!(range.assoc(), Some(Assoc::Before));
                assert!(range.is_empty());
                json!([range.head(), -1])
            })
            .collect();
        assert_eq!(json!(output), row["output"]);
        assert_eq!(json!(state.selection().main_index()), row["main"]);
    }
}

#[test]
fn raw_sets_are_strict_and_invalid_state_specs_cannot_change_text() {
    let state = EditorState::from_text(Text::new("😀x"));
    assert_eq!(
        ChangeSet::new(vec![Change::new(0..4, "a"), Change::new(0..5, "b")]),
        Err(EditError::OverlappingChanges)
    );
    assert_eq!(
        state.changes([(0..4, "a"), (1..1, "b")]),
        Err(EditError::InvalidRange)
    );
    let invalid = ChangeSet::single(Change::new(1..4, "b")).unwrap();
    assert_eq!(
        state.changes_from_specs(&[ChangeSpec::String(0..4, "a"), ChangeSpec::Set(&invalid)]),
        Err(EditError::InvalidRange)
    );
    assert_eq!(state.text().as_str(), "😀x");
}
