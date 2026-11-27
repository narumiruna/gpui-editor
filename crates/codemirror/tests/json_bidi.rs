//! JSON isolation marks from the pinned grammar and CodeMirror plugin.

use std::sync::Arc;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, EditorState, Extension, RangeValue, Text,
    language::{
        BidiIsolateOptions, bidi_isolates, configured_bidi_isolates,
        json::{JsonDocument, JsonNodeKind},
    },
    language_data::NativeLanguage,
    setup::{basic_setup_with_language, minimal_setup},
    view::{Viewport, bidi::Direction, keymap::KeyPlatform},
};
use serde_json::{Value, json};

#[test]
fn json_isolate_marks_match_the_pinned_plugin() {
    for row in include_str!("fixtures/json_bidi.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let id = case["id"].as_str().unwrap();
        let setup = case["setup"].as_str().unwrap();
        let options = BidiIsolateOptions {
            always_isolate: case["always"].as_bool().unwrap(),
            text_direction: if case["direction"] == 0 {
                Direction::Ltr
            } else {
                Direction::Rtl
            },
            per_line_text_direction: case["perLine"].as_bool().unwrap(),
        };
        let steps = case["steps"].as_array().unwrap();
        let initial = Text::new(steps[0]["source"].as_str().unwrap());
        let requested = configured_for(&initial, setup, options.always_isolate);
        assert_eq!(
            requested,
            (setup == "explicit").then_some(options.always_isolate),
            "{id}: opt-in"
        );
        let mut doc = JsonDocument::parse(initial);
        for step in steps {
            let name = step["name"].as_str().unwrap_or(id);
            if let Some(change) = step.get("change") {
                let (from, to, insert): (usize, usize, String) =
                    serde_json::from_value(change.clone()).unwrap();
                let edits = ChangeSet::single(Change::new(from..to, insert)).unwrap();
                doc = doc.reparse(&edits).unwrap();
                let fresh = JsonDocument::parse(doc.text().clone());
                check_snapshot(&fresh, step, requested, options, name);
            }
            check_snapshot(&doc, step, requested, options, name);
        }
    }
}

fn configured_for(text: &Text, setup: &str, always: bool) -> Option<bool> {
    let language = NativeLanguage::Json.support();
    let supported = Arc::new(language.parse(text.clone()).unwrap());
    let extensions = Extension::group([
        supported.language_data_extension(),
        if setup == "explicit" {
            bidi_isolates(always)
        } else {
            Extension::default()
        },
    ]);
    let state = EditorState::create_from_text(text.clone(), None, extensions).unwrap();
    match setup {
        "basic" => configured_bidi_isolates(
            basic_setup_with_language::<()>(state, KeyPlatform::Linux, language)
                .unwrap()
                .view()
                .state(),
        ),
        "minimal" => configured_bidi_isolates(
            minimal_setup::<()>(state, KeyPlatform::Linux)
                .view()
                .state(),
        ),
        "stock" | "explicit" => configured_bidi_isolates(&state),
        _ => unreachable!("unknown setup: {setup}"),
    }
}

fn check_snapshot(
    doc: &JsonDocument,
    step: &Value,
    requested: Option<bool>,
    options: BidiIsolateOptions,
    name: &str,
) {
    assert_eq!(
        doc.text().as_str(),
        step["source"].as_str().unwrap(),
        "{name}: raw text"
    );
    assert_eq!(
        doc.nodes().any(|node| node.kind() == JsonNodeKind::Error),
        step["recovered"].as_bool().unwrap(),
        "{name}: recovery"
    );
    let syntax: Vec<_> = doc
        .nodes()
        .filter(|node| node.kind().isolates_direction())
        .map(|node| json!([node.kind().name(), node.range().start, node.range().end]))
        .collect();
    assert_eq!(json!(syntax), step["syntax"], "{name}: syntax");

    let visible: Vec<[usize; 2]> = serde_json::from_value(step["visible"].clone()).unwrap();
    let viewport =
        Viewport::around(doc.text(), visible.into_iter().map(|[from, to]| from..to)).unwrap();
    let ranges: Vec<_> = if let Some(always_isolate) = requested {
        doc.bidi_isolate_marks(
            &viewport,
            BidiIsolateOptions {
                always_isolate,
                ..options
            },
        )
        .unwrap()
        .iter(0)
        .map(|mark| {
            assert_eq!(mark.value.class(), "cm-iso");
            assert_eq!(mark.value.bidi_direction(), None);
            assert!(mark.value.start_side() < 0 && mark.value.end_side() > 0);
            let node_text = doc.text().slice(mark.from..mark.to).unwrap();
            assert!(node_text.starts_with('"') && node_text.ends_with('"'));
            [mark.from, mark.to]
        })
        .collect()
    } else {
        Vec::new()
    };
    let mut expected: Vec<[usize; 2]> = serde_json::from_value(step["ranges"].clone()).unwrap();
    // A missing property name can get an empty upstream mark. It has no text
    // to render, and native inline marks must be nonempty.
    expected.retain(|[from, to]| from != to);
    assert_eq!(ranges, expected, "{name}: visible marks");
}

#[test]
fn checks_both_viewport_and_visible_ranges_against_raw_json_bytes() {
    let doc = JsonDocument::parse(Text::new(r#"["אב 🦀"]"#));
    let raw = doc.text().as_str();
    let len = raw.len();
    let unrelated = Text::new(" ".repeat(len + 1));
    let in_hebrew = raw.find('א').unwrap() + 1;
    let in_emoji = raw.find('🦀').unwrap() + 2;
    let viewports = [
        Viewport::new(&unrelated, 0..len, std::iter::once(in_hebrew..in_hebrew)).unwrap(),
        Viewport::new(&unrelated, 0..in_emoji, []).unwrap(),
        Viewport::new(&unrelated, 0..len + 1, []).unwrap(),
    ];
    for viewport in &viewports {
        assert_eq!(
            doc.bidi_isolate_marks(viewport, BidiIsolateOptions::default())
                .unwrap_err(),
            EditError::InvalidRange
        );
    }
}
