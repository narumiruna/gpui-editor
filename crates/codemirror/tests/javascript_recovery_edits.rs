//! Editor sequences from the SHA-512-verified JavaScript language and grammar.
//! Regenerate with `fixtures/javascript_recovery_edits.mjs`.

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    autocomplete::session::SourceResult,
    language::{
        IndentOptions, StreamSettings,
        javascript::{
            JavascriptDocument, JavascriptOptions, JavascriptScope, JavascriptScopeProperty,
            JavascriptScopeValue::{Function, Object, Value as Property},
            completion_path, local_completion_source, scope_completion_source,
        },
        lezer::ReuseStats,
    },
};
use serde_json::{Value, json};

fn number(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn check(doc: &JavascriptDocument, expected: &Value, context: &str) {
    let records: Vec<_> = doc
        .nodes()
        .map(|node| {
            let range = node.range();
            assert!(doc.text().slice(range.clone()).is_some(), "{context}");
            let depth = std::iter::successors(node.parent(), |parent| parent.parent()).count();
            json!([depth, node.kind().name(), range.start, range.end])
        })
        .collect();
    assert_eq!(json!(records), expected["nodes"], "tree: {context}");

    let lines: Vec<_> = (1..=doc.text().line_count())
        .map(|n| {
            let line = doc.text().line(n).unwrap();
            let indent = doc.indentation(
                line.range.start,
                StreamSettings::default(),
                IndentOptions::default(),
            );
            let fold = doc
                .foldable(line.range.start, line.range.end)
                .map(|range| [range.start, range.end]);
            json!([indent, fold])
        })
        .collect();
    assert_eq!(json!(lines), expected["lines"], "lines: {context}");

    let completion = |result: &SourceResult, include_boost| {
        let options: Vec<_> = result
            .options
            .iter()
            .map(|item| {
                if include_boost {
                    json!([item.label, item.kind, item.boost])
                } else {
                    json!([item.label, item.kind])
                }
            })
            .collect();
        json!([result.from, options])
    };
    let local = local_completion_source(doc, number(&expected["localPos"]), false);
    assert_eq!(
        json!(local.as_ref().map(|result| completion(result, false))),
        expected["local"],
        "local completion: {context}"
    );

    let pos = number(&expected["memberPos"]);
    let path = completion_path(doc, pos, false);
    assert_eq!(
        json!(path.map(|path| json!({"path": path.path, "name": path.name}))),
        expected["path"],
        "member path: {context}"
    );
    let property = |name, value| JavascriptScopeProperty { name, value };
    let theme = JavascriptScope {
        properties: &[
            property("field", Property),
            property("color", Function(None)),
        ],
        prototypes: &[],
    };
    let object = JavascriptScope {
        properties: &[property("theme", Object(&theme))],
        prototypes: &[],
    };
    let scope = JavascriptScope {
        properties: &[property("𐐀", Object(&object))],
        prototypes: &[],
    };
    let result = scope_completion_source(&scope)(doc, pos, false);
    assert_eq!(
        json!(result.as_ref().map(|result| completion(result, true))),
        expected["scope"],
        "scope completion: {context}"
    );
}

#[test]
fn recovers_scopes_and_delimiters_during_pinned_editor_sequences() {
    let fixture = include_str!("fixtures/javascript_recovery_edits.jsonl");
    for row in fixture.lines() {
        let fixture: Value = serde_json::from_str(row).unwrap();
        let mode = fixture["mode"].as_str().unwrap();
        let options = JavascriptOptions {
            typescript: mode.starts_with("ts"),
            jsx: mode.ends_with('x'),
        };
        let mut source = fixture["source"].as_str().unwrap().to_owned();
        let mut edited = JavascriptDocument::parse_with_options(Text::new(&source), options);
        for step in fixture["states"].as_array().unwrap() {
            let id = step["id"].as_str().unwrap();
            let context = format!("{mode}: {id}");
            let previous_recovered = edited.is_recovered();
            if let Some(edit) = step["edit"].as_array() {
                let range = number(&edit[0])..number(&edit[1]);
                let insert = edit[2].as_str().unwrap();
                edited = edited
                    .reparse(&ChangeSet::single(Change::new(range.clone(), insert)).unwrap())
                    .unwrap();
                source.replace_range(range, insert);
            }
            let fresh = JavascriptDocument::parse_with_options(Text::new(&source), options);
            let expected_recovery = step["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node[1] == "⚠");
            for doc in [&fresh, &edited] {
                assert_eq!(doc.text().as_str(), source, "text: {context}");
                assert_eq!(doc.is_recovered(), expected_recovery, "recovery: {context}");
                check(doc, step, &context);
            }
            assert!(
                edited
                    .highlights(0..source.len())
                    .unwrap()
                    .eq(fresh.highlights(0..source.len()).unwrap()),
                "highlights: {context}"
            );
            if previous_recovered || expected_recovery {
                assert_eq!(edited.reuse_stats(), ReuseStats::default(), "{context}");
            } else if id == "valid member edit" && mode == "js" {
                assert!(edited.reuse_stats().nodes_reused > 0, "{context}");
            }
        }
    }
}

#[test]
fn bounds_recovery_for_a_deep_incomplete_expression() {
    let source = format!("{}𐐀\r\n", "(".repeat(3_000));
    let doc =
        JavascriptDocument::parse_expression(Text::new(&source), JavascriptOptions::default());
    assert!(doc.is_recovered());
    assert_eq!(doc.reuse_stats(), ReuseStats::default());
    assert_eq!(doc.root().kind().name(), "SingleExpression");
    assert_eq!(doc.root().range(), 0..source.len());
    assert_eq!(doc.text().as_str(), source);
    assert!(doc.nodes().any(|node| node.kind().is_error()));
    assert!(doc.nodes().all(|node| source.get(node.range()).is_some()));
}
