//! Tree records from the SHA-512-verified npm tarballs for `@lezer/python`
//! 1.1.20, `@lezer/lr` 1.4.13, and `@lezer/common` 1.5.7. The external
//! oracle compared fresh and fragment parses on LF-normalized input and
//! mapped UTF-16 positions back to original UTF-8 bytes. A `u16:` endpoint
//! has no matching UTF-8 boundary. No JavaScript runs here.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    autocomplete::session::SourceResult,
    language::{
        IndentOptions, StreamSettings,
        lezer::ReuseStats,
        python::{PythonDocument, global_completion_source, local_completion_source},
    },
};
use serde_json::Value;

fn parse(source: &str) -> PythonDocument {
    PythonDocument::parse(Text::new(source))
}

fn records(doc: &PythonDocument) -> Vec<String> {
    doc.nodes()
        .map(|node| {
            let range = node.range();
            assert!(doc.text().slice(range.clone()).is_some());
            let depth = std::iter::successors(node.parent(), |parent| parent.parent()).count();
            format!("{} {} {} {depth}", node.name(), range.start, range.end)
        })
        .collect()
}

fn assert_completion(actual: Option<SourceResult>, fresh: Option<SourceResult>, id: &str) {
    match (actual, fresh) {
        (None, None) => {}
        (Some(actual), Some(fresh)) => {
            assert_eq!(actual.from, fresh.from, "{id}");
            assert_eq!(actual.to, fresh.to, "{id}");
            assert_eq!(actual.options, fresh.options, "{id}");
            assert_eq!(actual.filter, fresh.filter, "{id}");
            assert_eq!(actual.commit_characters, fresh.commit_characters, "{id}");
        }
        _ => panic!("completion availability changed: {id}"),
    }
}

fn assert_fresh(actual: &PythonDocument, fresh: &PythonDocument, id: &str) {
    assert_eq!(actual.text().as_str(), fresh.text().as_str(), "{id}");
    assert_eq!(actual.is_recovered(), fresh.is_recovered(), "{id}");
    assert_eq!(records(actual), records(fresh), "{id}");
    let full = 0..actual.text().len();
    assert!(
        actual
            .highlights(full.clone())
            .unwrap()
            .eq(fresh.highlights(full).unwrap()),
        "{id}"
    );
    for line in 1..=actual.text().line_count() {
        let range = actual.text().line(line).unwrap().range;
        assert_eq!(
            actual.foldable(range.start, range.end),
            fresh.foldable(range.start, range.end),
            "{id}"
        );
        let indent = |doc: &PythonDocument| {
            doc.indentation(
                range.start,
                StreamSettings::default(),
                IndentOptions::default(),
            )
        };
        assert_eq!(indent(actual), indent(fresh), "{id}");
    }
    for node in actual.nodes() {
        let at = node.range().end;
        for explicit in [true, false] {
            assert_completion(
                local_completion_source(actual, at, explicit),
                local_completion_source(fresh, at, explicit),
                id,
            );
            assert_completion(
                global_completion_source(actual, at, explicit),
                global_completion_source(fresh, at, explicit),
                id,
            );
        }
    }
}

#[test]
fn checkpoints_match_fresh_and_the_pinned_parser() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/python_incremental.json")).unwrap();
    for case in fixtures {
        let id = case["id"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let seed = parse(source);
        let changes = case["edits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|edit| {
                let at = edit[0].as_u64().unwrap() as usize;
                let remove = edit[1].as_str().unwrap();
                let range = at..at + remove.len();
                assert_eq!(&source[range.clone()], remove, "{id}");
                Change::new(range, edit[2].as_str().unwrap())
            })
            .collect();
        let changes = ChangeSet::new(changes).unwrap();
        let doc = seed.reparse(&changes).unwrap();
        let (resumed_at, nodes_reused) = match id {
            "dedent" | "blank-line" => (107, 23),   // Nested indent stack.
            "raw-format" | "bracketed" => (125, 7), // Inside the raw format string.
            "nested-replacement" => (96, 13),       // Inside the inner replacement call.
            "legacy-print-lookahead" | "eof-lookahead" => (6, 1), // Skip a later checkpoint.
            "legacy-print-prefix"
            | "split-crlf"
            | "insert-inside-crlf"
            | "append-crlf-at-eof"
            | "astral-escape" => (73, 6),
            "edit-before-first-checkpoint"
            | "glr-soft-keywords"
            | "repair-closers"
            | "parseable-line-prefix" => (0, 0),
            _ => panic!("missing reuse expectation: {id}"),
        };
        let expected = ReuseStats {
            resumed_at,
            nodes_reused,
        };
        assert_eq!(doc.reuse_stats(), expected, "{id}");
        let mut updated = source.to_owned();
        let edits: Vec<_> = changes.changes().collect();
        for (range, insert) in edits.into_iter().rev() {
            updated.replace_range(range.clone(), insert);
        }
        assert_fresh(&doc, &parse(&updated), id);
        let back = doc
            .reparse(&changes.inverted(seed.text()).unwrap())
            .unwrap();
        assert_fresh(&back, &seed, id);
        assert_eq!(back.reuse_stats(), expected, "inverse: {id}");
        let oracle: Vec<_> = case["oracle"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record.as_str().unwrap())
            .collect();
        let recovered = matches!(id, "repair-closers" | "parseable-line-prefix");
        assert_eq!(doc.is_recovered(), recovered, "{id}");
        let no_edit = ChangeSet::default();
        if recovered || id == "glr-soft-keywords" {
            assert!(seed.reparse(&no_edit).unwrap().reuse_stats().nodes_reused > 0);
            assert_eq!(
                doc.reparse(&no_edit).unwrap().reuse_stats(),
                ReuseStats::default(),
                "{id}"
            );
        } else if id == "eof-lookahead" {
            // The checkpoint at the end of the old identifier is reusable
            // only if the lookahead still sees EOF, not the appended scalar.
            let same = seed.reparse(&no_edit).unwrap();
            assert_eq!(same.reuse_stats().resumed_at, source.len());
            assert_fresh(&same, &seed, id);
        }
        let tree = records(&doc);
        match id {
            "parseable-line-prefix" => {
                // Native recovery marks the whole suffix, not only the line.
                assert_eq!(&tree[..7], &oracle[..7]);
                assert_eq!(&tree[7..], ["⚠ 68 97 1"]);
                assert_eq!(&oracle[7..9], ["⚠ 68 71 1", "ClassDefinition 71 97 1"]);
                let name = updated.find("Later").unwrap();
                let span = doc.highlights(name..name + 5).unwrap().next().unwrap();
                assert_eq!(span.tags[0].name(), "className");
            }
            "astral-escape" => {
                // Upstream ends the escape between the surrogate halves.
                assert_eq!(&tree[..11], &oracle[..11]);
                assert_eq!(&tree[11..], ["Escape 80 85 3"]);
                assert_eq!(&oracle[11..], ["Escape 80 u16:77 3"]);
            }
            _ => assert_eq!(tree, oracle, "{id}"),
        }
    }
}

#[test]
fn invalid_scalar_edits_do_not_change_the_seed() {
    let source = "# 🐍\r\nlabel = 'é'\r";
    let seed = parse(source);
    for at in [
        source.find('🐍').unwrap() + 1,
        source.find('é').unwrap() + 1,
        source.len() + 1,
    ] {
        let changes = ChangeSet::single(Change::new(at..at, "a")).unwrap();
        assert!(seed.reparse(&changes).is_err());
    }
    assert_eq!(seed.text().as_str(), source);
}
