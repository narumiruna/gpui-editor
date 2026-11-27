use std::ops::Range;

use gpui_editor_codemirror::{
    Text,
    language::{
        TagModifier,
        markdown::{
            MarkdownDialect, MarkdownDocument, MarkdownNode, MarkdownNodeKind::*, MarkdownOverlay,
        },
    },
};

// Recorded from @codemirror/lang-markdown 6.5.2 and @lezer/markdown 1.8.2.
// Tree and style queries use the underlying language parser. Fold queries use
// CodeMirror's markdown({base}) support. UTF-16 offsets are mapped to original
// UTF-8 bytes and LF positions are mapped back to CRLF where applicable.
fn assert_tree(doc: &MarkdownDocument, expected: &str) {
    fn visit(node: &MarkdownNode, indent: usize, lines: &mut Vec<String>) {
        let range = node.range();
        lines.push(format!(
            "{}{} {}..{}",
            " ".repeat(indent),
            node.kind().name(),
            range.start,
            range.end
        ));
        for child in node.children() {
            visit(child, indent + 1, lines);
        }
    }
    let mut lines = Vec::new();
    visit(doc.root(), 0, &mut lines);
    assert_eq!(lines.join("\n"), expected);
    assert_eq!(doc.nodes().count(), lines.len());
}

fn folds(doc: &MarkdownDocument) -> Vec<(usize, Range<usize>)> {
    (1..=doc.text().line_count())
        .filter_map(|number| {
            let line = doc.text().line(number)?;
            let fold = doc.foldable(line.range.start, line.range.end)?;
            Some((number, fold))
        })
        .collect()
}

