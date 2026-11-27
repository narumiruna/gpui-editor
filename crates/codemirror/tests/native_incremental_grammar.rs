//! Tree records from the SHA-512-verified `@lezer/javascript` 1.5.7 and
//! `@lezer/go` 1.0.1 tarballs, with `@lezer/lr` 1.4.13 and `@lezer/common` 1.5.7.
//! An external JavaScript run compared fresh and fragment parses. It used raw
//! JavaScript input and normalized Go line endings. The fixture records the
//! edited suffix in original UTF-8 bytes. The Rust tests do not run JavaScript.

use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        IndentOptions, StreamSettings,
        go::GoDocument,
        javascript::{JavascriptDocument, JavascriptOptions},
        lezer::ReuseStats,
    },
};
use serde_json::Value;

fn edit(range: Range<usize>, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(range, insert)).unwrap()
}

fn javascript(source: &str, mode: &str) -> JavascriptDocument {
    let options = JavascriptOptions {
        typescript: matches!(mode, "ts" | "tsx"),
        jsx: matches!(mode, "jsx" | "tsx"),
    };
    if mode == "expression" {
        JavascriptDocument::parse_expression(Text::new(source), options)
    } else {
        JavascriptDocument::parse_with_options(Text::new(source), options)
    }
}

fn go(source: &str) -> GoDocument {
    GoDocument::parse(Text::new(source))
}

macro_rules! assert_fresh {
    ($edited:expr, $fresh:expr) => {{
        let (edited, fresh) = (&$edited, &$fresh);
        let source = edited.text().as_str();
        assert_eq!(source, fresh.text().as_str());
        assert_eq!(edited.root().range(), 0..source.len());
        assert_eq!(edited.is_recovered(), fresh.is_recovered());
        assert_eq!(edited.nodes().len(), fresh.nodes().len());
        for (node, other) in edited.nodes().zip(fresh.nodes()) {
            assert_eq!(node.kind(), other.kind());
            assert_eq!(node.range(), other.range());
            assert!(source.get(node.range()).is_some());
            assert_eq!(node.children().len(), other.children().len());
            assert_eq!(node.fold_range(), other.fold_range());
        }
        let range = 0..source.len();
        assert!(
            edited
                .highlights(range.clone())
                .unwrap()
                .eq(fresh.highlights(range).unwrap())
        );
        for i in 1..=edited.text().line_count() {
            let line = edited.text().line(i).unwrap().range;
            assert_eq!(
                edited.foldable(line.start, line.end),
                fresh.foldable(line.start, line.end)
            );
            assert_eq!(
                edited.indentation(
                    line.start,
                    StreamSettings::default(),
                    IndentOptions::default()
                ),
                fresh.indentation(
                    line.start,
                    StreamSettings::default(),
                    IndentOptions::default()
                ),
            );
        }
    }};
}

macro_rules! oracle_records {
    ($doc:expr, $from:expr) => {{
        $doc.nodes()
            .filter(|node| node.range().start >= $from)
            .map(|node| {
                let depth = std::iter::successors(node.parent(), |parent| parent.parent()).count();
                let range = node.range();
                format!(
                    "{} {} {} {depth}",
                    node.kind().name(),
                    range.start,
                    range.end
                )
            })
            .collect::<Vec<_>>()
    }};
}

fn fixtures() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/native_incremental_grammar.json")).unwrap()
}

