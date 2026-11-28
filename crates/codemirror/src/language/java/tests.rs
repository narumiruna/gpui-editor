//! Pinned Java fixtures from the SHA-512-verified npm archives. Offsets are
//! UTF-8 bytes. Syntax and highlights use the raw parse; indentation and folds
//! use CodeMirror's logical document mapped back to the original bytes.

use serde_json::{Value, json};

use crate::{Change, ChangeSet};

use super::*;

const SNAPSHOT_KEYS: [&str; 4] = ["tree", "spans", "lines", "points"];

#[test]
fn malformed_edits_match_pinned_syntax_and_services() {
    let mut previous: Option<JavaDocument> = None;
    for fixture in records(include_str!(
        "../../../tests/fixtures/java_edit_oracle.jsonl"
    )) {
        let id = fixture["id"].as_str().unwrap();
        let edit = fixture["edit"].as_array().map(|parts| {
            ChangeSet::single(Change::new(
                byte(&parts[0])..byte(&parts[1]),
                parts[2].as_str().unwrap(),
            ))
            .unwrap()
        });
        let fresh = parse_fixture(&fixture);
        let label = format!("{id}, repaired: {}", edit.is_some());
        assert_snapshot(&fresh, &fixture, &label);
        if let Some(changes) = edit {
            let changed = previous.as_ref().unwrap().reparse(&changes).unwrap();
            assert_eq!(changed.parse_error(), fresh.parse_error(), "{label}");
            assert_snapshot(&changed, &fixture, &label);
        }
        previous = Some(fresh);
    }
}

#[test]
fn generic_recovery_frontier_matches_pinned_edits() {
    let mut previous: Option<JavaDocument> = None;
    let mut cases = 0;
    for fixture in records(include_str!(
        "../../../tests/fixtures/java_generic_recovery.jsonl"
    )) {
        let edit = fixture["edit"].as_array();
        let label = format!(
            "{} {} repaired: {}",
            fixture["top"],
            fixture["id"],
            edit.is_some()
        );
        let fresh = parse_fixture(&fixture);
        assert_snapshot(&fresh, &fixture, &label);
        assert_bracket_probes(&fresh, &fixture, &label);

        let syntax = match fresh.mode {
            parser::Mode::Program => parser(),
            parser::Mode::ClassContent => class_content_parser(),
        };
        let source = fresh.text().as_str();
        if let Some(edit) = edit {
            let changes = ChangeSet::single(Change::new(
                byte(&edit[0])..byte(&edit[1]),
                edit[2].as_str().unwrap(),
            ))
            .unwrap();
            let repaired = previous.take().unwrap().reparse(&changes).unwrap();
            assert_snapshot(&repaired, &fixture, &label);
            assert_bracket_probes(&repaired, &fixture, &label);
            let strict = syntax.parse_glr(source).unwrap();
            assert_eq!(
                tree_snapshot(&parser::collect(&strict, None)),
                fixture["tree"],
                "{label} strict GLR"
            );
        } else {
            assert_eq!(fresh.reuse_stats(), ReuseStats::default(), "{label}");
            assert!(
                matches!(
                    syntax.parse_glr(source),
                    Err(ParseError::UnexpectedToken { .. })
                ),
                "{label}"
            );
            let recovered = syntax.parse_recovering(source).unwrap();
            assert_eq!(
                tree_snapshot(&parser::collect(&recovered, None)),
                fixture["tree"],
                "{label} direct recovery"
            );
            previous = Some(fresh);
        }
        cases += 1;
    }
    assert_eq!(cases, 24);
}

fn assert_bracket_probes(doc: &JavaDocument, fixture: &Value, label: &str) {
    for probe in fixture["hits"].as_array().unwrap() {
        let pos = byte(&probe[0]);
        let actual = doc.bracket_near(pos).map_or(Value::Null, |hit| {
            json!([
                [hit.start.start, hit.start.end],
                hit.end.map(|end| [end.start, end.end]),
                hit.matched
            ])
        });
        assert_eq!(actual, probe[1], "{label} bracket at byte {pos}");
    }
}

