use gpui_editor_codemirror::{
    Text,
    language::{brackets::BracketConfig, json::JsonDocument},
};
use serde_json::Value;

type Mark = (usize, usize, bool);

#[test]
fn configured_json_brackets_match_stock_plugin_marks() {
    for row in include_str!("fixtures/json_bracket_options.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let name = case["name"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let options = &case["options"];
        let doc = JsonDocument::parse(Text::new(source));
        let config = BracketConfig {
            brackets: options["brackets"].as_str().unwrap_or_default(),
            max_scan_distance: options["maxScanDistance"].as_u64().unwrap_or_default() as usize,
            after_cursor: options["afterCursor"].as_bool().unwrap_or(true),
        };
        let checks: Vec<(usize, Vec<Mark>)> =
            serde_json::from_value(case["checks"].clone()).unwrap();
        for (head, expected) in checks {
            let mut actual = Vec::new();
            if let Some(hit) = doc.bracket_near_with_config(head, config) {
                actual.push((hit.start.start, hit.start.end, hit.matched));
                if let Some(end) = hit.end {
                    actual.push((end.start, end.end, hit.matched));
                }
                actual.sort_unstable();
            }
            assert_eq!(actual, expected, "{name}: {source:?} at {head}");
        }
        for head in 0..source.len() {
            if !source.is_char_boundary(head) {
                assert!(doc.bracket_near_with_config(head, config).is_none());
            }
        }
    }
}
