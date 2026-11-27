use gpui_editor_codemirror::{
    Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        python::{CLOSE_BRACKETS, LINE_COMMENT, PythonDocument, STRING_PREFIXES, indent_on_input},
    },
};

fn settings() -> StreamSettings {
    StreamSettings {
        indent_unit: 4,
        ..StreamSettings::default()
    }
}

// Exact outputs from SHA512-verified @lezer/python 1.1.20 and
// @codemirror/lang-python 6.2.1. Convert upstream UTF-16 offsets to UTF-8.
#[test]
fn context_highlights_follow_the_pinned_parser() {
    let source = "# note\ndef café(value):\n    return pkg.call(f\"Hi {value!r:>{4}}\")\n\nclass C:\n    text = \"\"\"first\nsecond\"\"\"\n";
    let doc = PythonDocument::parse(Text::new(source));
    assert!(!doc.is_recovered());
    assert_eq!(doc.text().as_str(), source);
    use TagModifier::{Definition, Function, Special};
    let actual = doc
        .highlights(0..source.len())
        .unwrap()
        .map(|span| {
            let [tag] = span.tags else {
                panic!("expected one tag")
            };
            (span.range, tag.name(), tag.modifiers())
        })
        .collect::<Vec<_>>();
    let expected: &[(std::ops::Range<usize>, &str, &[TagModifier])] = &[
        (0..6, "lineComment", &[]),
        (7..10, "definitionKeyword", &[]),
        (11..16, "variableName", &[Definition, Function]),
        (16..17, "paren", &[]),
        (17..22, "variableName", &[]),
        (22..23, "paren", &[]),
        (29..35, "controlKeyword", &[]),
        (36..39, "variableName", &[]),
        (39..40, "derefOperator", &[]),
        (40..44, "propertyName", &[Function]),
        (44..45, "paren", &[]),
        (45..50, "string", &[Special]),
        (50..51, "brace", &[]),
        (51..56, "variableName", &[]),
        (56..58, "modifier", &[]),
        (59..60, "modifier", &[]),
        (60..61, "brace", &[]),
        (61..62, "number", &[]),
        (62..63, "brace", &[]),
        (63..64, "brace", &[]),
        (64..65, "string", &[Special]),
        (65..66, "paren", &[]),
        (68..73, "definitionKeyword", &[]),
        (74..75, "className", &[Definition]),
        (81..85, "variableName", &[]),
        (86..87, "definitionOperator", &[]),
        (88..106, "string", &[]),
    ];
    assert_eq!(actual, expected);
    assert!(doc.highlights(14..15).is_none()); // Middle byte of `é`.
    assert!(doc.highlights(0..source.len() + 1).is_none());
    let (start, end) = (16, 15);
    assert!(doc.highlights(start..end).is_none());
    assert_eq!(doc.highlights(14..14).unwrap().count(), 0);
    assert_eq!(
        doc.highlights(14..16).unwrap().next().unwrap().range,
        14..16
    );
}

#[test]
fn tabs_soft_keywords_and_legacy_print_match_the_pinned_tree() {
    let source = "match pair:\n\tcase (a, b):\n\t\tshow(a, b)\n\tcase _:\n\t\tpass\n\n# outer\nmatch = 5\ncase = 7\nprint 1\nprint(2)\n";
    let doc = PythonDocument::parse(Text::new(source));
    assert!(!doc.is_recovered());
    let root = doc.root();
    assert_eq!(root.name(), "Script");
    assert_eq!(root.range(), 0..source.len());
    assert_eq!(
        root.children()
            .map(|node| (node.name(), node.range()))
            .collect::<Vec<_>>(),
        [
            ("MatchStatement", 0..55),
            ("Comment", 56..63),
            ("AssignStatement", 64..73),
            ("AssignStatement", 74..82),
            ("PrintStatement", 83..90),
            ("ExpressionStatement", 91..99),
        ]
    );
    assert_eq!(
        doc.nodes()
            .filter(|node| node.name() == "MatchClause")
            .map(|node| node.range())
            .collect::<Vec<_>>(),
        [13..39, 40..55]
    );
    assert!(
        root.children()
            .all(|node| node.parent().unwrap().name() == "Script")
    );
}

#[test]
fn nested_body_indents_and_folds_match_the_pinned_rules() {
    let source = "if flag:\n    if other:\n        call(1,\n             2)\n        # inside\n    else:\n        return\n\ntry:\n    data = [\n        1,\n        2,\n    ]\nexcept ValueError:\n    pass\n";
    let doc = PythonDocument::parse(Text::new(source));
    let mut indents = Vec::new();
    let mut folds = Vec::new();
    for n in 1..=doc.text().line_count() {
        let line = doc.text().line(n).unwrap();
        indents.push(doc.indentation(line.range.start, settings(), IndentOptions::default()));
        if let Some(range) = doc.foldable(line.range.start, line.range.end) {
            folds.push((line.range.start, range));
        }
    }
    assert_eq!(
        indents,
        [0, 4, 8, 13, 8, 4, 8, 0, 0, 4, 8, 8, 4, 0, 4, 0].map(Some)
    );
    assert_eq!(
        folds,
        [
            (0, 8..96),
            (9, 22..54),
            (72, 81..96),
            (98, 102..143),
            (103, 115..142),
            (144, 162..172)
        ]
    );
}