#[test]
fn bounded_recovery_above_the_checkpoint_cap_matches_the_pin() {
    let mut previous: Option<JavaDocument> = None;
    for mut fixture in records(include_str!(
        "../../../tests/fixtures/java_large_recovery.jsonl"
    )) {
        let source = format!(
            "/*é😀{}*/\r\n{}",
            "x".repeat(byte(&fixture["pad"])),
            fixture["tail"].as_str().unwrap()
        );
        fixture["source"] = json!(source);
        let name = fixture["id"].as_str().unwrap();
        let fresh = parse_fixture(&fixture);
        assert_snapshot(&fresh, &fixture, name);
        if let Some(edit) = fixture["edit"].as_array() {
            let before = previous.take().unwrap();
            let changes = ChangeSet::single(Change::new(
                byte(&edit[0])..byte(&edit[1]),
                edit[2].as_str().unwrap(),
            ))
            .unwrap();
            let repaired = before.reparse(&changes).unwrap();
            assert_snapshot(&repaired, &fixture, &format!("{name}: repaired"));
            if before.text().len() > 16 * 1024 {
                assert_eq!(repaired.reuse_stats(), ReuseStats::default(), "{name}");
            }
        } else {
            assert!(
                matches!(
                    fresh.parse_error(),
                    Some(ParseError::UnexpectedToken { .. })
                ),
                "{name}"
            );
            assert_eq!(fresh.reuse_stats(), ReuseStats::default(), "{name}");
            previous = Some(fresh);
        }
    }
}

#[test]
fn published_grammar_and_editor_cases_match_pinned_snapshots() {
    let fixture = include_str!("../../../tests/fixtures/java_pinned_grammar.jsonl");
    let mut rows = records(fixture);
    assert_node_properties(&rows.next().unwrap()["properties"]);
    let mut count = 0;
    for row in rows {
        let doc = parse_fixture(&row);
        assert_snapshot(&doc, &row, row["id"].as_str().unwrap());
        count += 1;
    }
    assert_eq!(count, 113 + 13);
}

fn assert_node_properties(expected: &Value) {
    let nodes: Vec<_> = (0..tables::NODE_NAMES.split(' ').count())
        .map(|term| Node {
            term: term.try_into().unwrap(),
            range: 0..0,
            parent: None,
            children: Vec::new(),
        })
        .collect();
    let actual: Vec<_> = (0..nodes.len())
        .map(|id| {
            let node = JavaNode { nodes: &nodes, id };
            json!([
                node.term(),
                node.name(),
                node.is_skipped(),
                node.group().into_iter().collect::<Vec<_>>(),
                node.opened_by().into_iter().collect::<Vec<_>>(),
                node.closed_by().into_iter().collect::<Vec<_>>(),
                node.isolates_direction().then_some("auto"),
                indent::rule(node).is_some(),
                fold_rule(node.name()).is_some()
            ])
        })
        .collect();
    assert_eq!(actual.len(), 210);
    assert_eq!(json!(actual), *expected);
}

// Generate input with `java_pinned_grammar.mjs <archive-root> --audit`, set
// JAVA_AUDIT_FIXTURE to that path and run this ignored test with --nocapture.
// This measures the current bounded recovery and GLR frontier, not a contract
// that these inputs must remain different from the pinned parser.
#[test]
#[ignore = "requires the pinned Java residual corpus from --audit"]
fn measure_pinned_java_residuals() {
    use std::collections::BTreeMap;

    #[derive(Debug, Default)]
    struct Counts {
        cases: usize,
        oracle_valid: usize,
        limits: usize,
        differences: [usize; SNAPSHOT_KEYS.len()],
        exact: usize,
        pruned: usize,
        pruned_tree_exact: usize,
    }

    let path = std::env::var_os("JAVA_AUDIT_FIXTURE").expect("set JAVA_AUDIT_FIXTURE");
    let corpus = std::fs::read_to_string(path).expect("read generated Java audit corpus");
    let mut counts = BTreeMap::<(String, &str), Counts>::new();
    for row in records(&corpus).skip(1) {
        let id = row["id"].as_str().unwrap();
        let group = if id.starts_with("delete:") || id.starts_with("insert:") {
            "mutations"
        } else if id.contains(".txt:") {
            "published"
        } else {
            match id.rsplit_once(' ').map(|(family, _)| family) {
                Some("adjacent generics") => "adjacent generics",
                Some("spaced generics") => "spaced generics",
                Some("cast") => "cast",
                Some("partial") => "partial",
                _ => "editor",
            }
        };
        let top = row["top"].as_str().unwrap().to_owned();
        let parser = if top == "Program" {
            parser()
        } else {
            class_content_parser()
        };
        let doc = parse_fixture(&row);
        let counter = counts.entry((top, group)).or_default();
        counter.cases += 1;
        let valid = !has_errors(&row);
        counter.oracle_valid += usize::from(valid);
        let source = doc.text().as_str();
        if valid && matches!(parser.parse(source), Err(ParseError::Ambiguous { .. })) {
            counter.pruned += 1;
            if let Ok(tree) = parser.parse_glr_pruned(source) {
                let nodes = parser::collect(&tree, None);
                counter.pruned_tree_exact += usize::from(tree_snapshot(&nodes) == row["tree"]);
            }
        }
        counter.limits += usize::from(matches!(
            doc.parse_error(),
            Some(ParseError::LimitExceeded { .. })
        ));
        let actual = snapshot(&doc, &row);
        let differences = SNAPSHOT_KEYS.map(|key| actual[key] != row[key]);
        if differences.iter().any(|&different| different) {
            let keys: Vec<_> = SNAPSHOT_KEYS
                .into_iter()
                .zip(differences)
                .filter_map(|(key, different)| different.then_some(key))
                .collect();
            println!(
                "{} {id}: {keys:?}, {:?}",
                doc.root().name(),
                doc.parse_error()
            );
        } else {
            counter.exact += 1;
        }
        for (count, different) in counter.differences.iter_mut().zip(differences) {
            *count += usize::from(different);
        }
    }
    println!("Differences are ordered {SNAPSHOT_KEYS:?}");
    for ((top, group), count) in counts {
        println!("{top:12} {group:18} {count:?}");
    }
}

