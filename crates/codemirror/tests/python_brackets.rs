use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        brackets::{BracketConfig, BracketMatch, bracket_near},
        lezer::ReuseStats,
        python::PythonDocument,
    },
};
use serde_json::{Value, json};

fn record(hit: Option<BracketMatch>) -> Value {
    hit.map_or(Value::Null, |hit| {
        json!([
            [hit.start.start, hit.start.end],
            hit.end.map(|end| [end.start, end.end]),
            hit.matched
        ])
    })
}

fn hit_at(probes: &Value, head: usize) -> &Value {
    probes
        .as_array()
        .unwrap()
        .iter()
        .find(|probe| probe[0] == head)
        .map_or(&Value::Null, |probe| &probe[1])
}

#[test]
fn python_bracket_feedback_matches_the_pinned_editor_across_edits() {
    let mut document = PythonDocument::parse(Text::new(""));
    for line in include_str!("fixtures/python_brackets.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        document = if let Some(source) = row["source"].as_str() {
            PythonDocument::parse(Text::new(source))
        } else {
            let (from, to, insert): (usize, usize, String) =
                serde_json::from_value(row["change"].clone()).unwrap();
            let changes = ChangeSet::single(Change::new(from..to, insert)).unwrap();
            document.reparse(&changes).unwrap()
        };
        let text = document.text();
        let fresh = PythonDocument::parse(text.clone());
        for doc in [&document, &fresh] {
            assert_eq!(doc.is_recovered(), row["recovered"], "{name}");
            // Also check every byte without a pinned hit, including the
            // interior of CRLF and multibyte scalars.
            for head in 0..=text.len() {
                let actual = record(doc.bracket_near(head));
                assert_eq!(&actual, hit_at(&row["hits"], head), "{name} at byte {head}");
            }
            assert!(doc.bracket_near(text.len() + 1).is_none(), "{name}");
        }
        for probe in row["plain"].as_array().unwrap() {
            let head = probe[0].as_u64().unwrap() as usize;
            let raw = record(bracket_near(
                text,
                head,
                BracketConfig::default(),
                |_, _| true,
            ));
            assert_eq!(raw, probe[1], "{name} at byte {head}");
            assert_ne!(&raw, hit_at(&row["hits"], head), "{name} at byte {head}");
        }
    }
}

#[test]
fn stray_parenthesis_keeps_later_syntax_without_checkpoints() {
    let seed = PythonDocument::parse(Text::new("0\n\n[]"));
    let change = ChangeSet::single(Change::new(2..2, ")")).unwrap();
    let edited = seed.reparse(&change).unwrap();
    let no_edit = ChangeSet::default();
    let same = edited.reparse(&no_edit).unwrap();
    assert!(seed.reparse(&no_edit).unwrap().reuse_stats().nodes_reused > 0);
    for doc in [&edited, &same] {
        assert!(doc.is_recovered());
        assert_eq!(doc.reuse_stats(), ReuseStats::default());
        assert_eq!(
            doc.nodes()
                .map(|node| (node.name(), node.range(), node.parent().map(|p| p.name())))
                .collect::<Vec<_>>(),
            [
                ("Script", 0..6, None),
                ("ExpressionStatement", 0..1, Some("Script")),
                ("Number", 0..1, Some("ExpressionStatement")),
                ("ExpressionStatement", 2..3, Some("Script")),
                ("TupleExpression", 2..3, Some("ExpressionStatement")),
                ("⚠", 2..2, Some("TupleExpression")),
                (")", 2..3, Some("TupleExpression")),
                ("ExpressionStatement", 4..6, Some("Script")),
                ("ArrayExpression", 4..6, Some("ExpressionStatement")),
                ("[", 4..5, Some("ArrayExpression")),
                ("]", 5..6, Some("ArrayExpression")),
            ]
        );
    }
    let restored = edited
        .reparse(&change.inverted(seed.text()).unwrap())
        .unwrap();
    assert!(!restored.is_recovered());
    assert_eq!(restored.reuse_stats(), ReuseStats::default());
    assert_eq!(restored.text().as_str(), seed.text().as_str());
}

