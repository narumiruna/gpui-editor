use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    autocomplete::session::SourceResult,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        less::{LessDocument, LessNode, LessNodeKind, less_completion_source},
    },
};
use serde_json::{Value, json};

#[test]
fn missing_attribute_closers_match_pinned_less_services() {
    for line in include_str!("fixtures/less_attribute_recovery.jsonl").lines() {
        let record: Value = serde_json::from_str(line).unwrap();
        let name = record["name"].as_str().unwrap();
        let source = record["source"].as_str().unwrap();
        let base = LessDocument::parse(Text::new(source));
        check(&base, &record["before"], name, "before");
        let Some([from, to]) = record["change"].as_array().map(Vec::as_slice) else {
            continue;
        };
        let (from, to) = (offset(from), offset(to));
        let deleted = ChangeSet::single(Change::new(from..to, "")).unwrap();
        let edited = base.reparse(&deleted).unwrap();
        let mut after = source.to_owned();
        after.replace_range(from..to, "");
        assert_eq!(edited.text().as_str(), after, "{name}: edited source");
        let fresh = LessDocument::parse(Text::new(&after));
        check(&fresh, &record["edited"], name, "fresh");
        check(&edited, &record["edited"], name, "edited");

        let prefix = |doc: &LessDocument| {
            doc.nodes()
                .filter(|node| node.range().end < from)
                .map(node_row)
                .collect::<Vec<_>>()
        };
        assert_eq!(prefix(&base), prefix(&edited), "{name}: unchanged prefix");
        assert_eq!(edited.reuse_stats(), Default::default(), "{name}: recovery");
        let inserted = ChangeSet::single(Change::new(from..from, &source[from..to])).unwrap();
        let restored = edited.reparse(&inserted).unwrap();
        assert_eq!(restored.text().as_str(), source, "{name}: restored source");
        assert_eq!(
            restored.reuse_stats(),
            Default::default(),
            "{name}: restore"
        );
        check(&restored, &record["before"], name, "restored");
        check(&base, &record["before"], name, "retained");
    }
}

#[test]
fn other_bracket_errors_keep_the_existing_fallback() {
    // A missing name or value can also produce a single empty GLR error.
    // Do not accept that result as a missing closer.
    for source in [
        ".a[ { color: red; }",
        ".a[data-x= { color: red; }",
        ".a[data-x=\"x\", .b { color: red; }",
        ".a[data-x=\"x\" { color: red;",
        "@a: @theme[key { color: red; }",
        ".a[data-x=\"{ { color: red; }",
        ".a[data-x /* x { color: red; }",
        ".a[data-x=\"x\" { color: red; }\n.b[data-x=\"y\" { color: red; }",
    ] {
        let doc = LessDocument::parse(Text::new(source));
        assert!(
            doc.nodes().any(|node| node.kind() == LessNodeKind::Error),
            "{source:?}"
        );
        assert!(
            !doc.nodes().any(|node| node.kind() == LessNodeKind::RuleSet),
            "{source:?}"
        );
    }
}

fn check(doc: &LessDocument, expected: &Value, name: &str, phase: &str) {
    let nodes: Vec<_> = doc.nodes().map(node_row).collect();
    assert_eq!(json!(nodes), expected["nodes"], "{name}: {phase} tree");
    let highlights: Vec<_> = doc
        .highlights(0..doc.text().len())
        .unwrap()
        .map(|span| {
            let [tag] = span.tags else {
                panic!("expected one Less tag")
            };
            let suffix = match tag.modifiers() {
                [] => "",
                [TagModifier::Constant] => ".constant",
                [TagModifier::Special] => ".special",
                modifiers => panic!("unexpected Less tag modifiers: {modifiers:?}"),
            };
            json!([
                span.range.start,
                span.range.end,
                format!("{}{suffix}", tag.name())
            ])
        })
        .collect();
    assert_eq!(
        json!(highlights),
        expected["highlights"],
        "{name}: {phase} highlights"
    );
    for row in expected["indents"].as_array().unwrap() {
        let pos = offset(&row[0]);
        let indent = |simulate_break, simulate_double_break| {
            doc.indentation(
                pos,
                StreamSettings::default(),
                IndentOptions {
                    simulate_break,
                    simulate_double_break,
                    ..IndentOptions::default()
                },
            )
        };
        let actual = json!([
            pos,
            indent(None, false),
            indent(Some(pos), false),
            indent(Some(pos), true)
        ]);
        assert_eq!(actual, *row, "{name}: {phase} indent at {pos}");
    }
    for row in expected["completions"].as_array().unwrap() {
        let pos = offset(&row[0]);
        let complete = |explicit| less_completion_source(doc, pos, explicit).map(signature);
        assert_eq!(
            json!([pos, complete(false), complete(true)]),
            *row,
            "{name}: {phase} completion at {pos}"
        );
    }
}

fn node_row(node: LessNode<'_>) -> Value {
    let range = node.range();
    let depth = std::iter::successors(node.parent(), |node| node.parent()).count();
    json!([node.kind().name(), range.start, range.end, depth])
}

fn offset(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn signature(result: SourceResult) -> Value {
    // FNV-1a over ordered labels, types, and insert text.
    let mut hash = 2_166_136_261u32;
    for item in result.options.iter() {
        for byte in item
            .label
            .bytes()
            .chain([0])
            .chain(item.kind.as_deref().unwrap().bytes())
            .chain([0])
            .chain(item.insert.as_deref().unwrap_or_default().bytes())
            .chain([b'\n'])
        {
            hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
        }
    }
    json!([result.from, result.to, result.options.len(), hash])
}
