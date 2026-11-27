use gpui_editor_codemirror::{
    Text,
    language::{IndentOptions, StreamSettings, lezer_grammar::LezerDocument},
};
use serde_json::{Value, json};

#[test]
fn mixed_editor_line_breaks_match_the_pinned_grammar_and_services() {
    for row in include_str!("fixtures/lezer_editor_lines.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let source = case["source"].as_str().unwrap();
        let doc = LezerDocument::parse(Text::new(source));
        let records: Vec<_> = doc
            .nodes()
            .map(|node| {
                let depth = std::iter::successors(node.parent(), |node| node.parent()).count();
                let span = node.range();
                json!([depth, node.kind().name(), span.start, span.end])
            })
            .collect();
        assert_eq!(json!(records), case["nodes"], "{source:?}");
        assert_eq!(doc.text().as_str(), source);
        let lines = case["lines"].as_array().unwrap();
        assert_eq!(doc.syntax_lines().line_count(), lines.len());
        for (index, line) in lines.iter().enumerate() {
            let start = line["range"][0].as_u64().unwrap() as usize;
            let end = line["range"][1].as_u64().unwrap() as usize;
            assert_eq!(
                doc.syntax_lines().line(index + 1).unwrap().range,
                start..end
            );
            let indent =
                doc.indentation(start, StreamSettings::default(), IndentOptions::default());
            assert_eq!(json!(indent), line["indent"], "{source:?} at {start}");
            let fold = doc.foldable(start, end).map(|fold| [fold.start, fold.end]);
            assert_eq!(json!(fold), line["fold"], "{source:?} at {start}");
        }
    }
}
