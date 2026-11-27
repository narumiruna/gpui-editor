//! Fixtures from the SRI-verified `@codemirror/lang-wast` 6.0.2 tarball, which
//! includes the Lezer parser. The oracle used `@codemirror/language` 6.13.1,
//! `@codemirror/state` 6.7.6, `@lezer/common` 1.5.7, `@lezer/lr` 1.4.13, and
//! `@lezer/highlight` 1.2.5. Results from `syntaxTree`, `highlightTree`,
//! `getIndentation`, and `foldable` use original UTF-8 bytes in place of
//! the editor's UTF-16 offsets and normalized line endings.

use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, Text,
    language::{
        IndentOptions, StreamSettings,
        wast::{BLOCK_COMMENT, CLOSE_BRACKETS, LINE_COMMENT, WastDocument, WastNode, WastNodeKind},
    },
};

fn parse(source: &str) -> WastDocument {
    WastDocument::parse(Text::new(source))
}

fn tree(node: WastNode<'_>) -> String {
    let kind = node.kind().name();
    let name = if matches!(kind, "(" | ")") {
        format!("{kind:?}")
    } else {
        kind.to_owned()
    };
    if node.children().len() == 0 {
        name
    } else {
        format!(
            "{name}({})",
            node.children().map(tree).collect::<Vec<_>>().join(",")
        )
    }
}

fn check_lines(doc: &WastDocument, expected: &[(usize, Option<Range<usize>>)]) {
    assert_eq!(doc.text().line_count(), expected.len());
    for (i, (indent, fold)) in expected.iter().enumerate() {
        let line = doc.text().line(i + 1).unwrap();
        assert_eq!(
            doc.indentation(
                line.range.start,
                StreamSettings::default(),
                IndentOptions::default()
            ),
            Some(*indent),
            "indentation on line {}: {:?}",
            i + 1,
            line.text,
        );
        assert_eq!(
            doc.foldable(line.range.start, line.range.end),
            *fold,
            "fold on line {}: {:?}",
            i + 1,
            line.text,
        );
    }
}

#[test]
fn reparse_checks_both_ends_when_an_edit_splits_crlf() {
    let rows = "  (func $f (result i32) i32.const 0) ;; 😀\r\n".repeat(80);
    let source = format!("(module\r\n{rows}  (export \"尾🦀\" (func $f))\r\n{rows})\r\n");
    let doc = parse(&source);
    let at = source.find('🦀').unwrap();
    let cr = source[at..].find('\r').unwrap() + at;
    let changes = ChangeSet::new(vec![
        Change::new(at..at + '🦀'.len_utf8(), "東京"),
        Change::new(cr..cr + 1, ""),
    ])
    .unwrap();
    let updated = doc.reparse(&changes).unwrap();
    let fresh = parse(updated.text().as_str());
    let records = |doc: &WastDocument| {
        doc.nodes()
            .map(|node| (node.kind(), node.range(), node.children().len()))
            .collect::<Vec<_>>()
    };
    assert_eq!(records(&updated), records(&fresh));
    assert!(updated.reuse_stats().nodes_reused > 100);
    assert!(updated.reused_suffix_actions() > 100);
    assert!(updated.text().as_str().contains("尾東京\" (func $f))\n"));
}

