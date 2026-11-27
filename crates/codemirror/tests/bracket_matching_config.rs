use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    Compartment, ConfigurationError, EditError, EditorSelection, EditorState, Extension, Range,
    SelectionRange, Text, Transaction,
    language::brackets::{BracketMatchingOptions, bracket_match_decorations, bracket_matching},
    setup::basic_setup,
    view::{MarkDecoration, keymap::KeyPlatform},
};
use serde_json::{Value, json};

#[test]
fn configured_match_marks_and_options_match_pinned_view() {
    for line in include_str!("fixtures/bracket_matching_config.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let source = Text::new(case["source"].as_str().unwrap());
        let callbacks = Arc::new(Mutex::new(Vec::new()));
        let extensions = case["options"].as_array().unwrap().iter().map(|options| {
            let mut config = BracketMatchingOptions {
                after_cursor: options["afterCursor"].as_bool(),
                brackets: options["brackets"].as_str().map(Arc::from),
                max_scan_distance: options["maxScanDistance"]
                    .as_u64()
                    .map(|limit| limit.try_into().unwrap()),
                render_match: None,
            };
            if options["render"] == true {
                let callbacks = callbacks.clone();
                let original = source.clone();
                config.render_match = Some(Arc::new(move |found, state| {
                    assert_eq!(state.text(), &original);
                    let start = [found.start.start, found.start.end];
                    let end = found.end.as_ref().map(|end| [end.start, end.end]);
                    callbacks.lock().unwrap().push(json!([
                        start,
                        end,
                        found.matched,
                        state.selection().ranges().len()
                    ]));
                    let label = if found.matched {
                        "custom-match"
                    } else {
                        "custom-miss"
                    };
                    vec![
                        MarkDecoration::new(label)
                            .range(found.start.clone())
                            .unwrap(),
                    ]
                }));
            }
            bracket_matching(config)
        });
        let ranges = case["selection"]
            .as_array()
            .unwrap()
            .iter()
            .map(|position| {
                let at: usize = position.as_u64().unwrap().try_into().unwrap();
                SelectionRange::new(at, at)
            })
            .collect();
        let selection = EditorSelection::new(ranges, 0).unwrap();
        let slot = Compartment::new();
        let state = EditorState::create_from_text(
            source.clone(),
            Some(selection.clone()),
            slot.of(Extension::group(extensions)),
        );
        if case["conflict"] == true {
            assert_eq!(
                state.unwrap_err(),
                EditError::Configuration(ConfigurationError::InvalidFieldData)
            );
            continue;
        }
        let mut editor = basic_setup::<()>(state.unwrap(), KeyPlatform::Linux).unwrap();
        for step in ["before", "after"] {
            if step == "after" {
                editor
                    .view_mut()
                    .update(
                        Transaction::default()
                            .with_selection(selection.clone())
                            .with_effects([slot.reconfigure(Extension::default())]),
                        (),
                        |_| (None, ()),
                    )
                    .unwrap();
            }
            let state = editor.view().state();
            let marks = editor.decorations().unwrap().bracket_matches;
            let actual = json!(
                marks
                    .iter(0)
                    .map(|mark| json!([mark.from, mark.to, mark.value.class()]))
                    .collect::<Vec<_>>()
            );
            assert_eq!(actual, case[step], "{step}: {line}");
            if step == "before" {
                assert_eq!(json!(*callbacks.lock().unwrap()), case["callbacks"]);
            }
            assert_eq!(state.text(), &source);
        }
    }
}

#[test]
fn removed_preset_stops_matching_and_invalid_renderer_is_rejected() {
    let original = "🐈 (x)";
    let mut editor = basic_setup::<()>(EditorState::new(original), KeyPlatform::Linux).unwrap();
    editor
        .view_mut()
        .set_selection(EditorSelection::single(5, 5))
        .unwrap();
    assert_eq!(editor.bracket_matches().len(), 1);
    assert!(
        !bracket_match_decorations(editor.view().state(), |_, _| true)
            .unwrap()
            .is_empty()
    );
    editor
        .view_mut()
        .update(
            Transaction::default().with_effects([Extension::default().reconfigure()]),
            (),
            |_| (None, ()),
        )
        .unwrap();
    assert!(editor.bracket_matches().is_empty());
    assert!(
        bracket_match_decorations(editor.view().state(), |_, _| true)
            .unwrap()
            .is_empty()
    );
    let renderer = bracket_matching(BracketMatchingOptions {
        render_match: Some(Arc::new(|_, _| {
            vec![Range::new(1, 2, MarkDecoration::new("broken"))]
        })),
        ..Default::default()
    });
    editor
        .view_mut()
        .update(
            Transaction::default().with_effects([renderer.append()]),
            (),
            |_| (None, ()),
        )
        .unwrap();
    let revision = editor.view().revision();
    assert_eq!(editor.bracket_matches().len(), 1);
    assert_eq!(
        bracket_match_decorations(editor.view().state(), |_, _| true).unwrap_err(),
        EditError::InvalidRange
    );
    assert_eq!(editor.view().revision(), revision);
    assert_eq!(editor.view().state().text().as_str(), original);
}
