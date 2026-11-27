//! Fixtures from `@lezer/xml` 1.0.6 (`test/tags.txt`) and
//! `@codemirror/lang-xml` 6.1.0 (`test/test-complete.ts`, commit
//! 6fb53b56f59167def8d13f6838113c7f09c70984).
//! Additional oracle results use `@lezer/lr` 1.4.13, `@lezer/highlight` 1.2.5,
//! and `@codemirror/language` 6.13.1. UTF-16 positions were converted to bytes.

use std::ops::Range;

use gpui_editor_codemirror::{
    Text,
    autocomplete::{Completion, CompletionMatcher, MatchMode},
    language::{
        IndentOptions, StreamSettings, TagModifier,
        xml::{
            BLOCK_COMMENT, XmlAttrSpec, XmlAttribute, XmlCompletionResult, XmlCompletionValidity,
            XmlDocument, XmlElementSpec, XmlNode, XmlNodeKind, XmlSchema, XmlValue,
            indent_on_input, xml_completion_source,
        },
    },
};

fn parse(source: &str) -> XmlDocument {
    XmlDocument::parse(Text::new(source))
}

fn tree(node: XmlNode<'_>) -> String {
    let mut result = node.kind().name().to_owned();
    if node.children().len() > 0 {
        result.push('(');
        result.push_str(&node.children().map(tree).collect::<Vec<_>>().join(","));
        result.push(')');
    }
    result
}

#[test]
fn parses_all_fifteen_upstream_grammar_fixtures() {
    for (source, expected) in [
        (
            "<foo/>",
            "Document(Element(SelfClosingTag(StartTag,TagName,SelfCloseEndTag)))",
        ),
        (
            "<foo>bar</foo>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Text,CloseTag(StartCloseTag,TagName,EndTag)))",
        ),
        (
            "<a><b>c</b><d/></a>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),Text,CloseTag(StartCloseTag,TagName,EndTag)),Element(SelfClosingTag(StartTag,TagName,SelfCloseEndTag)),CloseTag(StartCloseTag,TagName,EndTag)))",
        ),
        (
            r#"<a foo="bar"/>"#,
            "Document(Element(SelfClosingTag(StartTag,TagName,Attribute(AttributeName,Is,AttributeValue),SelfCloseEndTag)))",
        ),
        (
            "<a foo='bar'/>",
            "Document(Element(SelfClosingTag(StartTag,TagName,Attribute(AttributeName,Is,AttributeValue),SelfCloseEndTag)))",
        ),
        (
            r#"<a b="one" c='two' d="three" e='four' ></a>"#,
            "Document(Element(OpenTag(StartTag,TagName,Attribute(AttributeName,Is,AttributeValue),Attribute(AttributeName,Is,AttributeValue),Attribute(AttributeName,Is,AttributeValue),Attribute(AttributeName,Is,AttributeValue),EndTag),CloseTag(StartCloseTag,TagName,EndTag)))",
        ),
        (
            r#"<a attr="one&amp;two">&amp;&#67;</a>"#,
            "Document(Element(OpenTag(StartTag,TagName,Attribute(AttributeName,Is,AttributeValue(EntityReference)),EndTag),EntityReference,CharacterReference,CloseTag(StartCloseTag,TagName,EndTag)))",
        ),
        ("&amp&;", "Document(⚠,Text,⚠,Text)"),
        (
            "<!doctype html>\n<doc/>",
            "Document(DoctypeDecl,Text,Element(SelfClosingTag(StartTag,TagName,SelfCloseEndTag)))",
        ),
        (
            "<?foo?><bar><?baz?></bar>",
            "Document(ProcessingInst,Element(OpenTag(StartTag,TagName,EndTag),ProcessingInst,CloseTag(StartCloseTag,TagName,EndTag)))",
        ),
        (
            "<!-- top comment -->\n<element><!-- inner comment --> text</element>\n<!--c--->",
            "Document(Comment,Text,Element(OpenTag(StartTag,TagName,EndTag),Comment,Text,CloseTag(StartCloseTag,TagName,EndTag)),Text,Comment)",
        ),
        (
            "<a></b>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),MismatchedCloseTag(StartCloseTag,TagName,EndTag),⚠))",
        ),
        (
            "<a><b><c></c></x></a>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),CloseTag(StartCloseTag,TagName,EndTag)),MismatchedCloseTag(StartCloseTag,TagName,EndTag),MissingCloseTag),CloseTag(StartCloseTag,TagName,EndTag)))",
        ),
        (
            "<doc>\n  <\n    foo bar=\"10\">\n    blah\n  </oof>\n</doc>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Text,Element(OpenTag(StartTag,TagName,Attribute(AttributeName,Is,AttributeValue),EndTag),Text,MismatchedCloseTag(StartCloseTag,TagName,EndTag),Text,MissingCloseTag),CloseTag(StartCloseTag,TagName,EndTag)))",
        ),
        (
            "<doc><![CDATA[ hello ]]]></doc>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Cdata,CloseTag(StartCloseTag,TagName,EndTag)))",
        ),
    ] {
        assert_eq!(tree(parse(source).root()), expected, "{source:?}");
    }
}

