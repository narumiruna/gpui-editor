//! Differential fixtures from @codemirror/lang-angular 0.1.4 with the
//! pinned HTML, JavaScript, Lezer and CodeMirror state/language packages.
//! Convert upstream UTF-16 offsets to original UTF-8 byte ranges.

use std::ops::Range;

use gpui_editor_codemirror::{
    Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        angular::{
            AngularDocument, AngularNode, AngularRegion, CLOSE_BRACKETS, NAME, indent_on_input,
            language_data,
        },
        html::{HtmlNode, HtmlNodeKind, HtmlOptions},
        javascript::{JavascriptDocument, JavascriptNode, JavascriptOptions},
    },
};
use serde_json::{Value, json};

fn record(records: &mut Vec<String>, name: &str, range: Range<usize>) {
    records.push(format!("{name}:{}-{}", range.start, range.end));
}

fn javascript(node: JavascriptNode<'_>, offset: usize, records: &mut Vec<String>) {
    let range = node.range();
    record(
        records,
        node.kind().name(),
        offset + range.start..offset + range.end,
    );
    for child in node.children() {
        javascript(child, offset, records);
    }
}

fn angular(region: &AngularRegion, node: AngularNode<'_>, records: &mut Vec<String>) {
    let range = node.range();
    if matches!(
        node.name(),
        "InterpolationContent" | "AttributeInterpolation"
    ) && let Some(script) = region
        .scripts()
        .iter()
        .find(|script| script.range() == range)
    {
        javascript(script.document().root(), range.start, records);
        return;
    }
    record(records, node.name(), range);
    for child in node.children() {
        angular(region, child, records);
    }
}

fn html(doc: &AngularDocument, node: HtmlNode<'_>, records: &mut Vec<String>) {
    if matches!(node.kind(), HtmlNodeKind::Text | HtmlNodeKind::Attribute)
        && let Some(region) = doc
            .regions()
            .iter()
            .find(|region| region.root().range() == node.range())
    {
        angular(region, region.root(), records);
        return;
    }
    record(records, node.kind().name(), node.range());
    for child in node.children() {
        html(doc, child, records);
    }
}

fn settings() -> StreamSettings {
    StreamSettings {
        indent_unit: 2,
        ..StreamSettings::default()
    }
}

#[test]
fn matches_upstream_angular_mixed_syntax_and_editor_services() {
    let fixtures: Vec<Value> = serde_json::from_str(include_str!("fixtures/angular.json")).unwrap();
    for fixture in fixtures {
        let name = fixture["name"].as_str().unwrap();
        let source = fixture["input"].as_str().unwrap();
        let doc = if name == "different_base" {
            AngularDocument::parse_with_options(Text::new(source), HtmlOptions::default())
        } else {
            AngularDocument::parse(Text::new(source))
        };
        assert_eq!(doc.text().as_str(), source);
        let mut nodes = Vec::new();
        html(&doc, doc.html().root(), &mut nodes);
        assert_eq!(json!(nodes), fixture["nodes"], "{name}: tree");
        let highlights: Vec<_> = doc
            .highlights(0..source.len())
            .unwrap()
            .map(|span| {
                let tags = span
                    .tags
                    .iter()
                    .map(|tag| {
                        let mut name = tag.name().to_owned();
                        for modifier in tag.modifiers() {
                            name.push('.');
                            name.push_str(match modifier {
                                TagModifier::Definition => "definition",
                                TagModifier::Function => "function",
                                TagModifier::Special => "special",
                                _ => panic!("unexpected modifier {modifier:?}"),
                            });
                        }
                        name
                    })
                    .collect::<Vec<_>>()
                    .join("+");
                json!([span.range.start, span.range.end, tags])
            })
            .collect();
        assert_eq!(
            json!(highlights),
            fixture["highlights"],
            "{name}: highlights"
        );
        let lines: Vec<_> = (1..=doc.text().line_count())
            .map(|i| doc.text().line(i).unwrap().range)
            .collect();
        let folds: Vec<_> = lines
            .iter()
            .map(|line| {
                doc.foldable(line.start, line.end)
                    .map(|fold| [fold.start, fold.end])
            })
            .collect();
        assert_eq!(json!(folds), fixture["folds"], "{name}: folds");
        let indents: Vec<_> = lines
            .iter()
            .map(|line| doc.indentation(line.start, settings(), IndentOptions::default()))
            .collect();
        assert_eq!(json!(indents), fixture["indents"], "{name}: indentation");
    }
}

#[test]
fn keeps_original_bytes_and_clips_a_mounted_expression() {
    let source = "<div>\r\n{{ '\u{1f63a}' }}\r\n</div>";
    let doc = AngularDocument::parse(Text::new(source));
    assert_eq!(doc.text().as_str(), source);
    let mounted = doc
        .regions()
        .iter()
        .flat_map(|region| region.scripts())
        .next()
        .unwrap();
    assert_eq!(mounted.document().root().kind().name(), "SingleExpression");
    assert_eq!(mounted.document().text().as_str(), &source[mounted.range()]);
    let start = source.find('\u{1f63a}').unwrap();
    let spans: Vec<_> = doc.highlights(start..start + 4).unwrap().collect();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].range, start..start + 4);
    assert_eq!(spans[0].tags[0].name(), "string");
    assert!(doc.highlights(start + 1..start + 4).is_none());
    assert_eq!(doc.highlights(start..start).unwrap().count(), 0);
}

#[test]
fn incomplete_unicode_templates_keep_ordered_original_ranges() {
    let full = "<btn [title]=\"name\" (click)=\"log('🐱')\" title=\"Hi {{ member.name }}\">🌍 {{ quantity + 1 }}</btn>";
    for end in full.char_indices().map(|(at, _)| at).chain([full.len()]) {
        let source = &full[..end];
        let doc = AngularDocument::parse(Text::new(source));
        let mut previous = 0;
        for span in doc.highlights(0..end).unwrap() {
            assert!(
                previous <= span.range.start && span.range.start < span.range.end,
                "{source:?}: {span:?}"
            );
            assert!(
                source.get(span.range.clone()).is_some(),
                "{source:?}: {span:?}"
            );
            previous = span.range.end;
        }
        for region in doc.regions() {
            for node in region.nodes() {
                assert!(source.get(node.range()).is_some(), "{source:?}");
            }
            for script in region.scripts() {
                assert_eq!(script.document().text().as_str(), &source[script.range()]);
            }
        }
    }
}

#[test]
fn expressions_use_the_pinned_single_expression_top_rule() {
    let doc = JavascriptDocument::parse_expression(
        Text::new("{label: '字'}"),
        JavascriptOptions::default(),
    );
    assert!(!doc.is_recovered());
    assert_eq!(doc.root().kind().name(), "SingleExpression");
    assert_eq!(
        doc.root().children().next().unwrap().kind().name(),
        "ObjectExpression"
    );
}

#[test]
fn exposes_template_and_base_language_data() {
    assert_eq!(NAME, "angular");
    assert_eq!(CLOSE_BRACKETS, ['[', '{', '"']);
    assert_eq!(language_data().get("closeBrackets"), Some(&CLOSE_BRACKETS));
    for prefix in ["\t}", "\u{feff}]", "</article>"] {
        assert!(indent_on_input(prefix), "{prefix:?}");
    }
    for prefix in [" } ", "x]", "]]", "\u{85}}"] {
        assert!(!indent_on_input(prefix), "{prefix:?}");
    }
}
