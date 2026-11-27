//! Native fixtures from @codemirror/lang-yaml 6.1.3 with @lezer/yaml 1.0.4,
//! @codemirror/language 6.13.1, @codemirror/state 6.7.6, @lezer/common 1.5.7,
//! @lezer/highlight 1.2.5, and @lezer/lr 1.4.13. Expected values come from
//! syntaxTree, highlightTree, getIndentation, and foldable. UTF-16 positions
//! in normalized JS editor text were mapped back to the original UTF-8 bytes.

use std::ops::Range;

use gpui_editor_codemirror::{
    Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        yaml::{
            LINE_COMMENT, YamlDocument, YamlFrontmatter, YamlNode, YamlNodeKind, indent_on_input,
        },
    },
};

fn parse(source: &str) -> YamlDocument {
    YamlDocument::parse(Text::new(source))
}

fn tree(node: &YamlNode) -> String {
    let mut name = node.kind().name().to_owned();
    if !node.children().is_empty() {
        name.push('(');
        name.push_str(
            &node
                .children()
                .iter()
                .map(tree)
                .collect::<Vec<_>>()
                .join(" "),
        );
        name.push(')');
    }
    name
}

#[test]
fn parses_block_and_flow_grammar() {
    for (source, expected) in [
        ("", "Stream"),
        ("foo:bar\ncontinued here", "Stream(Document(Literal))"),
        (
            "- - one\n  - two\n- three",
            "Stream(Document(BlockSequence(- Item(BlockSequence(- Item(Literal) - Item(Literal))) - Item(Literal))))",
        ),
        (
            "one: \n  # pre\n  child: |2- # note\n    first\n\n    second\n  text: hi\n    continued\n  next: >\n    x\n# after\n",
            "Stream(Document(BlockMapping(Pair(Key(Literal) : BlockMapping(Comment Pair(Key(Literal) : BlockLiteral(BlockLiteralHeader Comment BlockLiteralContent)) Pair(Key(Literal) : Literal) Pair(Key(Literal) : BlockLiteral(BlockLiteralHeader BlockLiteralContent)))))) Comment)",
        ),
        (
            "[one: two, ? : four, x: ]",
            "Stream(Document(FlowSequence([ Item(FlowMapping(Pair(Key(Literal) : Literal))) , Item(FlowMapping(? Pair(: Literal))) , Item(FlowMapping(Pair(Key(Literal) :))) ])))",
        ),
        (
            "? [a, b]\n: yes\n? name\n: value\n: empty key\n",
            "Stream(Document(BlockMapping(? Pair(Key(FlowSequence([ Item(Literal) , Item(Literal) ])) : Literal) ? Pair(Key(Literal) : Literal) Pair(: Literal))))",
        ),
        (
            "? : val\n",
            "Stream(Document(BlockMapping(? Pair(: Literal))))",
        ),
        (
            "a: &x !thing\n- 1",
            "Stream(Document(BlockMapping(Pair(Key(Literal) : Anchored(Anchor Tagged(Tag BlockSequence(- Item(Literal))))))))",
        ),
        (
            r#"{: empty, ? : val, foo, a:b, "x":true, [1,2]: [3],}"#,
            "Stream(Document(FlowMapping({ Pair(: Key(Literal)) , ? Pair(: Literal) , Pair(Key(Literal)) , Pair(Key(Literal)) , Pair(Key(QuotedLiteral) : Literal) , Pair(Key(FlowSequence([ Item(Literal) , Item(Literal) ])) : FlowSequence([ Item(Literal) ])) , })))",
        ),
        (
            "%YAML 1.2\n---\na: !thing &id {b: c}\n...\n---\n- *id\n",
            "Stream(Document(Directive(DirectiveName DirectiveContent) DirectiveEnd BlockMapping(Pair(Key(Literal) : Tagged(Tag Anchored(Anchor FlowMapping({ Pair(Key(Literal) : Literal) })))))) DocEnd Document(DirectiveEnd BlockSequence(- Item(Alias))))",
        ),
    ] {
        let doc = parse(source);
        assert_eq!(tree(doc.root()), expected, "{source:?}");
        assert_ordered(doc.root(), source);
    }
}