#[test]
fn keeps_crlf_and_preserves_byte_ranges_for_unicode_and_all_highlight_tags() {
    let source = "<!DOCTYPE root>\r\n<?xml 😀?><名 attr=\"é&amp;二&#x31;😀\">\r\n  &amp;&#21;</bad><!--hi--><![CDATA[😀]]>\r\n</名>";
    let doc = parse(source);
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(
        doc.nodes()
            .map(|node| (node.kind().name(), node.range()))
            .collect::<Vec<_>>(),
        [
            ("Document", 0..115),
            ("DoctypeDecl", 0..15),
            ("Text", 15..17),
            ("ProcessingInst", 17..29),
            ("Element", 29..115),
            ("OpenTag", 29..62),
            ("StartTag", 29..30),
            ("TagName", 30..33),
            ("Attribute", 34..61),
            ("AttributeName", 34..38),
            ("Is", 38..39),
            ("AttributeValue", 39..61),
            ("EntityReference", 42..47),
            ("CharacterReference", 50..56),
            ("EndTag", 61..62),
            ("Text", 62..66),
            ("EntityReference", 66..71),
            ("CharacterReference", 71..76),
            ("MismatchedCloseTag", 76..82),
            ("StartCloseTag", 76..78),
            ("TagName", 78..81),
            ("EndTag", 81..82),
            ("Comment", 82..91),
            ("Cdata", 91..107),
            ("Text", 107..109),
            ("CloseTag", 109..115),
            ("StartCloseTag", 109..111),
            ("TagName", 111..114),
            ("EndTag", 114..115),
        ],
    );
    let spans = |range| {
        doc.highlights(range)
            .unwrap()
            .map(|span| {
                let tags = span
                    .tags
                    .iter()
                    .map(|tag| {
                        if tag.modifiers().is_empty() {
                            tag.name()
                        } else {
                            assert_eq!(tag.name(), "string");
                            assert_eq!(tag.modifiers(), [TagModifier::Special]);
                            "specialString"
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                (span.range, tags)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        spans(0..source.len()),
        [
            (0..15, "documentMeta"),
            (15..17, "content"),
            (17..29, "processingInstruction"),
            (29..30, "angleBracket"),
            (30..33, "tagName"),
            (34..38, "attributeName"),
            (38..39, "definitionOperator"),
            (39..42, "attributeValue"),
            (42..47, "character"),
            (47..50, "attributeValue"),
            (50..56, "character"),
            (56..61, "attributeValue"),
            (61..62, "angleBracket"),
            (62..66, "content"),
            (66..71, "character"),
            (71..76, "character"),
            (76..78, "angleBracket"),
            (78..81, "tagName invalid"),
            (81..82, "angleBracket"),
            (82..91, "blockComment"),
            (91..107, "specialString"),
            (107..109, "content"),
            (109..111, "angleBracket"),
            (111..114, "tagName"),
            (114..115, "angleBracket"),
        ]
        .map(|(range, tag)| (range, tag.to_owned())),
    );
    assert_eq!(
        spans(40..44),
        [
            (40..42, "attributeValue".into()),
            (42..44, "character".into())
        ]
    );
    assert!(spans(50..50).is_empty());
    assert!(doc.highlights(41..44).is_none());
    assert!(doc.highlights(0..116).is_none());
    assert!(doc.highlights(Range { start: 62, end: 61 }).is_none());
}

#[test]
fn recovers_incomplete_tags_references_and_delimited_nodes() {
    for (source, expected) in [
        ("", "Document(⚠)"),
        (
            "<doc><</doc>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,⚠),MissingCloseTag),CloseTag(StartCloseTag,TagName,EndTag)))",
        ),
        (
            "<doc></",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),StartCloseTag,⚠))",
        ),
        (
            "<doc></doc",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),CloseTag(StartCloseTag,TagName,⚠)))",
        ),
        (
            r#"<a x="&bad >"#,
            "Document(Element(OpenTag(StartTag,TagName,Attribute(AttributeName,Is,AttributeValue(⚠,⚠)),⚠),⚠))",
        ),
        ("<!--hi", "Document(Comment(⚠))"),
        ("<?hi", "Document(ProcessingInst(⚠))"),
        ("<![CDATA[foo", "Document(Cdata(⚠))"),
        (
            "<a1></a2>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),CloseTag(StartCloseTag,TagName,EndTag)))",
        ),
        (
            "<𐀀/>",
            "Document(Element(SelfClosingTag(StartTag,TagName,SelfCloseEndTag)))",
        ),
        (
            "<a><!DOCTYPE a></a>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),⚠),DoctypeDecl,MismatchedCloseTag(StartCloseTag,TagName,EndTag))",
        ),
    ] {
        assert_eq!(tree(parse(source).root()), expected, "{source:?}");
    }
    let doc = parse(r#"<a x="&bad >"#);
    assert_eq!(
        doc.highlights(5..12)
            .unwrap()
            .map(|span| span.range)
            .collect::<Vec<_>>(),
        [5..6, 7..12],
    );
    let doc = parse("<a><b></a>");
    let missing = doc
        .nodes()
        .find(|node| node.kind() == XmlNodeKind::MissingCloseTag)
        .unwrap();
    assert_eq!(missing.range(), 6..6);
    assert_eq!(missing.parent().unwrap().range(), 3..6);
}

