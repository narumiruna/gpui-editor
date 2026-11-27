//! Routing fixtures from @codemirror/lang-html 6.4.12 and @lezer/html 1.3.13.
//! The oracle identifies each mounted parser and converts its ranges to UTF-8.

use std::{cell::RefCell, ops::Range};

use gpui_editor_codemirror::{
    Text,
    language::{
        IndentOptions, StreamSettings,
        html::{
            HtmlAttributes, HtmlCustomDocument, HtmlDocument, HtmlEmbeddedDocument,
            HtmlEmbeddedLanguage, HtmlNestedAttribute, HtmlNestedLanguage, HtmlNestedParser,
            HtmlNesting, HtmlNodeKind, HtmlOptions,
        },
        json::{JsonDocument, JsonNodeKind},
    },
};
use serde_json::{Value, json};

fn json_parser(name: &'static str) -> HtmlNestedParser<'static> {
    HtmlNestedParser {
        name,
        parse: &|text| Box::new(JsonDocument::parse(text)),
    }
}

fn language<'a>(
    tag: &'a str,
    attrs: Option<&'a dyn Fn(&HtmlAttributes<'_>) -> bool>,
    name: &'static str,
) -> HtmlNestedLanguage<'a> {
    HtmlNestedLanguage {
        tag,
        attrs,
        parser: json_parser(name),
    }
}

#[test]
fn matches_pinned_custom_routing_fixtures() {
    let predicates = RefCell::new(Vec::new());
    let script = |attrs: &HtmlAttributes<'_>| {
        predicates.borrow_mut().push(json!(attrs));
        attrs.get("mode") == Some(&"custom")
    };
    let attribute = |name, tag_name, parser_name| HtmlNestedAttribute {
        name,
        tag_name,
        parser: json_parser(parser_name),
    };
    let nesting = HtmlNesting {
        nested_languages: &[
            language("script", Some(&script), "script-custom"),
            language(
                "script",
                Some(&|attrs| attrs.get("mode") == Some(&"alternate")),
                "script-alternate",
            ),
            language(
                "script",
                Some(&|attrs| attrs.get("mode") == Some(&"custom")),
                "never",
            ),
            language(
                "style",
                Some(&|attrs| {
                    attrs.get("lang") == Some(&"json") || attrs.get("mode") == Some(&"custom")
                }),
                "style-custom",
            ),
            language("textarea", None, "textarea"),
            language("SCRIPT", None, "uppercase-raw-overlay"),
            language(
                "x-data",
                Some(&|attrs| attrs.get("mode") == Some(&"skip")),
                "inner",
            ),
            language("x-data", None, "body"),
            language("X-DATA", None, "uppercase-body"),
            language("p", None, "paragraph"),
            language("li", None, "item"),
            language("script1", None, "digit-raw-overlay"),
            language("p1", None, "digit-ordinary-overlay"),
        ],
        nested_attributes: &[
            attribute("data-json", Some("x-data"), "scoped"),
            attribute("data-json", Some("X-DATA"), "uppercase-scoped"),
            attribute("data-json", None, "global"),
            attribute("style", Some("x-data"), "style-override"),
            attribute("onclick", Some("x-data"), "click-override"),
            attribute("__proto__", None, "proto"),
            attribute("data-first", Some(""), "empty-scope-first"),
            attribute("data-first", Some("x-data"), "never-specific"),
        ],
    };
    for line in include_str!("fixtures/html_custom_nesting.jsonl").lines() {
        predicates.borrow_mut().clear();
        let fixture: Value = serde_json::from_str(line).unwrap();
        let case = fixture["name"].as_str().unwrap();
        let source = fixture["source"].as_str().unwrap();
        let options = HtmlOptions {
            match_closing_tags: fixture["options"]["matchClosingTags"] != false,
            self_closing_tags: fixture["options"]["selfClosingTags"] == true,
        };
        let doc = HtmlDocument::parse_with_nesting(Text::new(source), options, nesting);
        let mut previous = 0;
        let mounts: Vec<_> = doc
            .mounts()
            .iter()
            .map(|mount| {
                let range = mount.range();
                assert!(previous <= range.start, "{case}: {range:?}");
                previous = range.end;
                let text = &source[range.clone()];
                let name = match mount.language() {
                    HtmlEmbeddedLanguage::Custom(name) => {
                        let HtmlEmbeddedDocument::Custom(child) = mount.document() else {
                            panic!("{case}: expected the custom document for {name}");
                        };
                        let json = child.downcast_ref::<JsonDocument>().unwrap();
                        assert_eq!(json.text().as_str(), text, "{case}");
                        assert_eq!(json.root().kind(), JsonNodeKind::JsonText, "{case}");
                        assert_eq!(json.root().range(), 0..text.len(), "{case}");
                        assert!(child.downcast_ref::<TextDocument>().is_none(), "{case}");
                        name.to_owned()
                    }
                    language => format!("{language:?}"),
                };
                json!([range.start, range.end, name, text])
            })
            .collect();
        assert_eq!(json!(mounts), fixture["mounts"], "{case}");
        assert_eq!(json!(*predicates.borrow()), fixture["predicates"], "{case}");
    }
}

