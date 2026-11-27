use std::sync::Arc;

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, Text, Transaction,
    search::{
        HighlightSelectionOptions, SelectionMatchConfig, selection_match_extension,
        selection_match_options, word_characters,
    },
    setup::{basic_setup, minimal_setup},
    view::keymap::KeyPlatform,
};
use serde_json::{Value, json};

fn read_config(config: &Value) -> SelectionMatchConfig {
    let number = |key: &str| config[key].as_u64().map(|value| value.try_into().unwrap());
    SelectionMatchConfig {
        highlight_word_around_cursor: config["highlightWordAroundCursor"].as_bool(),
        min_selection_length: number("minSelectionLength"),
        max_matches: number("maxMatches"),
        whole_words: config["wholeWords"].as_bool(),
    }
}

#[test]
fn selection_match_options_combine_and_reconfigure_as_in_the_pinned_view() {
    for line in include_str!("fixtures/search_selection_config.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let source = case["source"].as_str().unwrap();
        let inputs = case["config"].as_array().unwrap();
        let configs = Extension::group(
            inputs
                .iter()
                .map(|config| selection_match_extension(read_config(config))),
        );
        let byte =
            |field: &str, index: usize| case[field][index].as_u64().unwrap().try_into().unwrap();
        let words = if case["wordScope"].is_array() {
            let scope = byte("wordScope", 0)..=byte("wordScope", 1);
            let chars: Arc<str> = case["wordScope"][2].as_str().unwrap().into();
            word_characters(move |_, pos| scope.contains(&pos).then(|| chars.clone()))
        } else {
            Extension::default()
        };
        let selection = EditorSelection::single(byte("selection", 0), byte("selection", 1));
        let slot = Compartment::new();
        let state = EditorState::create_from_text(
            Text::new(source),
            Some(selection.clone()),
            Extension::group([words, slot.of(configs)]),
        );
        if case["conflict"] == true {
            assert_eq!(
                state.unwrap_err(),
                EditError::Configuration(ConfigurationError::InvalidFieldData)
            );
            continue;
        }
        let setup = case["setup"] == true;
        let mut editor = if setup {
            basic_setup::<()>(state.unwrap(), KeyPlatform::Linux).unwrap()
        } else {
            minimal_setup::<()>(state.unwrap(), KeyPlatform::Linux)
        };
        for step in ["before", "stock", "after"] {
            let (extension, installed) = match step {
                "before" => (None, setup || !inputs.is_empty()),
                "stock" if case[step].is_array() => (
                    Some(selection_match_extension(SelectionMatchConfig::default())),
                    true,
                ),
                "after" => (Some(Extension::default()), setup),
                _ => continue,
            };
            if let Some(extension) = extension {
                editor
                    .view_mut()
                    .update(
                        Transaction::default()
                            .with_selection(selection.clone())
                            .with_effects([slot.reconfigure(extension)]),
                        (),
                        |_| (None, ()),
                    )
                    .unwrap();
            }
            let options = selection_match_options(editor.view().state()).unwrap();
            assert_eq!(options.is_some(), installed, "{line}: {step}");
            if step == "stock" {
                assert_eq!(options, Some(HighlightSelectionOptions::default()));
            }
            let marks = editor.decorations().unwrap().selection_matches;
            let actual = json!(
                marks
                    .iter(0)
                    .map(|mark| json!([mark.from, mark.to, mark.value.class()]))
                    .collect::<Vec<_>>()
            );
            assert_eq!(actual, case[step], "{line}: {step}");
            assert_eq!(editor.view().state().text().as_str(), source);
        }
    }
}

#[test]
fn a_conflicting_update_does_not_commit_or_remove_marks() {
    let slot = Compartment::new();
    let installed = |words| {
        selection_match_extension(SelectionMatchConfig {
            whole_words: Some(words),
            ..Default::default()
        })
    };
    let state = EditorState::create_from_text(
        Text::new("cat cat"),
        Some(EditorSelection::single(0, 3)),
        slot.of(installed(true)),
    )
    .unwrap();
    let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
    let revision = editor.view().revision();
    let change = ChangeSet::single(Change::new(4..7, "dog")).unwrap();
    let result = editor.view_mut().update(
        Transaction::new(change).with_effects([
            slot.reconfigure(Extension::group([installed(true), installed(false)]))
        ]),
        (),
        |_| (None, ()),
    );
    assert_eq!(
        result.unwrap_err(),
        EditError::Configuration(ConfigurationError::InvalidFieldData)
    );
    assert_eq!(editor.view().revision(), revision);
    assert_eq!(editor.view().state().text().as_str(), "cat cat");
    let marks = editor.decorations().unwrap().selection_matches;
    let mut ranges = marks.iter(0);
    let only = ranges.next().unwrap();
    assert_eq!(only.from..only.to, 4..7);
    assert!(ranges.next().is_none());
}

#[test]
fn removing_the_preset_removes_its_selection_marks() {
    let state = EditorState::new("cat cat");
    let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
    editor
        .view_mut()
        .set_selection(EditorSelection::single(0, 3))
        .unwrap();
    assert!(!editor.decorations().unwrap().selection_matches.is_empty());
    editor
        .view_mut()
        .update(
            Transaction::default().with_effects([Extension::default().reconfigure()]),
            (),
            |_| (None, ()),
        )
        .unwrap();
    assert!(editor.decorations().unwrap().selection_matches.is_empty());
}
