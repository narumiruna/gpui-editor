//! Node names, byte ranges, and depths from @lezer/yaml 1.0.4 with
//! @lezer/common 1.5.7 and @lezer/lr 1.4.13. Sources include all 129
//! published basics and spec inputs, plus cases for recovery and comment ownership.
//! The oracle mapped UTF-16 positions to UTF-8 bytes. No JavaScript runs here.

use gpui_editor_codemirror::{
    Text,
    language::yaml::{YamlDocument, YamlNode},
};
use serde_json::Value;

fn records(node: &YamlNode, depth: usize, out: &mut Vec<String>) {
    let range = node.range();
    out.push(format!(
        "{} {} {} {depth}",
        node.kind().name(),
        range.start,
        range.end
    ));
    for child in node.children() {
        records(child, depth + 1, out);
    }
}

#[test]
fn matches_pinned_trees() {
    for line in include_str!("fixtures/yaml_pinned_grammar.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let name = case["name"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let doc = YamlDocument::parse(Text::new(source));
        let mut tree = Vec::new();
        records(doc.root(), 0, &mut tree);
        let oracle: Vec<_> = case["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record.as_str().unwrap())
            .collect();
        assert_eq!(tree, oracle, "{name}: {source:?}");
        assert_eq!(
            doc.is_recovered(),
            oracle.iter().any(|node| node.starts_with('⚠')),
            "{name}"
        );
    }
}
