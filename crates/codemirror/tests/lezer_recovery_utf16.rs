//! Recovery order for unrecognized supplementary scalars in Lezer LR 1.4.13.

use gpui_editor_codemirror::{Text, language::json::JsonDocument};
use serde_json::{Value, json};

#[test]
fn keeps_recovery_steps_separate_without_splitting_utf8() {
    for line in include_str!("fixtures/lezer_recovery_utf16.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let source = case["source"].as_str().unwrap();
        let doc = JsonDocument::parse(Text::new(source));
        assert!(doc.diagnostic().is_some(), "{source:?}");

        let mut nodes = Vec::new();
        let mut pending = vec![(0, doc.root())];
        while let Some((depth, node)) = pending.pop() {
            let range = node.range();
            assert!(source.get(range.clone()).is_some(), "{source:?} {range:?}");
            nodes.push(json!([depth, node.kind().name(), range.start, range.end]));
            pending.extend(node.children().rev().map(|child| (depth + 1, child)));
        }
        assert_eq!(json!(nodes), case["nodes"], "{source:?}");
    }
}
