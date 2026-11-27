//! The fixture records the four upstream commands in a real EditorView in
//! headless Google Chrome 158.0.8127.37. The SHA-512 hashes of commands 6.11.2,
//! state 6.7.6, view 6.44.0, and language 6.13.1 match the upstream manifest.
//! The oracle configured `wordChars` through the language data facet. Positions
//! are UTF-8 byte lengths of `sliceDoc(0, pos)`, including explicit CRLF breaks.
//!
//! Native motion has no optional `Intl.Segmenter` word-boundary pass. Its Han
//! results do not depend on a browser or its dictionary. Native motion follows
//! logical order, not visual BiDi order, and does not store visual affinity.
//! The narrow LTR rows verify that a soft wrap is not itself a subword boundary:
//! Chrome rendered one and three display lines but reached the same byte.

use std::sync::Arc;

use gpui_editor_codemirror::{
    EditorSelection, EditorState, Extension, READ_ONLY, SelectionRange, Transaction,
    commands::{
        cursor_subword_backward, cursor_subword_forward, select_subword_backward,
        select_subword_forward,
    },
    word_characters,
};
use serde_json::{Value, json};

fn cases() -> impl Iterator<Item = Value> {
    include_str!("fixtures/commands_subword.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn number(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn ranges(selection: &EditorSelection) -> Value {
    json!(
        selection
            .ranges()
            .iter()
            .map(|range| [range.anchor(), range.head()])
            .collect::<Vec<_>>()
    )
}

fn state_for(case: &Value) -> EditorState {
    let mut extensions = vec![READ_ONLY.of(case["readOnly"] == true)];
    if let Some(chars) = case["word"].as_str() {
        let chars: Arc<str> = chars.into();
        extensions.push(word_characters(move |_, _| Some(Arc::clone(&chars))));
    }
    if let Some(scope) = case["scope"].as_array() {
        let scope_range = number(&scope[0])..number(&scope[1]);
        let chars: Arc<str> = scope[2].as_str().unwrap().into();
        extensions.push(word_characters(move |_, position| {
            scope_range.contains(&position).then(|| Arc::clone(&chars))
        }));
    }
    let mut state = EditorState::with_extensions(
        case["source"].as_str().unwrap(),
        Extension::group(extensions),
    )
    .unwrap();
    let before = case["before"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| SelectionRange::new(number(&pair[0]), number(&pair[1])))
        .collect();
    let selection = EditorSelection::new(before, number(&case["beforeMain"])).unwrap();
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    state
}

fn check(case: &Value, expected: &Value) {
    let name = case["name"].as_str().unwrap();
    let mut state = state_for(case);
    let command = match case["command"].as_str().unwrap() {
        "cursorSubwordForward" => cursor_subword_forward,
        "cursorSubwordBackward" => cursor_subword_backward,
        "selectSubwordForward" => select_subword_forward,
        "selectSubwordBackward" => select_subword_backward,
        command => panic!("unexpected command: {command}"),
    };
    let result = command(&state).unwrap();
    assert_eq!(ranges(state.selection()), case["before"], "{name}: input");
    assert_eq!(
        state.selection().main_index(),
        number(&case["beforeMain"]),
        "{name}: input main"
    );
    assert_eq!(result.is_some(), case["handled"] == true, "{name}: handled");
    let events = if let Some(transaction) = result {
        assert!(
            transaction.selection().is_some(),
            "{name}: selection update"
        );
        let events = json!([{
            "userEvent": transaction.user_event(),
            "scroll": transaction.scroll_into_view(),
            "docChanged": !transaction.changes().is_empty()
        }]);
        state.update(transaction).unwrap();
        events
    } else {
        json!([])
    };
    assert_eq!(events, case["events"], "{name}: dispatch");
    assert_eq!(state.text().as_str(), case["source"], "{name}: text");
    assert_eq!(&ranges(state.selection()), expected, "{name}: result");
    assert_eq!(
        state.selection().main_index(),
        number(&case["afterMain"]),
        "{name}: result main"
    );
}

#[test]
fn matches_pinned_chrome_oracle_without_view_or_dictionary_differences() {
    for case in cases().filter(|case| case["difference"].is_null()) {
        check(&case, &case["after"]);
    }
}

#[test]
fn native_subword_motion_uses_logical_order_without_platform_dictionaries() {
    for case in cases().filter(|case| !case["difference"].is_null()) {
        let (chrome, native) = match case["name"].as_str().unwrap() {
            "dictionary-han-forward" => (6, 18),
            "dictionary-han-backward" => (12, 0),
            "visual-bidi-backward" => (15, 7),
            name => panic!("unexpected difference: {name}"),
        };
        assert_eq!(case["after"], json!([[chrome, chrome]]));
        check(&case, &json!([[native, native]]));
    }
}

#[test]
fn motion_from_within_a_cluster_finishes_on_a_grapheme_boundary() {
    for (source, inside, start, end) in [("a\r\nb", 2, 0, 4), ("e\u{301}", 1, 0, 3)] {
        let mut state = EditorState::new(source);
        state
            .update(Transaction::default().with_selection(EditorSelection::single(inside, inside)))
            .unwrap();
        for (command, expected) in [
            (cursor_subword_forward as fn(&EditorState) -> _, end),
            (cursor_subword_backward, start),
        ] {
            let transaction = command(&state).unwrap().unwrap();
            assert_eq!(
                transaction.selection(),
                Some(&EditorSelection::single(expected, expected)),
                "{source:?}"
            );
        }
    }
}
