//! Fixtures from @codemirror/lang-json 6.0.2 / @lezer/json 1.0.3,
//! with @lezer/lr 1.4.13 and @codemirror/language 6.13.1.
//! Tree and highlight offsets are converted from UTF-16 to UTF-8. Fold and
//! indentation queries use logical lines; the native document retains CRLF.

use std::{num::NonZeroUsize, ops::Range};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Text,
    autocomplete::brackets::CloseBracketSyntax,
    language::{
        IndentOptions, StreamSettings,
        json::{
            CLOSE_BRACKETS, JsonDocument, JsonNode, JsonNodeKind, indent_on_input, language_data,
        },
    },
    lint::Severity,
};

fn parse(source: &str) -> JsonDocument {
    JsonDocument::parse(Text::new(source))
}

fn tree(node: JsonNode<'_>) -> String {
    let mut result = node.kind().name().to_owned();
    if node.children().len() != 0 {
        result.push('(');
        result.push_str(&node.children().map(tree).collect::<Vec<_>>().join(" "));
        result.push(')');
    }
    result
}

#[test]
fn reparse_combines_prefix_reuse_with_shifted_suffix_decisions() {
    let row = "  {\"value\": true, \"items\": [1, false, null]},\r\n".repeat(80);
    let source = format!("[\r\n{row}  \"😀\",\r\n  4242,\r\n{row}  false\r\n]\r\n");
    let mut doc = parse(&source);
    for (before_text, after_text, before_number, after_number) in [
        ("😀", "東京", "4242", "-10020"),
        ("東京", "一", "-10020", "481"),
    ] {
        let source = doc.text().as_str();
        let text = source.find(before_text).unwrap();
        let number = source.find(before_number).unwrap();
        let changes = ChangeSet::new(vec![
            Change::new(text..text + before_text.len(), after_text),
            Change::new(number..number + before_number.len(), after_number),
        ])
        .unwrap();
        doc = doc.reparse(&changes).unwrap();
        let fresh = parse(doc.text().as_str());
        let records = |doc: &JsonDocument| {
            doc.nodes()
                .map(|node| (node.kind(), node.range(), node.children().len()))
                .collect::<Vec<_>>()
        };
        assert_eq!(records(&doc), records(&fresh));
        assert!(
            doc.reuse_stats().nodes_reused > 100,
            "{:?}",
            doc.reuse_stats()
        );
        assert!(
            doc.reused_suffix_actions() > 100,
            "{}",
            doc.reused_suffix_actions()
        );
        assert!(doc.diagnostic().is_none());
    }
    let close = doc.text().as_str().rfind(']').unwrap();
    let malformed = doc
        .reparse(&ChangeSet::single(Change::new(close..close + 1, "")).unwrap())
        .unwrap();
    assert_eq!(malformed.reuse_stats().nodes_reused, 0);
    assert_eq!(malformed.reused_suffix_actions(), 0);
    assert!(malformed.diagnostic().is_some());
}

