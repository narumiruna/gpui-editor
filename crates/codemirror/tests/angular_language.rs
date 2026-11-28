//! Differential fixtures from @codemirror/lang-angular 0.1.4 with the
//! pinned HTML, JavaScript, Lezer and CodeMirror state/language packages.
//! Convert upstream UTF-16 offsets to original UTF-8 byte ranges.

use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        angular::{
            AngularDocument, AngularNode, AngularRegion, CLOSE_BRACKETS, NAME, indent_on_input,
            language_data,
        },
        html::{
            HtmlAttributes, HtmlCustomDocument, HtmlEmbeddedDocument, HtmlEmbeddedLanguage,
            HtmlNestedAttribute, HtmlNestedLanguage, HtmlNestedParser, HtmlNesting, HtmlNode,
            HtmlNodeKind, HtmlOptions,
        },
        javascript::{JavascriptDocument, JavascriptNode, JavascriptOptions},
        json::JsonDocument,
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
        assert_fixture(&doc, &fixture);
    }
}

#[test]
fn matches_upstream_recovery_of_one_missing_binding_name_closer() {
    let fixtures: Vec<Value> = include_str!("fixtures/angular_binding_recovery.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for fixture in &fixtures {
        let doc = AngularDocument::parse(Text::new(fixture["input"].as_str().unwrap()));
        assert_fixture(&doc, fixture);
        assert_eq!(
            doc.regions().iter().filter(|r| r.is_recovered()).count(),
            usize::from(fixture["name"] != "balanced control"),
            "{}: recovered regions",
            fixture["name"],
        );
    }

    let balanced = fixtures.last().unwrap();
    let doc = AngularDocument::parse(Text::new(balanced["input"].as_str().unwrap()));
    let at = doc.text().as_str().find(']').unwrap();
    let deleted = ChangeSet::single(Change::new(at..at + 1, "")).unwrap();
    let doc = doc.reparse(&deleted).unwrap();
    assert_fixture(&doc, &fixtures[0]);
    let inserted = ChangeSet::single(Change::new(at..at, "]")).unwrap();
    assert_fixture(&doc.reparse(&inserted).unwrap(), balanced);
}

#[test]
fn binding_name_recovery_does_not_keep_other_malformed_values() {
    for source in [
        "<comp [title=name></comp>",
        "<comp [=\"name\"></comp>",
        "<comp [(model=\"state\"></comp>",
        "<comp [(model.]=\"state\"></comp>",
    ] {
        let doc = AngularDocument::parse(Text::new(source));
        let [binding] = doc.regions() else {
            panic!("{source:?}: expected one region");
        };
        assert!(binding.is_recovered(), "{source:?}");
        assert!(binding.scripts().is_empty(), "{source:?}");
    }
}

fn assert_fixture(doc: &AngularDocument, fixture: &Value) {
    let name = fixture["name"].as_str().unwrap();
    let source = fixture["input"].as_str().unwrap();
    assert_eq!(doc.text().as_str(), source);
    let mut nodes = Vec::new();
    html(doc, doc.html().root(), &mut nodes);
    assert_eq!(json!(nodes), fixture["nodes"], "{name}: tree");
    assert_services(doc, fixture, name, false);
}

fn assert_services(doc: &AngularDocument, fixture: &Value, name: &str, merge: bool) {
    let mut highlights: Vec<(usize, usize, String)> = Vec::new();
    for span in doc.highlights(0..doc.text().len()).unwrap() {
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
        let Range { start, end } = span.range;
        if merge
            && let Some(last) = highlights.last_mut()
            && last.1 == start
            && last.2 == tags
        {
            last.1 = end;
        } else {
            highlights.push((start, end, tags));
        }
    }
    let expected = fixture["highlights"].as_array().unwrap();
    for (span, expected) in highlights.iter().zip(expected) {
        assert_eq!(
            json!(span),
            *expected,
            "{name}: highlight at byte {}",
            span.0
        );
    }
    assert_eq!(highlights.len(), expected.len(), "{name}: highlight count");
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

#[test]
fn matches_pinned_configured_html_on_parse_and_reparse() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/angular_custom_html.jsonl")).unwrap();
    let source = fixture["source"].as_str().unwrap();
    let json_parser = HtmlNestedParser {
        name: "json",
        parse: &|text| Box::new(JsonDocument::parse(text)),
    };
    let is_data = |attrs: &HtmlAttributes<'_>| attrs.get("mode") == Some(&"data");
    let nesting = HtmlNesting {
        nested_languages: &[
            HtmlNestedLanguage {
                tag: "x-data",
                attrs: Some(&is_data),
                parser: json_parser,
            },
            HtmlNestedLanguage {
                tag: "script",
                attrs: Some(&is_data),
                parser: json_parser,
            },
            HtmlNestedLanguage {
                tag: "script",
                attrs: Some(&is_data),
                parser: HtmlNestedParser {
                    name: "shadowed",
                    parse: &|_| panic!("the first accepted body route must win"),
                },
            },
        ],
        nested_attributes: &["data-json", "[value]", "(select)", "[broken"].map(|name| {
            HtmlNestedAttribute {
                name,
                tag_name: None,
                parser: json_parser,
            }
        }),
    };
    let options = HtmlOptions::default();
    let doc = AngularDocument::parse_with_nesting(Text::new(source), options, nesting);
    assert_configured(&doc, &fixture["before"], "parse");
    assert!(
        doc.html()
            .nodes()
            .all(|node| node.kind() != HtmlNodeKind::SelfClosingTag)
    );

    let edits: Vec<(usize, usize, String)> =
        serde_json::from_value(fixture["edits"].clone()).unwrap();
    let changes = ChangeSet::new(
        edits
            .into_iter()
            .map(|(from, to, insert)| Change::new(from..to, insert))
            .collect(),
    )
    .unwrap();
    let updated = doc.reparse_with_nesting(&changes, nesting).unwrap();
    assert_configured(&updated, &fixture["after"], "reparse");
    let fresh = AngularDocument::parse_with_nesting(updated.text().clone(), options, nesting);
    assert_configured(&fresh, &fixture["after"], "fresh parse");

    let stock = doc.reparse(&changes).unwrap();
    assert!(
        stock
            .mounts()
            .all(|mount| !matches!(mount.language(), HtmlEmbeddedLanguage::Custom(_)))
    );
    assert_eq!(doc.text().as_str(), source);
}

