//! Oracle rows from the SHA-512 verified `@lezer/xml` 1.0.6 and
//! `@codemirror/lang-xml` 6.1.0 tarballs, with `@lezer/lr` 1.4.13,
//! `@lezer/common` 1.5.7, `@lezer/highlight` 1.2.5 and
//! `@codemirror/language` 6.13.1. See `docs/codemirror-upstream.json`.
//! Tree and highlight positions map directly from UTF-16 to UTF-8 bytes.
//! Indent and fold queries use LF-normalized CodeMirror logical lines.
//! Regenerate the fixture with `fixtures/xml_parser.mjs`.

use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, Text,
    language::{IndentOptions, StreamSettings, TagModifier, lezer::ReuseStats, xml::XmlDocument},
};
use serde_json::{Value, json};

fn parse(source: impl Into<String>) -> XmlDocument {
    XmlDocument::parse(Text::new(source))
}

fn nodes(doc: &XmlDocument) -> Vec<Value> {
    doc.nodes()
        .map(|node| {
            let depth = std::iter::successors(node.parent(), |node| node.parent()).count();
            let range = node.range();
            assert!(doc.text().slice(range.clone()).is_some());
            json!([depth, node.kind().name(), range.start, range.end])
        })
        .collect()
}

fn highlights(doc: &XmlDocument) -> Vec<Value> {
    doc.highlights(0..doc.text().len())
        .unwrap()
        .map(|span| {
            let tags = span
                .tags
                .iter()
                .map(|tag| {
                    if tag.modifiers().is_empty() {
                        tag.name()
                    } else {
                        assert_eq!(tag.name(), "string");
                        assert_eq!(tag.modifiers(), [TagModifier::Special]);
                        "specialString"
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            json!([span.range.start, span.range.end, tags])
        })
        .collect()
}

#[test]
fn matches_pinned_tree_ranges_and_language_hooks() {
    for row in include_str!("fixtures/xml_parser.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let name = row["label"].as_str().unwrap();
        let doc = parse(row["source"].as_str().unwrap());
        assert_eq!(json!(nodes(&doc)), row["nodes"], "{name}: tree");
        assert_eq!(
            json!(highlights(&doc)),
            row["highlights"],
            "{name}: highlights"
        );

        let handles: Vec<_> = doc
            .nodes()
            .filter_map(|node| {
                let tag = node.range();
                let name = node.bracket_matching_handle()?.range();
                Some([tag.start, tag.end, name.start, name.end])
            })
            .collect();
        assert_eq!(json!(handles), row["handles"], "{name}: bracket handles");

        let lines: Vec<_> = (1..=doc.text().line_count())
            .map(|number| {
                let line = doc.text().line(number).unwrap();
                let indent = doc.indentation(
                    line.range.start,
                    StreamSettings::default(),
                    IndentOptions::default(),
                );
                let fold = doc.foldable(line.range.start, line.range.end);
                json!([indent, fold.map(|range| [range.start, range.end])])
            })
            .collect();
        assert_eq!(json!(lines), row["lines"], "{name}: indent and fold");
    }
}

fn update(doc: &XmlDocument, changes: &ChangeSet) -> XmlDocument {
    let updated = doc.reparse(changes).unwrap();
    let fresh = XmlDocument::parse(updated.text().clone());
    assert_eq!(nodes(&updated), nodes(&fresh));
    assert_eq!(highlights(&updated), highlights(&fresh));
    updated
}

#[test]
fn reuses_native_lr_records_with_open_ancestors_and_raw_byte_offsets() {
    let source = format!(
        "<根>\r\n{}<child>text</根>",
        "<葉 a=\"😀&amp;\"/>\r\n".repeat(40)
    );
    let at = source.rfind("\r\n").unwrap() + 1;
    let tail = source.find("text").unwrap();
    let doc = parse(&source);
    assert_eq!(doc.reuse_stats(), ReuseStats::default());
    let updated = update(
        &doc,
        &ChangeSet::new(vec![
            Change::new(at..at + 1, "\n<!-- changed -->\r\n"),
            Change::new(tail..tail + 4, "é&#42;"),
        ])
        .unwrap(),
    );
    let stats = updated.reuse_stats();
    assert!(
        stats.resumed_at > at / 2 && stats.resumed_at <= at,
        "{stats:?}"
    );
    assert!(source.is_char_boundary(stats.resumed_at));
    assert!(stats.nodes_reused > 100, "{stats:?}");
    assert_eq!(doc.text().as_str(), source);

    let unchanged = update(&updated, &ChangeSet::default());
    assert!(unchanged.reuse_stats().nodes_reused > 100);
    assert_eq!(unchanged.text(), updated.text());
}

#[test]
fn rejects_a_checkpoint_that_already_read_the_edited_tag_name() {
    // The checkpoint after the last `<` depends on the following name even
    // though the token only consumes `<`. Keep only the earlier checkpoint.
    let prefix = "<x/>".repeat(16);
    let source = format!("{prefix}<ab><c/></ab>");
    let first = prefix.len() + 1;
    let last = source.rfind("ab").unwrap();
    let doc = parse(&source);
    let unchanged = update(&doc, &ChangeSet::default());
    assert_eq!(unchanged.reuse_stats().resumed_at, first);
    let changed = update(
        &doc,
        &ChangeSet::new(vec![
            Change::new(first..first + 2, "名"),
            Change::new(last..last + 2, "名"),
        ])
        .unwrap(),
    );
    let stats = changed.reuse_stats();
    assert!(stats.resumed_at < first, "{stats:?}");
    assert!(stats.nodes_reused > 0, "{stats:?}");
}

#[test]
fn appending_text_does_not_reuse_a_checkpoint_that_read_eof() {
    let source = format!("{}{}", "<x/>".repeat(50), "z".repeat(100));
    let doc = parse(&source);
    let end = source.len();
    let unchanged = update(&doc, &ChangeSet::default());
    assert_eq!(unchanged.reuse_stats().resumed_at, end);
    let changed = update(
        &doc,
        &ChangeSet::single(Change::new(end..end, "😀")).unwrap(),
    );
    let stats = changed.reuse_stats();
    assert!(stats.resumed_at < end - 100, "{stats:?}");
    assert!(stats.nodes_reused > 0, "{stats:?}");
}

#[test]
fn recovery_drops_the_checkpoint_and_invalid_edits_leave_the_input_unchanged() {
    let source = format!("<a>{}😀</a>", "<x/>".repeat(50));
    let close = source.rfind("</").unwrap();
    let doc = parse(&source);
    let split_scalar = ChangeSet::single(Change::new(close - 1..close, "")).unwrap();
    assert!(matches!(
        doc.reparse(&split_scalar),
        Err(EditError::InvalidRange)
    ));
    assert_eq!(doc.text().as_str(), source);
    let broken = update(
        &doc,
        &ChangeSet::single(Change::new(close..source.len(), "<!-- unfinished")).unwrap(),
    );
    assert_eq!(broken.reuse_stats(), ReuseStats::default());
    let fixed = update(
        &broken,
        &ChangeSet::single(Change::new(close..broken.text().len(), "</a>")).unwrap(),
    );
    assert_eq!(fixed.reuse_stats(), ReuseStats::default());
    assert_eq!(fixed.text().as_str(), source);
}

#[test]
fn eof_self_close_recovery_follows_edits_without_keeping_checkpoints() {
    let prefix = "<葉 a=\"😀\"/>\r\n".repeat(40);
    let source = format!("{prefix}<漢😀 x=\"()\"/>");
    let value = source.rfind("\"()\"").unwrap();
    let start = prefix.len();
    let doc = parse(&source);
    let adjacent = update(
        &doc,
        &ChangeSet::single(Change::new(value..value + 4, "()")).unwrap(),
    );
    let spaced = update(
        &adjacent,
        &ChangeSet::single(Change::new(value + 1..value + 1, "\r\n\t")).unwrap(),
    );
    let number = update(
        &spaced,
        &ChangeSet::single(Change::new(value..value + 1, "1")).unwrap(),
    );
    for broken in [&adjacent, &spaced, &number] {
        let end = broken.text().len();
        assert_eq!(broken.reuse_stats(), ReuseStats::default());
        let hit = broken.bracket_near(start).unwrap();
        assert_eq!(hit.start, start..start + 1);
        assert_eq!(hit.end, Some(end - 2..end));
        assert!(hit.matched);
    }

    let end = spaced.text().len();
    let extended = update(
        &spaced,
        &ChangeSet::single(Change::new(end..end, "\r\n")).unwrap(),
    );
    assert_eq!(extended.reuse_stats(), ReuseStats::default());
    assert!(!extended.bracket_near(start).unwrap().matched);
    assert!(extended.bracket_near(end + 1).is_none());

    let fixed = update(
        &extended,
        &ChangeSet::new(vec![
            Change::new(value..end - 2, "\"()\""),
            Change::new(end..end + 2, ""),
        ])
        .unwrap(),
    );
    assert_eq!(fixed.text().as_str(), source);
    assert_eq!(fixed.reuse_stats(), ReuseStats::default());
    let unchanged = update(&fixed, &ChangeSet::default());
    assert!(unchanged.reuse_stats().nodes_reused > 100);
}
