//! Oracle fixtures from the SRI-verified `@codemirror/lang-javascript` 6.2.5
//! and `@lezer/javascript` 1.5.7 npm tarballs. The oracle ran outside the
//! repository with `@codemirror/language` 6.13.1, `@codemirror/state` 6.7.6,
//! `@lezer/lr` 1.4.13, `@lezer/common` 1.5.7, and `@lezer/highlight` 1.2.5.
//! The mounted fixture used `@codemirror/lang-html` 6.4.12. UTF-16 offsets
//! were converted to original UTF-8 bytes.

use std::ops::Range;

use gpui_editor_codemirror::{
    Text,
    language::{
        IndentOptions, StreamSettings,
        html::{HtmlDocument, HtmlEmbeddedDocument, HtmlEmbeddedLanguage},
        javascript::{JavascriptDocument, JavascriptOptions, indent_on_input},
    },
};

fn parse(source: &str) -> JavascriptDocument {
    JavascriptDocument::parse(Text::new(source))
}

fn spans(doc: &JavascriptDocument) -> Vec<(Range<usize>, String)> {
    doc.highlights(0..doc.text().len())
        .unwrap()
        .map(|span| {
            let [tag] = span.tags else {
                panic!("unexpected tags: {:?}", span.tags)
            };
            let mut label = tag.name().to_owned();
            for modifier in tag.modifiers() {
                label.push_str(&format!(".{modifier:?}").to_ascii_lowercase());
            }
            (span.range, label)
        })
        .collect()
}

fn check_lines(doc: &JavascriptDocument, expected: &[(Option<usize>, Option<Range<usize>>)]) {
    assert_eq!(doc.text().line_count(), expected.len());
    for (index, (indent, fold)) in expected.iter().enumerate() {
        let number = index + 1;
        let line = doc.text().line(number).unwrap();
        assert_eq!(
            doc.indentation(
                line.range.start,
                StreamSettings::default(),
                IndentOptions::default()
            ),
            *indent,
            "indentation on line {number}"
        );
        assert_eq!(
            doc.foldable(line.range.start, line.range.end),
            *fold,
            "fold on line {number}"
        );
    }
}

fn check_byte_ranges(doc: &JavascriptDocument) {
    let source = doc.text().as_str();
    assert_eq!(doc.root().range(), 0..source.len());
    for node in doc.nodes() {
        let range = node.range();
        assert!(source.get(range.clone()).is_some());
        let mut end = range.start;
        for child in node.children() {
            assert!(end <= child.range().start);
            assert!(child.range().end <= range.end);
            assert_eq!(child.parent().unwrap().range(), range);
            end = child.range().end;
        }
    }
    let mut end = 0;
    for (range, _) in spans(doc) {
        assert!(end <= range.start);
        assert!(
            source
                .get(range.clone())
                .is_some_and(|span| !span.is_empty())
        );
        end = range.end;
    }
}

#[test]
fn uses_asi_regexp_and_postfix_context_with_original_byte_offsets() {
    let doc = parse(concat!(
        "function f() {\n",
        "  return /* here\n  */ 𐐀;\n",
        "  return // after\n    /x[😃]/gu\n",
        "}\n",
        "x\r\n++y\ncontinue label\nlet z = 0\nz / 2\n",
    ));
    assert!(!doc.is_recovered());
    check_byte_ranges(&doc);
    let nodes: Vec<_> = doc
        .nodes()
        .filter_map(|node| {
            let name = node.kind().name();
            matches!(
                name,
                "ReturnStatement" | "ExpressionStatement" | "RegExp" | "ArithOp"
            )
            .then(|| (name, node.range(), node.parent().unwrap().kind().name()))
        })
        .collect();
    assert_eq!(
        nodes,
        [
            ("ReturnStatement", 17..42, "Block"),
            ("ReturnStatement", 45..51, "Block"),
            ("ExpressionStatement", 65..76, "Block"),
            ("RegExp", 65..76, "ExpressionStatement"),
            ("ExpressionStatement", 79..80, "Script"),
            ("ExpressionStatement", 82..85, "Script"),
            ("ArithOp", 82..84, "UnaryExpression"),
            ("ExpressionStatement", 111..116, "Script"),
            ("ArithOp", 113..114, "BinaryExpression"),
        ]
    );
}