#[test]
fn reuses_lr_records_and_matches_the_oracle_in_all_dialects() {
    for fixture in fixtures() {
        let mode = fixture["mode"].as_str().unwrap();
        let source = fixture["source"].as_str().unwrap();
        let at = fixture["edit"].as_u64().unwrap() as usize;
        let end = at + fixture["remove"].as_str().unwrap().len();
        assert_eq!(&source[at..end], fixture["remove"].as_str().unwrap());
        let insert = fixture["insert"].as_str().unwrap();
        let changes = edit(at..end, insert);
        let mut updated = source.to_owned();
        updated.replace_range(at..end, insert);
        let from = fixture["from"].as_u64().unwrap() as usize;
        let oracle: Vec<&str> = fixture["oracle"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node.as_str().unwrap())
            .collect();
        let (actual, reuse) = if mode == "go" {
            let seed = go(source);
            assert!(!seed.is_recovered());
            let doc = seed.reparse(&changes).unwrap();
            assert_fresh!(doc, go(&updated));
            assert!(!doc.is_recovered());
            (oracle_records!(doc, from), doc.reuse_stats())
        } else {
            let seed = javascript(source, mode);
            assert!(!seed.is_recovered(), "{mode}");
            let doc = seed.reparse(&changes).unwrap();
            assert_fresh!(doc, javascript(&updated, mode));
            assert!(!doc.is_recovered(), "{mode}");
            (oracle_records!(doc, from), doc.reuse_stats())
        };
        assert_eq!(actual, oracle, "{mode}");
        let (resumed_at, nodes_reused) = match mode {
            "go" => (76, 19),
            "expression" => (65, 33),
            _ => (69, 23),
        };
        assert_eq!(
            reuse,
            ReuseStats {
                resumed_at,
                nodes_reused
            },
            "{mode}"
        );
    }
}

#[test]
fn splits_and_joins_crlf_in_go_without_moving_raw_ranges() {
    let fixtures = fixtures();
    let source = fixtures.last().unwrap()["source"].as_str().unwrap();
    let at = source.find("3\r\n").unwrap() + 2;
    let seed = go(source);
    let next = seed.reparse(&edit(at..at + 1, "")).unwrap();
    let mut updated = source.to_owned();
    updated.remove(at);
    assert_fresh!(next, go(&updated));
    let back = next.reparse(&edit(at..at, "\n")).unwrap();
    assert_fresh!(back, seed);
    let reuse = ReuseStats {
        resumed_at: 76,
        nodes_reused: 19,
    };
    assert_eq!(next.reuse_stats(), reuse);
    assert_eq!(back.reuse_stats(), reuse);
}

#[test]
fn malformed_trees_do_not_keep_checkpoints() {
    let fixtures = fixtures();
    let source = fixtures[0]["source"].as_str().unwrap();
    let at = source.rfind('}').unwrap();
    let broken = [&source[..at], &source[at + 1..]].concat();
    let seed = javascript(source, "js");
    let recovered = seed.reparse(&edit(at..at + 1, "")).unwrap();
    assert!(recovered.is_recovered());
    assert_eq!(recovered.reuse_stats(), ReuseStats::default());
    assert_fresh!(recovered, javascript(&broken, "js"));
    let repaired = recovered.reparse(&edit(at..at, "}")).unwrap();
    assert_eq!(repaired.reuse_stats(), ReuseStats::default());
    assert_fresh!(repaired, seed);

    let fixture = fixtures.last().unwrap();
    let start = fixture["from"].as_u64().unwrap() as usize;
    let prefix = &fixture["source"].as_str().unwrap()[..start];
    let broken = format!("{prefix}func f() {{\r\n  println(\"😀\")\r\n");
    let valid = format!("{broken}}}\r\n");
    let seed = go(&valid);
    assert!(!seed.is_recovered());
    let recovered = seed.reparse(&edit(broken.len()..valid.len(), "")).unwrap();
    assert!(recovered.is_recovered());
    assert_eq!(recovered.reuse_stats(), ReuseStats::default());
    assert_fresh!(recovered, go(&broken));
    let repaired = recovered
        .reparse(&edit(broken.len()..broken.len(), "}\r\n"))
        .unwrap();
    assert_eq!(repaired.reuse_stats(), ReuseStats::default());
    assert_fresh!(repaired, seed);
}