type LineQuery = (usize, Option<Range<usize>>);

#[test]
fn matches_oracle_folds_and_indentation_on_logical_lines() {
    let fixtures: &[(&str, &[LineQuery])] = &[
        (
            "<outer>\r\n\t<名字\r\n\t  attr=\"😀\"\r\n\t>\r\n   x\r\n\t</名字\r\n\t>\r\n</outer>",
            &[
                (0, Some(7..60)),
                (2, None),
                (6, None),
                (6, Some(37..46)),
                (6, None),
                (4, None),
                (6, None),
                (0, None),
            ],
        ),
        (
            "<a><b>\n\n</b></a>",
            &[(0, Some(3..12)), (2, None), (0, None)],
        ),
        (
            "<a>\n<x>\n</a>",
            &[(0, Some(3..8)), (2, Some(7..8)), (0, None)],
        ),
        ("<a>\n  <b/>", &[(0, Some(3..10)), (2, None)]),
        ("<a>\n", &[(0, Some(3..4)), (2, None)]),
        ("<a>\n</", &[(0, Some(3..6)), (0, None)]),
        (
            "   <a><b\n   x=\"v\"\n   />\n   \n   </a>",
            &[(0, Some(6..31)), (8, None), (8, None), (5, None), (3, None)],
        ),
        ("<a>\n</a\n", &[(0, Some(3..4)), (0, None), (2, None)]),
        ("<a>\n  <b\n", &[(0, Some(3..9)), (2, None), (4, None)]),
    ];
    for (source, expected) in fixtures {
        let doc = parse(source);
        assert_eq!(doc.text().line_count(), expected.len());
        for (index, (indent, fold)) in expected.iter().enumerate() {
            let line = doc.text().line(index + 1).unwrap();
            assert_eq!(
                doc.indentation(
                    line.range.start,
                    StreamSettings::default(),
                    IndentOptions::default()
                ),
                Some(*indent),
                "{source:?} line {}",
                index + 1
            );
            assert_eq!(
                doc.foldable(line.range.start, line.range.end),
                *fold,
                "{source:?} line {}",
                index + 1
            );
        }
    }
    assert_eq!(BLOCK_COMMENT, ("<!--", "-->"));
    assert!(indent_on_input("\t\u{feff}</"));
    assert!(!indent_on_input("  </ "));
    assert!(!indent_on_input("  </a"));
    assert!(!indent_on_input("\u{85}</"));
}