fn highlight_names(doc: &MarkdownDocument, range: Range<usize>) -> String {
    doc.highlights(range)
        .unwrap()
        .map(|span| {
            let tags = span
                .tags
                .iter()
                .map(|tag| {
                    if tag.modifiers() == [TagModifier::Special] {
                        format!("{}.special", tag.name())
                    } else {
                        tag.name().to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!("{}..{} {tags}", span.range.start, span.range.end)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn oracle_nested() {
    let doc = MarkdownDocument::parse(Text::new(
        "# Café 🙂 #\r\n\r\n> 1. **one**\r\n>    - `a`\r\n>      ```rust more\r\n>      fn x() {}\r\n>      ```\r\n\r\n## Next\r\n\r\nDone.\r\n",
    ));
    assert_tree(
        &doc,
        r#"Document 0..115
 ATXHeading1 0..14
  HeaderMark 0..1
  HeaderMark 13..14
 Blockquote 18..93
  QuoteMark 18..19
  OrderedList 20..93
   ListItem 20..93
    ListMark 20..22
    Paragraph 23..30
     StrongEmphasis 23..30
      EmphasisMark 23..25
      EmphasisMark 28..30
    QuoteMark 32..33
    BulletList 37..93
     ListItem 37..93
      ListMark 37..38
      Paragraph 39..42
       InlineCode 39..42
        CodeMark 39..40
        CodeMark 41..42
      QuoteMark 44..45
      FencedCode 51..93
       CodeMark 51..54
       CodeInfo 54..63
       QuoteMark 65..66
       CodeText 72..81
       QuoteMark 83..84
       CodeMark 90..93
 ATXHeading2 97..104
  HeaderMark 97..99
 Paragraph 108..113"#,
    );
    assert_eq!(
        folds(&doc),
        [
            (1, 14..113),
            (3, 30..93),
            (4, 42..93),
            (5, 63..93),
            (9, 104..113)
        ]
    );
    assert_eq!(
        doc.text().as_str(),
        "# Café 🙂 #\r\n\r\n> 1. **one**\r\n>    - `a`\r\n>      ```rust more\r\n>      fn x() {}\r\n>      ```\r\n\r\n## Next\r\n\r\nDone.\r\n"
    );
    assert_eq!(
        doc.overlays().collect::<Vec<_>>(),
        [MarkdownOverlay::Code {
            info: "rust more",
            ranges: std::iter::once(72..81).collect()
        }]
    );
    assert_eq!(doc.node_at(51, false).unwrap().kind(), CodeMark);
    assert!(doc.node_at(9, false).is_none());
    assert!(doc.highlights(9..11).is_none());
    assert_eq!(highlight_names(&doc, 2..12), "2..12 heading1");
    assert!(doc.highlights(12..12).unwrap().next().is_none());
    assert!(doc.indentation(72).is_none());
}

#[test]
fn oracle_inline() {
    let doc = MarkdownDocument::parse(Text::new(
        "Hello *é **bold*** ![alt](pic.png \"tip\") [a *b*](<ab> 'xyz') <https://ex.org> &amp; \\*  \nnext\\\nline <i>x</i> <!-- hi -->\n",
    ));
    assert_tree(
        &doc,
        r#"Document 0..122
 Paragraph 0..121
  Emphasis 6..19
   EmphasisMark 6..7
   StrongEmphasis 10..18
    EmphasisMark 10..12
    EmphasisMark 16..18
   EmphasisMark 18..19
  Image 20..41
   LinkMark 20..22
   LinkMark 25..26
   LinkMark 26..27
   URL 27..34
   LinkTitle 35..40
   LinkMark 40..41
  Link 42..61
   LinkMark 42..43
   Emphasis 45..48
    EmphasisMark 45..46
    EmphasisMark 47..48
   LinkMark 48..49
   LinkMark 49..50
   URL 50..54
   LinkTitle 55..60
   LinkMark 60..61
  Autolink 62..78
   LinkMark 62..63
   URL 63..77
   LinkMark 77..78
  Entity 79..84
  Escape 85..87
  HardBreak 87..90
  HardBreak 94..96
  HTMLTag 101..104
  HTMLTag 105..109
  Comment 110..121"#,
    );
    assert_eq!(folds(&doc), [(1, 89..121)]);
    assert_eq!(
        highlight_names(&doc, 0..doc.text().len()),
        r#"0..6 content
6..7 emphasis processingInstruction
7..10 emphasis
10..12 emphasis strong processingInstruction
12..16 emphasis strong
16..18 emphasis strong processingInstruction
18..19 emphasis processingInstruction
19..20 content
20..22 link processingInstruction
22..25 link
25..26 link processingInstruction
26..27 link processingInstruction
27..34 link url
34..35 link
35..40 link string
40..41 link processingInstruction
41..42 content
42..43 link processingInstruction
43..45 link
45..46 link emphasis processingInstruction
46..47 link emphasis
47..48 link emphasis processingInstruction
48..49 link processingInstruction
49..50 link processingInstruction
50..54 link url
54..55 link
55..60 link string
60..61 link processingInstruction
61..62 content
62..63 processingInstruction
63..77 url
77..78 processingInstruction
78..79 content
79..84 character
84..85 content
85..87 escape
87..90 processingInstruction
90..94 content
94..96 processingInstruction
96..101 content
104..105 content
109..110 content
110..121 comment"#
    );
}

#[test]
fn oracle_unfinished() {
    let doc = MarkdownDocument::parse(Text::new(
        "Intro [x]( and *open `code\n\n> - setup\n>   ~~~js\n>   first\n>   second\n",
    ));
    assert_tree(
        &doc,
        r#"Document 0..69
 Paragraph 0..26
  Link 6..9
   LinkMark 6..7
   LinkMark 8..9
 Blockquote 28..68
  QuoteMark 28..29
  BulletList 30..68
   ListItem 30..68
    ListMark 30..31
    Paragraph 32..37
    QuoteMark 38..39
    FencedCode 42..68
     CodeMark 42..45
     CodeInfo 45..47
     QuoteMark 48..49
     CodeText 52..58
     QuoteMark 58..59
     CodeText 62..68"#,
    );
    assert_eq!(folds(&doc), [(3, 37..68), (4, 47..68)]);
    assert_eq!(
        doc.overlays().collect::<Vec<_>>(),
        [MarkdownOverlay::Code {
            info: "js",
            ranges: vec![52..58, 62..68]
        }]
    );
}

#[test]
fn oracle_blocks() {
    let doc = MarkdownDocument::parse(Text::new(
        "Title\n=====\n\ntext\nsecond\n\n    abc\n    def\n\n---\n\n<!-- hi\nbye -->\n\n<div>\nx\n</div>\n\n[ref]: /a \"hello\"\n\n[a][ref] [ref][] [ref] [unknown] [xx][missing]\n",
    ));
    assert_tree(
        &doc,
        r#"Document 0..147
 SetextHeading1 0..11
  HeaderMark 6..11
 Paragraph 13..24
 CodeBlock 30..41
  CodeText 30..34
  CodeText 38..41
 HorizontalRule 43..46
 CommentBlock 48..63
 HTMLBlock 65..79
 LinkReference 81..98
  LinkLabel 81..86
  LinkMark 86..87
  URL 88..90
  LinkTitle 91..98
 Paragraph 100..146
  Link 100..108
   LinkMark 100..101
   LinkMark 102..103
   LinkLabel 103..108
  Link 109..116
   LinkMark 109..110
   LinkMark 113..114
   LinkLabel 114..116
  Link 117..122
   LinkMark 117..118
   LinkMark 121..122
  Link 123..132
   LinkMark 123..124
   LinkMark 131..132
  Link 133..146
   LinkMark 133..134
   LinkMark 136..137
   LinkLabel 137..146"#,
    );
    assert_eq!(
        folds(&doc),
        [
            (1, 5..146),
            (4, 17..24),
            (7, 33..41),
            (12, 55..63),
            (15, 70..79)
        ]
    );
}

#[test]
fn oracle_gfm() {
    let doc = MarkdownDocument::parse_with_dialect(
        Text::new(
            "- [x] task\n- [ ] new\n\n| a | **b** |\n| --- | :-: |\n| x | y |\n\n~~gone~~ ~low~ ^hi^ :wave: https://ex.org/a test@example.com\n",
        ),
        MarkdownDialect::Extended,
    );
    assert_tree(
        &doc,
        r#"Document 0..122
 BulletList 0..20
  ListItem 0..10
   ListMark 0..1
   Task 2..10
    TaskMarker 2..5
  ListItem 11..20
   ListMark 11..12
   Task 13..20
    TaskMarker 13..16
 Table 22..59
  TableHeader 22..35
   TableDelimiter 22..23
   TableCell 24..25
   TableDelimiter 26..27
   TableCell 28..33
    StrongEmphasis 28..33
     EmphasisMark 28..30
     EmphasisMark 31..33
   TableDelimiter 34..35
  TableDelimiter 36..49
  TableRow 50..59
   TableDelimiter 50..51
   TableCell 52..53
   TableDelimiter 54..55
   TableCell 56..57
   TableDelimiter 58..59
 Paragraph 61..121
  Strikethrough 61..69
   StrikethroughMark 61..63
   StrikethroughMark 67..69
  Subscript 70..75
   SubscriptMark 70..71
   SubscriptMark 74..75
  Superscript 76..80
   SuperscriptMark 76..77
   SuperscriptMark 79..80
  Emoji 81..87
  URL 88..104
  URL 105..121"#,
    );
    assert_eq!(folds(&doc), [(4, 35..59)]);
    assert_eq!(
        highlight_names(&doc, 0..doc.text().len()),
        r#"0..1 list processingInstruction
1..2 list
2..5 list atom
5..10 list list
10..11 list
11..12 list processingInstruction
12..13 list
13..16 list atom
16..20 list list
22..23 heading processingInstruction
23..24 heading
24..25 heading content
25..26 heading
26..27 heading processingInstruction
27..28 heading
28..30 heading strong processingInstruction
30..31 heading strong
31..33 heading strong processingInstruction
33..34 heading
34..35 heading processingInstruction
36..49 processingInstruction
50..51 processingInstruction
52..53 content
54..55 processingInstruction
56..57 content
58..59 processingInstruction
61..63 strikethrough processingInstruction
63..67 strikethrough
67..69 strikethrough processingInstruction
69..70 content
70..71 processingInstruction
71..74 content.special
74..75 processingInstruction
75..76 content
76..77 processingInstruction
77..79 content.special
79..80 processingInstruction
80..81 content
81..87 character
87..88 content
88..104 url
104..105 content
105..121 url"#
    );
}

#[test]
fn oracle_recovery() {
    let doc = MarkdownDocument::parse(Text::new(
        "#\n\n[unknown] [hello]( [xx][missing]\n[x](/foo\n\n> \n> \n\n- \n\n```\n\n  a\n\n    b\n\n",
    ));
    assert_tree(
        &doc,
        r#"Document 0..74
 ATXHeading1 0..1
  HeaderMark 0..1
 Paragraph 3..44
  Link 3..12
   LinkMark 3..4
   LinkMark 11..12
  Link 13..20
   LinkMark 13..14
   LinkMark 19..20
  Link 22..35
   LinkMark 22..23
   LinkMark 25..26
   LinkLabel 26..35
  Link 36..39
   LinkMark 36..37
   LinkMark 38..39
 Blockquote 46..51
  QuoteMark 46..47
  QuoteMark 49..50
 BulletList 53..55
  ListItem 53..55
   ListMark 53..54
 FencedCode 57..74
  CodeMark 57..60
  CodeText 61..74"#,
    );
    assert_eq!(
        folds(&doc),
        [(1, 1..74), (3, 35..44), (6, 48..51), (11, 60..74)]
    );
}

#[test]
fn oracle_gfm_sparse() {
    let doc = MarkdownDocument::parse_with_dialect(
        Text::new(
            "| X | Y |\n| - | - |\n| | z | extra |\nnot a pipe\n\n- ~~no~ ~ok~ ^up^ :smile: <i>\n",
        ),
        MarkdownDialect::Extended,
    );
    assert_tree(
        &doc,
        r#"Document 0..78
 Table 0..46
  TableHeader 0..9
   TableDelimiter 0..1
   TableCell 2..3
   TableDelimiter 4..5
   TableCell 6..7
   TableDelimiter 8..9
  TableDelimiter 10..19
  TableRow 20..35
   TableDelimiter 20..21
   TableDelimiter 22..23
   TableCell 24..25
   TableDelimiter 26..27
   TableCell 28..33
   TableDelimiter 34..35
  TableRow 36..46
   TableCell 36..46
 BulletList 48..77
  ListItem 48..77
   ListMark 48..49
   Paragraph 50..77
    Subscript 56..60
     SubscriptMark 56..57
     SubscriptMark 59..60
    Superscript 61..65
     SuperscriptMark 61..62
     SuperscriptMark 64..65
    Emoji 66..73
    HTMLTag 74..77"#,
    );
    assert_eq!(folds(&doc), [(1, 9..46)]);
}

#[test]
fn oracle_markup_edits() {
    // CodeMirror 6.5.2 commands, with default nonTightLists, at one cursor.
    let cases = [
        (
            "1. First\n2. Next\n3. Last\n",
            8,
            "1. First\n2. \n3. Next\n4. Last\n",
            12,
        ),
        ("- \n", 2, "\n", 0),
        ("- First\n- \n", 10, "- First\n\n- \n", 11),
        (
            "1. First\n2. Next\n3. \n4. Last\n",
            20,
            "1. First\n2. Next\n\n3. Last\n",
            17,
        ),
        (
            "1. First\n\n2. Next\n",
            8,
            "1. First\n\n2. \n\n3. Next\n",
            13,
        ),
        ("> \n> \n", 5, "\n\n", 1),
        ("> Text\n", 6, "> Text\n> \n", 9),
        ("- [x] Done\n", 10, "- [x] Done\n- [ ] \n", 17),
        ("- a\n  continued\n", 15, "- a\n  continued\n  \n", 18),
        (
            "- foo\n  - bar\n  - baz\n- done\n",
            13,
            "- foo\n  - bar\n  - \n  - baz\n- done\n",
            18,
        ),
    ];
    for (source, position, expected, cursor) in cases {
        let mut text = Text::new(source);
        let doc = MarkdownDocument::parse_with_dialect(text.clone(), MarkdownDialect::Extended);
        let edit = doc.continue_markup(position, true).unwrap();
        edit.changes.apply_to(&mut text).unwrap();
        assert_eq!(text.as_str(), expected);
        assert_eq!(edit.cursor, cursor);
    }
    for (source, position, expected, cursor) in [
        ("> text\n", 2, "text\n", 0),
        ("- a\n- b\n", 6, "- a\n  b\n", 6),
        ("-   text\n", 4, "- text\n", 2),
        (
            "- foo\n  - bar\n  - baz\n- done\n",
            10,
            "- foo\n  bar\n  - baz\n- done\n",
            8,
        ),
    ] {
        let mut text = Text::new(source);
        let doc = MarkdownDocument::parse(text.clone());
        let edit = doc.delete_markup_backward(position).unwrap();
        edit.changes.apply_to(&mut text).unwrap();
        assert_eq!(text.as_str(), expected);
        assert_eq!(edit.cursor, cursor);
    }
    let doc = MarkdownDocument::parse(Text::new("> ```\n> a\n> ```\n"));
    assert!(doc.continue_markup(9, true).is_none());
    let doc = MarkdownDocument::parse(Text::new("> <div>\n> x\n> </div>\n"));
    assert!(doc.continue_markup(11, true).is_none());
}

#[test]
fn markup_edits_keep_utf8_and_crlf() {
    let mut text = Text::new("1. café 🙂\r\n2. next\r\n");
    let doc = MarkdownDocument::parse(text.clone());
    let line = text.line(1).unwrap();
    let end = line.range.end;
    assert!(doc.continue_markup(end - 1, true).is_none());
    assert!(doc.continue_markup(end + 1, true).is_none());
    let edit = doc.continue_markup(end, true).unwrap();
    edit.changes.apply_to(&mut text).unwrap();
    assert_eq!(text.as_str(), "1. café 🙂\r\n2. \r\n3. next\r\n");
    assert_eq!(edit.cursor, text.line(2).unwrap().range.end);
}

#[test]
fn mounts_available_languages_and_projects_spans_across_host_markup() {
    use gpui_editor_codemirror::language_data::{NativeLanguage, NativeSyntax};
    let source = concat!(
        "> ```sql extra\r\n",
        "> SELECT '字\r\n",
        "> 猫';\r\n",
        "> ```\r\n\r\n",
        "<div title='猫'>字</div>\r\n\r\n",
        "```other\r\n",
        "not parsed\r\n",
        "```"
    );
    let doc = MarkdownDocument::parse(Text::new(source));
    let mounts: Vec<_> = doc
        .mounts(|name| (name == "sql").then_some(NativeLanguage::Sql(Default::default())))
        .collect();
    let [sql, html] = mounts.as_slice() else {
        panic!("expected SQL and HTML overlays");
    };
    assert!(matches!(sql.syntax(), NativeSyntax::Sql(_)));
    assert!(matches!(html.syntax(), NativeSyntax::Html(_)));
    assert_eq!(sql.syntax().text().as_str(), "SELECT '字\r\n猫';");
    assert_eq!(
        sql.source_ranges()
            .map(|range| &source[range])
            .collect::<Vec<_>>(),
        ["SELECT '字\r\n", "猫';"]
    );
    assert_eq!(
        sql.highlights()
            .filter(|span| span.tags[0].name() == "string")
            .map(|span| &source[span.range])
            .collect::<Vec<_>>(),
        ["'字\r\n", "猫'"]
    );
    assert!(
        html.highlights()
            .any(|span| &source[span.range] == "div" && span.tags[0].name() == "tagName")
    );
}

#[test]
fn base_dialect_and_html_overlays() {
    use gpui_editor_codemirror::language::markdown::{BLOCK_COMMENT, language_data};

    assert_eq!(language_data().get("commentTokens"), Some(&BLOCK_COMMENT));
    let doc = MarkdownDocument::parse(Text::new(
        "- [x] a\n\n~~a~~ ^b^ :c: www.example.org\n\n> <div>\n> text\n> </div>\n",
    ));
    for kind in [Task, Strikethrough, Superscript, Emoji, URL] {
        assert!(!doc.nodes().any(|node| node.kind() == kind));
    }
    let overlays = doc.overlays().collect::<Vec<_>>();
    let [MarkdownOverlay::Html { ranges }] = overlays.as_slice() else {
        panic!("expected one HTML overlay")
    };
    assert_eq!(
        ranges
            .iter()
            .map(|r| doc.text().slice(r.clone()).unwrap())
            .collect::<Vec<_>>(),
        ["<div>\n", " text\n", " </div>"]
    );
    assert!(
        doc.indentation(doc.text().as_str().find("text").unwrap())
            .is_none()
    );
}

#[test]
fn incomplete_root_comment_and_single_quote_marker() {
    let doc = MarkdownDocument::parse(Text::new(">\n\n<!-- incomplete\n"));
    assert_tree(
        &doc,
        "Document 0..19\n Blockquote 0..1\n  QuoteMark 0..1\n CommentBlock 3..19",
    );
}

#[test]
fn deep_and_bare_cr_input_keeps_source_order() {
    let source = format!("{}one\r\n\r1. a\r1. b\n", ">".repeat(300));
    let doc = MarkdownDocument::parse(Text::new(source.clone()));
    assert_eq!(doc.root().range(), 0..source.len());
    fn check(node: &MarkdownNode, source: &str) {
        let parent = node.range();
        let mut end = parent.start;
        for child in node.children() {
            let range = child.range();
            assert!(range.start >= end && range.end <= parent.end);
            assert!(source.get(range.clone()).is_some());
            end = range.end;
            check(child, source);
        }
    }
    check(doc.root(), &source);
}