#[test]
fn strict_checkpoints_keep_wast_ranges_in_original_bytes() {
    let source = format!(
        "(module\r\n{}  (export \"fin 😀\" (func $f)))",
        "  (func $f (result i32) i32.const 0) ;; 😀\r\n".repeat(120)
    );
    let mut doc = parse(&source);
    let records = |doc: &WastDocument| {
        doc.nodes()
            .map(|node| (node.kind(), node.range(), node.children().len()))
            .collect::<Vec<_>>()
    };
    let change = |range, insert| ChangeSet::single(Change::new(range, insert)).unwrap();
    let edit = source.rfind("fin").unwrap();
    let line_break = source[..edit].rfind("\r\n").unwrap();
    let source = doc.text().clone();
    for changes in [
        change(edit..edit + 3, "終"),
        change(line_break + 1..line_break + 2, ""),
    ] {
        doc = doc.reparse(&changes).unwrap();
        let stats = doc.reuse_stats();
        assert!(stats.nodes_reused > 100);
        assert!(stats.resumed_at <= changes.changes().next().unwrap().0.start);
        let fresh = parse(doc.text().as_str());
        assert_eq!(records(&doc), records(&fresh));
    }
    assert_ne!(doc.text(), &source);
    let close = doc.text().len() - 1;
    let bad = doc.reparse(&change(close..close + 1, "")).unwrap();
    assert_eq!(bad.reuse_stats(), Default::default());
    assert_eq!(bad.reused_suffix_actions(), 0);
    let repaired = bad.reparse(&change(close..close, ")")).unwrap();
    assert_eq!(repaired.text(), doc.text());
    assert_eq!(records(&repaired), records(&doc));
    assert!(repaired.reuse_stats().nodes_reused > 100);
    assert!(repaired.reuse_stats().resumed_at <= close);

    let early = ChangeSet::new(vec![
        Change::new(0..0, ";; early\r"),
        Change::new(close..close, ")"),
    ])
    .unwrap();
    let updated = bad.reparse(&early).unwrap();
    assert_eq!(updated.reuse_stats(), Default::default());
    assert_eq!(updated.reused_suffix_actions(), 0);
    assert_eq!(records(&updated), records(&parse(updated.text().as_str())));
}

#[test]
fn matches_wat_tree_highlights_indents_and_folds() {
    let source = concat!(
        "(module\r\n  ;; entrée 😀\r\n  (; a\r\n    (b) ;)\r\n",
        "  (func $f (result i32)\r\n    i32.const -0x1_f)\r\n",
        "  (export \"résultat\\\" 😀\" (func $f)))",
    );
    let doc = parse(source);
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(doc.root().range(), 0..136);
    assert_eq!(
        tree(doc.root()),
        r#"Module(App("(",Keyword,LineComment,BlockComment,App("(",Keyword,Identifier,App("(",Keyword,Type,")"),Keyword,Number,")"),App("(",Keyword,String,App("(",Keyword,Identifier,")"),")"),")"))"#,
    );
    let expected = [
        (0..1, "paren"),
        (1..7, "keyword"),
        (11..26, "lineComment"),
        (30..46, "blockComment"),
        (50..51, "paren"),
        (51..55, "keyword"),
        (56..58, "variableName"),
        (59..60, "paren"),
        (60..66, "keyword"),
        (67..70, "typeName"),
        (70..71, "paren"),
        (77..86, "keyword"),
        (87..93, "number"),
        (93..94, "paren"),
        (98..99, "paren"),
        (99..105, "keyword"),
        (106..124, "string"),
        (125..126, "paren"),
        (126..130, "keyword"),
        (131..133, "variableName"),
        (133..134, "paren"),
        (134..135, "paren"),
        (135..136, "paren"),
    ];
    let spans = doc.highlights(0..source.len()).unwrap().map(|span| {
        let [tag] = span.tags else {
            panic!("{span:?}");
        };
        assert!(tag.modifiers().is_empty());
        (span.range, tag.name())
    });
    assert_eq!(spans.collect::<Vec<_>>(), expected);
    let emoji = source.rfind('😀').unwrap();
    let mut spans = doc.highlights(emoji..emoji + 4).unwrap();
    let span = spans.next().unwrap();
    assert_eq!(span.range, emoji..emoji + 4);
    assert_eq!(span.tags[0].name(), "string");
    assert!(spans.next().is_none());
    assert!(doc.highlights(emoji + 1..emoji + 4).is_none());
    assert!(doc.highlights(emoji..emoji).unwrap().next().is_none());
    assert!(doc.highlights(Range { start: 2, end: 1 }).is_none());
    assert!(doc.foldable(0, emoji + 1).is_none());
    assert!(
        doc.indentation(
            emoji + 1,
            StreamSettings::default(),
            IndentOptions::default()
        )
        .is_none()
    );
    check_lines(
        &doc,
        &[
            (0, Some(1..135)),
            (2, None),
            (2, Some(32..44)),
            (2, None),
            (2, Some(51..93)),
            (4, None),
            (2, None),
        ],
    );
    assert_eq!(LINE_COMMENT, ";;");
    assert_eq!(BLOCK_COMMENT, ("(;", ";)"));
    assert_eq!(CLOSE_BRACKETS, ['(', '"']);
}

