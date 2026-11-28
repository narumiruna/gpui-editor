use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        css::CssDocument,
        html::{
            HtmlAttributes, HtmlEmbeddedDocument, HtmlEmbeddedLanguage, HtmlNestedAttribute,
            HtmlNestedLanguage, HtmlNestedParser, HtmlNesting, HtmlNodeKind, HtmlOptions,
        },
        json::JsonDocument,
        liquid::{LiquidDocument, LiquidOptions},
    },
};
use serde_json::{Value, json};

fn offset(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn bounds(range: Range<usize>) -> [usize; 2] {
    [range.start, range.end]
}

fn assert_snapshot(doc: &LiquidDocument, expected: &Value) {
    let html = doc.html().unwrap();
    let compact = html.document();
    let source = doc.text().as_str();
    let pieces = |range| html.source_ranges(range).unwrap();
    let ranges: Vec<_> = html
        .ranges()
        .iter()
        .map(|r| bounds(r.source.clone()))
        .collect();
    assert_eq!(json!(ranges), expected["ranges"]);
    let mounts: Vec<_> = compact
        .mounts()
        .iter()
        .map(|mount| {
            let (name, text) = match (mount.language(), mount.document()) {
                (HtmlEmbeddedLanguage::Custom("json"), HtmlEmbeddedDocument::Custom(child)) => {
                    ("json", child.downcast_ref::<JsonDocument>().unwrap().text())
                }
                (HtmlEmbeddedLanguage::JavaScript, HtmlEmbeddedDocument::Javascript(child)) => {
                    ("JavaScript", child.text())
                }
                pair => panic!("unexpected mount: {pair:?}"),
            };
            let pieces: Vec<_> = pieces(mount.range()).collect();
            let projected: String = pieces.iter().map(|r| &source[r.clone()]).collect();
            assert_eq!(text.as_str(), projected);
            (name, pieces.into_iter().map(bounds).collect::<Vec<_>>())
        })
        .collect();
    assert_eq!(json!(mounts), expected["mounts"]);
    let self_closing: Vec<_> = compact
        .nodes()
        .filter(|node| node.kind() == HtmlNodeKind::SelfClosingTag)
        .map(|node| pieces(node.range()).map(bounds).collect::<Vec<_>>())
        .collect();
    assert_eq!(json!(self_closing), expected["selfClosing"]);

    let mut highlights: Vec<(usize, usize, String)> = Vec::new();
    for span in doc.highlights(0..source.len()).unwrap() {
        let style = span
            .tags
            .iter()
            .map(|tag| match tag.modifiers() {
                [] => tag.name().to_owned(),
                [TagModifier::Definition] => format!("{}.definition", tag.name()),
                other => panic!("unexpected modifiers: {other:?}"),
            })
            .collect::<Vec<_>>()
            .join("+");
        let Range { start, end } = span.range;
        if let Some(last) = highlights.last_mut()
            && last.1 == start
            && last.2 == style
        {
            last.1 = end;
        } else {
            highlights.push((start, end, style));
        }
    }
    let want = expected["highlights"].as_array().unwrap();
    assert_eq!(highlights.len(), want.len());
    for (span, expected) in highlights.into_iter().zip(want) {
        assert_eq!(json!(span), *expected, "at byte {}", span.0);
    }
    let folds: Vec<_> = (1..=doc.text().line_count())
        .map(|i| {
            let line = doc.text().line(i).unwrap().range;
            doc.foldable(line.start, line.end).map(bounds)
        })
        .collect();
    assert_eq!(json!(folds), expected["folds"]);
    for entry in expected["indents"].as_array().unwrap() {
        let at = offset(&entry[0]);
        let indent = doc.indentation(at, StreamSettings::default(), IndentOptions::default());
        assert_eq!(json!(indent), entry[1], "indent at byte {at}");
    }
}

#[test]
fn matches_pinned_configured_html_before_and_after_edit() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/liquid_custom_html.jsonl")).unwrap();
    let source = fixture["source"].as_str().unwrap();
    let options = LiquidOptions {
        html: HtmlOptions {
            self_closing_tags: true,
            ..HtmlOptions::default()
        },
    };
    let json = HtmlNestedParser {
        name: "json",
        parse: &|text| Box::new(JsonDocument::parse(text)),
    };
    let is_data = |attrs: &HtmlAttributes<'_>| attrs.get("mode") == Some(&"data");
    let nesting = HtmlNesting {
        nested_languages: &[
            HtmlNestedLanguage {
                tag: "x-data",
                attrs: None,
                parser: json,
            },
            HtmlNestedLanguage {
                tag: "script",
                attrs: Some(&is_data),
                parser: json,
            },
            HtmlNestedLanguage {
                tag: "script",
                attrs: Some(&is_data),
                parser: HtmlNestedParser {
                    name: "shadowed",
                    parse: &|text| Box::new(CssDocument::parse(text)),
                },
            },
        ],
        nested_attributes: &[HtmlNestedAttribute {
            name: "onclick",
            tag_name: Some("x-data"),
            parser: json,
        }],
    };

    let doc = LiquidDocument::parse_with_nesting(Text::new(source), options, nesting);
    assert_snapshot(&doc, &fixture["before"]);
    let [from, to, insert] = fixture["edit"].as_array().unwrap().as_slice() else {
        panic!("expected one edit");
    };
    let edit = ChangeSet::single(Change::new(
        offset(from)..offset(to),
        insert.as_str().unwrap(),
    ))
    .unwrap();
    let edited = doc.reparse_with_nesting(&edit, nesting).unwrap();
    assert!(edited.reuse_stats().nodes_reused > 0);
    assert_snapshot(&edited, &fixture["after"]);
    let fresh = LiquidDocument::parse_with_nesting(edited.text().clone(), options, nesting);
    assert_snapshot(&fresh, &fixture["after"]);

    let stock = doc.reparse(&edit).unwrap();
    let html = stock.html().unwrap().document();
    assert_eq!(
        html.mounts()
            .iter()
            .map(|mount| mount.language())
            .collect::<Vec<_>>(),
        [HtmlEmbeddedLanguage::JavaScript; 3]
    );
    assert!(
        html.nodes()
            .any(|node| node.kind() == HtmlNodeKind::SelfClosingTag)
    );
    assert_eq!(doc.text().as_str(), source);
}
