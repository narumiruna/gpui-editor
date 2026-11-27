//! Oracle: real plugin decoration sets in headless Chrome 142.0.7444.175.
//! The oracle used the SHA-512-verified @codemirror/view 6.44.0 and
//! @codemirror/state 6.7.6 artifacts from docs/codemirror-upstream.json.
//! The fixture records snapshots from EditorView, including real replacement
//! gaps and scrolling. Positions were converted from UTF-16 to UTF-8 while
//! preserving CRLF byte lengths. The fixture does not compare DOM paint.

use std::ops::Range as ByteRange;

use gpui_editor_codemirror::{
    Compartment, EditError, EditorSelection, EditorState, Extension, SelectionRange, Text,
    Transaction,
    setup::{basic_setup, try_minimal_setup},
    view::{
        ViewHighlight, Viewport,
        decorations::{DecorationKind, DecorationSet},
        highlight_active_line, highlight_active_line_for_blocks, highlight_trailing_whitespace,
        highlight_whitespace,
        keymap::KeyPlatform,
    },
};
use serde_json::{Value, json};

fn decorations(set: &DecorationSet, lines: bool) -> Value {
    Value::Array(
        set.iter(0)
            .map(|range| {
                let class = match range.value.kind() {
                    DecorationKind::Line(line) if lines => line.class(),
                    DecorationKind::Mark(mark) if !lines => mark.class(),
                    other => panic!("unexpected decoration kind: {other:?}"),
                };
                json!([range.from, range.to, class])
            })
            .collect(),
    )
}

fn position(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn offsets(value: &Value) -> ByteRange<usize> {
    position(&value[0])..position(&value[1])
}

#[test]
fn matches_pinned_browser_highlight_decorations() {
    for row in include_str!("fixtures/view_highlights.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let text = Text::new(case["text"].as_str().unwrap());
        let selection = EditorSelection::new(
            case["selection"]
                .as_array()
                .unwrap()
                .iter()
                .map(|range| SelectionRange::new(position(&range[0]), position(&range[1])))
                .collect(),
            position(&case["main"]),
        )
        .unwrap();
        let viewport = Viewport::new(
            &text,
            offsets(&case["interval"]),
            case["visible"].as_array().unwrap().iter().map(offsets),
        )
        .unwrap();
        for (name, result) in [
            ("active", highlight_active_line(&text, &selection)),
            ("whitespace", highlight_whitespace(&text, &viewport)),
            ("trailing", highlight_trailing_whitespace(&text, &viewport)),
        ] {
            assert_eq!(
                decorations(&result.unwrap(), name == "active"),
                case[name],
                "{}: {name}",
                case["name"],
            );
        }
    }
}

#[test]
fn installed_highlights_follow_pinned_compartment_transitions() {
    let kinds = [
        ViewHighlight::ActiveLine,
        ViewHighlight::Whitespace,
        ViewHighlight::TrailingWhitespace,
    ];
    let slot = Compartment::new();
    let mut state =
        EditorState::with_extensions("猫 alpha\nbeta γ\nend  \t", slot.of(Extension::default()))
            .unwrap();
    for row in include_str!("fixtures/view_highlight_extensions.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        if case["kind"] != "config" {
            continue;
        }
        let enabled = case["enabled"].as_array().unwrap();
        let extensions = Extension::group(enabled.iter().flat_map(|index| {
            let extension = kinds[position(index)].extension();
            [extension.clone(), extension]
        }));
        state
            .update(Transaction::default().with_effects([slot.reconfigure(extensions)]))
            .unwrap();
        let viewport = Viewport::complete(state.text());
        for (index, highlight) in kinds.into_iter().enumerate() {
            assert_eq!(highlight.enabled(&state), enabled.contains(&json!(index)));
            assert_eq!(
                decorations(&highlight.project(&state, &viewport).unwrap(), index == 0),
                case["sets"][index]
            );
        }
        assert_eq!(state.text().as_str(), case["doc"].as_str().unwrap());
        assert_eq!(state.selection().main().head(), 0);
    }
}