#[test]
fn customizes_indent_queries_without_changing_the_document() {
    let settings = StreamSettings::default();
    let doc = parse("\t<a></a>");
    let options = IndentOptions {
        override_indentation: Some(&|start| (start == 0).then_some(6)),
        simulate_break: Some(4),
        simulate_double_break: false,
    };
    assert_eq!(doc.indentation(4, settings, options), Some(6));
    assert_eq!(
        doc.indentation(
            4,
            settings,
            IndentOptions {
                simulate_double_break: true,
                ..options
            }
        ),
        Some(8)
    );
    assert_eq!(doc.text().as_str(), "\t<a></a>");
    assert_eq!(parse("<中/>").indentation(2, settings, options), None);
}

fn with_cursor<T>(marked: &str, f: impl FnOnce(&XmlDocument, usize) -> T) -> T {
    let pos = marked.find('|').unwrap();
    let doc = parse(&marked.replacen('|', "", 1));
    f(&doc, pos)
}

fn sorted_labels(result: Option<&XmlCompletionResult<'_>>) -> String {
    let Some(result) = result else {
        return String::new();
    };
    let mut options = result
        .options
        .iter()
        .map(|option| option.as_completion())
        .collect::<Vec<_>>();
    options.sort_by(|a, b| b.boost.cmp(&a.boost).then_with(|| a.label.cmp(b.label)));
    options
        .iter()
        .map(|c| c.label)
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn completes_all_upstream_cursor_fixtures() {
    const KEYWORD: Completion<'_> = Completion {
        kind: Some("keyword"),
        ..Completion::new("")
    };
    let schema = XmlSchema {
        elements: &[
            XmlElementSpec {
                name: "doc",
                top: true,
                attributes: &[
                    XmlAttribute::Named("attr1"),
                    XmlAttribute::Named("attr2"),
                    XmlAttribute::Inline(XmlAttrSpec {
                        name: "attr3",
                        values: Some(&[XmlValue::Text("x"), XmlValue::Text("y")]),
                        completion: Some(KEYWORD),
                        ..XmlAttrSpec::default()
                    }),
                ],
                children: Some(&["head", "body"]),
                completion: Some(KEYWORD),
                ..XmlElementSpec::default()
            },
            XmlElementSpec {
                name: "head",
                attributes: &[XmlAttribute::Named("attr2")],
                ..XmlElementSpec::default()
            },
            XmlElementSpec {
                name: "body",
                children: Some(&[]),
                ..XmlElementSpec::default()
            },
        ],
        attributes: &[
            XmlAttrSpec {
                name: "attr2",
                values: Some(&[XmlValue::Text("one"), XmlValue::Text("two")]),
                ..XmlAttrSpec::default()
            },
            XmlAttrSpec {
                name: "attrglobal",
                global: true,
                ..XmlAttrSpec::default()
            },
        ],
    };
    with_cursor("<foo>|", |doc, pos| {
        let result = doc.complete(pos, true, XmlSchema::default()).unwrap();
        assert_eq!(result.range, 5..5);
        assert_eq!(sorted_labels(Some(&result)), "</foo>");
        assert_eq!(result.options[0].as_completion().boost, 2);
    });
    for (source, explicit, expected, range) in [
        (
            "<doc |",
            true,
            "attr1, attr2, attr3, attrglobal",
            Some(5..5),
        ),
        (
            "<doc att|>",
            true,
            "attr1, attr2, attr3, attrglobal",
            Some(5..8),
        ),
        (
            "<doc attr1=\"ok\" |>",
            true,
            "attr1, attr2, attr3, attrglobal",
            Some(16..16),
        ),
        ("<doc attr2=|>", true, "\"one\", \"two\"", Some(11..11)),
        ("<doc attr2=\"o|>", true, "\"one\", \"two\"", Some(11..13)),
        ("<doc attr3=|", true, "\"x\", \"y\"", Some(11..11)),
        ("<doc attr1=|", true, "", None),
        ("<doc><b|</doc>", true, "body, head", Some(6..7)),
        ("<doc><|</doc>", true, "body, head", Some(6..6)),
        ("<doc></|", true, "doc>", Some(7..7)),
        ("<doc></d|", true, "doc>", Some(7..8)),
        ("<doc>foo|bar", true, "</doc>, <body, <head", Some(8..8)),
        ("<doc>|</doc>", true, "<body, <head", Some(5..5)),
        ("<doc>|</doc>", false, "", None),
        ("|", true, "<doc", Some(0..0)),
        ("<doc></d|>", false, "doc>", Some(7..9)),
        (
            "<doc><head attr2=\"o|\" /></doc>",
            false,
            "\"one\", \"two\"",
            Some(17..20),
        ),
    ] {
        with_cursor(source, |doc, pos| {
            let result = doc.complete(pos, explicit, schema);
            assert_eq!(sorted_labels(result.as_ref()), expected, "{source:?}");
            assert_eq!(result.map(|result| result.range), range, "{source:?}");
        });
    }
    for (source, label) in [("<d|", "doc"), ("<doc attr|", "attr3")] {
        with_cursor(source, |doc, pos| {
            let result = doc.complete(pos, true, schema).unwrap();
            let option = result
                .options
                .iter()
                .find(|option| option.label == label)
                .unwrap();
            assert_eq!(option.as_completion().kind, Some("keyword"));
        });
    }
    let schema = XmlSchema {
        elements: &[XmlElementSpec {
            name: "top",
            children: Some(&[]),
            text_content: &["true", "false"],
            ..XmlElementSpec::default()
        }],
        ..XmlSchema::default()
    };
    for (source, expected) in [("<top>|", "</top>, false, true"), ("<top>a|</top>", "")] {
        with_cursor(source, |doc, pos| {
            assert_eq!(
                sorted_labels(doc.complete(pos, true, schema).as_ref()),
                expected
            );
        });
    }
}

