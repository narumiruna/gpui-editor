//! The oracle fixture uses SHA-512-verified npm tarballs: `@lezer/cpp` 1.1.6,
//! `@lezer/java` 1.2.1, `@lezer/rust` 1.0.3, `@lezer/lr` 1.4.13,
//! `@lezer/common` 1.5.7, and `@lezer/highlight` 1.2.5. An external
//! JavaScript run compared fresh and fragment trees, verified shared tree
//! objects, and converted the changed tail to original UTF-8 byte ranges.
//! Java used raw input, C/C++ normalized CRLF and lone CR, and Rust normalized
//! CRLF only. The Rust tests do not run JavaScript.

use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, Text,
    language::{
        IndentOptions, StreamSettings,
        cpp::{CppDocument, CppNode},
        java::{self, JavaDocument},
        lezer::{ReuseStats, TreeFragment},
        rust::{RustDocument, RustNode},
    },
};
use serde_json::Value;

fn edit(range: Range<usize>, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(range, insert)).unwrap()
}

fn depth<N: Copy>(node: N, parent: impl Fn(N) -> Option<N>) -> usize {
    std::iter::successors(parent(node), |node| parent(*node)).count()
}

macro_rules! assert_fresh {
    ($edited:expr, $fresh:expr, $kind:ident) => {{
        let (edited, fresh) = (&$edited, &$fresh);
        let source = edited.text().as_str();
        assert_eq!(source, fresh.text().as_str());
        assert_eq!(edited.root().range(), 0..source.len());
        assert_eq!(edited.is_recovered(), fresh.is_recovered());
        assert_eq!(edited.nodes().count(), fresh.nodes().count());
        for (node, other) in edited.nodes().zip(fresh.nodes()) {
            assert_eq!(node.$kind(), other.$kind());
            assert_eq!(node.range(), other.range());
            assert!(source.get(node.range()).is_some());
            assert_eq!(
                depth(node, |node| node.parent()),
                depth(other, |node| node.parent())
            );
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

fn assert_prefix(reuse: ReuseStats, text: &Text, end: usize) {
    assert!((64..=end).contains(&reuse.resumed_at), "{reuse:?}");
    assert!(reuse.nodes_reused > 0, "{reuse:?}");
    assert!(text.as_str().is_char_boundary(reuse.resumed_at));
    assert!(
        text.as_str()
            .as_bytes()
            .get(reuse.resumed_at - 1..=reuse.resumed_at)
            != Some(b"\r\n")
    );
}

#[test]
fn reuses_prefix_records_and_matches_the_pinned_oracle() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/native_cjr_incremental.json")).unwrap();
    for case in cases {
        let field = |key: &str| case[key].as_str().unwrap();
        let mode = field("mode");
        let tail = field("tail");
        let prefix = format!(
            "{}{}",
            field("header"),
            field("padding").repeat(case["copies"].as_u64().unwrap() as usize)
        );
        let from = prefix.len();
        let at = from + tail.find(field("remove")).unwrap();
        let source = prefix + tail;
        let range = at..at + field("remove").len();
        let changes = edit(range.clone(), field("insert"));
        let mut updated = source.clone();
        updated.replace_range(range, field("insert"));
        let rows: Vec<_> = case["oracle"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node.as_str().unwrap())
            .collect();
        // These offsets include the CR bytes removed before C/C++ and Rust parsing.
        let (resumed_at, nodes_reused) = match mode {
            "c" => (4813, 1149),
            "cpp" => (4648, 1437),
            "java" => (4885, 2102),
            "rust" => (4844, 1851),
            _ => unreachable!(),
        };
        let expected = ReuseStats {
            resumed_at,
            nodes_reused,
        };
        macro_rules! compare {
            ($doc:ident, $kind:ident, $name:expr) => {{
                let seed = $doc::parse(Text::new(&source));
                assert_eq!(seed.reuse_stats(), ReuseStats::default());
                assert!(!seed.is_recovered(), "{mode}");
                let next = seed.reparse(&changes).unwrap();
                assert!(!next.is_recovered(), "{mode}");
                assert_prefix(next.reuse_stats(), next.text(), at);
                assert_eq!(next.reuse_stats(), expected, "{mode}");
                assert_fresh!(next, $doc::parse(Text::new(&updated)), $kind);
                let records: Vec<_> = next
                    .nodes()
                    .filter(|node| node.range().start >= from)
                    .map(|node| {
                        let range = node.range();
                        let depth = depth(node, |node| node.parent());
                        format!("{} {} {} {depth}", ($name)(node), range.start, range.end)
                    })
                    .collect();
                assert_eq!(records, rows, "{mode}");
            }};
        }
        match mode {
            "c" | "cpp" => compare!(CppDocument, kind, |n: CppNode<'_>| n.kind().name()),
            "java" => compare!(JavaDocument, term, java::JavaNode::name),
            "rust" => compare!(RustDocument, kind, |n: RustNode<'_>| n.kind().name()),
            _ => unreachable!(),
        }
    }
}

const CPP_PREFIX: &str =
    "int one = 1;\r\nint two = 2;\r\nint three = 3;\r\nint four = 4;\r\nint five = 5;\r\n";
const JAVA_PREFIX: &str =
    "package fixture;\r\n// this prefix contains 🧪\r\nclass Seed { int a = 10; int b = 20; }\r\n";
const RUST_PREFIX: &str = "const ONE: i32 = 1;\r\nconst TWO: i32 = 2;\r\nconst THREE: i32 = 3;\r\nconst FOUR: i32 = 4;\r\n";

#[test]
fn preserves_original_bytes_at_crlf_and_rejects_invalid_edits() {
    macro_rules! check {
        ($doc:ident, $kind:ident, $prefix:expr) => {{
            let prefix = $prefix;
            let source = format!("{prefix}/* 😀\r\n continued */\r\n");
            let at = source.find("😀\r\n").unwrap() + "😀\r".len();
            let seed = $doc::parse(Text::new(&source));
            let mut split = source.clone();
            assert_eq!(split.remove(at), '\n');
            let next = seed.reparse(&edit(at..at + 1, "")).unwrap();
            assert_prefix(next.reuse_stats(), next.text(), at - 1);
            assert_fresh!(next, $doc::parse(Text::new(&split)), $kind);
            let back = next.reparse(&edit(at..at, "\n")).unwrap();
            assert_prefix(back.reuse_stats(), back.text(), at - 1);
            assert_fresh!(back, seed, $kind);

            let astral = source.find('😀').unwrap();
            assert_eq!(
                seed.reparse(&edit(astral + 1..astral + 1, "x"))
                    .unwrap_err(),
                EditError::InvalidRange
            );
            assert_eq!(
                seed.reparse(&edit(source.len() + 1..source.len() + 1, "x"))
                    .unwrap_err(),
                EditError::InvalidRange
            );
            assert_eq!(seed.text().as_str(), source);

            let changes = ChangeSet::new(vec![
                Change::new(0..0, "// new\r\n"),
                Change::new(astral..astral + '😀'.len_utf8(), "other"),
            ])
            .unwrap();
            let next = seed.reparse(&changes).unwrap();
            assert_eq!(next.reuse_stats(), ReuseStats::default());
            assert_fresh!(
                next,
                $doc::parse(Text::new(format!(
                    "// new\r\n{}",
                    source.replace('😀', "other")
                ))),
                $kind
            );
        }};
    }
    check!(CppDocument, kind, CPP_PREFIX);
    check!(JavaDocument, term, JAVA_PREFIX);
    check!(RustDocument, kind, RUST_PREFIX);
}

#[test]
fn recovery_reports_no_reuse_and_strict_repairs_can_restore_a_prefix() {
    macro_rules! check {
        ($doc:ident, $kind:ident, $prefix:expr, $tail:expr, $removed:literal $(, $error:ident)?) => {{
            let source = format!("{}{}", $prefix, $tail);
            let seed = $doc::parse(Text::new(&source));
            assert!(!seed.is_recovered());
            let at = source.rfind($removed).unwrap();
            let broken = seed.reparse(&edit(at..at + 1, "")).unwrap();
            assert!(broken.is_recovered());
            assert_eq!(broken.reuse_stats(), ReuseStats::default());
            let fresh = $doc::parse(Text::new([&source[..at], &source[at + 1..]].concat()));
            assert_fresh!(broken, fresh, $kind);
            $(assert_eq!(broken.$error(), fresh.$error());)?
            let repaired = broken.reparse(&edit(at..at, $removed)).unwrap();
            assert_prefix(repaired.reuse_stats(), repaired.text(), at);
            assert_fresh!(repaired, seed, $kind);
            let next = repaired.reparse(&ChangeSet::default()).unwrap();
            assert_prefix(next.reuse_stats(), next.text(), source.len());
        }};
    }
    check!(CppDocument, kind, CPP_PREFIX, "int last = 1;\r\n", ";");
    check!(
        JavaDocument,
        term,
        JAVA_PREFIX,
        "class Demo { int value = 1; }\r\n",
        "}",
        parse_error
    );
    check!(
        RustDocument,
        kind,
        RUST_PREFIX,
        "const LAST: i32 = 1;\r\n",
        ";"
    );
}

#[test]
fn glr_restarts_without_checkpoints() {
    macro_rules! check {
        ($doc:ident, $kind:ident, $prefix:expr, $tail:expr) => {{
            let prefix = $prefix;
            let seed = $doc::parse(Text::new(prefix));
            let end = prefix.len();
            let unchanged = seed.reparse(&ChangeSet::default()).unwrap();
            assert_prefix(unchanged.reuse_stats(), unchanged.text(), end);
            let glr = seed.reparse(&edit(end..end, $tail)).unwrap();
            assert!(!glr.is_recovered());
            assert_eq!(glr.reuse_stats(), ReuseStats::default());
            assert_fresh!(
                glr,
                $doc::parse(Text::new(format!("{prefix}{}", $tail))),
                $kind
            );
            let back = glr.reparse(&edit(end..glr.text().len(), "")).unwrap();
            assert_eq!(back.reuse_stats(), ReuseStats::default());
            assert_fresh!(back, seed, $kind);
        }};
    }
    check!(CppDocument, kind, CPP_PREFIX, "Box<Box<int>> value;\r\n");
    check!(
        JavaDocument,
        term,
        JAVA_PREFIX,
        "record Demo(int value) {}\r\n"
    );
    check!(
        RustDocument,
        kind,
        RUST_PREFIX,
        "fn f() { if left && right { 1 } else { 0 } }\r\n"
    );
}

#[test]
fn java_top_rules_do_not_share_checkpoints() {
    let source = "class Fixture {}\r\n".repeat(12);
    let end = source.len();
    for (parser, other) in [
        (java::parser(), java::class_content_parser()),
        (java::class_content_parser(), java::parser()),
    ] {
        let tree = parser.parse_incremental(&source, None).unwrap();
        let fragment = TreeFragment {
            tree: &tree,
            old_range: 0..end,
            new_range: 0..end,
        };
        let same = parser
            .parse_incremental(&source, Some(fragment.clone()))
            .unwrap();
        assert!(same.reuse_stats().nodes_reused > 0);
        let diff = other.parse_incremental(&source, Some(fragment)).unwrap();
        assert_ne!(diff.root().term(), tree.root().term());
        assert_eq!(
            diff.root().term(),
            other.parse(&source).unwrap().root().term()
        );
        assert_eq!(diff.reuse_stats(), ReuseStats::default());
    }
}