#[test]
fn restores_automatic_semicolon_context_at_a_checkpoint() {
    let fixtures = fixtures();
    let js = fixtures[0]["source"].as_str().unwrap();
    let prefix = &js[..js.find("const c").unwrap()];
    let source = format!("{prefix}𐐀 \r\n++𐐀;\r\n");
    let at = source.rfind('𐐀').unwrap();
    let doc = javascript(&source, "js")
        .reparse(&edit(at..at + '𐐀'.len_utf8(), "x"))
        .unwrap();
    assert_fresh!(doc, javascript(&format!("{prefix}𐐀 \r\n++x;\r\n"), "js"));
    assert!(!doc.is_recovered());
    let reuse = ReuseStats {
        resumed_at: 69,
        nodes_reused: 22,
    };
    assert_eq!(doc.reuse_stats(), reuse);
    assert_eq!(&source[reuse.resumed_at..at], "++");
    assert!(doc.nodes().any(|node| {
        node.range() == (reuse.resumed_at..at + 1) && node.kind().name() == "UnaryExpression"
    }));

    let source = fixtures.last().unwrap()["source"].as_str().unwrap();
    let source = source.replace("b = 1", "b=1 // 🧪");
    let at = source.find('🧪').unwrap();
    let doc = go(&source)
        .reparse(&edit(at..at + '🧪'.len_utf8(), "test"))
        .unwrap();
    assert_fresh!(doc, go(&source.replace('🧪', "test")));
    assert!(!doc.is_recovered());
    assert_eq!(
        doc.reuse_stats(),
        ReuseStats {
            resumed_at: 76,
            nodes_reused: 20
        }
    );
    assert_eq!(&source[doc.reuse_stats().resumed_at..at], " // ");
}

#[test]
fn ambiguous_javascript_and_go_restart_with_glr() {
    let fixtures = fixtures();
    let js = &fixtures[0]["source"].as_str().unwrap()[..90];
    let go_source = &fixtures.last().unwrap()["source"].as_str().unwrap()[..102];
    macro_rules! check_glr {
        ($parse:expr, $prefix:expr, $tail:expr, $node:literal) => {{
            let parse = $parse;
            let prefix = $prefix;
            let end = prefix.len();
            let seed = parse(prefix);
            assert!(
                seed.reparse(&ChangeSet::default())
                    .unwrap()
                    .reuse_stats()
                    .nodes_reused
                    > 0
            );
            let doc = seed.reparse(&edit(end..end, $tail)).unwrap();
            assert!(!doc.is_recovered());
            assert_eq!(doc.reuse_stats(), ReuseStats::default());
            assert!(
                doc.nodes()
                    .any(|node| node.range().start >= end && node.kind().name() == $node)
            );
            assert_fresh!(doc, parse(&format!("{prefix}{}", $tail)));
            let back = doc.reparse(&edit(end..doc.text().len(), "")).unwrap();
            assert_eq!(back.reuse_stats(), ReuseStats::default());
            assert_fresh!(back, seed);
        }};
    }
    check_glr!(
        |source| javascript(source, "js"),
        js,
        "const f = (x) => x;\r\n",
        "ArrowFunction"
    );
    check_glr!(go, go_source, "var x = make([]int, 1)\r\n", "CallExpr");
}

#[test]
fn rejects_edits_through_a_utf8_scalar() {
    let fixtures = fixtures();
    let js = fixtures[0]["source"].as_str().unwrap();
    let go_source = fixtures.last().unwrap()["source"].as_str().unwrap();
    let js_doc = javascript(js, "js");
    let go_doc = go(go_source);
    let invalid = |source: &str| {
        let at = source.find('😀').unwrap() + 1;
        edit(at..at, "_")
    };
    assert!(js_doc.reparse(&invalid(js)).is_err());
    assert!(go_doc.reparse(&invalid(go_source)).is_err());
    assert_eq!(js_doc.text().as_str(), js);
    assert_eq!(go_doc.text().as_str(), go_source);
}
