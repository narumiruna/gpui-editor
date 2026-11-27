use std::{sync::Arc, time::Duration};

use gpui_editor_codemirror::{
    Assoc, Compartment, EditorSelection, EditorState, Extension, Precedence, Text, Transaction,
    language::{
        LanguageData,
        brackets::{BracketMatchingOptions, bracket_matching},
        language_data_provider,
    },
    language_data::{CloseBracketData, wast_extension},
    setup::{SetupEditor, basic_setup},
    view::keymap::{KeyEvent, KeyModifiers, KeyPlatform},
};
use serde_json::{Value, json};

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn setup(source: &str, selection: EditorSelection, extension: Extension) -> SetupEditor<()> {
    let state =
        EditorState::create_from_text(Text::new(source), Some(selection), extension).unwrap();
    basic_setup(state, KeyPlatform::Linux).unwrap()
}

#[test]
fn setup_input_uses_attached_wast_syntax_and_language_tokens() {
    for line in include_str!("fixtures/wast_setup_input.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let source = row["source"].as_str().unwrap();
        let initial =
            EditorSelection::single(number(&row["initial"][0]), number(&row["initial"][1]));
        let mut editor = setup(source, initial, wast_extension());
        for step in row["steps"].as_array().unwrap() {
            if let Some(input) = step["input"].as_str() {
                assert!(editor.insert_text(input, Duration::ZERO, ()).unwrap());
            } else {
                let (key, _) = editor
                    .key_event(
                        KeyEvent::new("Backspace", KeyModifiers::default()),
                        Duration::ZERO,
                        (),
                    )
                    .unwrap();
                assert!(key.handled);
            }
            let state = editor.view().state();
            assert_eq!(
                state.text().as_str(),
                step["body"].as_str().unwrap(),
                "{}",
                row["name"]
            );
            let main = state.selection().main();
            assert_eq!(
                json!([main.anchor(), main.head()]),
                step["selection"],
                "{}",
                row["name"]
            );
        }
        assert_eq!(editor.view_mut().undo(), Some(()), "{}", row["name"]);
        assert_eq!(
            editor.view().state().text().as_str(),
            source,
            "{}",
            row["name"]
        );
    }
}

#[test]
fn setup_matches_current_wast_syntax_with_the_installed_bracket_options() {
    let rows = include_str!("fixtures/grammar_bracket_options.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap());
    for row in rows.filter(|row| row["mode"] == "wast") {
        let source = row["source"].as_str().unwrap();
        let options = &row["options"];
        let extension = bracket_matching(BracketMatchingOptions {
            after_cursor: options["afterCursor"].as_bool(),
            brackets: options["brackets"].as_str().map(Arc::from),
            max_scan_distance: options["maxScanDistance"]
                .as_u64()
                .map(|value| usize::try_from(value).unwrap()),
            ..Default::default()
        });
        let mut editor = setup(
            source,
            EditorSelection::single(0, 0),
            Extension::group([wast_extension(), extension]),
        );
        for check in row["checks"].as_array().unwrap() {
            let head = number(&check[0]);
            editor
                .view_mut()
                .set_selection(EditorSelection::single(head, head))
                .unwrap();
            let hits = editor.bracket_matches();
            assert!(hits.len() <= 1);
            let hit = hits.first().map(|found| {
                json!([
                    [found.start.start, found.start.end],
                    found.end.as_ref().map(|range| [range.start, range.end]),
                    found.matched
                ])
            });
            assert_eq!(json!(hit), check[1], "{} at {head}", row["name"]);
        }
    }
}

#[test]
fn setup_matches_the_edited_tree_and_stops_when_the_matching_extension_is_removed() {
    let source = "(module ;; 😀 )\r\n";
    let tail = source.len();
    let mut editor = setup(source, EditorSelection::single(0, 0), wast_extension());
    assert_eq!(editor.bracket_matches()[0].end, None);
    editor
        .view_mut()
        .set_selection(EditorSelection::single(tail, tail))
        .unwrap();
    assert!(editor.insert_text(")", Duration::ZERO, ()).unwrap());
    let select_start = |editor: &mut SetupEditor<()>| {
        editor
            .view_mut()
            .set_selection(EditorSelection::single(0, 0))
            .unwrap();
    };
    select_start(&mut editor);
    assert_eq!(editor.bracket_matches()[0].end, Some(tail..tail + 1));
    assert_eq!(editor.view_mut().undo(), Some(()));
    select_start(&mut editor);
    assert_eq!(editor.bracket_matches()[0].end, None);
    assert_eq!(editor.view_mut().redo(), Some(()));
    select_start(&mut editor);
    assert_eq!(editor.bracket_matches()[0].end, Some(tail..tail + 1));
    editor
        .view_mut()
        .update(
            Transaction::default().with_effects([wast_extension().reconfigure()]),
            (),
            |_| (None, ()),
        )
        .unwrap();
    assert!(editor.bracket_matches().is_empty());
    editor
        .view_mut()
        .set_selection(EditorSelection::single(tail, tail))
        .unwrap();
    assert!(editor.insert_text("(", Duration::ZERO, ()).unwrap());
    assert_eq!(editor.view().state().text().as_str(), format!("{source}()"));
    let (backspace, _) = editor
        .key_event(
            KeyEvent::new("Backspace", KeyModifiers::default()),
            Duration::ZERO,
            (),
        )
        .unwrap();
    assert!(backspace.handled);
    assert_eq!(editor.view().state().text().as_str(), format!("{source})"));
}

#[test]
fn setup_uses_the_first_close_brackets_provider_at_the_main_head() {
    let provider = language_data_provider(|_, pos, side| {
        (pos == 6 && side == Assoc::Before)
            .then(|| {
                Arc::new(LanguageData::default().with(
                    "closeBrackets",
                    CloseBracketData {
                        brackets: Some(&["["]),
                        ..Default::default()
                    },
                ))
            })
            .into_iter()
            .collect()
    })
    .with_precedence(Precedence::High);
    let mut editor = setup(
        "(data 字)",
        EditorSelection::single(9, 6),
        Extension::group([wast_extension(), provider]),
    );
    assert!(editor.insert_text("[", Duration::ZERO, ()).unwrap());
    let state = editor.view().state();
    assert_eq!(state.text().as_str(), "(data [字])");
    let main = state.selection().main();
    assert_eq!((main.anchor(), main.head()), (10, 7));
}

#[test]
fn setup_falls_back_to_plain_matching_after_removing_the_language() {
    let source = "(module ;; 😀 )\r\n)";
    let language = Compartment::new();
    let mut editor = setup(
        source,
        EditorSelection::single(0, 0),
        language.of(wast_extension()),
    );
    let end = |editor: &SetupEditor<()>| editor.bracket_matches()[0].end.clone();
    assert_eq!(end(&editor), Some(source.len() - 1..source.len()));
    editor
        .view_mut()
        .update(
            Transaction::default().with_effects([language.reconfigure(Extension::default())]),
            (),
            |_| (None, ()),
        )
        .unwrap();
    let comment = source.find(')').unwrap();
    assert_eq!(end(&editor), Some(comment..comment + 1));
    editor
        .view_mut()
        .set_selection(EditorSelection::single(source.len(), source.len()))
        .unwrap();
    assert!(editor.insert_text("[", Duration::ZERO, ()).unwrap());
    assert_eq!(editor.view().state().text().as_str(), format!("{source}[]"));
}
