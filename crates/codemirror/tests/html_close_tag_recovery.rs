use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::html::{HtmlDocument, HtmlNode, HtmlOptions},
};
use serde_json::{Value, json};

fn rows(document: &HtmlDocument) -> Value {
    fn visit(node: HtmlNode<'_>, depth: usize, rows: &mut Vec<Value>) {
        let range = node.range();
        rows.push(json!([depth, node.kind().name(), range.start, range.end]));
        for child in node.children() {
            visit(child, depth + 1, rows);
        }
    }
    let mut rows = Vec::new();
    visit(document.root(), 0, &mut rows);
    json!(rows)
}

fn edit(range: std::ops::Range<usize>, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(range, insert)).unwrap()
}

#[test]
fn close_tag_recovery_preserves_following_content_and_repairs() {
    for row in include_str!("fixtures/html_close_tag_recovery.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let source = row["source"].as_str().unwrap();
        let options = HtmlOptions {
            match_closing_tags: row["matchClosingTags"].as_bool().unwrap(),
            ..HtmlOptions::default()
        };
        let initial = HtmlDocument::parse_with_options(Text::new(source), options);
        assert_eq!(rows(&initial), row["initial"], "{source:?}: {options:?}");
        let Some(broken) = row["broken"].as_str() else {
            continue;
        };
        let insert = row["insert"].as_str().unwrap();
        let at = row["at"].as_u64().unwrap() as usize;
        let changed = initial.reparse(&edit(at..at, insert)).unwrap();
        let fresh = HtmlDocument::parse_with_options(Text::new(broken), options);
        assert_eq!(changed.text().as_str(), broken);
        assert_eq!(rows(&changed), row["recovered"], "{broken:?}: {options:?}");
        assert_eq!(rows(&fresh), rows(&changed));
        if let Some(mount) = initial.mounts().first() {
            let shifted = changed.mounts().first().unwrap();
            assert_eq!(mount.language(), shifted.language());
            assert_eq!(mount.range().start + insert.len(), shifted.range().start);
            assert_eq!(mount.range().end + insert.len(), shifted.range().end);
        }

        let repaired = changed.reparse(&edit(at..at + insert.len(), "")).unwrap();
        assert_eq!(rows(&repaired), row["initial"], "repair: {broken:?}");
        assert_eq!(rows(&initial), row["initial"], "retained: {source:?}");
        assert_eq!(changed.text().as_str(), broken);
        assert_eq!(repaired.text().as_str(), source);
    }
}
