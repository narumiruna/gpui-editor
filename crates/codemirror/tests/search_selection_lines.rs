use gpui_editor_codemirror::{
    EditorSelection, EditorState, Extension, LINE_SEPARATOR, SelectionRange, Text,
    search::{
        SelectionMatchConfig, highlight_selection_matches, select_next_occurrence,
        select_selection_matches, selection_match_extension,
    },
    setup::basic_setup,
    view::{Viewport, keymap::KeyPlatform},
};
use serde_json::{Value, json};

#[test]
fn selected_text_matching_uses_code_mirror_line_endings() {
    let mut differences = Vec::new();
    for line in include_str!("fixtures/search_selection_lines.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let original = Text::new(case["source"].as_str().unwrap());
        let ranges: Vec<_> = case["selection"]
            .as_array()
            .unwrap()
            .iter()
            .map(|range| SelectionRange::new(byte(&range[0]), byte(&range[1])))
            .collect();
        let main = ranges.len() - 1;
        let selection = EditorSelection::new(ranges, main).unwrap();
        let options = selection_match_extension(SelectionMatchConfig {
            min_selection_length: Some(byte(&case["min"])),
            ..Default::default()
        });
        let separator = case["separator"]
            .as_str()
            .map_or_else(Extension::default, |value| LINE_SEPARATOR.of(value.into()));
        let state = EditorState::create_from_text(
            original.clone(),
            Some(selection),
            Extension::group([options, separator]),
        )
        .unwrap();
        let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
        let operation = case["operation"].as_str().unwrap();
        let actual = if operation == "highlight" {
            let marks = editor.decorations().unwrap().selection_matches;
            json!(
                marks
                    .iter(0)
                    .map(|mark| [mark.from, mark.to])
                    .collect::<Vec<_>>()
            )
        } else {
            let state = editor.view().state();
            let tr = match operation {
                "all" => select_selection_matches(state),
                "next" => select_next_occurrence(state),
                _ => panic!("unexpected operation {operation}"),
            };
            let handled = tr.is_some();
            if let Some(tr) = tr {
                editor.view_mut().update(tr, (), |_| (None, ())).unwrap();
            }
            let selection = editor.view().state().selection();
            json!({"handled": handled, "main": selection.main_index(), "ranges":
                selection.ranges().iter().map(|range| [range.anchor(), range.head()]).collect::<Vec<_>>()})
        };
        assert_eq!(editor.view().state().text(), &original);
        if actual != case["result"] {
            differences.push(format!("{line}: got {actual}"));
        }
    }
    assert!(
        differences.is_empty(),
        "{} mismatches:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

#[test]
fn projected_marks_do_not_paint_partial_crlf_and_native_selection_keeps_its_bytes() {
    let source = "é\r\nY | é\r\nY";
    for (selection, full_match, clipped) in [(0..4, 8..12, 8..11), (2..5, 10..13, 11..13)] {
        let state = EditorState::create_from_text(
            Text::new(source),
            Some(EditorSelection::single(selection.start, selection.end)),
            Extension::default(),
        )
        .unwrap();
        let editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
        let state = editor.view().state();
        let complete = Viewport::complete(state.text());
        let clipped = Viewport::around(state.text(), std::iter::once(clipped)).unwrap();
        let marks = highlight_selection_matches(state, &complete, Default::default()).unwrap();
        let mut ranges = marks.iter(0);
        let only = ranges.next().unwrap();
        assert_eq!(only.from..only.to, full_match);
        assert!(ranges.next().is_none());
        assert!(
            highlight_selection_matches(state, &clipped, Default::default())
                .unwrap()
                .is_empty()
        );
    }
    let state = EditorState::create_from_text(
        Text::new("é\r\nY | é\r\nY | é\nY"),
        Some(EditorSelection::single(0, 3)),
        Extension::default(),
    )
    .unwrap();
    let editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
    let state = editor.view().state();
    let selected = select_selection_matches(state).unwrap();
    assert_eq!(
        selected.selection().unwrap().ranges(),
        [SelectionRange::new(0, 3), SelectionRange::new(8, 11)]
    );
    assert_eq!(state.text().as_str(), "é\r\nY | é\r\nY | é\nY");
}

fn byte(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}
