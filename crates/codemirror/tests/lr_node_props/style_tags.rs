use std::ops::Range;

use gpui_editor_codemirror::language::{
    HighlightTag, TagModifier,
    lezer::{StyleTags, SyntaxNode, style_tags},
};

use super::*;

fn offset(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn names(tags: &[HighlightTag]) -> Vec<&str> {
    tags.iter()
        .map(|tag| {
            if tag.name() == "variableName" && tag.modifiers() == [TagModifier::Definition] {
                "definition(variableName)"
            } else {
                assert!(tag.modifiers().is_empty());
                tag.name()
            }
        })
        .collect()
}

fn rule(name: &str, range: Range<usize>, style: StyleTags<'_>) -> Value {
    json!([
        name,
        range.start,
        range.end,
        names(style.tags),
        style.opaque,
        style.inherit
    ])
}

fn raw_rules<'a>(node: SyntaxNode<'a>, ancestors: &mut Vec<&'a str>, rules: &mut Vec<Value>) {
    if let Some(style) = node.style_tags(ancestors) {
        rules.push(rule(node.name(), node.range(), style));
    }
    ancestors.push(node.name());
    for child in node.children() {
        raw_rules(child, ancestors, rules);
    }
    ancestors.pop();
}

fn editor_rules(doc: &SupportedDocument) -> Vec<Value> {
    match doc.syntax() {
        NativeSyntax::Json(doc) => doc
            .nodes()
            .filter_map(|node| Some(rule(node.kind().name(), node.range(), node.style_tags()?)))
            .collect(),
        NativeSyntax::LezerGrammar(doc) => doc
            .nodes()
            .filter_map(|node| Some(rule(node.kind().name(), node.range(), node.style_tags()?)))
            .collect(),
        _ => unreachable!(),
    }
}

// Native spans carry raw tags. Upstream can emit a new span when equal tags
// apply to adjacent nodes. Compare the ordered tag list on each continuous run.
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

fn check_clips(doc: &SupportedDocument, clips: &Value, mounted: bool, label: &str) {
    for clip in clips.as_array().unwrap() {
        let range = offset(&clip[0])..offset(&clip[1]);
        let spans = doc.highlights(range.clone()).unwrap();
        let repeated = doc.highlights(range.clone()).unwrap();
        for (span, other) in spans.iter().zip(&repeated) {
            assert!(range.start <= span.range.start && span.range.end <= range.end);
            assert!(ptr::eq(span.tags, other.tags));
        }
        let actual = join_runs(
            spans
                .iter()
                .filter(|span| !mounted || doc.highlight_context(&span.range).is_some())
                .map(|span| json!([span.range.start, span.range.end, names(span.tags)])),
        );
        assert_eq!(
            actual,
            join_runs(clip[2].as_array().unwrap().iter().cloned()),
            "{label}: {range:?}"
        );
    }
}

#[test]
fn lr_style_paths_spans_and_edits_match_pinned_parsers() {
    let rows: Vec<Value> = include_str!("../fixtures/lr_style_tags.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 10);
    for row in rows {
        let [stock, _] = handles(&row);
        let input = row["source"].as_str().unwrap();
        let step = row["step"].as_str().unwrap();
        let label = format!("{}: {step}", row["name"].as_str().unwrap());
        let sources: Vec<_> = row["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|source| {
                style_tags(source.as_array().unwrap().iter().map(|entry| {
                    (
                        entry[0].as_str().unwrap(),
                        entry[1]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|tag| HighlightTag::new(tag.as_str().unwrap()))
                            .collect(),
                    )
                }))
                .unwrap()
            })
            .collect();
        let parser = with_props(&stock, &sources);
        if step == "mount" {
            let pending = parser.clone();
            let state = EditorState::with_extensions(
                input,
                markdown_with_parser(
                    MarkdownConfig::default(),
                    MarkdownParserConfig {
                        code_languages: Some(MarkdownCodeLanguages::Configured {
                            resolve: Arc::new(move |info| {
                                (info == "selected").then(|| pending.clone())
                            }),
                            fallback: None,
                        }),
                        ..Default::default()
                    },
                ),
            )
            .unwrap();
            let snapshot = markdown_snapshot(&state).unwrap();
            check_clips(&snapshot, &row["clips"], true, &label);
            continue;
        }
        let raw = with_lr!(&parser, handle => handle.parse(input).unwrap());
        let mut raw_rows = Vec::new();
        raw_rules(raw.root(), &mut Vec::new(), &mut raw_rows);
        assert_eq!(json!(raw_rows), row["raw"], "{label}: raw rules");
        for strict in [false, true] {
            let extension =
                with_lr!(&parser, handle => handle.configure(Some(strict), None).extension());
            let mut state = EditorState::with_extensions(input, extension).unwrap();
            let before = attached_language_snapshot(&state).unwrap();
            assert_eq!(
                json!(editor_rules(&before)),
                row["rules"],
                "{label}: attached rules"
            );
            check_clips(&before, &row["clips"], false, &label);
            let at = offset(&row["edit"]);
            let edit = ChangeSet::single(Change::new(at..at + '名'.len_utf8(), "字字")).unwrap();
            state.update(Transaction::new(edit)).unwrap();
            let after = attached_language_snapshot(&state).unwrap();
            assert_eq!(after.text().as_str(), row["edited"]);
            check_clips(&after, &row["editClips"], false, &label);
            check_clips(&before, &row["clips"], false, &label);
            assert!(before.highlights(at + 1..input.len()).is_none());
        }
    }
}
