use std::sync::Arc;

use gpui_editor_codemirror::{
    EditError, EditorState, Extension, Text,
    language::{BidiIsolateOptions, bidi_isolates, configured_bidi_isolates, java::JavaDocument},
    language_data::NativeLanguage,
    view::Viewport,
};
use serde_json::{Value, json};

#[test]
fn java_isolate_marks_match_the_pinned_plugin() {
    for row in include_str!("fixtures/java_bidi.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let name = case["name"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let text = Text::new(source);
        let support = Arc::new(NativeLanguage::Java.support().parse(text.clone()).unwrap());
        let state = EditorState::create_from_text(
            text.clone(),
            None,
            Extension::group([
                support.language_data_extension(),
                if case["explicit"].as_bool().unwrap() {
                    bidi_isolates(case["always"].as_bool().unwrap())
                } else {
                    Extension::default()
                },
            ]),
        )
        .unwrap();
        let doc = JavaDocument::parse(text);
        let viewport = Viewport::complete(doc.text());
        let actual: Vec<_> = configured_bidi_isolates(&state)
            .map(|always_isolate| {
                doc.bidi_isolate_marks(
                    &viewport,
                    BidiIsolateOptions {
                        always_isolate,
                        ..Default::default()
                    },
                )
                .unwrap()
                .iter(0)
                .map(|entry| {
                    assert_eq!(entry.value.class(), "cm-iso");
                    assert_eq!(entry.value.bidi_direction(), None);
                    json!([entry.from, entry.to, "auto"])
                })
                .collect()
            })
            .unwrap_or_default();
        assert_eq!(doc.text().as_str(), source, "{name}");
        assert_eq!(json!(actual), case["marks"], "{name}");
    }
}

#[test]
fn java_bidi_rejects_a_viewport_from_a_different_text() {
    let doc = JavaDocument::parse(Text::new("// 😀"));
    let other = Text::new("0123456789");
    for viewport in [
        Viewport::complete(&other),
        Viewport::around(&other, std::iter::once(4..5)).unwrap(),
    ] {
        assert!(matches!(
            doc.bidi_isolate_marks(&viewport, BidiIsolateOptions::default()),
            Err(EditError::InvalidRange),
        ));
    }
}