#[test]
fn matches_lezer_tags_and_preserves_utf8_and_crlf() {
    let source = "%YAML 1.2 # version\r\n---\r\n名: !thing &id {\"鍵\": [true, null, 12], empty: []}\r\ncopy: *id # copy\r\n? complex\r\n: \"😀\"\r\nraw: |+\r\n  # scalar\r\n...\r\n";
    let doc = parse(source);
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(doc.root().range(), 0..source.len());
    assert_ordered(doc.root(), source);
    let expected = [
        ("%YAML", "keyword"),
        ("1.2", "attributeValue"),
        ("# version", "lineComment"),
        ("---", "meta"),
        ("名", "propertyName.definition"),
        (":", "separator"),
        ("!thing", "typeName"),
        ("&id", "labelName"),
        ("{", "brace"),
        ("\"鍵\"", "propertyName.definition"),
        (":", "separator"),
        ("[", "squareBracket"),
        ("true", "content"),
        (",", "separator"),
        ("null", "content"),
        (",", "separator"),
        ("12", "content"),
        ("]", "squareBracket"),
        (",", "separator"),
        ("empty", "propertyName.definition"),
        (":", "separator"),
        ("[", "squareBracket"),
        ("]", "squareBracket"),
        ("}", "brace"),
        ("copy", "propertyName.definition"),
        (":", "separator"),
        ("*id", "labelName"),
        ("# copy", "lineComment"),
        ("?", "punctuation"),
        ("complex", "propertyName.definition"),
        (":", "separator"),
        ("\"😀\"", "string"),
        ("raw", "propertyName.definition"),
        (":", "separator"),
        ("|+", "string.special"),
        ("  # scalar", "content"),
        ("...", "meta"),
    ];
    let mut last = 0;
    let mut spans = doc.highlights(0..source.len()).unwrap();
    for (text, style) in expected {
        let span = spans.next().unwrap();
        let start = last + source[last..].find(text).unwrap();
        assert_eq!(span.range, start..start + text.len());
        last = span.range.end;
        let [tag] = span.tags else { panic!("{span:?}") };
        let (name, mods) = style.split_once('.').unwrap_or((style, ""));
        assert_eq!(tag.name(), name);
        assert_eq!(
            tag.modifiers(),
            match mods {
                "definition" => &[TagModifier::Definition][..],
                "special" => &[TagModifier::Special],
                _ => &[],
            }
        );
    }
    assert!(spans.next().is_none());
    let start = source.find('😀').unwrap();
    let clipped = doc.highlights(start..start + 4).unwrap().next().unwrap();
    assert_eq!(clipped.range, start..start + 4);
    assert_eq!(clipped.tags[0].name(), "string");
    assert!(doc.highlights(start + 1..start + 4).is_none());
    assert!(doc.highlights(start..start).unwrap().next().is_none());
    assert!(doc.highlights(Range { start: 2, end: 1 }).is_none());
}

#[test]
fn matches_upstream_folding_and_indentation_per_line() {
    for (source, indents, folds) in [
        (
            "outer:\n   - name: first\n     notes: >-\n       alpha\n       beta\n   - name: second\nlist:\n- |2\n  # scalar content\n- plain\n  continued\n",
            &[
                None,
                Some(0),
                Some(5),
                Some(7),
                Some(7),
                Some(3),
                Some(0),
                Some(0),
                Some(2),
                Some(0),
                None,
                None,
            ][..],
            &[
                (1, 6..81),
                (2, 23..63),
                (3, 38..63),
                (7, 87..131),
                (8, 92..111),
                (10, 119..131),
            ][..],
        ),
        (
            "flow: [ 1,\n   {\n     key: \"line\nmore\",\n     other: [x,\n            y]\n   },\n   3\n]\n",
            &[
                None,
                Some(8),
                Some(5),
                Some(5),
                Some(5),
                Some(13),
                Some(3),
                Some(8),
                Some(6),
                None,
            ],
            &[(1, 10..82), (2, 15..74), (3, 31..37), (5, 54..69)],
        ),
        (
            "rows:\n  child:\n      deeper: |\n        first\n        second\n  next: x\n# out\n",
            &[
                None,
                Some(2),
                Some(6),
                Some(8),
                Some(8),
                Some(2),
                None,
                None,
            ],
            &[(1, 5..69), (2, 14..59), (3, 30..59)],
        ),
        (
            "root:\r\n  value: |\r\n    one\r\n    two\r\nlast: plain\r\n",
            &[None, Some(2), Some(4), Some(4), Some(0), None],
            &[(1, 5..35), (2, 17..35)],
        ),
        (
            "a: { # c\n  x: y\n}",
            &[None, Some(5), Some(3)],
            &[(1, 8..17)],
        ),
    ] {
        let doc = parse(source);
        assert_eq!(doc.text().line_count(), indents.len());
        for (idx, &indent) in indents.iter().enumerate() {
            let line = doc.text().line(idx + 1).unwrap();
            assert_eq!(
                doc.indentation(
                    line.range.start,
                    StreamSettings::default(),
                    IndentOptions::default()
                ),
                indent,
                "line {} in {source:?}",
                idx + 1
            );
            let expected = folds
                .iter()
                .find(|(n, _)| *n == idx + 1)
                .map(|(_, range)| range.clone());
            assert_eq!(
                doc.foldable(line.range.start, line.range.end),
                expected,
                "line {} in {source:?}",
                idx + 1
            );
        }
    }
    assert_eq!(LINE_COMMENT, "#");
    assert!(indent_on_input("\t  ]"));
    assert!(indent_on_input("  }"));
    assert!(!indent_on_input(" } "));
    assert!(!indent_on_input("x: }"));
}

