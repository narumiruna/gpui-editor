use gpui_editor_codemirror::{
    Text,
    language::{IndentOptions, StreamSettings, json::JsonDocument},
};
use serde_json::{Value, json};

#[test]
fn recovers_long_malformed_json_with_the_pinned_tree_and_services() {
    let mut differences = Vec::new();
    for line in include_str!("fixtures/json_recovery_budget.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let name = &case["name"];
        let source = case["source"].as_str().unwrap();
        let doc = JsonDocument::parse(Text::new(source));
        let mut hash = 2_166_136_261_u32;
        let mut count = 0;
        let mut pending = vec![(0, doc.root())];
        while let Some((depth, node)) = pending.pop() {
            let range = node.range();
            for byte in format!(
                "{depth}:{}:{}:{};",
                node.kind().name(),
                range.start,
                range.end
            )
            .bytes()
            {
                hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
            }
            count += 1;
            pending.extend(node.children().rev().map(|child| (depth + 1, child)));
        }
        let line_end = usize::try_from(case["lineEnd"].as_u64().unwrap()).unwrap();
        let fold = doc
            .foldable(0, line_end)
            .map(|range| [range.start, range.end]);
        let indent = doc.indentation(
            source.len(),
            StreamSettings::default(),
            IndentOptions::default(),
        );
        let actual = json!([count, hash, fold, indent]);
        let expected = json!([
            case["count"],
            case["hash"],
            case["fold"],
            case["indentAtEof"]
        ]);
        if actual != expected {
            differences.push(format!("{name}: got {actual}; expected {expected}"));
        }
        assert_eq!(doc.reuse_stats(), Default::default(), "{name}");
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