#[test]
fn wrong_call_closer_keeps_the_array_and_error_without_checkpoints() {
    let seed = PythonDocument::parse(Text::new("f([1]) + [2]"));
    let edit = ChangeSet::single(Change::new(5..6, "]")).unwrap();
    let recovered = seed.reparse(&edit).unwrap();
    let fresh = PythonDocument::parse(recovered.text().clone());
    for doc in [&recovered, &fresh] {
        assert!(doc.is_recovered());
        assert_eq!(doc.reuse_stats(), ReuseStats::default());
        let error = doc.nodes().find(|node| node.name() == "⚠").unwrap();
        assert_eq!(error.range(), 5..6);
        assert_eq!(error.parent().map(|node| node.name()), Some("ArgList"));
        assert_eq!(
            error.children().map(|node| node.name()).collect::<Vec<_>>(),
            ["]"]
        );
        assert_eq!(
            doc.bracket_near_with_config(6, BracketConfig::default()),
            Some(BracketMatch {
                start: 5..6,
                end: None,
                matched: false,
            })
        );
    }
    let repaired = recovered
        .reparse(&edit.inverted(seed.text()).unwrap())
        .unwrap();
    assert_eq!(repaired.text(), seed.text());
    assert!(!repaired.is_recovered());
    assert_eq!(repaired.reuse_stats(), ReuseStats::default());
}

#[test]
fn closer_repair_keeps_prefix_fallback_outside_its_scope() {
    for source in [
        "0\n)\n \t\n",  // No later nonblank line.
        "0\n); []",     // Later code on the same line.
        "0\n42)\n[]",   // Deleted token, not a recovered tuple.
        "0\nf([1)\n[]", // ')' closes a call after a missing ']'.
        "0\nf([]]\n[]", // Extra recovery after the wrong call closer.
        "0\nx=[(1]]",   // Unexpected ']' outside a call.
        "0\nf([]]]",    // More than one bad call closer.
        "0\n)\n[",      // Extra recovery at EOF.
        "0\n)\n[]\n?",  // Extra recovery after a valid later line.
    ] {
        let doc = PythonDocument::parse(Text::new(source));
        assert!(doc.is_recovered(), "{source:?}");
        assert_eq!(
            doc.root()
                .children()
                .map(|node| (node.name(), node.range()))
                .collect::<Vec<_>>(),
            [("ExpressionStatement", 0..1), ("⚠", 2..source.len())],
            "{source:?}"
        );
    }
}

#[test]
fn stray_parenthesis_repair_obeys_the_normalized_repair_cap() {
    let head = "0\r\n)\r\n[]\r\n#";
    let normalized_len = head.len() - head.matches('\r').count();
    let cap = 64 * 1024;
    for (len, within_cap) in [(cap, true), (cap + 1, false)] {
        let source = format!("{head}{}", " ".repeat(len - normalized_len));
        let doc = PythonDocument::parse(Text::new(source));
        assert!(doc.is_recovered());
        assert_eq!(
            doc.bracket_near(6),
            within_cap.then_some(BracketMatch {
                start: 6..7,
                end: Some(7..8),
                matched: true,
            }),
            "{len} normalized bytes"
        );
    }
}

#[test]
fn only_unmarked_brackets_use_the_normalized_utf16_scan_limit() {
    for separator in ["\r\n", "\r"] {
        for count in [4999, 5000] {
            let source = format!("# ({}{separator}# )", "😀".repeat(count));
            let end = source.len();
            let doc = PythonDocument::parse(Text::new(source));
            assert!(!doc.is_recovered());
            assert_eq!(
                doc.bracket_near(2),
                (count == 4999).then_some(BracketMatch {
                    start: 2..3,
                    end: Some(end - 1..end),
                    matched: true,
                })
            );
        }
    }
    let structural = format!("({}\r\n1)", " ".repeat(10_001));
    let end = structural.len();
    let doc = PythonDocument::parse(Text::new(structural));
    assert!(!doc.is_recovered());
    assert_eq!(
        doc.bracket_near(0),
        Some(BracketMatch {
            start: 0..1,
            end: Some(end - 1..end),
            matched: true,
        })
    );
}