fn records(input: &str) -> impl Iterator<Item = Value> {
    input
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn parse_fixture(fixture: &Value) -> JavaDocument {
    let text = Text::new(fixture["source"].as_str().unwrap());
    match fixture["top"].as_str().unwrap() {
        "Program" => JavaDocument::parse(text),
        "ClassContent" => JavaDocument::parse_class_content(text),
        top => panic!("Unknown Java top rule: {top}"),
    }
}

fn byte(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn has_errors(fixture: &Value) -> bool {
    fixture["tree"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node[1] == "⚠")
}

fn assert_snapshot(doc: &JavaDocument, fixture: &Value, label: &str) {
    assert_eq!(doc.text().as_str(), fixture["source"], "{label} raw text");
    assert_eq!(doc.is_recovered(), has_errors(fixture), "{label} recovery");
    let actual = snapshot(doc, fixture);
    for key in SNAPSHOT_KEYS {
        assert_eq!(actual[key], fixture[key], "{label} {key}");
    }
}

fn tree_snapshot(nodes: &[Node]) -> Value {
    let tree: Vec<_> = (0..nodes.len())
        .map(|id| {
            let node = JavaNode { nodes, id };
            let depth = std::iter::successors(node.parent(), |node| node.parent()).count();
            let range = node.range();
            json!([depth, node.name(), range.start, range.end])
        })
        .collect();
    json!(tree)
}

fn snapshot(doc: &JavaDocument, fixture: &Value) -> Value {
    let tree = tree_snapshot(&doc.nodes);
    let spans: Vec<_> = doc
        .highlights(0..doc.text().len())
        .unwrap()
        .map(|span| {
            let [tag] = span.tags else {
                panic!("One tag per Java span");
            };
            let mut name = tag.name().to_owned();
            for modifier in tag.modifiers() {
                name.push_str(&format!(".{modifier:?}").to_ascii_lowercase());
            }
            json!([span.range.start, span.range.end, name])
        })
        .collect();
    let indent = |pos, options| doc.indentation(pos, StreamSettings::default(), options);
    let lines: Vec<_> = (1..=doc.syntax_lines().line_count())
        .map(|number| {
            let range = doc.syntax_lines().line(number).unwrap().range;
            let fold = doc
                .foldable(range.start, range.end)
                .map(|range| [range.start, range.end]);
            json!([
                range.start,
                range.end,
                indent(range.start, IndentOptions::default()),
                fold
            ])
        })
        .collect();
    let points: Vec<_> = fixture["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|point| {
            let pos = byte(&point[0]);
            let split = IndentOptions {
                simulate_break: Some(pos),
                ..Default::default()
            };
            let double = IndentOptions {
                simulate_double_break: true,
                ..split
            };
            let normal = indent(pos, IndentOptions::default());
            if point.as_array().unwrap().len() == 4 {
                json!([pos, normal, indent(pos, split), indent(pos, double)])
            } else {
                json!([pos, normal, indent(pos, double)])
            }
        })
        .collect();
    json!({ "tree": tree, "spans": spans, "lines": lines, "points": points })
}