fn assert_configured(doc: &AngularDocument, fixture: &Value, label: &str) {
    let source = doc.text().as_str();
    let mounts: Vec<_> = doc
        .mounts()
        .map(|mount| {
            let (name, top, text) = match (mount.language(), mount.document()) {
                (HtmlEmbeddedLanguage::Custom("json"), HtmlEmbeddedDocument::Custom(child)) => {
                    let child = child.downcast_ref::<JsonDocument>().unwrap();
                    ("json", child.root().kind().name(), child.text())
                }
                (HtmlEmbeddedLanguage::JavaScript, HtmlEmbeddedDocument::Javascript(child)) => {
                    ("JavaScript", child.root().kind().name(), child.text())
                }
                (HtmlEmbeddedLanguage::Css, HtmlEmbeddedDocument::Css(child)) => {
                    ("Css", child.root().kind().name(), child.text())
                }
                other => panic!("{label}: unexpected mount {other:?}"),
            };
            let Range { start, end } = mount.range();
            assert_eq!(text.as_str(), &source[start..end], "{label}: mount text");
            (name, start, end, top)
        })
        .collect();
    assert_eq!(json!(mounts), fixture["mounts"], "{label}: HTML mounts");
    let regions: Vec<_> = doc
        .regions()
        .iter()
        .map(|region| {
            let scripts: Vec<_> = region
                .scripts()
                .iter()
                .map(|script| {
                    let Range { start, end } = script.range();
                    let child = script.document();
                    assert_eq!(child.text().as_str(), &source[start..end]);
                    (child.root().kind().name(), start, end)
                })
                .collect();
            let root = region.root();
            let Range { start, end } = root.range();
            (root.name(), start, end, scripts)
        })
        .collect();
    assert_eq!(
        json!(regions),
        fixture["regions"],
        "{label}: Angular mounts"
    );
    assert_services(doc, fixture, label, true);
}

#[test]
fn an_angular_attribute_masks_the_base_values_fold_and_indent() {
    #[derive(Debug)]
    struct BaseValue(usize);

    impl HtmlCustomDocument for BaseValue {
        fn foldable(&self, _line_start: usize, line_end: usize) -> Option<Range<usize>> {
            Some(line_end..self.0)
        }

        fn indentation(
            &self,
            _position: usize,
            _settings: StreamSettings,
            _options: IndentOptions<'_>,
        ) -> Option<usize> {
            Some(99)
        }
    }

    let doc = AngularDocument::parse_with_nesting(
        Text::new("<div data-json='{{ value }}\n more\n end'></div>"),
        HtmlOptions::default(),
        HtmlNesting {
            nested_attributes: &[HtmlNestedAttribute {
                name: "data-json",
                tag_name: None,
                parser: HtmlNestedParser {
                    name: "base-value",
                    parse: &|text| Box::new(BaseValue(text.len())),
                },
            }],
            ..HtmlNesting::default()
        },
    );
    assert_eq!(doc.mounts().count(), 0);
    let line = doc.text().line(1).unwrap().range;
    assert!(doc.html().foldable(line.start, line.end).is_some());
    assert_eq!(doc.foldable(line.start, line.end), None);
    let position = doc.text().line(2).unwrap().range.start;
    let options = IndentOptions::default();
    assert_eq!(
        doc.html().indentation(position, settings(), options),
        Some(99)
    );
    assert_eq!(doc.indentation(position, settings(), options), Some(2));
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