#[test]
fn projects_native_highlights_folds_and_indent_to_the_host() {
    let source =
        "😀\r\n  <x-json data-json='\"é&amp;😀\"'>\r\n    {\r\n\"clé\": 1\r\n    }\r\n  </x-json>";
    let parser = json_parser("json");
    let doc = HtmlDocument::parse_with_nesting(
        Text::new(source),
        HtmlOptions::default(),
        HtmlNesting {
            nested_languages: &[HtmlNestedLanguage {
                tag: "x-json",
                attrs: None,
                parser,
            }],
            nested_attributes: &[HtmlNestedAttribute {
                name: "data-json",
                tag_name: None,
                parser,
            }],
        },
    );
    let spans = |range: Range<usize>| {
        doc.highlights(range)
            .unwrap()
            .map(|span| (&source[span.range], span.tags[0].name()))
            .collect::<Vec<_>>()
    };
    let value = doc.mounts()[0].range();
    assert_eq!(
        spans(value.start - 1..value.end + 1),
        [
            ("'", "attributeValue"),
            ("\"é&amp;😀\"", "string"),
            ("'", "attributeValue"),
        ]
    );
    let property = source.find("\"clé\"").unwrap();
    assert_eq!(spans(property + 1..property + 5), [("clé", "propertyName")]);
    assert!(doc.highlights(property + 4..property + 5).is_none());
    let opener = source.find('{').unwrap();
    let line = doc.text().line_at(opener).unwrap().range;
    let fold = doc.foldable(line.start, line.end).unwrap();
    assert_eq!(&source[fold], "\r\n\"clé\": 1\r\n    ");
    assert_eq!(
        doc.indentation(
            property,
            StreamSettings::default(),
            IndentOptions::default()
        ),
        Some(6)
    );
    let over = |position| (position == line.start).then_some(8);
    assert_eq!(
        doc.indentation(
            property,
            StreamSettings::default(),
            IndentOptions {
                override_indentation: Some(&over),
                ..IndentOptions::default()
            },
        ),
        Some(10)
    );
}

#[derive(Debug)]
struct TextDocument(Text);

impl HtmlCustomDocument for TextDocument {}

#[test]
fn custom_parser_can_capture_state_without_replacing_the_host_tree() {
    let source = "<x-data>\r\n<em>é&amp;</em>\r\n</x-data>";
    let calls = RefCell::new(Vec::new());
    let parse = |text: Text| -> Box<dyn HtmlCustomDocument> {
        calls.borrow_mut().push(text.as_str().to_owned());
        Box::new(TextDocument(text))
    };
    let doc = HtmlDocument::parse_with_nesting(
        Text::new(source),
        HtmlOptions::default(),
        HtmlNesting {
            nested_languages: &[HtmlNestedLanguage {
                tag: "x-data",
                attrs: None,
                parser: HtmlNestedParser {
                    name: "text",
                    parse: &parse,
                },
            }],
            ..HtmlNesting::default()
        },
    );
    assert_eq!(*calls.borrow(), ["\r\n<em>é&amp;</em>\r\n"]);
    let mount = &doc.mounts()[0];
    let HtmlEmbeddedDocument::Custom(child) = mount.document() else {
        panic!("expected the custom text document");
    };
    let text = &child.downcast_ref::<TextDocument>().unwrap().0;
    assert_eq!(text.as_str(), &source[mount.range()]);
    assert_eq!(doc.highlights(mount.range()).unwrap().count(), 0);
    assert!(doc.nodes().any(|node| {
        node.kind() == HtmlNodeKind::EntityReference && &source[node.range()] == "&amp;"
    }));
    let inner = source.find("<em>").unwrap();
    let settings = StreamSettings::default();
    let options = IndentOptions::default();
    assert_eq!(
        doc.indentation(inner, settings, options),
        HtmlDocument::parse(Text::new(source)).indentation(inner, settings, options)
    );
}