#[test]
fn changes_reference_indentation_for_an_override_or_simulated_break() {
    let doc = parse("  box: [one,\n      two\n    ]\n");
    let get = |pos, options| doc.indentation(pos, StreamSettings::default(), options);
    let override_first = &|pos| (pos == 0).then_some(5);
    assert_eq!(
        get(
            13,
            IndentOptions {
                override_indentation: Some(override_first),
                ..Default::default()
            }
        ),
        Some(11)
    );
    assert_eq!(
        get(
            8,
            IndentOptions {
                override_indentation: Some(override_first),
                simulate_break: Some(8),
                ..Default::default()
            }
        ),
        Some(7)
    );
    assert_eq!(
        get(
            23,
            IndentOptions {
                simulate_break: Some(23),
                simulate_double_break: true,
                ..Default::default()
            }
        ),
        Some(8)
    );
    assert_eq!(get(23, IndentOptions::default()), Some(7));
}

#[test]
fn parses_front_matter_and_leaves_body_for_the_callers_language() {
    let text = Text::new("---\r\nname: \"😀\"\r\n---\r\n# Markdown\r\n");
    let front = YamlFrontmatter::parse(&text).unwrap();
    assert_eq!(front.opening(), 0..5);
    assert_eq!(front.yaml().root().range(), 5..19);
    assert_eq!(front.closing(), Some(19..24));
    assert_eq!(text.slice(front.body()), Some("# Markdown\r\n"));
    assert_eq!(front.yaml().text(), &text);
    assert_eq!(
        front
            .highlights(0..text.len())
            .unwrap()
            .map(|span| (text.slice(span.range).unwrap(), span.tags[0].name()))
            .collect::<Vec<_>>(),
        [
            ("---\r\n", "meta"),
            ("name", "propertyName"),
            (":", "separator"),
            ("\"😀\"", "string"),
            ("---\r\n", "meta")
        ],
    );
    let text = Text::new("---\n---");
    let front = YamlFrontmatter::parse(&text).unwrap();
    assert_eq!(front.yaml().root().range(), 4..4);
    assert_eq!(front.closing(), Some(4..7));
    let text = Text::new("---\nname: café\n--- # not a delimiter");
    let front = YamlFrontmatter::parse(&text).unwrap();
    assert_eq!(front.yaml().root().range(), 4..text.len());
    assert_eq!(front.closing(), None);
    assert_eq!(front.body(), text.len()..text.len());
    for source in [
        "",
        "---",
        "---\n",
        "\u{feff}---\nk: v",
        "\n---\nk: v\n---",
        "--- \nk: v\n---",
        "hi\n---\nk: v\n---",
    ] {
        assert!(
            YamlFrontmatter::parse(&Text::new(source)).is_none(),
            "{source:?}"
        );
    }
}

#[test]
fn incomplete_input_has_stable_ranges_and_keeps_editor_services() {
    let source = "a: [\n  x,\n  y";
    let doc = parse(source);
    assert_eq!(doc.foldable(0, 4), Some(4..source.len()));
    for at in [5, 10, source.len()] {
        assert_eq!(
            doc.indentation(at, StreamSettings::default(), IndentOptions::default()),
            Some(2)
        );
    }
    assert!(tree(doc.root()).contains(YamlNodeKind::Error.name()));
    let deep = format!("{}名{}", "[".repeat(1024), "]".repeat(1024));
    for source in [
        source,
        &deep,
        "a: {💣, key: [a}",
        "?\n: ,\n? é",
        "\"bad🚦",
        "\0",
    ] {
        assert_ordered(parse(source).root(), source);
    }
}

fn assert_ordered(node: &YamlNode, source: &str) {
    let range = node.range();
    assert!(source.get(range.clone()).is_some(), "{node:?}");
    let mut end = range.start;
    for child in node.children() {
        assert!(
            end <= child.range().start && child.range().end <= range.end,
            "{node:?}"
        );
        assert_ordered(child, source);
        end = child.range().end;
    }
}
