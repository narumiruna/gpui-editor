use gpui_editor_codemirror::{Text, language::rust::RustDocument};
use serde_json::{Value, json};

#[test]
fn rust_brackets_match_the_pinned_editor() {
    for line in include_str!("fixtures/rust_brackets.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let source = row["source"].as_str().unwrap();
        let doc = RustDocument::parse(Text::new(source));
        for check in row["checks"].as_array().unwrap() {
            let pos = check[0].as_u64().unwrap() as usize;
            let actual = doc.bracket_near(pos).map_or(Value::Null, |hit| {
                json!([
                    [hit.start.start, hit.start.end],
                    hit.end.map(|span| [span.start, span.end]),
                    hit.matched
                ])
            });
            assert_eq!(actual, check[1], "{source:?} at byte {pos}");
        }
        if let Some(pos) = source.find('😀') {
            assert!(doc.bracket_near(pos + 1).is_none());
        }
        assert!(doc.bracket_near(source.len() + 1).is_none());
    }
}