#[test]
fn parses_lezer_number_string_literal_object_and_array_fixtures() {
    for (source, expected) in [
        ("[ ]", "JsonText(Array([ ]))"),
        ("{ }", "JsonText(Object({ }))"),
        (
            r#"["The more the merrier", 1e5, true, { }, ["I'm", "nested"]]"#,
            "JsonText(Array([ String , Number , True , Object({ }) , Array([ String , String ]) ]))",
        ),
        (
            r#"{"foo":123,"bar":"I'm a bar!","obj":{},"arr":[1,2,3]}"#,
            "JsonText(Object({ Property(PropertyName : Number) , Property(PropertyName : String) , Property(PropertyName : Object({ })) , Property(PropertyName : Array([ Number , Number , Number ])) }))",
        ),
        (r#""\"\\\/\b\f\n\r\t""#, "JsonText(String)"),
        (r#""\u005C""#, "JsonText(String)"),
        (r#""\uD800""#, "JsonText(String)"),
        ("true", "JsonText(True)"),
        ("false", "JsonText(False)"),
        ("null", "JsonText(Null)"),
    ] {
        let doc = parse(source);
        assert_eq!(tree(doc.root()), expected, "{source:?}");
        assert!(doc.diagnostic().is_none(), "{source:?}");
    }
    for number in [
        "0", "42", "-53", "123.4", "1e5", "1E5", "1e+5", "1E-5", "-0", "1e9999",
    ] {
        let doc = parse(number);
        assert_eq!(tree(doc.root()), "JsonText(Number)", "{number:?}");
        assert!(doc.diagnostic().is_none(), "{number:?}");
    }
    for source in [
        r#"{"dup":1,"dup":2}"#,
        "\"\u{2028}\u{2029}\u{10ffff}\"",
        "\rnull\r",
    ] {
        assert!(parse(source).diagnostic().is_none(), "{source:?}");
    }
}

#[test]
fn keeps_source_byte_ranges_and_exposes_all_eight_highlight_tags() {
    let source = "\t{\r\n  \"名\": [\"😀\", -0.5E+2, true, false, null],\r\n  \"child\": {}\r\n}\n";
    let doc = parse(source);
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(
        doc.nodes()
            .map(|node| (node.kind().name(), node.range()))
            .collect::<Vec<_>>(),
        [
            ("JsonText", 0..69),
            ("Object", 1..68),
            ("{", 1..2),
            ("Property", 6..49),
            ("PropertyName", 6..11),
            (":", 11..12),
            ("Array", 13..49),
            ("[", 13..14),
            ("String", 14..20),
            (",", 20..21),
            ("Number", 22..29),
            (",", 29..30),
            ("True", 31..35),
            (",", 35..36),
            ("False", 37..42),
            (",", 42..43),
            ("Null", 44..48),
            ("]", 48..49),
            (",", 49..50),
            ("Property", 54..65),
            ("PropertyName", 54..61),
            (":", 61..62),
            ("Object", 63..65),
            ("{", 63..64),
            ("}", 64..65),
            ("}", 67..68),
        ],
    );
    let highlighted = |range| {
        doc.highlights(range)
            .unwrap()
            .map(|span| {
                assert_eq!(span.tags.len(), 1);
                (doc.text().slice(span.range).unwrap(), span.tags[0].name())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        highlighted(0..source.len()),
        [
            ("{", "brace"),
            ("\"名\"", "propertyName"),
            (":", "separator"),
            ("[", "squareBracket"),
            ("\"😀\"", "string"),
            (",", "separator"),
            ("-0.5E+2", "number"),
            (",", "separator"),
            ("true", "bool"),
            (",", "separator"),
            ("false", "bool"),
            (",", "separator"),
            ("null", "null"),
            ("]", "squareBracket"),
            (",", "separator"),
            ("\"child\"", "propertyName"),
            (":", "separator"),
            ("{", "brace"),
            ("}", "brace"),
            ("}", "brace"),
        ]
    );
    assert_eq!(highlighted(15..19), [("😀", "string")]);
    assert_eq!(
        highlighted(15..23),
        [("😀\"", "string"), (",", "separator"), ("-", "number")]
    );
    assert!(highlighted(15..15).is_empty());
    assert!(doc.highlights(16..19).is_none());
    assert!(doc.highlights(0..source.len() + 1).is_none());
    assert!(doc.highlights(Range { start: 4, end: 3 }).is_none());
}

#[test]
fn exposes_json_bracket_data_and_resolves_byte_safe_syntax_context() {
    let data = language_data();
    assert_eq!(data.get("closeBrackets"), Some(&CLOSE_BRACKETS));
    let indent: &fn(&str) -> bool = data.get("indentOnInput").unwrap();
    assert!(indent("  }"));
    assert!(!indent("  }x"));
    let doc = parse(r#"{"name":"字   }"}"#);
    let quote = doc.text().as_str().find("\"字").unwrap();
    assert_eq!(
        doc.node_at(quote, Some(Assoc::After)).unwrap().kind(),
        JsonNodeKind::String
    );
    assert_ne!(
        doc.node_at(quote, Some(Assoc::Before)).unwrap().kind(),
        JsonNodeKind::String
    );
    assert!(doc.node_at(quote + 2, None).is_none());
    let string = doc.resolve(quote + 1, None).unwrap();
    assert!(
        doc.text()
            .slice(doc.range(string))
            .unwrap()
            .starts_with('"')
    );
    let property = doc.parent(string).unwrap();
    let name = doc.first_child(property).unwrap();
    assert_eq!(doc.text().slice(doc.range(name)), Some("\"name\""));
}

#[test]
fn recovers_missing_and_unexpected_tokens_in_the_tree() {
    for (source, expected) in [
        ("", "JsonText(⚠)"),
        (" \t\r\n", "JsonText(⚠)"),
        ("[0123]", "JsonText(Array([ Number ⚠(Number) ]))"),
        ("123.", "JsonText(Number ⚠)"),
        ("42e", "JsonText(Number ⚠)"),
        ("[1,]", "JsonText(Array([ Number , ⚠ ]))"),
        ("[1,,3]", "JsonText(Array([ Number , ⚠ , Number ]))"),
        ("[,1]", "JsonText(Array([ ⚠(,) Number ]))"),
        (
            "[1 true 3]",
            "JsonText(Array([ Number ⚠(True) ⚠(Number) ]))",
        ),
        ("[1 {}]", "JsonText(Array([ Number ⚠ Object({ }) ]))"),
        ("[] []", "JsonText(Array([ ]) ⚠ Array([ ]))"),
        (
            r#"{"a":}"#,
            "JsonText(Object({ Property(PropertyName : ⚠) }))",
        ),
        (r#"{"a"}"#, "JsonText(Object({ Property(PropertyName ⚠) }))"),
        (
            r#"{"a" 1}"#,
            "JsonText(Object({ Property(PropertyName ⚠ Number) }))",
        ),
        (
            r#"{"a":1 "b":2}"#,
            "JsonText(Object({ Property(PropertyName : Number) ⚠ Property(PropertyName : Number) }))",
        ),
        (
            r#"{"a":1,}"#,
            "JsonText(Object({ Property(PropertyName : Number) , ⚠ }))",
        ),
        (
            r#"{a:1,"b":2}"#,
            "JsonText(Object({ ⚠ Property(PropertyName(⚠) : Number) , Property(PropertyName : Number) }))",
        ),
        (
            r#"{"a":[1,2}"#,
            "JsonText(Object({ Property(PropertyName : Array([ Number , Number ⚠)) }))",
        ),
        (
            r#"[{"a":1]"#,
            "JsonText(Array([ Object({ Property(PropertyName : Number) ⚠) ]))",
        ),
        ("[1 }", "JsonText(Array([ Number ⚠(})))"),
        ("}]", "JsonText(⚠(}) Array(⚠ ]))"),
        ("}@", "JsonText(⚠(}))"),
        (r#""bad\q""#, "JsonText(⚠)"),
        ("\"line\ntext\"", "JsonText(⚠ ⚠)"),
        (
            "/* hello */ [1,2]",
            "JsonText(⚠ ⚠ ⚠ Array([ Number , Number ]))",
        ),
    ] {
        let doc = parse(source);
        assert_eq!(tree(doc.root()), expected, "{source:?}");
        assert!(doc.diagnostic().is_some(), "{source:?}");
    }
    let doc = parse("[\"😀\", 0123]");
    let error = doc
        .nodes()
        .find(|node| node.kind() == JsonNodeKind::Error)
        .unwrap();
    assert_eq!(error.range(), 10..13);
    assert_eq!(
        error.children().next().unwrap().kind(),
        JsonNodeKind::Number
    );
    let span = doc.highlights(11..13).unwrap().next().unwrap();
    assert_eq!((span.range, span.tags[0].name()), (11..13, "number"));
    assert_eq!(
        parse(" \t\r\n").root().children().next().unwrap().range(),
        4..4
    );
    let doc = parse("[🐍@👻,🐍]");
    let errors: Vec<_> = doc
        .nodes()
        .filter(|node| node.kind() == JsonNodeKind::Error)
        .map(JsonNode::range)
        .collect();
    assert_eq!(errors, [1..10, 10..15]);
}

#[test]
fn reports_native_recovery_errors_at_utf8_boundaries() {
    for (source, position, message) in [
        ("[\"😀\", 0123]", 10, "Expected ',' or ']'"),
        ("[1,\n  , 3]", 6, "Expected JSON array element"),
        (r#"{"a" 1}"#, 5, "Expected ':'"),
        ("\"😀\\q\"", 6, "Invalid escape"),
        ("\"\\u12💥\"", 5, "Invalid Unicode escape"),
        ("42e+", 4, "Expected a digit"),
        ("\"hi", 3, "Unterminated JSON string"),
        ("\"line\r\ntext\"", 5, "Unescaped control character"),
        ("[true, #, 2], #", 7, "Expected a JSON value"),
        ("[1, 2", 5, "Expected ']'"),
        ("nullx", 4, "Unexpected content"),
    ] {
        let doc = parse(source);
        let diagnostic = doc.diagnostic().expect("native recovery error");
        assert_eq!(diagnostic.severity, Severity::Error);
        assert_eq!(diagnostic.range, position..position, "{source:?}");
        assert!(source.is_char_boundary(position));
        assert!(
            diagnostic.message.contains(message),
            "{source:?}: {diagnostic:?}"
        );
    }
}

#[test]
fn finds_folds_for_closed_and_unfinished_containers() {
    let doc = parse("[[\n  1\n]]");
    assert_eq!(doc.foldable(0, 2), Some(1..8));
    assert_eq!(doc.foldable(3, 6), None);
    assert_eq!(doc.foldable(7, 9), None);
    let ranges = |doc: &JsonDocument| {
        doc.nodes()
            .filter_map(JsonNode::fold_range)
            .collect::<Vec<_>>()
    };
    assert_eq!(ranges(&doc), [1..8, 2..7]);
    assert!(ranges(&parse("[]")).is_empty());
    assert_eq!(
        parse("{ }").root().children().next().unwrap().fold_range(),
        Some(1..2)
    );
    assert_eq!(
        parse("[1, 2")
            .root()
            .children()
            .next()
            .unwrap()
            .fold_range(),
        Some(1..5)
    );
    assert!(ranges(&parse("{")).is_empty());
    let doc = parse("{\n  \"unfinished\": [\n    10\n");
    assert_eq!(doc.foldable(0, 1), Some(1..27));
    assert_eq!(doc.foldable(2, 19), Some(19..27));
    assert_eq!(ranges(&doc), [1..27, 19..27]);
    assert_eq!(parse("[\"😀\"]").foldable(3, 4), None);
}

#[test]
fn indents_by_container_reference_lines_and_matching_closers() {
    for (source, expected) in [
        (
            "\t{\r\n\"a\": [\r\ntrue,\r\nfalse\r\n],\r\n\"b\":1\r\n}\r\n",
            &[0, 6, 2, 2, 0, 6, 4, 0][..],
        ),
        ("  [\n      1, [\n       2\n      ]\n  ]", &[0, 4, 8, 6, 2]),
        ("{\n \"a\": {\"b\": [\n1\n ]}}", &[0, 2, 3, 1]),
        (
            "[\n   [\n      1\n       ], [\n        2\n      ]\n]",
            &[0, 2, 5, 3, 5, 3, 0],
        ),
        ("{\n  \"unfinished\": [\n    10\n", &[0, 2, 4, 4]),
        ("[\n  1,\n  [\n    2\n  }\n", &[0, 2, 2, 4, 4, 2]),
    ] {
        let doc = parse(source);
        let got: Vec<_> = (1..=doc.text().line_count())
            .map(|n| doc.text().line(n).unwrap().range.start)
            .map(|pos| {
                doc.indentation(pos, StreamSettings::default(), IndentOptions::default())
                    .unwrap()
            })
            .collect();
        assert_eq!(got, expected, "{source:?}");
    }
    let doc = parse("\t{  }\n\"😀\"");
    let settings = StreamSettings {
        tab_size: NonZeroUsize::new(8).unwrap(),
        indent_unit: 3,
    };
    let options = IndentOptions {
        simulate_break: Some(2),
        ..IndentOptions::default()
    };
    assert_eq!(doc.indentation(2, settings, options), Some(8));
    assert_eq!(
        doc.indentation(
            2,
            settings,
            IndentOptions {
                simulate_double_break: true,
                override_indentation: Some(&|line| (line == 0).then_some(5)),
                ..options
            }
        ),
        Some(8)
    );
    assert_eq!(doc.indentation(8, settings, options), None);
    assert_eq!(
        doc.indentation(doc.text().len() + 1, settings, options),
        None
    );
    assert_eq!(CLOSE_BRACKETS, ['[', '{', '"']);
    assert!(indent_on_input(" \t\u{feff}]"));
    assert!(indent_on_input("\t}"));
    assert!(!indent_on_input("}],"));
    assert!(!indent_on_input("] "));
}

#[test]
fn retains_json_syntax_across_valid_edits_and_recovers_after_invalid_edits() {
    let source = format!(
        "[\r\n{}  {{\"tail\": \"字\"}}\r\n]",
        "  {\"x\": \"😀\"},\r\n".repeat(512)
    );
    let mut doc = parse(&source);
    for replacement in ["猫", "ß", "東京"] {
        let start = doc.text().as_str().rfind(": \"").unwrap() + 3;
        let end = start + doc.text().as_str()[start..].find('"').unwrap();
        let changes = ChangeSet::single(Change::new(start..end, replacement)).unwrap();
        let next = doc.reparse(&changes).unwrap();
        assert!(next.reuse_stats().nodes_reused > 1000);
        assert!(next.reuse_stats().resumed_at <= start);
        assert!(next.diagnostic().is_none());
        let fresh = parse(next.text().as_str());
        let records = |doc: &JsonDocument| {
            doc.nodes()
                .map(|node| (node.kind(), node.range()))
                .collect::<Vec<_>>()
        };
        assert_eq!(records(&next), records(&fresh));
        assert_eq!(next.foldable(0, 1), fresh.foldable(0, 1));
        assert!(next.text().as_str().contains("\r\n"));
        doc = next;
    }
    let non_boundary = doc.text().as_str().find('東').unwrap() + 1;
    let illegal = ChangeSet::single(Change::new(non_boundary..non_boundary, "]")).unwrap();
    assert!(doc.reparse(&illegal).is_err());
    let close = doc.text().len() - 1;
    let malformed = doc
        .reparse(&ChangeSet::single(Change::new(close..close + 1, "")).unwrap())
        .unwrap();
    assert!(malformed.diagnostic().is_some());
    assert_eq!(malformed.reuse_stats().nodes_reused, 0);
    let repaired = malformed
        .reparse(&ChangeSet::single(Change::new(close..close, "]")).unwrap())
        .unwrap();
    assert!(repaired.diagnostic().is_none());
    assert_eq!(repaired.text(), doc.text());
    assert_eq!(tree(repaired.root()), tree(doc.root()));
}

#[test]
fn parses_and_drops_deep_syntax_without_recursing() {
    let depth = 12_000;
    let source = ["[".repeat(depth), "0".into(), "]".repeat(depth)].concat();
    let doc = parse(&source);
    assert!(doc.diagnostic().is_none());
    // The pinned parser splits deep trees after forcing the large reductions.
    assert_eq!(doc.nodes().len(), 52_404);
    assert_eq!(doc.highlights(depth..depth + 1).unwrap().count(), 1);
    // The unfinished tree also follows the pinned parser's forced reductions.
    let unfinished = parse(&"[".repeat(depth));
    assert_eq!(unfinished.nodes().len(), 32_397);
    assert!(unfinished.diagnostic().is_some());
}
