use std::time::Duration;

use gpui_editor_codemirror::{
    EditError, EditorSelection, EditorState, History, Text, Transaction,
    commands::{
        Indentation, indent_selection_with_projection, insert_blank_line_with_projection,
        insert_newline_and_indent_with_projection,
    },
    language::IndentOptions,
    language_data::NativeLanguage,
    setup::{SetupKeyContext, basic_setup},
    view::keymap::{KeyEvent, KeyModifiers, KeyPlatform},
};
use serde_json::{Value, json};

#[test]
fn syntax_indentation_matches_pinned_logical_lines_and_keeps_raw_bytes() {
    for row in include_str!("fixtures/projected_syntax_indent.jsonl").lines() {
        let fixture: Value = serde_json::from_str(row).unwrap();
        let name = fixture["name"].as_str().unwrap();
        let source = fixture["source"].as_str().unwrap();
        let mut state = EditorState::new(source);
        let [anchor, head]: [usize; 2] =
            serde_json::from_value(fixture["selection"].clone()).unwrap();
        let selection = EditorSelection::single(anchor, head);
        state
            .update(Transaction::default().with_selection(selection.clone()))
            .unwrap();
        let language = fixture_language(&fixture);
        let syntax = language.parse(state.text().clone()).unwrap();
        let lines = syntax.syntax_lines().unwrap();
        let indentation_at =
            |at, settings, options: IndentOptions<'_>| syntax.indentation(at, settings, options);
        let indent = Indentation::default();
        let bracket_gap = |at| syntax.bracket_gap_for_indent(at);
        let transaction = match fixture["command"].as_str() {
            Some("typeCloser") => {
                let document = language.support().parse(state.text().clone()).unwrap();
                document
                    .reindent_input(
                        &state,
                        state
                            .replace_selection("}")
                            .unwrap()
                            .with_user_event("input.type"),
                        indent,
                    )
                    .map(Some)
            }
            Some("indentSelection") => {
                indent_selection_with_projection(&state, lines, indent, indentation_at)
            }
            Some("insertBlankLine") => {
                insert_blank_line_with_projection(&state, lines, indent, indentation_at, "\r\n")
            }
            Some("insertNewlineAndIndent") => insert_newline_and_indent_with_projection(
                &state,
                lines,
                indent,
                indentation_at,
                bracket_gap,
                "\r\n",
            ),
            _ => unreachable!(),
        }
        .unwrap();
        let Some(output) = fixture["output"].as_str() else {
            assert!(transaction.is_none(), "{name}");
            assert_eq!(state.text().as_str(), source);
            continue;
        };
        let transaction = transaction.unwrap_or_else(|| panic!("{name}: rejected"));
        assert_eq!(json!(transaction.user_event()), fixture["event"], "{name}");
        assert_eq!(
            json!(transaction.scroll_into_view()),
            fixture["scroll"],
            "{name}"
        );
        let mut history = History::new(state);
        history.update(transaction, (), |_| (None, ())).unwrap();
        assert_eq!(history.state().text().as_str(), output, "{name}");
        let range = history.state().selection().main();
        assert_eq!(
            json!([range.anchor(), range.head()]),
            fixture["select"],
            "{name}"
        );
        assert_eq!(history.undo_depth(), 1, "{name}");
        assert!(history.undo().is_some());
        assert_eq!(history.state().text().as_str(), source);
        assert_eq!(history.state().selection(), &selection);
    }
}

fn fixture_language(fixture: &Value) -> NativeLanguage {
    match fixture["language"].as_str() {
        Some("cpp") => NativeLanguage::Cpp,
        Some("go") => NativeLanguage::Go,
        Some("java") => NativeLanguage::Java,
        Some("rust") => NativeLanguage::Rust,
        Some("json") => NativeLanguage::Json,
        _ => unreachable!(),
    }
}

#[test]
fn basic_setup_replays_pinned_syntax_indent_shortcuts_on_projected_lines() {
    for row in include_str!("fixtures/projected_syntax_indent.jsonl").lines() {
        let fixture: Value = serde_json::from_str(row).unwrap();
        let (key, modifiers) = match fixture["command"].as_str() {
            Some("insertNewlineAndIndent") => ("Enter", KeyModifiers::default()),
            Some("insertBlankLine") => (
                "Enter",
                KeyModifiers {
                    meta: true,
                    ..Default::default()
                },
            ),
            Some("indentSelection") => (
                "\\",
                KeyModifiers {
                    meta: true,
                    alt: true,
                    ..Default::default()
                },
            ),
            _ => continue,
        };
        let name = fixture["name"].as_str().unwrap();
        let source = fixture["source"].as_str().unwrap();
        let [anchor, head]: [usize; 2] =
            serde_json::from_value(fixture["selection"].clone()).unwrap();
        let mut state = EditorState::new(source);
        let selection = EditorSelection::single(anchor, head);
        state
            .update(Transaction::default().with_selection(selection.clone()))
            .unwrap();
        let syntax = fixture_language(&fixture)
            .parse(state.text().clone())
            .unwrap();
        let mut editor = basic_setup(state, KeyPlatform::Mac).unwrap();
        let context = SetupKeyContext {
            syntax: Some(&syntax),
            ..Default::default()
        };
        let result = editor
            .key_event_with(
                KeyEvent::new(key, modifiers),
                Duration::from_secs(1),
                (),
                context,
            )
            .unwrap();
        let view = editor.view();
        if let Some(expected) = fixture["output"].as_str() {
            assert!(result.key.handled, "{name}");
            assert_eq!(view.state().text().as_str(), expected, "{name}");
            let range = view.state().selection().main();
            assert_eq!(
                json!([range.anchor(), range.head()]),
                fixture["select"],
                "{name}"
            );
            assert_eq!(view.undo_depth(), 1, "{name}");
            assert!(editor.view_mut().undo().is_some());
        } else {
            assert_eq!(view.undo_depth(), 0, "{name}");
        }
        assert_eq!(editor.view().state().text().as_str(), source, "{name}");
        assert_eq!(editor.view().state().selection(), &selection, "{name}");
    }
}

#[test]
fn syntax_indent_rejects_a_projection_with_different_content() {
    let state = EditorState::new("a\rb");
    let fake = Text::new("a\nc");
    assert_eq!(
        indent_selection_with_projection(&state, &fake, Indentation::default(), |_, _, _| Some(2))
            .unwrap_err(),
        EditError::InvalidRange
    );
}
