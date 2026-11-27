use gpui_editor_codemirror::{
    Text,
    language::{
        brackets::{BracketConfig, bracket_near},
        css::CssDocument,
    },
};
use serde_json::{Value, json};

#[test]
fn css_brackets_match_the_pinned_editor() {
    for line in include_str!("fixtures/css_brackets.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let source = row["source"].as_str().unwrap();
        let doc = if row["mode"] == "styles" {
            CssDocument::parse_styles(Text::new(source))
        } else {
            CssDocument::parse(Text::new(source))
        };
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

#[test]
fn call_pairing_does_not_choose_a_bracket_inside_a_string() {
    let text = Text::new(r#"a { --v: f(")"); }"#);
    let start = text.as_str().find('(').unwrap();
    let end = text.as_str().rfind(')').unwrap();
    let plain = bracket_near(&text, start, BracketConfig::default(), |_, _| true).unwrap();
    assert_ne!(plain.end, Some(end..end + 1));
    let doc = CssDocument::parse(text);
    let pair = doc.bracket_near(start).unwrap();
    assert!(pair.matched);
    assert_eq!(pair.end, Some(end..end + 1));
}