#[test]
fn matches_template_escape_and_interpolation_highlighting() {
    let doc = parse(
        "const é = tag`😀 \\u{1f600} \\${notExpr} ${fn(\"a\\n\", { b: 2 })} ${`hi ${name}`}`;\n",
    );
    assert!(!doc.is_recovered());
    check_byte_ranges(&doc);
    let actual = spans(&doc);
    assert_eq!(
        actual
            .iter()
            .map(|(r, tag)| (r.clone(), tag.as_str()))
            .collect::<Vec<_>>(),
        [
            (0..5, "definitionKeyword"),
            (6..8, "variableName.definition"),
            (9..10, "definitionOperator"),
            (11..14, "variableName.function"),
            (14..20, "string.special"),
            (20..29, "escape"),
            (29..30, "string.special"),
            (30..32, "escape"),
            (32..42, "string.special"),
            (42..44, "brace.special"),
            (44..46, "variableName.function"),
            (46..47, "paren"),
            (47..49, "string"),
            (49..51, "escape"),
            (51..52, "string"),
            (52..53, "separator"),
            (54..55, "brace"),
            (56..57, "propertyName.definition"),
            (57..58, "punctuation"),
            (59..60, "number"),
            (61..62, "brace"),
            (62..63, "paren"),
            (63..64, "brace.special"),
            (64..65, "string.special"),
            (65..67, "brace.special"),
            (67..71, "string.special"),
            (71..73, "brace.special"),
            (73..77, "variableName"),
            (77..78, "brace.special"),
            (78..79, "string.special"),
            (79..80, "brace.special"),
            (80..81, "string.special"),
            (81..82, "separator"),
        ]
    );
    assert!(doc.highlights(7..8).is_none());
    assert!(doc.highlights(18..20).is_none());
    assert_eq!(
        doc.highlights(15..19).unwrap().next().unwrap().range,
        15..19
    );
    assert_eq!(doc.highlights(19..19).unwrap().count(), 0);
}