#[test]
fn matches_script_and_lexical_grammar() {
    let doc = parse(r#"(assert_return (invoke "add" (i32.const 1)) (i32.const 2))"#);
    assert_eq!(
        tree(doc.root()),
        r#"Module(App("(",Keyword,App("(",Keyword,String,App("(",Keyword,Number,")"),")"),App("(",Keyword,Number,")"),")"))"#
    );
    assert_eq!(
        doc.root().children().next().unwrap().fold_range(),
        Some(1..57)
    );
    check_lines(&doc, &[(0, None)]);

    use WastNodeKind::{Error, Identifier, Keyword, Number, Type};
    let doc = parse(concat!(
        "anyref dataref eqref externref i31ref funcref i8 i16 i32 i64 f32 f64 ",
        "i32.load v128 ref null nan:0x1 inf +inf -12.3_4e-5_6 +0xF_A.B_CP+e_f ",
        "0X10 0x 1E- 1__2 $ $$ $foo^bar $é",
    ));
    let leaves: Vec<_> = doc
        .root()
        .children()
        .map(|node| (node.kind(), doc.text().slice(node.range()).unwrap()))
        .collect();
    assert_eq!(
        &leaves[..12],
        [
            "anyref",
            "dataref",
            "eqref",
            "externref",
            "i31ref",
            "funcref",
            "i8",
            "i16",
            "i32",
            "i64",
            "f32",
            "f64"
        ]
        .map(|name| (Type, name))
    );
    assert_eq!(
        &leaves[12..],
        [
            (Keyword, "i32.load"),
            (Keyword, "v128"),
            (Keyword, "ref"),
            (Keyword, "null"),
            (Keyword, "nan:0x1"),
            (Keyword, "inf"),
            (Error, "+"),
            (Keyword, "inf"),
            (Number, "-12.3_4e-5_6"),
            (Number, "+0xF_A.B_CP+e_f"),
            (Number, "0"),
            (Error, "X"),
            (Number, "10"),
            (Number, "0"),
            (Keyword, "x"),
            (Number, "1"),
            (Error, "E-"),
            (Number, "1"),
            (Error, "__"),
            (Number, "2"),
            (Error, "$"),
            (Identifier, "$$"),
            (Identifier, "$foo^bar"),
            (Error, "$é"),
        ]
    );
    assert_eq!(
        tree(parse("\"line\n two\" \"escaped\\😀\"").root()),
        "Module(String,String)"
    );
    let doc = parse(";; x\raaa");
    assert_eq!(tree(doc.root()), "Module(LineComment,Keyword)");
    assert_eq!(doc.root().children().next().unwrap().range(), 0..4);
}

#[test]
fn recovers_errors_and_preserves_ranges() {
    let source = ")(module\n  (func ??? \n    (i32.const 😶\n";
    let doc = parse(source);
    assert_eq!(
        doc.nodes()
            .map(|node| (node.kind().name(), node.range()))
            .collect::<Vec<_>>(),
        [
            ("Module", 0..42),
            ("⚠", 0..1),
            (")", 0..1),
            ("App", 1..42),
            ("(", 1..2),
            ("Keyword", 2..8),
            ("App", 11..42),
            ("(", 11..12),
            ("Keyword", 12..16),
            ("⚠", 17..20),
            ("App", 26..42),
            ("(", 26..27),
            ("Keyword", 27..36),
            ("⚠", 37..41),
            ("⚠", 42..42),
            ("⚠", 42..42),
        ],
    );
    check_lines(
        &doc,
        &[
            (0, Some(2..42)),
            (2, Some(12..42)),
            (4, Some(27..42)),
            (4, None),
        ],
    );
    assert_eq!(
        doc.highlights(0..1).unwrap().next().unwrap().tags[0].name(),
        "paren"
    );
    assert_eq!(doc.highlights(37..41).unwrap().count(), 0);

    for (source, expected) in [
        ("", "Module"),
        (" \t\r\n", "Module"),
        (
            "(; outer (; inner ;) still inside ;)??",
            r#"Module(BlockComment,Keyword,Keyword,⚠,⚠(")"))"#,
        ),
        ("(; unmatched", r#"Module(App("(",⚠,Keyword,⚠))"#),
        ("(;)", r#"Module(App("(",⚠,")"))"#),
        ("(;;)", "Module(BlockComment)"),
        ("(foo\n  \"bad", r#"Module(App("(",Keyword,⚠,Keyword,⚠))"#),
    ] {
        assert_eq!(tree(parse(source).root()), expected, "{source:?}");
    }
    let comment = parse("(;;)");
    assert_eq!(
        comment.root().children().next().unwrap().fold_range(),
        Some(2..2)
    );
    assert_eq!(comment.foldable(0, 4), None);
}

#[test]
fn maps_published_tokens_to_bytes_across_mixed_line_endings() {
    let source = concat!(
        ";; 🦀\r(module\r\n  (func $f (result funcref)\r\n",
        "    (export \"f\\\r\n😀\")))\r\n",
    );
    let doc = parse(source);
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(doc.root().range(), 0..73);
    assert_eq!(
        tree(doc.root()),
        r#"Module(LineComment,App("(",Keyword,App("(",Keyword,Identifier,App("(",Keyword,Type,")"),App("(",Keyword,String,")"),")"),")"))"#,
    );
    let highlights: Vec<_> = doc
        .highlights(0..source.len())
        .unwrap()
        .map(|span| (span.range, span.tags[0].name()))
        .collect();
    assert_eq!(
        highlights,
        [
            (0..7, "lineComment"),
            (8..9, "paren"),
            (9..15, "keyword"),
            (19..20, "paren"),
            (20..24, "keyword"),
            (25..27, "variableName"),
            (28..29, "paren"),
            (29..35, "keyword"),
            (36..43, "typeName"),
            (43..44, "paren"),
            (50..51, "paren"),
            (51..57, "keyword"),
            (58..68, "string"),
            (68..69, "paren"),
            (69..70, "paren"),
            (70..71, "paren"),
        ],
    );
    assert_eq!(doc.text().slice(58..68), Some("\"f\\\r\n😀\""));
}

#[test]
fn matches_recovered_app_ranges_after_comments_and_crlf() {
    let doc = parse(concat!(
        "(module\r\n (; outer\r\n  (; inner ;)\r\n  ;; tail\r\n",
        " (func $incomplete\r\n  (i32.const 1)\r\n",
    ));
    assert_eq!(
        doc.nodes()
            .map(|node| (node.kind().name(), node.range()))
            .collect::<Vec<_>>(),
        [
            ("Module", 0..83),
            ("App", 0..83),
            ("(", 0..1),
            ("Keyword", 1..7),
            ("BlockComment", 10..33),
            ("LineComment", 37..44),
            ("App", 47..83),
            ("(", 47..48),
            ("Keyword", 48..52),
            ("Identifier", 53..64),
            ("App", 68..81),
            ("(", 68..69),
            ("Keyword", 69..78),
            ("Number", 79..80),
            (")", 80..81),
            ("⚠", 83..83),
            ("⚠", 83..83),
        ],
    );
    assert_eq!(
        tree(doc.root()),
        r#"Module(App("(",Keyword,BlockComment,LineComment,App("(",Keyword,Identifier,App("(",Keyword,Number,")"),⚠),⚠))"#,
    );
    check_lines(
        &doc,
        &[
            (0, Some(1..83)),
            (2, Some(12..31)),
            (2, None),
            (2, None),
            (2, Some(48..83)),
            (3, None),
            (3, None),
        ],
    );
}

#[test]
fn indentation_uses_the_reference_line_without_alignment() {
    check_lines(
        &parse("  (module\n\t(func\n  ) (func\nx\n)\n)"),
        &[
            (0, Some(3..31)),
            (4, Some(12..19)),
            (4, Some(22..29)),
            (6, None),
            (4, None),
            (2, None),
        ],
    );
    check_lines(
        &parse("  (; block\r\nend ;) (outer (inner\r\n    x\r\n  ))\r\n"),
        &[
            (0, Some(4..16)),
            (0, Some(20..44)),
            (4, None),
            (2, None),
            (0, None),
        ],
    );
    check_lines(
        &parse("(foo\n?\n\n"),
        &[(0, Some(1..8)), (2, None), (2, None), (0, None)],
    );
    // A multiline token can supply a reference line, unless it starts at
    // the same position as an ancestor of the expression.
    check_lines(
        &parse("\"a\n\tb\" (bar\na\n)"),
        &[(0, None), (0, Some(8..14)), (6, None), (4, None)],
    );
    check_lines(
        &parse(" \"a\n\tb\" (bar\na\n)"),
        &[(0, None), (0, Some(9..15)), (3, None), (1, None)],
    );

    // Indentation follows the editor's whitespace rules, not the lexer.
    check_lines(
        &parse("\u{b}(\n \tname\n)"),
        &[(0, Some(2..10)), (3, None), (1, None)],
    );

    let doc = parse("  (module (func\n    i32.const 1\n )\n)");
    let pos = 32;
    let override_indent = |at| (at == 0).then_some(6);
    let indentation = |options| doc.indentation(pos, StreamSettings::default(), options);
    assert_eq!(indentation(IndentOptions::default()), Some(2));
    assert_eq!(
        indentation(IndentOptions {
            override_indentation: Some(&override_indent),
            ..Default::default()
        }),
        Some(6)
    );
    let options = IndentOptions {
        simulate_break: Some(pos),
        simulate_double_break: true,
        ..Default::default()
    };
    assert_eq!(indentation(options), Some(4));
    assert_eq!(
        indentation(IndentOptions {
            override_indentation: Some(&override_indent),
            ..options
        }),
        Some(8)
    );
}

#[test]
fn indentation_uses_editor_line_breaks_for_lone_cr() {
    // In EditorState, the reference line starts after the CR, not at the tab.
    let doc = parse("\t\r(");
    assert_eq!(tree(doc.root()), r#"Module(App("(",⚠))"#);
    assert_eq!(
        doc.indentation(3, StreamSettings::default(), IndentOptions::default()),
        Some(2)
    );

    let source = " \t;; 🦀\r  (func\r\n    (i32.const 0)\r  )\n";
    let doc = parse(source);
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(doc.text().line_count(), 3);
    assert_eq!(doc.syntax_lines().line_count(), 5);
    assert_eq!(doc.syntax_lines().line_at(13).unwrap().range, 10..17);
    assert_eq!(doc.foldable(10, 17), Some(13..39));
    let first = doc.highlights(0..source.len()).unwrap().next().unwrap();
    assert_eq!((first.range, first.tags[0].name()), (2..9, "lineComment"));
    let indent = |pos, options| doc.indentation(pos, StreamSettings::default(), options);
    for (pos, expected) in [(0, 0), (10, 0), (19, 4), (37, 2), (41, 0)] {
        assert_eq!(
            indent(pos, IndentOptions::default()),
            Some(expected),
            "byte {pos}"
        );
    }
    let override_indent = |start| (start == 10).then_some(7);
    assert_eq!(
        indent(
            19,
            IndentOptions {
                override_indentation: Some(&override_indent),
                ..Default::default()
            }
        ),
        Some(9)
    );
    assert_eq!(
        indent(
            37,
            IndentOptions {
                simulate_break: Some(37),
                simulate_double_break: true,
                ..Default::default()
            }
        ),
        Some(4)
    );
}

#[test]
fn deep_expressions_do_not_require_recursion() {
    let depth = 16_384;
    let source = format!("{}\n1\n", "(".repeat(depth));
    let doc = parse(&source);
    assert_eq!(doc.nodes().len(), depth * 3 + 2);
    assert_eq!(doc.foldable(0, depth), Some(1..source.len()));
    assert_eq!(
        doc.indentation(
            source.len(),
            StreamSettings::default(),
            IndentOptions::default()
        ),
        Some(2)
    );
    for node in doc.nodes() {
        let range = node.range();
        assert!(source.get(range.clone()).is_some());
        let mut end = range.start;
        for child in node.children() {
            let child_range = child.range();
            assert_eq!(child.parent().unwrap().range(), range);
            assert!(end <= child_range.start && child_range.end <= range.end);
            end = child_range.end;
        }
    }
}
