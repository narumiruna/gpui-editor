//! Oracles from the SRI-verified `@codemirror/lang-rust` 6.0.2 and
//! `@lezer/rust` 1.0.3 npm tarballs. The JS fixtures used
//! `@codemirror/language` 6.13.1, `@codemirror/state` 6.7.6, `@lezer/lr` 1.4.13,
//! `@lezer/common` 1.5.7, and `@lezer/highlight` 1.2.5. Offsets were converted
//! from UTF-16 to original UTF-8 bytes.

use std::ops::Range;

use gpui_editor_codemirror::{
    Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        rust::{
            BLOCK_COMMENT, LINE_COMMENT, NAME, RustDocument, RustNode, RustNodeKind,
            STRING_PREFIXES, indent_on_input,
        },
    },
};

fn parse(source: &str) -> RustDocument {
    RustDocument::parse(Text::new(source))
}

fn settings() -> StreamSettings {
    StreamSettings {
        indent_unit: 4,
        ..StreamSettings::default()
    }
}

fn tree(node: RustNode<'_>) -> String {
    let name = node.kind().name();
    let name = if node.kind() != RustNodeKind::Error
        && name != "_"
        && !name.as_bytes()[0].is_ascii_alphabetic()
    {
        format!("{name:?}")
    } else {
        name.to_owned()
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

fn styles(doc: &RustDocument) -> Vec<(Range<usize>, String)> {
    doc.highlights(0..doc.text().as_str().len())
        .unwrap()
        .map(|span| {
            let names = span
                .tags
                .iter()
                .map(|tag| {
                    let mut name = tag.name().to_owned();
                    for modifier in tag.modifiers() {
                        name.push_str(match modifier {
                            TagModifier::Definition => ".definition",
                            TagModifier::Function => ".function",
                            TagModifier::Special => ".special",
                            tag => panic!("unexpected tag modifier: {tag:?}"),
                        });
                    }
                    name
                })
                .collect::<Vec<_>>()
                .join(" ");
            (span.range, names)
        })
        .collect()
}

fn check_styles(doc: &RustDocument, expected: &[(Range<usize>, &str)]) {
    assert_eq!(
        styles(doc),
        expected
            .iter()
            .map(|(range, style)| (range.clone(), (*style).to_owned()))
            .collect::<Vec<_>>()
    );
}

fn check_lines(doc: &RustDocument, expected: &[(Option<usize>, Option<Range<usize>>)]) {
    assert_eq!(doc.text().line_count(), expected.len());
    for (i, (indent, fold)) in expected.iter().enumerate() {
        let line = doc.text().line(i + 1).unwrap();
        assert_eq!(
            doc.indentation(line.range.start, settings(), IndentOptions::default()),
            *indent,
            "indentation on line {}: {:?}",
            i + 1,
            line.text
        );
        assert_eq!(
            doc.foldable(line.range.start, line.range.end),
            *fold,
            "fold on line {}",
            i + 1
        );
    }
}

#[test]
fn matches_generic_types_and_contextual_highlighting() {
    let doc = parse(concat!(
        "#[derive(Debug)]\n",
        "pub struct Point<T> {\n",
        "    pub value: T,\n",
        "}\n\n",
        "fn read(p: Point<Vec<u32>>) {\n",
        "    foo(p.value());\n",
        "}\n",
    ));
    assert!(!doc.is_recovered());
    assert_eq!(
        tree(doc.root()),
        concat!(
            r#"SourceFile(AttributeItem(Attribute("[",MetaItem(Identifier,ParenthesizedTokens("(",Identifier,")")),"]"),"#,
            r#"StructItem(Vis(pub),struct,TypeIdentifier,TypeParamList("<",TypeIdentifier,">"),"#,
            r#"FieldDeclarationList("{",FieldDeclaration(Vis(pub),FieldIdentifier,":",TypeIdentifier),",","}"))),"#,
            r#"FunctionItem(fn,BoundIdentifier,ParamList("(",Parameter(BoundIdentifier,":",GenericType(TypeIdentifier,"#,
            r#"TypeArgList("<",GenericType(TypeIdentifier,TypeArgList("<",TypeIdentifier,">")),">"))),")"),"#,
            r#"Block("{",ExpressionStatement(CallExpression(Identifier,ArgList("(",CallExpression("#,
            r#"FieldExpression(Identifier,FieldIdentifier),ArgList("(",")")),")")),";"),"}")))"#,
        )
    );
    check_styles(
        &doc,
        &[
            (0..1, "meta"),
            (1..2, "meta squareBracket"),
            (2..8, "meta variableName"),
            (8..9, "meta paren"),
            (9..14, "meta variableName"),
            (14..15, "meta paren"),
            (15..16, "meta squareBracket"),
            (17..20, "modifier"),
            (21..27, "definitionKeyword"),
            (28..33, "typeName"),
            (34..35, "typeName"),
            (37..38, "brace"),
            (43..46, "modifier"),
            (47..52, "propertyName"),
            (54..55, "typeName"),
            (55..56, "separator"),
            (57..58, "brace"),
            (60..62, "definitionKeyword"),
            (63..67, "variableName.definition.function"),
            (67..68, "paren"),
            (68..69, "variableName.definition"),
            (71..76, "typeName"),
            (77..80, "typeName"),
            (81..84, "typeName"),
            (86..87, "paren"),
            (88..89, "brace"),
            (94..97, "variableName.function"),
            (97..98, "paren"),
            (98..99, "variableName"),
            (100..105, "propertyName.function"),
            (105..106, "paren"),
            (106..107, "paren"),
            (107..108, "paren"),
            (108..109, "separator"),
            (110..111, "brace"),
        ],
    );
    check_lines(
        &doc,
        &[
            (Some(0), None),
            (Some(0), Some(38..57)),
            (Some(4), None),
            (Some(0), None),
            (Some(0), None),
            (Some(0), Some(89..110)),
            (Some(4), None),
            (Some(0), None),
            (Some(0), None),
        ],
    );
}

#[test]
fn matches_literals_comments_and_utf8_ranges() {
    let source = concat!(
        "#!/usr/bin/env rust-script\n",
        "/* a\n   /* 😀 */ z\n*/\n",
        "fn λ() {\n",
        "    let x = br##\"\"#😀\"##;\n",
        "    let y = b\"a\\n\nb\";\n",
        "    let z = 1.25e+2f32;\n",
        "    let f = |a| a + 1;\n",
        "}\n",
    );
    let doc = parse(source);
    assert!(!doc.is_recovered());
    assert_eq!(doc.root().range(), 0..160);
    assert_eq!(
        tree(doc.root()),
        concat!(
            r#"SourceFile(Shebang,BlockComment(BlockComment),FunctionItem(fn,BoundIdentifier,ParamList("(",")"),"#,
            r#"Block("{",LetDeclaration(let,BoundIdentifier,"=",RawString,";"),LetDeclaration(let,BoundIdentifier,"=",String(Escape),";"),"#,
            r#"LetDeclaration(let,BoundIdentifier,"=",Float,";"),LetDeclaration(let,BoundIdentifier,"=",ClosureExpression("#,
            r#"ParamList("|",Parameter(BoundIdentifier),"|"),BinaryExpression(Identifier,ArithOp,Integer)),";"),"}")))"#,
        )
    );
    check_styles(
        &doc,
        &[
            (0..26, "lineComment"),
            (27..50, "blockComment"),
            (51..53, "definitionKeyword"),
            (54..56, "variableName.definition.function"),
            (56..57, "paren"),
            (57..58, "paren"),
            (59..60, "brace"),
            (65..68, "definitionKeyword"),
            (69..70, "variableName.definition"),
            (71..72, "definitionOperator"),
            (73..87, "string.special"),
            (87..88, "separator"),
            (93..96, "definitionKeyword"),
            (97..98, "variableName.definition"),
            (99..100, "definitionOperator"),
            (101..104, "string"),
            (106..109, "string"),
            (109..110, "separator"),
            (115..118, "definitionKeyword"),
            (119..120, "variableName.definition"),
            (121..122, "definitionOperator"),
            (123..133, "float"),
            (133..134, "separator"),
            (139..142, "definitionKeyword"),
            (143..144, "variableName.definition"),
            (145..146, "definitionOperator"),
            (148..149, "variableName.definition"),
            (151..152, "variableName"),
            (153..154, "arithmeticOperator"),
            (155..156, "integer"),
            (156..157, "separator"),
            (158..159, "brace"),
        ],
    );
    check_lines(
        &doc,
        &[
            (Some(0), None),
            (Some(0), Some(29..48)),
            (None, None),
            (None, None),
            (Some(0), Some(60..158)),
            (Some(4), None),
            (Some(4), None),
            (None, None),
            (Some(4), None),
            (Some(4), None),
            (Some(0), None),
            (Some(0), None),
        ],
    );

    let crlf = source.replace('\n', "\r\n");
    let lines = parse(&crlf);
    assert!(!lines.is_recovered());
    let remap = |offset| {
        offset
            + source.as_bytes()[..offset]
                .iter()
                .filter(|&&b| b == b'\n')
                .count()
    };
    for (lf, crlf) in doc.nodes().zip(lines.nodes()) {
        assert_eq!(lf.kind(), crlf.kind());
        assert_eq!(crlf.range(), remap(lf.range().start)..remap(lf.range().end));
        assert_eq!(
            lf.parent().map(RustNode::kind),
            crlf.parent().map(RustNode::kind)
        );
    }
    assert_eq!(
        styles(&lines),
        styles(&doc)
            .into_iter()
            .map(|(r, tags)| (remap(r.start)..remap(r.end), tags))
            .collect::<Vec<_>>()
    );
    let emoji = crlf.find('😀').unwrap();
    let highlighted = lines.highlights(emoji..emoji + 4).unwrap().next().unwrap();
    assert_eq!(highlighted.range, emoji..emoji + 4);
    assert_eq!(highlighted.tags[0].name(), "blockComment");
    assert!(lines.highlights(emoji + 1..emoji + 4).is_none());
    assert!(lines.highlights(emoji..emoji).unwrap().next().is_none());
    assert!(lines.highlights(Range { start: 2, end: 1 }).is_none());
    assert!(lines.foldable(0, emoji + 1).is_none());
    assert!(
        lines
            .indentation(emoji + 1, settings(), IndentOptions::default())
            .is_none()
    );
}

#[test]
fn matches_indentation_and_folding_strategies() {
    let source = concat!(
        "fn example() {\n",
        "    if true\n",
        "    {\n",
        "        log(1,\n",
        "            2);\n",
        "    }\n",
        "    else {\n",
        "        let y =\n",
        "            1;\n",
        "    }\n",
        "    match 1 {\n",
        "        1 =>\n",
        "            2,\n",
        "        _ => 3,\n",
        "    }\n",
        "}\n",
    );
    let doc = parse(source);
    assert!(!doc.is_recovered());
    check_lines(
        &doc,
        &[
            (Some(0), Some(14..182)),
            (Some(4), None),
            (Some(4), Some(32..68)),
            (Some(8), Some(45..61)),
            (Some(12), None),
            (Some(4), None),
            (Some(4), Some(80..116)),
            (Some(8), None),
            (Some(12), None),
            (Some(4), None),
            (Some(4), Some(131..180)),
            (Some(8), None),
            (Some(12), None),
            (Some(8), None),
            (Some(4), None),
            (Some(0), None),
            (Some(0), None),
        ],
    );
    let position = source.find("2);").unwrap();
    let reference = source.find("        log").unwrap();
    assert_eq!(
        doc.indentation(
            position,
            settings(),
            IndentOptions {
                override_indentation: Some(&|line| (line == reference).then_some(12)),
                ..IndentOptions::default()
            }
        ),
        Some(16)
    );
    assert_eq!(
        doc.indentation(
            position,
            settings(),
            IndentOptions {
                simulate_break: Some(position),
                simulate_double_break: true,
                ..IndentOptions::default()
            }
        ),
        Some(12)
    );
}

#[test]
fn matches_attribute_and_macro_token_groups() {
    let doc = parse("#[cfg(\n    test\n)]\nfn f() {}\n");
    assert!(!doc.is_recovered());
    assert_eq!(
        tree(doc.root()),
        r#"SourceFile(AttributeItem(Attribute("[",MetaItem(Identifier,ParenthesizedTokens("(",Identifier,")")),"]"),FunctionItem(fn,BoundIdentifier,ParamList("(",")"),Block("{","}"))))"#
    );
    check_lines(
        &doc,
        &[
            (Some(0), Some(6..16)),
            (Some(4), None),
            (Some(0), None),
            (Some(0), None),
            (Some(0), None),
        ],
    );
    let attr = styles(&doc);
    assert_eq!(
        attr[4..7],
        [
            (6..11, "meta".into()),
            (11..15, "meta variableName".into()),
            (15..16, "meta".into())
        ]
    );

    let doc = parse("fn example() {\n    bar! {\n        [1 + 2]\n    }\n}\n");
    assert!(!doc.is_recovered());
    assert_eq!(
        tree(doc.root()),
        concat!(
            r#"SourceFile(FunctionItem(fn,BoundIdentifier,ParamList("(",")"),Block("{",MacroInvocation(Identifier,"!","#,
            r#"BracedTokens("{",BracketedTokens("[",Integer,ArithOp,Integer,"]"),"}")),"}")))"#,
        )
    );
    check_lines(
        &doc,
        &[
            (Some(0), Some(14..48)),
            (Some(4), Some(25..46)),
            (Some(8), None),
            (Some(4), None),
            (Some(0), None),
            (Some(0), None),
        ],
    );
    let macro_styles = styles(&doc);
    assert_eq!(
        macro_styles[5..7],
        [(19..22, "macroName".into()), (22..23, "macroName".into())]
    );
}

#[test]
fn parses_glr_conflicts_and_recovers_unfinished_input() {
    let doc = parse("fn f() {\n    if left && right {\n        let x = r#\"a\"#;\n    }\n}\n");
    assert!(!doc.is_recovered());
    assert!(doc.nodes().all(|node| node.kind() != RustNodeKind::Error));
    assert!(
        doc.nodes()
            .any(|node| node.kind() == RustNodeKind::IfExpression)
    );
    assert!(
        doc.nodes()
            .any(|node| node.kind() == RustNodeKind::BinaryExpression)
    );
    check_lines(
        &doc,
        &[
            (Some(0), Some(8..62)),
            (Some(4), Some(31..60)),
            (Some(8), None),
            (Some(4), None),
            (Some(0), None),
            (Some(0), None),
        ],
    );
    assert!(styles(&doc).contains(&(21..23, "logicOperator".into())));
    assert!(styles(&doc).contains(&(48..54, "string.special".into())));

    let doc = parse("fn f() {\n    let ink = br#\"😀\"#;\n    /* unclosed\n");
    assert!(doc.is_recovered());
    let errors: Vec<_> = doc
        .nodes()
        .filter(|node| node.kind() == RustNodeKind::Error)
        .map(|node| (node.parent().unwrap().kind(), node.range()))
        .collect();
    assert_eq!(
        errors,
        [
            (RustNodeKind::BlockComment, 51..51),
            (RustNodeKind::Block, 51..51)
        ]
    );
    check_lines(
        &doc,
        &[
            (Some(0), Some(8..51)),
            (Some(4), None),
            (Some(4), None),
            (None, None),
        ],
    );
    assert!(styles(&doc).contains(&(39..51, "blockComment".into())));
}

#[test]
fn exposes_upstream_language_data() {
    assert_eq!(NAME, "rust");
    assert_eq!(LINE_COMMENT, "//");
    assert_eq!(BLOCK_COMMENT, ("/*", "*/"));
    assert_eq!(STRING_PREFIXES, ["b", "r", "br"]);
    for text in ["{", "}", " \t}", "\u{feff}{"] {
        assert!(indent_on_input(text));
    }
    for text in ["", "{}", "x {", "{ ", "}\n", "\u{85}}"] {
        assert!(!indent_on_input(text));
    }
}
