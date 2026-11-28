use gpui_editor_codemirror::{
    Text,
    language::{
        IndentOptions, StreamSettings,
        json::{JsonDocument, JsonParseJob, parse_raw},
        lezer::SyntaxTree,
    },
};
use serde_json::{Value, json};

#[test]
fn matches_the_pinned_tree_and_services_under_recovery_and_stack_pressure() {
    let mut differences = Vec::new();
    for line in include_str!("fixtures/json_recovery_budget.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let name = &case["name"];
        let source = if let Some(source) = case["source"].as_str() {
            source.to_owned()
        } else if let Some(depth) = case["arrayDepth"].as_u64() {
            let depth = usize::try_from(depth).unwrap();
            "[".repeat(depth) + &"]".repeat(depth)
        } else {
            let depth = usize::try_from(case["objectDepth"].as_u64().unwrap()).unwrap();
            "{\"x\":".repeat(depth) + "0" + &"}".repeat(depth)
        };
        let doc = JsonDocument::parse(Text::new(&source));
        let mut hash = 2_166_136_261_u32;
        let mut count = 0;
        let mut pending = vec![(0, doc.root())];
        while let Some((depth, node)) = pending.pop() {
            hash = hash_node(hash, depth, node.kind().name(), node.range());
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
        if !case["source"].is_string() {
            let mut job = JsonParseJob::new(Text::new(&source));
            let output = job
                .advance(source.len(), usize::MAX)
                .unwrap()
                .output
                .unwrap();
            assert!(output.complete, "{name}");
            for (mode, tree) in [
                ("raw", parse_raw(&source, false).unwrap()),
                ("job", output.tree),
            ] {
                let actual = hash_tree(&tree, &source);
                let expected = json!([case["count"], case["hash"]]);
                if actual != expected {
                    differences.push(format!("{name} {mode}: got {actual}; expected {expected}"));
                }
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

fn hash_tree(tree: &SyntaxTree, source: &str) -> Value {
    let mut hash = 2_166_136_261_u32;
    let mut count = 0;
    let mut pending = vec![(0, tree.root())];
    while let Some((depth, node)) = pending.pop() {
        let range = node.range();
        assert!(source.get(range.clone()).is_some(), "{range:?}");
        hash = hash_node(hash, depth, node.name(), range);
        count += 1;
        pending.extend(node.children().rev().map(|child| (depth + 1, child)));
    }
    json!([count, hash])
}

fn hash_node(hash: u32, depth: usize, name: &str, range: std::ops::Range<usize>) -> u32 {
    format!("{depth}:{name}:{}:{};", range.start, range.end)
        .bytes()
        .fold(hash, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        })
}
