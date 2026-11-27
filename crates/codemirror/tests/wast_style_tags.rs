//! Replay exact selector and highlight results from `@lezer/highlight` 1.2.5.

use std::ptr;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorState, Transaction,
    language::{
        HighlightTag,
        lezer::SyntaxNode,
        wast::{WastNodeKind, style_tags},
    },
    language_data::{NativeSyntax, attached_language_snapshot, wast_language},
};
use serde_json::{Value, json};

fn items(value: &Value) -> &[Value] {
    value.as_array().unwrap()
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn offset(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn names(tags: &[HighlightTag]) -> Vec<&str> {
    tags.iter().map(HighlightTag::name).collect()
}

// Lezer emits classes; native code emits raw tags. Join touching runs with
// identical tag lists on both sides, without changing tag order or duplicates.
fn join_runs(rows: impl IntoIterator<Item = Value>) -> Vec<Value> {
    let mut joined: Vec<Value> = Vec::new();
    for row in rows {
        if let Some(last) = joined.last_mut() {
            assert!(offset(&last[1]) <= offset(&row[0]));
            if last[1] == row[0] && last[2] == row[2] {
                last[1] = row[1].clone();
                continue;
            }
        }
        joined.push(row);
    }
    joined
}

fn raw_rules<'a>(node: SyntaxNode<'a>, ancestors: &mut Vec<&'a str>, rows: &mut Vec<Value>) {
    if let Some(rule) = node.wast_style_tags(ancestors) {
        let range = node.range();
        rows.push(json!([
            node.name(),
            range.start,
            range.end,
            names(rule.tags),
            rule.opaque,
            rule.inherit
        ]));
    }
    ancestors.push(node.name());
    for child in node.children() {
        raw_rules(child, ancestors, rows);
    }
    ancestors.pop();
}

#[test]
fn configured_snapshots_match_pinned_rules_and_clipped_highlights() {
    for line in include_str!("fixtures/wast_style_tags.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let sources: Vec<_> = items(&row["sources"])
            .iter()
            .map(|source| {
                style_tags(items(source).iter().map(|entry| {
                    let tags = items(&entry[1])
                        .iter()
                        .map(|tag| HighlightTag::new(text(tag)));
                    (text(&entry[0]), tags.collect())
                }))
                .unwrap()
            })
            .collect();
        let configured = wast_language().with_node_props(&sources);
        let doc = text(&row["source"]);
        for language in [configured.clone(), configured.configure(Some(true), None)] {
            let raw = language.parse(doc).unwrap();
            let mut raw_rows = Vec::new();
            raw_rules(raw.root(), &mut Vec::new(), &mut raw_rows);
            assert_eq!(json!(raw_rows), row["rules"], "{}: raw rules", row["name"]);
            let state = EditorState::with_extensions(doc, language.extension()).unwrap();
            let snapshot = attached_language_snapshot(&state).unwrap();
            let NativeSyntax::Wast(wast) = snapshot.syntax() else {
                panic!("expected WAST")
            };
            let rules: Vec<_> = wast
                .nodes()
                .filter_map(|node| {
                    let rule = node.style_tags()?;
                    let range = node.range();
                    Some(json!([
                        node.kind().name(),
                        range.start,
                        range.end,
                        names(rule.tags),
                        rule.opaque,
                        rule.inherit
                    ]))
                })
                .collect();
            assert_eq!(json!(rules), row["rules"], "{}", row["name"]);
            for clip in items(&row["clips"]) {
                let range = offset(&clip[0])..offset(&clip[1]);
                let spans = snapshot.highlights(range.clone()).unwrap();
                let again = snapshot.highlights(range.clone()).unwrap();
                assert_eq!(spans, again);
                for (a, b) in spans.iter().zip(&again) {
                    assert!(range.start <= a.range.start && a.range.end <= range.end);
                    assert!(ptr::eq(a.tags, b.tags));
                }
                let actual = join_runs(
                    spans
                        .iter()
                        .map(|span| json!([span.range.start, span.range.end, names(span.tags)])),
                );
                assert_eq!(
                    actual,
                    join_runs(items(&clip[2]).iter().cloned()),
                    "{}: {range:?}",
                    row["name"]
                );
            }
            let not_a_boundary = doc.find('é').unwrap() + 1;
            assert!(snapshot.highlights(not_a_boundary..doc.len()).is_none());
            assert!(snapshot.highlights(0..not_a_boundary).is_none());
            assert!(snapshot.highlights(doc.len() + 1..doc.len()).is_none());
            assert!(snapshot.highlights(0..doc.len() + 1).is_none());
        }
    }
}

#[test]
fn validates_selector_syntax() {
    for path in [
        "/App",
        "App/",
        "App//Keyword",
        "App/*",
        "App!!",
        "\"\"",
        r#""\q""#,
    ] {
        assert_eq!(
            style_tags([(path, vec![])]).err().unwrap().to_string(),
            format!("invalid WAST style path: {path}")
        );
    }
    assert!(style_tags([(r#""/" "!" "*" "...""#, vec![])]).is_ok());
}

#[test]
fn effective_tags_and_spans_stay_borrowed_from_their_snapshot_after_an_edit() {
    let source = style_tags([("App/Keyword", vec![HighlightTag::new("atom")])]).unwrap();
    let language = wast_language().with_node_props(&[source]);
    let mut state = EditorState::with_extensions("(module)", language.extension()).unwrap();
    let before = attached_language_snapshot(&state).unwrap();
    let styled = |snapshot: &gpui_editor_codemirror::language_data::SupportedDocument| {
        let NativeSyntax::Wast(wast) = snapshot.syntax() else {
            panic!("expected WAST")
        };
        let node = wast
            .nodes()
            .find(|node| node.kind() == WastNodeKind::Keyword)
            .unwrap();
        let spans = snapshot.highlights(node.range()).unwrap();
        let [span] = spans.as_slice() else {
            panic!("expected one span")
        };
        let selected = node.style_tags().unwrap();
        assert_eq!(names(span.tags), ["atom"]);
        assert!(ptr::eq(span.tags, selected.tags));
        span.range.clone()
    };
    assert_eq!(styled(&before), 1..7);
    let changes = ChangeSet::new(vec![Change::new(1..7, "func")]).unwrap();
    state.update(Transaction::new(changes)).unwrap();
    let after = attached_language_snapshot(&state).unwrap();
    assert_eq!(styled(&after), 1..5);
    assert_eq!(styled(&before), 1..7);
}

#[test]
fn deep_inheritance_only_flattens_tags_for_spans_the_query_visits() {
    let source = "(".repeat(4_000);
    let props = style_tags([("App/...", vec![HighlightTag::new("emphasis")])]).unwrap();
    let language = wast_language().with_node_props(&[props]);
    let state = EditorState::with_extensions(&source, language.extension()).unwrap();
    let snapshot = attached_language_snapshot(&state).unwrap();
    let spans = snapshot.highlights(1_500..1_501).unwrap();
    let [span] = spans.as_slice() else {
        panic!("expected one span")
    };
    let (last, inherited) = span.tags.split_last().unwrap();
    assert_eq!(span.range, 1_500..1_501);
    assert_eq!(last.name(), "paren");
    assert_eq!(inherited.len(), 1_501);
    assert!(inherited.iter().all(|tag| tag.name() == "emphasis"));
}