#[test]
fn completes_predefined_entities_in_text_and_attribute_values_only() {
    let schema = XmlSchema::default();
    for (source, contents, typed, expected) in [
        (
            "<a>😀\r\n&|</a>",
            "&",
            "&",
            "&amp;, &apos;, &gt;, &lt;, &quot;",
        ),
        ("<a x='é&am|;' />", "&am;", "&am", "&amp;"),
        ("<a x=\"&qu|\" />", "&qu", "&qu", "&quot;"),
    ] {
        with_cursor(source, |doc, pos| {
            let result = doc.complete(pos, false, schema).unwrap();
            assert_eq!(result.valid_for, XmlCompletionValidity::Entity);
            assert!(result.valid_for.matches(typed));
            assert_eq!(doc.text().slice(result.range), Some(contents));
            let matcher = CompletionMatcher::new(typed, MatchMode::Strict);
            let filtered = result
                .options
                .iter()
                .filter(|opt| matcher.match_label(&opt.label).is_some())
                .map(|opt| opt.label.as_ref())
                .collect::<Vec<_>>()
                .join(", ");
            assert_eq!(filtered, expected);
        });
    }
    for source in [
        "<!-- &am| -->",
        "<![CDATA[&am|]]>",
        "<?pi &am|?>",
        "<!DOCTYPE &am|>",
        "<a>&#x|</a>",
    ] {
        with_cursor(source, |doc, pos| {
            assert!(doc.complete(pos, false, schema).is_none(), "{source:?}")
        });
    }
    assert!(parse("<中/>").complete(2, true, schema).is_none());
}

#[test]
fn session_source_preserves_schema_metadata_and_explicit_replacement_end() {
    with_cursor("<word>字</wo|>", |doc, position| {
        let schema = XmlSchema {
            elements: &[XmlElementSpec {
                name: "word",
                completion: Some(Completion {
                    detail: Some("schema detail"),
                    boost: 3,
                    ..Completion::new("")
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let borrowed = doc.complete(position, false, schema).unwrap();
        let owned = xml_completion_source(doc, position, false, schema).unwrap();
        assert_eq!(owned.from..owned.to.unwrap(), borrowed.range);
        assert_eq!(
            owned.options[0].as_completion(),
            borrowed.options[0].as_completion()
        );
        assert_eq!(owned.options[0].label, "word>");
        assert_eq!(owned.options[0].detail.as_deref(), Some("schema detail"));
        assert_eq!(owned.options[0].boost, 3);
    });
}

#[test]
fn parses_deeply_nested_elements_without_call_stack_recursion() {
    let depth = 10_000;
    let doc = parse(&format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth)));
    assert_eq!(
        doc.nodes()
            .filter(|node| node.kind() == XmlNodeKind::Element)
            .count(),
        depth
    );
    assert!(!doc.nodes().any(|node| node.kind() == XmlNodeKind::Error));
}
