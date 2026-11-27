//! Differential tests for state 6.7.6 word categories and ranges.
//! The generator verifies the archives and the exact JavaScript it loads.
//! Lone surrogates have no native string representation. Surrogate interiors
//! and the middle of CRLF have no shared coordinates. A lone CR uses the
//! native line policy; the oracle uses the default JavaScript line policy.

use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{CharCategory, EditorState, Extension, word_characters};
use serde_json::Value;

fn fixture() -> impl Iterator<Item = Value> {
    let mut lines = include_str!("fixtures/state_word.jsonl")
        .lines()
        .map(|row| serde_json::from_str::<Value>(row).unwrap());
    let metadata = lines.next().unwrap();
    let manifest: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let pin = rows(&manifest["packages"])
        .iter()
        .find(|pin| pin["name"] == "@codemirror/state")
        .unwrap();
    for field in ["name", "version", "integrity"] {
        assert_eq!(metadata["pin"][field], pin[field], "{field}");
    }
    assert_eq!(metadata["cluster"]["name"], "@marijn/find-cluster-break");
    assert_eq!(metadata["cluster"]["version"], "1.0.4");
    assert_eq!(metadata["unicode"], "17.0");
    lines
}

fn rows(value: &Value) -> &[Value] {
    value.as_array().unwrap()
}

fn number(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn range(value: &Value) -> Range<usize> {
    number(&value[0])..number(&value[1])
}

fn state(case: &Value) -> EditorState {
    let extensions = rows(&case["providers"]).iter().map(|provider| {
        let chars: Arc<str> = provider["chars"].as_str().unwrap().into();
        let scope = (!provider["scope"].is_null()).then(|| range(&provider["scope"]));
        word_characters(move |_, pos| {
            scope
                .as_ref()
                .is_none_or(|scope| scope.contains(&pos))
                .then(|| Arc::clone(&chars))
        })
    });
    EditorState::with_extensions(
        case["source"].as_str().unwrap(),
        Extension::group(extensions.collect::<Vec<_>>()),
    )
    .unwrap()
}

#[test]
fn categories_match_pinned_node() {
    let mut differences = Vec::new();
    for case in fixture() {
        let state = state(&case);
        for probe in rows(&case["categories"]) {
            let position = number(&probe[0]);
            let cluster = probe[1].as_str().unwrap();
            let expected = match probe[2].as_str().unwrap() {
                "Space" => CharCategory::Space,
                "Word" => CharCategory::Word,
                "Other" => CharCategory::Other,
                value => panic!("unexpected category: {value}"),
            };
            let actual = state.char_categorizer(position).category(cluster);
            if actual != expected {
                differences.push(format!(
                    "{} {position} {cluster:?}: Node {expected:?}, native {actual:?}",
                    case["name"]
                ));
            }
        }
    }
    assert!(differences.is_empty(), "\n{}", differences.join("\n"));
}

#[test]
fn word_ranges_match_pinned_node_in_raw_byte_coordinates() {
    let mut differences = Vec::new();
    for case in fixture() {
        let state = state(&case);
        assert_eq!(state.text().as_str(), case["source"].as_str().unwrap());
        for probe in rows(&case["words"]) {
            let position = number(&probe[0]);
            let expected = (!probe[1].is_null()).then(|| range(&probe[1]));
            let actual = state.word_range_at(position);
            if actual != expected {
                differences.push(format!(
                    "{} {position}: Node {expected:?}, native {actual:?}",
                    case["name"]
                ));
            }
        }
    }
    assert!(differences.is_empty(), "\n{}", differences.join("\n"));
}

#[test]
fn native_offsets_keep_invalid_position_fallback_and_raw_newlines() {
    let state = EditorState::with_extensions(
        "é-\r\nx\ny\rz 😀",
        word_characters(|state, at| {
            assert!(state.text().as_str().is_char_boundary(at));
            Some(Arc::from("-"))
        }),
    )
    .unwrap();
    assert_eq!(state.text().as_str(), "é-\r\nx\ny\rz 😀");
    for position in [1, 12, 13, 14, state.text().len() + 1, usize::MAX] {
        let category = state.char_categorizer(position);
        assert_eq!(category.category("-"), CharCategory::Other);
        assert_eq!(category.category("é"), CharCategory::Word);
        assert_eq!(state.word_range_at(position), None);
    }
    assert_eq!(state.word_range_at(3), Some(0..3));
    assert_eq!(state.word_range_at(4), None); // Inside CRLF.
    assert_eq!(state.word_range_at(5), Some(5..6));
    assert_eq!(state.word_range_at(7), Some(7..8));
    assert_eq!(state.word_range_at(9), Some(9..10));
    assert_eq!(state.text().line_at(9).unwrap().text, "y\rz 😀"); // A lone CR is content.
}
