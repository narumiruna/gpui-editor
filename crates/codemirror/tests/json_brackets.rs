use gpui_editor_codemirror::{Text, language::json::JsonDocument};
use serde_json::{Value, json};

#[test]
fn json_brackets_match_the_pinned_syntax_and_plain_fallback() {
    for row in include_str!("fixtures/json_brackets.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let source = case["source"].as_str().unwrap();
        let doc = JsonDocument::parse(Text::new(source));
        assert!(doc.root().parent().is_none());
        for node in doc.nodes() {
            for child in node.children() {
                assert_eq!(child.parent().unwrap().range(), node.range());
                assert_eq!(child.parent().unwrap().kind(), node.kind());
            }
        }
        for probe in case["hits"].as_array().unwrap() {
            let head = probe[0].as_u64().unwrap() as usize;
            let coordinates = |range: std::ops::Range<usize>| [range.start, range.end];
            let hit = doc.bracket_near(head).map(|hit| {
                json!([
                    coordinates(hit.start),
                    hit.end.map(coordinates),
                    hit.matched
                ])
            });
            assert_eq!(json!(hit), probe[1], "{source:?} at {head}");
        }
        for head in 0..source.len() {
            if !source.is_char_boundary(head) {
                assert!(doc.bracket_near(head).is_none());
            }
        }
    }
}
