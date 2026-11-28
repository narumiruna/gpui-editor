use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        IndentOptions, StreamSettings,
        lezer::ParseError,
        sql::{
            SqlDialect, SqlDocument, SqlNamespace, SqlNodeKind, SqlSchemaCompletionSource,
            SqlSchemaConfig, schema_completion_source,
        },
    },
};
use serde_json::{Value, json};

fn offset(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn edit(from: usize, to: usize, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(from..to, insert)).unwrap()
}

fn statements(doc: &SqlDocument) -> Vec<&str> {
    doc.root()
        .children()
        .filter(|node| node.kind() == SqlNodeKind::Statement)
        .map(|node| doc.text().slice(node.range()).unwrap())
        .collect()
}

fn observe(doc: &SqlDocument, probes: &Value, schema: &SqlSchemaCompletionSource) -> Value {
    let tree: Vec<_> = doc
        .nodes()
        .map(|node| {
            let range = node.range();
            let depth = std::iter::successors(node.parent(), |node| node.parent()).count();
            json!([node.kind().name(), range.start, range.end, depth])
        })
        .collect();
    let lines: Vec<_> = (1..=doc.text().line_count())
        .map(|number| {
            let line = doc.text().line(number).unwrap().range;
            let indent = doc.indentation(
                line.start,
                StreamSettings::default(),
                IndentOptions::default(),
            );
            let fold = doc.foldable(line.start, line.end).map(|r| [r.start, r.end]);
            json!([line.start, line.end, indent, fold])
        })
        .collect();
    let completions: Vec<_> = probes["completions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let pos = offset(&row[0]);
            let found = schema.complete(doc, pos, false).map(|result| {
                let options: Vec<_> = result
                    .options
                    .iter()
                    .map(|option| json!([option.label, option.kind]))
                    .collect();
                json!([result.from, result.to.unwrap_or(pos), options])
            });
            json!([pos, found])
        })
        .collect();
    json!({"source": doc.text().as_str(), "tree": tree, "lines": lines, "completions": completions})
}

#[test]
fn semicolon_edit_fresh_parse_and_repair_match_pinned_services() {
    let schema = SqlNamespace::Namespaces(vec![(
        "users".to_owned(),
        SqlNamespace::Completions(vec!["id".into()]),
    )]);
    for line in include_str!("fixtures/sql_semicolon_recovery.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let name = case["name"].as_str().unwrap();
        let dialect = SqlDialect::from_export(case["dialect"].as_str().unwrap()).unwrap();
        let language = dialect.language();
        let schema = schema_completion_source(SqlSchemaConfig {
            dialect: dialect.spec(),
            schema: Some(&schema),
            ..SqlSchemaConfig::default()
        });
        let at = offset(&case["editAt"]);
        let before = &case["before"];
        let after = &case["after"];
        let original = language.parse(Text::new(before["source"].as_str().unwrap()));
        let edited = language.reparse(&original, &edit(at, at, ";")).unwrap();
        let fresh = language.parse(Text::new(after["source"].as_str().unwrap()));
        let repaired = language.reparse(&edited, &edit(at, at + 1, "")).unwrap();
        for (doc, expected, stage) in [
            (&original, before, "original"),
            (&edited, after, "edited"),
            (&fresh, after, "fresh"),
            (&repaired, before, "repaired"),
        ] {
            assert_eq!(
                observe(doc, expected, &schema),
                *expected,
                "{name}: {stage}"
            );
        }
        assert!(matches!(
            edited.parse_error(),
            Some(ParseError::UnexpectedToken { offset, .. }) if offset == at
        ));
        assert_eq!(edited.parse_error(), fresh.parse_error(), "{name}");
        assert!(!original.is_recovered());
        assert!(!repaired.is_recovered());
    }
}

#[test]
fn other_malformed_inputs_keep_native_statement_boundaries() {
    for (source, expected) in [
        (
            "SELECT (u.i;\nFROM users u;",
            vec!["SELECT (u.i;", "FROM users u;"],
        ),
        (
            "SELECT (u.i;]\nFROM users u;",
            vec!["SELECT (u.i;", "FROM users u;"],
        ),
        (
            "SELECT (u.;)\nFROM users u;",
            vec!["SELECT (u.;", "FROM users u;"],
        ),
        (
            "SELECT (1; 2;)\nSELECT 3;",
            vec!["SELECT (1;", "2;", "SELECT 3;"],
        ),
        (
            "SELECT {1; 2};\nSELECT 3;",
            vec!["SELECT {1;", "2};", "SELECT 3;"],
        ),
        (
            "SELECT (u.i;)\nFROM users u; SELECT (2",
            vec!["SELECT (u.i;", "FROM users u;", "SELECT (2"],
        ),
    ] {
        let doc = SqlDocument::parse(Text::new(source));
        assert!(doc.is_recovered(), "{source}");
        assert_eq!(statements(&doc), expected, "{source}");
        let group = doc
            .root()
            .children()
            .next()
            .unwrap()
            .children()
            .find(|node| matches!(node.kind(), SqlNodeKind::Parens | SqlNodeKind::Braces))
            .unwrap();
        let last = group.children().next_back().unwrap();
        assert_eq!(last.kind(), SqlNodeKind::Error, "{source}");
        let at = source.find(';').unwrap();
        assert_eq!(last.range(), at..at, "{source}");
    }
}

#[test]
fn oversized_input_uses_native_recovery() {
    let source = format!("SELECT (u.i;){}\nFROM users u;", " ".repeat(20_000));
    let doc = SqlDocument::parse(Text::new(source));
    assert!(matches!(
        doc.parse_error(),
        Some(ParseError::UnexpectedToken { offset: 11, .. })
    ));
    assert_eq!(statements(&doc), ["SELECT (u.i;", "FROM users u;"]);
}

#[test]
fn repairing_recovered_input_does_not_reuse_a_prefix() {
    let language = SqlDialect::StandardSql.language();
    let prefix = "SELECT 1;\n".repeat(192);
    let source = format!("{prefix}SELECT (u.i)\nFROM users u;");
    let original = language.parse(Text::new(source.clone()));
    let at = prefix.len() + 11;
    let regular_edit = language.reparse(&original, &edit(at - 1, at, "j")).unwrap();
    assert!(regular_edit.reuse_stats().resumed_at > 0);

    let recovered = language.reparse(&original, &edit(at, at, ";")).unwrap();
    assert!(recovered.is_recovered());
    assert_eq!(recovered.reuse_stats(), Default::default());
    let recovered_statements = statements(&recovered);
    assert_eq!(recovered_statements.len(), 193);
    assert_eq!(
        recovered_statements.last(),
        Some(&"SELECT (u.i;)\nFROM users u;")
    );

    let repaired = language.reparse(&recovered, &edit(at, at + 1, "")).unwrap();
    assert!(!repaired.is_recovered());
    assert_eq!(repaired.text().as_str(), source);
    assert_eq!(repaired.reuse_stats(), Default::default());
}
