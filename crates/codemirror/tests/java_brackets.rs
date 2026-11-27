use gpui_editor_codemirror::{
    Text,
    language::{
        brackets::{BracketConfig, bracket_near},
        java::JavaDocument,
    },
};
use serde_json::{Value, json};

#[test]
fn plain_scan_confuses_a_string_closer_with_a_java_delimiter() {
    let source = r#"class A { void f(){ log(")"); } }"#;
    let text = Text::new(source);
    let from = source.find("log(").unwrap() + 3;
    let expected = source.find("\");").unwrap() + 1;
    let plain = bracket_near(&text, from, BracketConfig::default(), |_, _| true).unwrap();
    assert_ne!(plain.end, Some(expected..expected + 1));
    assert_eq!(
        JavaDocument::parse(text).bracket_near(from).unwrap().end,
        Some(expected..expected + 1)
    );
}

#[test]
fn java_bracket_feedback_matches_the_pinned_editor() {
    for row in include_str!("fixtures/java_brackets.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let source = row["source"].as_str().unwrap();
        let document = if row["top"] == "ClassContent" {
            JavaDocument::parse_class_content(Text::new(source))
        } else {
            JavaDocument::parse(Text::new(source))
        };
        for probe in row["hits"].as_array().unwrap() {
            let pos = probe[0].as_u64().unwrap() as usize;
            let actual = document.bracket_near(pos).map_or(Value::Null, |hit| {
                json!([
                    [hit.start.start, hit.start.end],
                    hit.end.map(|end| [end.start, end.end]),
                    hit.matched
                ])
            });
            assert_eq!(actual, probe[1], "{source:?} at byte {pos}");
        }
        assert!(document.bracket_near(source.len() + 1).is_none());
        if let Some(pos) = source.find('😀') {
            assert!(document.bracket_near(pos + 1).is_none());
        }
    }
}
