use std::{collections::HashMap, ops::Range};

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    commands::CommentTokens,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        html::{
            HtmlAttributes, HtmlCustomDocument, HtmlDocument, HtmlEmbeddedDocument,
            HtmlEmbeddedLanguage, HtmlNestedAttribute, HtmlNestedLanguage, HtmlNestedParser,
            HtmlNesting, HtmlNode, HtmlNodeKind, HtmlOptions,
        },
        json::JsonDocument,
        php::{CloseBrackets, PhpDocument, PhpOptions, indent_on_input, language_data},
    },
};
use serde_json::{Value, json};

// Captured with Node outside the repository, using the pinned lang-php and
// Lezer PHP tarballs plus @codemirror/lang-html 6.4.12 and @lezer/html 1.3.13.
// The oracle uses CodeMirror EditorState, syntaxTree, highlightTree, foldable,
// and getIndentation. Positions are translated back to original UTF-8 bytes.
const ORACLE: &str = include_str!("../src/language/php/oracle.jsonl");

#[test]
fn matches_upstream_syntax_trees_and_highlights() {
    for fixture in ORACLE
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
    {
        let name = fixture["label"].as_str().unwrap();
        let doc = parse(&fixture);
        assert!(!doc.is_recovered(), "{name}");
        let mut seen = HashMap::new();
        let nodes: Vec<_> = doc
            .nodes()
            .enumerate()
            .map(|(index, node)| {
                let range = node.range();
                let key =
                    |n: gpui_editor_codemirror::language::php::PhpNode<'_>| (n.name(), n.range());
                let parent = node.parent().map(|parent| seen[&key(parent)]);
                seen.insert(key(node), index);
                json!([node.name(), range.start, range.end, parent])
            })
            .collect();
        assert_eq!(
            nodes,
            fixture["raw"].as_array().unwrap().as_slice(),
            "{name}: tree"
        );
        let spans: Vec<_> = doc
            .highlights(0..doc.text().len())
            .unwrap()
            .map(|span| {
                let tags = span
                    .tags
                    .iter()
                    .map(|tag| {
                        let mut name = tag.name().to_owned();
                        for modifier in tag.modifiers() {
                            name.push_str(match modifier {
                                TagModifier::Definition => ".definition",
                                TagModifier::Function => ".function",
                                TagModifier::Special => ".special",
                                _ => panic!("unexpected modifier {modifier:?}"),
                            });
                        }
                        name
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                json!([span.range.start, span.range.end, tags])
            })
            .collect();
        assert_eq!(
            spans,
            fixture["highlighted"].as_array().unwrap().as_slice(),
            "{name}: highlights"
        );
        if let Some(outer) = doc.html() {
            let html = outer.document();
            assert_eq!(
                html_shape(html.root()),
                fixture["base"].as_str().unwrap(),
                "{name}: HTML tree"
            );
            let mapped: Vec<_> = outer
                .source_ranges(0..html.text().len())
                .unwrap()
                .into_iter()
                .map(|range| [range.start, range.end])
                .collect();
            assert_eq!(
                json!(mapped),
                fixture["ranges"],
                "{name}: HTML source ranges"
            );
        } else {
            assert!(fixture.get("base").is_none(), "{name}: expected HTML tree");
        }
    }
}

fn html_shape(node: HtmlNode<'_>) -> String {
    let mut result = node.kind().name().to_owned();
    if node.children().len() > 0 {
        result.push('(');
        result.push_str(
            &node
                .children()
                .map(html_shape)
                .collect::<Vec<_>>()
                .join(","),
        );
        result.push(')');
    }
    result
}

#[test]
fn matches_upstream_fold_and_indentation_fixtures() {
    for fixture in ORACLE
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
    {
        let name = fixture["label"].as_str().unwrap();
        let doc = parse(&fixture);
        for index in 0..doc.text().line_count() {
            let line = doc.text().line(index + 1).unwrap();
            let fold = doc
                .foldable(line.range.start, line.range.end)
                .map(|fold| [fold.start, fold.end]);
            let indent = doc.indentation(
                line.range.start,
                StreamSettings::default(),
                IndentOptions::default(),
            );
            assert_eq!(
                json!(fold),
                fixture["folds"][index],
                "{name}: fold on line {}",
                line.number
            );
            assert_eq!(
                json!(indent),
                fixture["indents"][index],
                "{name}: indent on line {}",
                line.number
            );
        }
    }
}

fn parse(fixture: &Value) -> PhpDocument {
    PhpDocument::parse_with_options(
        Text::new(fixture["source"].as_str().unwrap()),
        PhpOptions {
            plain: fixture["plain"].as_bool().unwrap(),
            html: fixture["html"].as_bool().unwrap(),
        },
    )
}

#[test]
fn metadata_and_parse_options_are_explicit() {
    let data = language_data();
    let comments = data.get::<CommentTokens<'_>>("commentTokens").unwrap();
    assert_eq!(comments.line, Some("//"));
    assert_eq!(comments.block, Some(("/*", "*/")));
    assert_eq!(data.get::<&str>("wordChars"), Some(&"$"));
    assert_eq!(
        data.get::<CloseBrackets>("closeBrackets")
            .unwrap()
            .string_prefixes,
        ["b", "B"]
    );
    for yes in [
        "\tcase ",
        "\u{feff}default:",
        "endforeach",
        "endswitch",
        "else",
        "}",
    ] {
        assert!(indent_on_input(yes));
    }
    for no in [
        " case",
        "\u{85}else",
        "default: ",
        "ENDWHILE",
        "elseif ",
        "case  ",
    ] {
        assert!(!indent_on_input(no));
    }
    let plain = PhpDocument::parse_with_options(
        Text::new("echo 1;"),
        PhpOptions {
            plain: true,
            html: true,
        },
    );
    assert_eq!(plain.root().name(), "Program");
    assert!(plain.html().is_none());
    let template = PhpDocument::parse(Text::new("echo 1;"));
    assert_eq!(template.root().name(), "Template");
    assert!(template.html().is_some());
    let pure = PhpDocument::parse(Text::new("<?php echo 1;"));
    assert!(pure.html().is_none());
}

#[test]
fn stock_html_routes_read_original_php_template_attributes() {
    fn bounds(range: Range<usize>) -> [usize; 2] {
        [range.start, range.end]
    }

    fn snapshot(doc: &PhpDocument) -> Value {
        assert!(!doc.is_recovered());
        let outer = doc.html().unwrap();
        let html = outer.document();
        let ranges = outer.source_ranges(0..html.text().len()).unwrap();
        let mounts: Vec<_> = html
            .mounts()
            .iter()
            .map(|mount| {
                assert_eq!(mount.language(), HtmlEmbeddedLanguage::JavaScript);
                let pieces: Vec<_> = outer
                    .source_ranges(mount.range())
                    .unwrap()
                    .into_iter()
                    .map(bounds)
                    .collect();
                json!(["JavaScript", pieces])
            })
            .collect();
        let strings: Vec<_> = doc
            .highlights(0..doc.text().len())
            .unwrap()
            .filter(|span| span.tags.iter().any(|tag| tag.name() == "string"))
            .map(|span| bounds(span.range))
            .collect();
        json!({
            "ranges": ranges.into_iter().map(bounds).collect::<Vec<_>>(),
            "mounts": mounts,
            "strings": strings,
        })
    }

    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/php_html_routes.jsonl")).unwrap();
    let before = PhpDocument::parse(Text::new(fixture["source"].as_str().unwrap()));
    assert_eq!(snapshot(&before), fixture["before"]);

    let (start, end, insert): (usize, usize, String) =
        serde_json::from_value(fixture["edit"].clone()).unwrap();
    let change = ChangeSet::single(Change::new(start..end, insert)).unwrap();
    let after = before.reparse(&change).unwrap();
    // The edit changes only the original spelling of the MIME type.
    assert_eq!(
        before.html().unwrap().document().text(),
        after.html().unwrap().document().text(),
    );
    assert_eq!(snapshot(&after), fixture["after"]);
    assert_eq!(
        snapshot(&PhpDocument::parse(after.text().clone())),
        fixture["after"]
    );
    assert!(after.reuse_stats().nodes_reused > 0);
}

#[test]
fn configured_html_routes_match_upstream_after_original_source_edits() {
    let json = HtmlNestedParser {
        name: "json",
        parse: &|text| Box::new(JsonDocument::parse(text)) as Box<dyn HtmlCustomDocument>,
    };
    let later = HtmlNestedParser {
        name: "css",
        parse: &|_| panic!("the first matching route must win"),
    };
    let when_data = |attrs: &HtmlAttributes<'_>| attrs.get("mode") == Some(&"data");
    let language = |tag, parser| HtmlNestedLanguage {
        tag,
        attrs: Some(&when_data),
        parser,
    };
    let onclick = |parser| HtmlNestedAttribute {
        name: "onclick",
        tag_name: Some("x-data"),
        parser,
    };
    let nesting = HtmlNesting {
        nested_languages: &[
            language("x-data", json),
            language("script", json),
            language("script", later),
        ],
        nested_attributes: &[
            onclick(json),
            onclick(later),
            HtmlNestedAttribute {
                name: "data-json",
                tag_name: None,
                parser: json,
            },
        ],
    };
    let dialect = HtmlOptions {
        match_closing_tags: true,
        self_closing_tags: true,
    };
    let parse = |text, nesting| {
        PhpDocument::parse_with_html_nesting(text, PhpOptions::default(), dialect, nesting)
    };
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/php_custom_html.jsonl")).unwrap();
    let source = fixture["source"].as_str().unwrap();
    let stock = PhpDocument::parse(Text::new(source));
    assert_eq!(
        json!(html_dialect_nodes(stock.html().unwrap().document())),
        fixture["defaultDialect"],
    );
    let mut doc = parse(Text::new(source), nesting);
    assert_eq!(custom_html_snapshot(&doc), fixture["before"]);

    let inside_emoji = source.find('🦀').unwrap() + 1;
    let invalid = ChangeSet::single(Change::new(inside_emoji..inside_emoji + 1, "")).unwrap();
    assert!(doc.reparse_with_html_nesting(&invalid, nesting).is_err());
    assert_eq!(doc.text().as_str(), source);

    for (index, step) in fixture["steps"].as_array().unwrap().iter().enumerate() {
        let (start, end, insert): (usize, usize, String) =
            serde_json::from_value(step["edit"].clone()).unwrap();
        let edit = ChangeSet::single(Change::new(start..end, insert)).unwrap();
        let after = doc.reparse_with_html_nesting(&edit, nesting).unwrap();
        if index == 0 {
            // The PHP-only edit changes which route wins without changing the projection.
            assert_eq!(
                after.html().unwrap().document().text(),
                doc.html().unwrap().document().text(),
            );
            let stock_routes = doc.reparse(&edit).unwrap();
            assert_eq!(
                custom_html_snapshot(&stock_routes),
                custom_html_snapshot(&parse(after.text().clone(), HtmlNesting::default())),
            );
        }
        assert!(after.reuse_stats().resumed_at > 0);
        assert!(after.reuse_stats().nodes_reused > 0);
        assert_eq!(custom_html_snapshot(&after), step["after"]);
        let fresh = parse(after.text().clone(), nesting);
        assert_eq!(custom_html_snapshot(&fresh), step["after"]);
        assert!(
            after
                .nodes()
                .map(|node| (node.name(), node.range()))
                .eq(fresh.nodes().map(|node| (node.name(), node.range())))
        );
        doc = after;
    }

    for options in [
        PhpOptions {
            plain: true,
            html: true,
        },
        PhpOptions {
            plain: false,
            html: false,
        },
    ] {
        let doc =
            PhpDocument::parse_with_html_nesting(Text::new(source), options, dialect, nesting);
        assert!(doc.html().is_none());
    }
}

fn html_dialect_nodes(html: &HtmlDocument) -> Vec<&'static str> {
    html.nodes()
        .map(|node| node.kind())
        .filter(|kind| {
            matches!(
                kind,
                HtmlNodeKind::SelfClosingTag | HtmlNodeKind::MismatchedCloseTag
            )
        })
        .map(HtmlNodeKind::name)
        .collect()
}

fn custom_html_snapshot(doc: &PhpDocument) -> Value {
    assert!(!doc.is_recovered());
    let outer = doc.html().unwrap();
    let html = outer.document();
    let bounds = |range: Range<usize>| [range.start, range.end];
    let pieces = |range| outer.source_ranges(range).unwrap();
    let mounts: Vec<_> = html
        .mounts()
        .iter()
        .map(|mount| {
            let (name, text, top) = match (mount.language(), mount.document()) {
                (HtmlEmbeddedLanguage::Custom("json"), HtmlEmbeddedDocument::Custom(child)) => {
                    let child = child.downcast_ref::<JsonDocument>().unwrap();
                    ("json", child.text(), child.root().kind().name())
                }
                (HtmlEmbeddedLanguage::JavaScript, HtmlEmbeddedDocument::Javascript(child)) => {
                    ("JavaScript", child.text(), child.root().kind().name())
                }
                _ => panic!("unexpected mount: {:?}", mount.language()),
            };
            let ranges = pieces(mount.range());
            let projected: String = ranges
                .iter()
                .map(|range| doc.text().slice(range.clone()).unwrap())
                .collect();
            assert_eq!(text.as_str(), projected);
            json!([
                name,
                ranges.into_iter().map(bounds).collect::<Vec<_>>(),
                top
            ])
        })
        .collect();
    let strings: Vec<_> = doc
        .highlights(0..doc.text().len())
        .unwrap()
        .filter(|span| span.tags.iter().any(|tag| tag.name() == "string"))
        .map(|span| bounds(span.range))
        .collect();
    json!({
        "dialect": html_dialect_nodes(html),
        "ranges": pieces(0..html.text().len()).into_iter().map(bounds).collect::<Vec<_>>(),
        "mounts": mounts,
        "strings": strings,
    })
}

#[test]
fn mapped_html_has_no_php_ranges_and_recovery_keeps_real_prefix() {
    let source = "<p title='é<?= $x ?>🙂'>Hi</p>";
    let doc = PhpDocument::parse(Text::new(source));
    let html = doc.html().unwrap();
    assert_eq!(html.document().text().as_str(), "<p title='é🙂'>Hi</p>");
    let php = source.find("<?=").unwrap();
    assert_eq!(html.projected_offset(php), None);
    let emoji = source.find('🙂').unwrap();
    assert_eq!(html.projected_offset(emoji), Some(php));
    assert_eq!(html.projected_offset(emoji + 1), None);
    assert!(doc.highlights(emoji + 1..emoji + 2).is_none());
    let spans: Vec<_> = doc.highlights(emoji..emoji + 4).unwrap().collect();
    assert_eq!(spans[0].range, emoji..emoji + 4);
    assert_eq!(spans[0].tags[0].name(), "attributeValue");

    let doc = PhpDocument::parse_with_options(
        Text::new("echo 1;\n$x = [;"),
        PhpOptions {
            plain: true,
            html: false,
        },
    );
    assert!(doc.is_recovered());
    let nodes: Vec<_> = doc
        .root()
        .children()
        .map(|node| (node.name(), node.range()))
        .collect();
    assert_eq!(nodes, [("EchoStatement", 0..7), ("⚠", 8..15)]);
    assert_eq!(doc.highlights(8..15).unwrap().count(), 0);
}