#[test]
fn active_line_geometry_matches_pinned_joined_line_blocks() {
    for row in include_str!("fixtures/view_highlight_extensions.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        if case["kind"] != "blocks" {
            continue;
        }
        let text = Text::new(case["doc"].as_str().unwrap());
        let selection = EditorSelection::new(
            case["endpoints"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| SelectionRange::new(position(&entry[0]), position(&entry[1])))
                .collect(),
            2,
        )
        .unwrap();
        let mut blocks = case["blocks"].as_array().unwrap().iter();
        let result = highlight_active_line_for_blocks(&text, &selection, |head| {
            let block = blocks.next().unwrap();
            assert_eq!(head, position(&block[0]));
            Some(position(&block[1])..position(&block[2]))
        })
        .unwrap();
        assert!(blocks.next().is_none());
        assert_eq!(
            decorations(&result, true),
            case["active"],
            "{}",
            case["name"]
        );
    }
}

#[test]
fn line_blocks_validate_geometry_and_basic_setup_installs_active_lines() {
    let state = EditorState::new("猫 one");
    let selection = EditorSelection::single(3, 3);
    for invalid in [None, Some(0..0), Some(1..3), Some(0..8)] {
        assert_eq!(
            highlight_active_line_for_blocks(state.text(), &selection, |_| invalid.clone())
                .unwrap_err(),
            EditError::InvalidRange
        );
    }
    let basic = basic_setup::<()>(state.clone(), KeyPlatform::Mac).unwrap();
    let minimal = try_minimal_setup::<()>(state, KeyPlatform::Mac).unwrap();
    assert_eq!(basic.decorations().unwrap().active_lines.iter(0).count(), 1);
    assert_eq!(
        minimal.decorations().unwrap().active_lines.iter(0).count(),
        0
    );
}

#[test]
fn rejects_invalid_utf8_or_out_of_bounds_endpoints() {
    let text = Text::new("a😀b");
    for (anchor, head) in [(2, 0), (0, 2), (7, 0), (0, 7)] {
        assert_eq!(
            highlight_active_line(&text, &EditorSelection::single(anchor, head)).unwrap_err(),
            EditError::InvalidSelection,
        );
    }
    let ascii = Text::new("abcdefg");
    for viewport in [
        Viewport::new(&ascii, 0..6, std::iter::once(2..6)).unwrap(),
        Viewport::new(&ascii, 2..6, []).unwrap(),
        Viewport::complete(&ascii),
    ] {
        assert_eq!(
            highlight_whitespace(&text, &viewport).unwrap_err(),
            EditError::InvalidRange,
        );
        assert_eq!(
            highlight_trailing_whitespace(&text, &viewport).unwrap_err(),
            EditError::InvalidRange,
        );
    }
}

#[test]
fn native_partial_viewport_does_not_expand_a_complete_visible_range() {
    let text = Text::new("é😀 x  \tg");
    let viewport = Viewport::around(&text, std::iter::once(9..11)).unwrap();
    assert_eq!(
        decorations(&highlight_whitespace(&text, &viewport).unwrap(), false),
        json!([[9, 10, "cm-highlightSpace"], [10, 11, "cm-highlightTab"]]),
    );
    assert_eq!(
        decorations(
            &highlight_trailing_whitespace(&text, &viewport).unwrap(),
            false,
        ),
        json!([[9, 11, "cm-trailingSpace"]]),
    );
}

#[test]
fn native_byte_boundaries_inside_crlf_do_not_add_line_ending_marks() {
    let text = Text::new("字 \r\n\t");
    for (interval, expected) in [(0..5, 3..4), (5..7, 6..7)] {
        let viewport = Viewport::around(&text, std::iter::once(interval)).unwrap();
        assert_eq!(
            decorations(
                &highlight_trailing_whitespace(&text, &viewport).unwrap(),
                false,
            ),
            json!([[expected.start, expected.end, "cm-trailingSpace"]]),
        );
    }
    assert_eq!(
        decorations(
            &highlight_active_line(&text, &EditorSelection::single(5, 5)).unwrap(),
            true,
        ),
        json!([[0, 0, "cm-activeLine"]]),
    );
}