#[test]
fn delimiter_alignment_respects_indentation_query_options() {
    let source = "x = call(first,\n         second,\n         )\ny = [\n    x,\n]\nmessage = \"\"\"x\n  y\n\"\"\"\n";
    let doc = PythonDocument::parse(Text::new(source));
    let query = |position, options| doc.indentation(position, settings(), options);
    assert_eq!(query(33, IndentOptions::default()), Some(8));
    assert_eq!(query(74, IndentOptions::default()), None);
    assert_eq!(
        query(
            9,
            IndentOptions {
                simulate_break: Some(9),
                ..IndentOptions::default()
            }
        ),
        Some(4)
    );
    assert_eq!(
        query(
            16,
            IndentOptions {
                override_indentation: Some(&|line| (line == 0).then_some(2)),
                ..IndentOptions::default()
            }
        ),
        Some(11)
    );
    assert_eq!(
        query(
            33,
            IndentOptions {
                simulate_break: Some(33),
                simulate_double_break: true,
                ..IndentOptions::default()
            }
        ),
        Some(9)
    );
}

#[test]
fn crlf_and_escape_spans_are_valid_byte_ranges() {
    let source = "# 🐍\r\ndef f():\r\n    return '↑\\u03c0'\r\n";
    let doc = PythonDocument::parse(Text::new(source));
    assert!(!doc.is_recovered());
    let range = source.find("\\u03c0").unwrap()..source.find("\\u03c0").unwrap() + 6;
    let spans = doc.highlights(range.clone()).unwrap().collect::<Vec<_>>();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].range, range);
    assert_eq!(spans[0].tags[0].name(), "escape");
    let header = doc.text().line(2).unwrap().range;
    assert_eq!(
        doc.foldable(header.start, header.end),
        Some(header.end..source.len())
    );
    for node in doc.nodes() {
        assert!(doc.text().slice(node.range()).is_some());
    }
}

#[test]
fn malformed_lines_keep_prefix_syntax_and_suffix_highlights() {
    let source = "def ok():\n    return 3\n?\nclass Later:\n    data = 1\n";
    let doc = PythonDocument::parse(Text::new(source));
    assert!(doc.is_recovered());
    assert_eq!(
        doc.root()
            .children()
            .map(|node| node.name())
            .collect::<Vec<_>>(),
        ["FunctionDefinition", "⚠"]
    );
    let start = source.find("Later").unwrap();
    let span = doc.highlights(start..start + 5).unwrap().next().unwrap();
    assert_eq!(span.tags[0].name(), "className");
    assert_eq!(span.tags[0].modifiers(), [TagModifier::Definition]);
    let start = source.find("    data").unwrap();
    assert_eq!(
        doc.indentation(start, settings(), IndentOptions::default()),
        Some(4)
    );
}

#[test]
fn unfinished_delimiters_keep_a_tree_and_do_not_leak_synthetic_text() {
    let source = "call(1,\n  [3, 4";
    let doc = PythonDocument::parse(Text::new(source));
    assert!(doc.is_recovered());
    assert_eq!(doc.nodes().filter(|node| node.name() == "⚠").count(), 2);
    assert!(doc.nodes().any(|node| node.name() == "ArrayExpression"));
    assert_eq!(doc.text().as_str(), source);
    assert!(
        doc.highlights(0..source.len())
            .unwrap()
            .all(|span| span.range.end <= source.len())
    );
}

#[test]
fn fallback_is_safe_at_the_document_and_context_limits() {
    let large = format!("{}\nprint(1)\n", " ".repeat(1 << 20));
    let deep = format!("{}0{}\nprint(1)\n", "(".repeat(257), ")".repeat(257));
    for source in [&large, &deep] {
        let doc = PythonDocument::parse(Text::new(source.as_str()));
        assert!(doc.is_recovered());
        assert_eq!(doc.root().range(), 0..source.len());
        let start = source.find("print").unwrap();
        assert_eq!(
            doc.highlights(start..start + 5)
                .unwrap()
                .next()
                .unwrap()
                .tags[0]
                .name(),
            "variableName"
        );
    }
}

#[test]
fn language_data_and_indent_triggers() {
    assert_eq!(LINE_COMMENT, "#");
    assert_eq!(CLOSE_BRACKETS, ["(", "[", "{", "'", "\"", "'''", "\"\"\""]);
    assert!(STRING_PREFIXES.contains(&"RF"));
    for text in [
        "\t)",
        "  else:",
        "    elif ",
        "except ",
        "finally:",
        " case [x, y]:",
        "case x",
    ] {
        assert!(indent_on_input(text), "{text:?}");
    }
    for text in [
        "else",
        "elif",
        "except:",
        "case",
        "case = {1: 2}",
        " pass",
        "x)",
    ] {
        assert!(!indent_on_input(text), "{text:?}");
    }
}
