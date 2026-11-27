use std::collections::HashMap;

use gpui_editor_codemirror::{
    Text,
    commands::CommentTokens,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        html::HtmlNode,
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