#[test]
fn keeps_typescript_and_jsx_dialects_independent() {
    let ts_options = JavascriptOptions {
        typescript: true,
        jsx: false,
    };
    let jsx_options = JavascriptOptions {
        typescript: false,
        jsx: true,
    };
    let ts = "interface Box<T> { value: T }\nconst val = <Thing> value;";
    let jsx = "const x = <main ready={true}>😀<Child /></main>;";
    assert!(parse(ts).is_recovered());
    assert!(parse(jsx).is_recovered());
    let ts_doc = JavascriptDocument::parse_with_options(Text::new(ts), ts_options);
    let jsx_doc = JavascriptDocument::parse_with_options(Text::new(jsx), jsx_options);
    assert!(!ts_doc.is_recovered());
    assert!(!jsx_doc.is_recovered());
    assert!(JavascriptDocument::parse_with_options(Text::new(ts), jsx_options).is_recovered());
    assert!(JavascriptDocument::parse_with_options(Text::new(jsx), ts_options).is_recovered());
    assert!(ts_doc.nodes().any(|n| n.kind().name() == "PrefixCast"));
    let style = spans(&jsx_doc);
    assert!(
        style
            .iter()
            .any(|(range, tag)| &jsx[range.clone()] == "main" && tag == "tagName.standard")
    );
    assert!(
        style
            .iter()
            .any(|(range, tag)| &jsx[range.clone()] == "ready" && tag == "attributeName")
    );
    assert!(
        style
            .iter()
            .any(|(range, tag)| &jsx[range.clone()] == "😀" && tag == "content")
    );
    check_byte_ranges(&jsx_doc);

    let source = concat!(
        "type Props = { name: string };\n",
        "const identity = <T extends unknown>(x: T) => x;\n",
        "const heading = <H name={identity<Props>({name:\"𐐀\"})} />;\n",
    );
    let doc = JavascriptDocument::parse_with_options(
        Text::new(source),
        JavascriptOptions {
            typescript: true,
            jsx: true,
        },
    );
    assert!(!doc.is_recovered());
    let significant = doc
        .nodes()
        .filter_map(|node| {
            let name = node.kind().name();
            matches!(
                name,
                "TypeParamList" | "TypeArgList" | "JSXElement" | "JSXEscape"
            )
            .then_some((name, node.range()))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        significant,
        [
            ("TypeParamList", 48..67),
            ("JSXElement", 96..139),
            ("JSXEscape", 104..136),
            ("TypeArgList", 113..120),
        ]
    );
    check_byte_ranges(&doc);
}

#[test]
fn matches_fold_and_indent_rules_for_javascript_and_jsx() {
    let doc = parse(concat!(
        "switch (x) {\ncase 1:\nwork(2,\n3);\ndefault:\n",
        "let obj = {a: 1,\nb: 2};\n}\n/* 🦀\nnote */\nconst nested = `hi\nname`;\n",
    ));
    assert!(!doc.is_recovered());
    check_lines(
        &doc,
        &[
            (Some(0), Some(12..66)),
            (Some(2), None),
            (Some(4), None),
            (Some(5), None),
            (Some(2), None),
            (Some(4), Some(53..63)),
            (Some(11), None),
            (Some(0), None),
            (Some(0), Some(70..81)),
            (None, None),
            (Some(0), None),
            (None, None),
            (Some(0), None),
        ],
    );
    let jsx = JavascriptDocument::parse_with_options(
        Text::new(
            "const x = <main\nready=\"yes\"\naria-label=\"hello\"\n>\n<Child\nstatus=\"😀\"\n/>\n</main>;\n",
        ),
        JavascriptOptions {
            jsx: true,
            ..JavascriptOptions::default()
        },
    );
    assert!(!jsx.is_recovered());
    check_lines(
        &jsx,
        &[
            (Some(0), Some(15..47)),
            (Some(12), None),
            (Some(12), None),
            (Some(12), Some(48..73)),
            (Some(2), Some(55..70)),
            (Some(2), None),
            (Some(2), None),
            (Some(0), None),
            (Some(0), None),
        ],
    );
    assert!(indent_on_input("\tcase "));
    assert!(indent_on_input("\u{a0}</"));
    assert!(!indent_on_input("case  "));
    assert!(!indent_on_input("default"));
}

#[test]
fn repairs_unfinished_delimiters_without_exposing_synthesized_bytes() {
    let doc = parse("function f() {\n  return foo(\n    \"😀\"\n");
    assert!(doc.is_recovered());
    check_byte_ranges(&doc);
    assert_eq!(
        doc.nodes()
            .filter(|node| node.kind().is_error())
            .map(|node| (node.range(), node.parent().unwrap().kind().name()))
            .collect::<Vec<_>>(),
        [(40..40, "ArgList"), (40..40, "Block")]
    );
    check_lines(
        &doc,
        &[
            (Some(0), Some(14..40)),
            (Some(2), None),
            (Some(4), None),
            (Some(4), None),
        ],
    );
    let other = parse("let x = @\nlet y = \"😀\"\n");
    assert!(other.is_recovered());
    check_byte_ranges(&other);
    assert!(
        spans(&other).iter().any(
            |(range, tag)| &other.text().as_str()[range.clone()] == "\"😀\"" && tag == "string"
        )
    );
}

#[test]
fn mounted_tsx_nodes_and_highlights_keep_byte_coordinates() {
    let body = "\nconst root = <article>{1 as number}😀</article>;\n";
    let prefix = "<!-- α😀 -->\n<script type=\"text/typescript-jsx\">";
    let source = format!("{prefix}{body}</script>");
    let doc = HtmlDocument::parse(Text::new(&source));
    let [mount] = doc.mounts() else {
        panic!("expected one script mount")
    };
    assert_eq!(mount.language(), HtmlEmbeddedLanguage::Tsx);
    assert_eq!(mount.range(), prefix.len()..prefix.len() + body.len());
    let HtmlEmbeddedDocument::Javascript(script) = mount.document() else {
        panic!("expected a native JavaScript tree")
    };
    assert!(!script.is_recovered());
    check_byte_ranges(script);
    let text = script
        .nodes()
        .find(|node| node.kind().name() == "JSXText")
        .unwrap();
    let local = text.range();
    let range = local.start + prefix.len()..local.end + prefix.len();
    assert_eq!(range, 87..91);
    assert_eq!(&source[range.clone()], "😀");
    let highlights: Vec<_> = doc.highlights(range.clone()).unwrap().collect();
    assert_eq!(highlights.len(), 1);
    assert_eq!(highlights[0].range, range);
    assert_eq!(highlights[0].tags[0].name(), "content");
    assert!(doc.highlights(range.start + 1..range.end).is_none());
}
