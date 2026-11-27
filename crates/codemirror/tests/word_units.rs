//! Pinned search and command behavior where the JS implementation inspects
//! one UTF-16 code unit inside or beside a UTF-8 character.

use std::sync::Arc;

use gpui_editor_codemirror::{
    EditorSelection, EditorState, Transaction,
    commands::{cursor_subword_forward, select_subword_forward},
    search::{HighlightSelectionOptions, highlight_selection_matches},
    view::Viewport,
    word_characters,
};
use serde_json::{Value, json};

#[test]
fn matches_single_code_unit_word_checks() {
    let mut differences = Vec::new();
    for line in include_str!("fixtures/word_units_browser.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let source = case["source"].as_str().unwrap();
        let extra: Arc<str> = case["word"].as_str().unwrap().into();
        let mut state =
            EditorState::with_extensions(source, word_characters(move |_, _| Some(extra.clone())))
                .unwrap();
        let expected = &case["selection"];
        let mismatch = if let Some(extend) = case["extend"].as_bool() {
            let transaction = if extend {
                select_subword_forward(&state)
            } else {
                cursor_subword_forward(&state)
            }
            .unwrap();
            assert_eq!(transaction.is_some(), case["handled"] == true);
            state.update(transaction.unwrap()).unwrap();
            let range = state.selection().main();
            let actual = json!([range.anchor(), range.head()]);
            (&actual != expected).then_some(actual)
        } else {
            let offset = |index: usize| usize::try_from(expected[index].as_u64().unwrap()).unwrap();
            state
                .update(
                    Transaction::default()
                        .with_selection(EditorSelection::single(offset(0), offset(1))),
                )
                .unwrap();
            let marks = highlight_selection_matches(
                &state,
                &Viewport::complete(state.text()),
                HighlightSelectionOptions {
                    whole_words: true,
                    highlight_word_around_cursor: case["cursor"] == true,
                    ..Default::default()
                },
            )
            .unwrap();
            let actual = json!(
                marks
                    .iter(0)
                    .map(|mark| { json!([mark.from, mark.to, mark.value.class()]) })
                    .collect::<Vec<_>>()
            );
            (actual != case["marks"]).then_some(actual)
        };
        assert_eq!(state.text().as_str(), source);
        if let Some(actual) = mismatch {
            differences.push(format!("{case}: got {actual}"));
        }
    }
    assert!(
        differences.is_empty(),
        "{} mismatches:\n{}",
        differences.len(),
        differences.join("\n")
    );
}
