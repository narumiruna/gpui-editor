use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        IndentOptions, StreamSettings,
        yaml::{YamlDocument, YamlNode, YamlNodeKind},
    },
};
use serde_json::{Value, json};

#[test]
fn yaml_indents_and_folds_mixed_line_endings_in_original_bytes() {
    for row in include_str!("fixtures/yaml_editor_lines.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let source = case["source"].as_str().unwrap();
        let doc = YamlDocument::parse(Text::new(source));
        let settings = StreamSettings::default();
        let options = IndentOptions::default();
        assert_eq!(doc.text().as_str(), source);
        for line in case["lines"].as_array().unwrap() {
            let (start, end, fold, indent, end_indent): (
                usize,
                usize,
                Option<[usize; 2]>,
                Option<usize>,
                Option<usize>,
            ) = serde_json::from_value(line.clone()).unwrap();
            assert_eq!(
                doc.foldable(start, end),
                fold.map(|[a, b]| a..b),
                "{source:?} byte {start}"
            );
            assert_eq!(
                doc.indentation(start, settings, options),
                indent,
                "{source:?} byte {start}"
            );
            assert_eq!(
                doc.indentation(end, settings, options),
                end_indent,
                "{source:?} byte {end}"
            );
        }
        let lines: Vec<_> = (1..=doc.syntax_lines().line_count())
            .map(|number| {
                let range = doc.syntax_lines().line(number).unwrap().range;
                json!([range.start, range.end])
            })
            .collect();
        let pinned: Vec<_> = case["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| json!([line[0], line[1]]))
            .collect();
        assert_eq!(lines, pinned, "{source:?}");
    }
}

#[test]
fn yaml_document_after_comments_keeps_in_bounds_node_ranges() {
    let source = "a: x\n...\n# pre\n# 📦\nb: y";
    let doc = YamlDocument::parse(Text::new(source));
    let document = doc
        .root()
        .children()
        .iter()
        .filter(|node| node.kind() == YamlNodeKind::Document)
        .nth(1)
        .unwrap();
    // Match the pinned parser without TreeBuffer's wrapped start offset.
    assert_eq!(document.range(), 8..source.len());
    assert_eq!(document.children()[0].range(), document.range());
    assert_eq!(document.children()[0].children()[0].range(), 15..21);
}

#[test]
fn yaml_flow_recovery_indentation_matches_pinned_editor() {
    let mut previous: Option<YamlDocument> = None;
    for row in include_str!("fixtures/yaml_flow_recovery_indent.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let name = case["name"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let expected_errors: Vec<[usize; 2]> =
            serde_json::from_value(case["errors"].clone()).unwrap();
        let indents: Vec<(usize, Option<usize>)> =
            serde_json::from_value(case["indents"].clone()).unwrap();
        let newline_indent: Option<usize> =
            serde_json::from_value(case["newlineIndent"].clone()).unwrap();
        let change: Option<(usize, usize, String)> =
            serde_json::from_value(case["change"].clone()).unwrap();
        let fresh = YamlDocument::parse(Text::new(source));
        let updated = change.map(|(from, to, insert)| {
            let changes = ChangeSet::single(Change::new(from..to, insert)).unwrap();
            previous.as_ref().unwrap().reparse(&changes).unwrap()
        });
        for doc in std::iter::once(&fresh).chain(&updated) {
            assert_eq!(doc.text().as_str(), source, "{name}");
            let mut errors = Vec::new();
            collect_errors(doc.root(), &mut errors);
            assert_eq!(errors, expected_errors, "{name}");
            let settings = StreamSettings::default();
            for &(at, indent) in &indents {
                assert_eq!(
                    doc.indentation(at, settings, IndentOptions::default()),
                    indent,
                    "{name} at byte {at}"
                );
            }
            let eof = source.len();
            assert_eq!(
                doc.indentation(
                    eof,
                    settings,
                    IndentOptions {
                        simulate_break: Some(eof),
                        ..IndentOptions::default()
                    },
                ),
                newline_indent,
                "{name} with a simulated newline at EOF"
            );
        }
        previous = Some(updated.unwrap_or(fresh));
    }
}

fn collect_errors(node: &YamlNode, errors: &mut Vec<[usize; 2]>) {
    if node.kind() == YamlNodeKind::Error {
        let range = node.range();
        errors.push([range.start, range.end]);
    }
    for child in node.children() {
        collect_errors(child, errors);
    }
}
