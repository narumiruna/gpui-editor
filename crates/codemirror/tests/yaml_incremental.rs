//! Oracle records from @lezer/yaml 1.0.4, @lezer/common 1.5.7, and @lezer/lr
//! 1.4.13. The oracle normalized CR and CRLF and mapped UTF-16 node positions
//! back to UTF-8 byte positions in the original text. No JavaScript runs here.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        IndentOptions, StreamSettings,
        lezer::ReuseStats,
        yaml::{YamlDocument, YamlFrontmatter, YamlNode},
    },
};
use serde_json::Value;

fn parse(source: &str) -> YamlDocument {
    YamlDocument::parse(Text::new(source))
}

fn records(doc: &YamlDocument) -> Vec<String> {
    fn walk(node: &YamlNode, text: &Text, depth: usize, out: &mut Vec<String>) {
        let range = node.range();
        assert!(text.slice(range.clone()).is_some());
        out.push(format!(
            "{} {} {} {depth}",
            node.kind().name(),
            range.start,
            range.end
        ));
        for child in node.children() {
            walk(child, text, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(doc.root(), doc.text(), 0, &mut out);
    out
}

fn assert_fresh(doc: &YamlDocument, fresh: &YamlDocument, id: &str) {
    assert_eq!(doc.text(), fresh.text(), "{id}");
    assert_eq!(doc.is_recovered(), fresh.is_recovered(), "{id}");
    assert_eq!(records(doc), records(fresh), "{id}");
    let full = 0..doc.text().len();
    assert!(
        doc.highlights(full.clone())
            .unwrap()
            .eq(fresh.highlights(full).unwrap()),
        "{id}"
    );
    for line in 1..=doc.text().line_count() {
        let range = doc.text().line(line).unwrap().range;
        assert_eq!(
            doc.foldable(range.start, range.end),
            fresh.foldable(range.start, range.end),
            "{id} line {line}"
        );
        let indent = |doc: &YamlDocument| {
            doc.indentation(
                range.start,
                StreamSettings::default(),
                IndentOptions::default(),
            )
        };
        assert_eq!(indent(doc), indent(fresh), "{id} line {line}");
    }
}

#[test]
fn strict_checkpoints_match_fresh_and_the_pinned_parser() {
    for line in include_str!("fixtures/yaml_incremental.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let id = case["id"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let seed = parse(source);
        assert!(!seed.is_recovered(), "seed: {id}");
        let mut updated = source.to_owned();
        let mut edits = Vec::new();
        for edit in case["edits"].as_array().unwrap() {
            let at = edit[0].as_u64().unwrap() as usize;
            let removed = edit[1].as_str().unwrap();
            let inserted = edit[2].as_str().unwrap();
            let range = at..at + removed.len();
            assert_eq!(&source[range.clone()], removed, "{id}");
            edits.push(Change::new(range, inserted));
        }
        let changes = ChangeSet::new(edits).unwrap();
        for (range, insert) in changes.changes().collect::<Vec<_>>().into_iter().rev() {
            updated.replace_range(range.clone(), insert);
        }
        let doc = seed.reparse(&changes).unwrap();
        assert!(!doc.is_recovered(), "updated: {id}");
        let oracle: Vec<_> = case["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record.as_str().unwrap())
            .collect();
        assert_eq!(records(&doc), oracle, "{id}");
        assert_fresh(&doc, &parse(&updated), id);
        let reuse = doc.reuse_stats();
        let boundary = changes.changes().next().unwrap().0.start;
        if id == "early-edit" {
            assert_eq!(reuse, ReuseStats::default());
        } else {
            assert!(
                reuse.resumed_at > 0 && reuse.resumed_at <= boundary && reuse.nodes_reused > 0,
                "{id}: {reuse:?}"
            );
            assert!(source.is_char_boundary(reuse.resumed_at), "{id}: {reuse:?}");
        }
        let inverse = changes.inverted(seed.text()).unwrap();
        assert_fresh(&doc.reparse(&inverse).unwrap(), &seed, id);
    }
}

#[test]
fn embedded_checkpoints_report_absolute_offsets() {
    let source = format!(
        "---\r\n{}tail: old\r\n---\r\nbody",
        "row: [名, '😀']\r\n".repeat(12)
    );
    let original = YamlFrontmatter::parse(&Text::new(&source)).unwrap();
    let at = source.find("old").unwrap();
    let edit = ChangeSet::single(Change::new(at..at + 3, "new value")).unwrap();
    let doc = original.yaml().reparse(&edit).unwrap();
    let reuse = doc.reuse_stats();
    assert!(reuse.nodes_reused > 0 && reuse.resumed_at > original.opening().end);
    assert!(reuse.resumed_at <= at && source.is_char_boundary(reuse.resumed_at));
    let mut updated = source;
    updated.replace_range(at..at + 3, "new value");
    let fresh = YamlFrontmatter::parse(&Text::new(&updated)).unwrap();
    assert_fresh(&doc, fresh.yaml(), "embedded value edit");
}

#[test]
fn recovery_drops_checkpoints_and_invalid_edits_leave_the_seed_unchanged() {
    let source = format!("{}last: [x, 名]\r\n", "entry: {label: '😀'}\r\n".repeat(12));
    let seed = parse(&source);
    let close = source.rfind(']').unwrap();
    let changes = ChangeSet::single(Change::new(close..close + 1, "")).unwrap();
    let broken = seed.reparse(&changes).unwrap();
    assert!(broken.is_recovered());
    assert_eq!(broken.reuse_stats(), ReuseStats::default());
    let mut expected = source.clone();
    expected.remove(close);
    assert_fresh(&broken, &parse(&expected), "removed flow closer");
    let restored = broken
        .reparse(&changes.inverted(seed.text()).unwrap())
        .unwrap();
    assert_eq!(restored.reuse_stats(), ReuseStats::default());
    assert_fresh(&restored, &seed, "restore flow closer");
    let astral = source.find('😀').unwrap();
    for pos in [astral + 1, source.len() + 1] {
        let invalid = ChangeSet::single(Change::new(pos..pos, "a")).unwrap();
        assert!(seed.reparse(&invalid).is_err());
    }
    assert_eq!(seed.text().as_str(), source);
}
