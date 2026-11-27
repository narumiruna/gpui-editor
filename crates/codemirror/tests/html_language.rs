//! Grammar fixtures from @lezer/html 1.3.13, test/tags.txt.

use std::ops::Range;

use gpui_editor_codemirror::{
    Text,
    commands::CommentTokens,
    language::{
        IndentOptions, StreamSettings,
        css::CssNodeKind,
        html::{
            BLOCK_COMMENT, HtmlDocument, HtmlEmbeddedDocument, HtmlEmbeddedLanguage, HtmlNode,
            HtmlNodeKind, HtmlOptions, NAME, WORD_CHARS, indent_on_input, language_data,
        },
    },
};

fn parse(source: &str) -> HtmlDocument {
    HtmlDocument::parse(Text::new(source))
}

fn tree(node: HtmlNode<'_>) -> String {
    let mut result = node.kind().name().to_owned();
    if node.children().len() > 0 {
        result.push('(');
        result.push_str(&node.children().map(tree).collect::<Vec<_>>().join(","));
        result.push(')');
    }
    result
}

#[test]
fn parses_upstream_grammar_fixtures() {
    for (source, expected, dialect) in [
        (
            "<foo>bar</foo>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Text,CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            "<a><b>c</b><br></a>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),Text,CloseTag(StartCloseTag,TagName,EndTag)),Element(SelfClosingTag(StartTag,TagName,EndTag)),CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            r#"<br foo="bar">"#,
            "Document(Element(SelfClosingTag(StartTag,TagName,Attribute(AttributeName,Is,AttributeValue),EndTag)))",
            "",
        ),
        (
            r#"<a x="one" y="two" z="three"></a>"#,
            "Document(Element(OpenTag(StartTag,TagName,Attribute(AttributeName,Is,AttributeValue),Attribute(AttributeName,Is,AttributeValue),Attribute(AttributeName,Is,AttributeValue),EndTag),CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            r#"<a x y="one" z></a>"#,
            "Document(Element(OpenTag(StartTag,TagName,Attribute(AttributeName),Attribute(AttributeName,Is,AttributeValue),Attribute(AttributeName),EndTag),CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            "<a x=one y z=two></a>",
            "Document(Element(OpenTag(StartTag,TagName,Attribute(AttributeName,Is,UnquotedAttributeValue),Attribute(AttributeName),Attribute(AttributeName,Is,UnquotedAttributeValue),EndTag),CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            "<link as=font crossorigin=anonymous href=/fonts/google-sans/regular/latin.woff2 rel=preload>",
            "Document(Element(SelfClosingTag(StartTag,TagName,Attribute(AttributeName,Is,UnquotedAttributeValue),Attribute(AttributeName,Is,UnquotedAttributeValue),Attribute(AttributeName,Is,UnquotedAttributeValue),Attribute(AttributeName,Is,UnquotedAttributeValue),EndTag)))",
            "",
        ),
        (
            "<link x='one' z='two&amp;'>",
            "Document(Element(SelfClosingTag(StartTag,TagName,Attribute(AttributeName,Is,AttributeValue),Attribute(AttributeName,Is,AttributeValue(EntityReference)),EndTag)))",
            "",
        ),
        (
            r#"<a attr="one&amp;two">&amp;&#67;</a>"#,
            "Document(Element(OpenTag(StartTag,TagName,Attribute(AttributeName,Is,AttributeValue(EntityReference)),EndTag),EntityReference,CharacterReference,CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            r#"<!doctype html>
<doc></doc>"#,
            "Document(DoctypeDecl,Text,Element(OpenTag(StartTag,TagName,EndTag),CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            "<?foo?><bar><?baz?></bar>",
            "Document(ProcessingInst,Element(OpenTag(StartTag,TagName,EndTag),ProcessingInst,CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            r#"<!-- top comment -->
<element><!-- inner comment --> text</element>
<!---->
<!--
-->"#,
            "Document(Comment,Text,Element(OpenTag(StartTag,TagName,EndTag),Comment,Text,CloseTag(StartCloseTag,TagName,EndTag)),Text,Comment,Text,Comment)",
            "",
        ),
        (
            "<a></b>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),MismatchedCloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            "<a>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag)))",
            "",
        ),
        (
            "<br/>",
            "Document(Element(SelfClosingTag(StartTag,TagName,EndTag)))",
            "",
        ),
        (
            "<p>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag)))",
            "",
        ),
        (
            "<a><b><c></c></x></a>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),CloseTag(StartCloseTag,TagName,EndTag)),MismatchedCloseTag(StartCloseTag,TagName,EndTag),⚠),CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            "<html><body></</html>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),IncompleteCloseTag,⚠),CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            "<a><b><c></x></c></a>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),MismatchedCloseTag(StartCloseTag,TagName,EndTag),CloseTag(StartCloseTag,TagName,EndTag)),⚠),CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            "<a></a></a>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),CloseTag(StartCloseTag,TagName,EndTag)),MismatchedCloseTag(StartCloseTag,TagName,EndTag))",
            "",
        ),
        (
            "<a><img src=blah></a>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(SelfClosingTag(StartTag,TagName,Attribute(AttributeName,Is,UnquotedAttributeValue),EndTag)),CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            "<dl><dd>Hello</dl>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),Text),CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            r#"<div>
  <p>Foo
  <p>Bar
</div>"#,
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Text,Element(OpenTag(StartTag,TagName,EndTag),Text),Element(OpenTag(StartTag,TagName,EndTag),Text),CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            r#"<p>Foo
<p>Bar"#,
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Text),Element(OpenTag(StartTag,TagName,EndTag),Text))",
            "",
        ),
        (
            r#"<p>Enter something: <textarea code-lang=javascript>function foo() {
  return "</bar>"
}</textarea>"#,
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Text,Element(OpenTag(StartTag,TagName,Attribute(AttributeName,Is,UnquotedAttributeValue),EndTag),TextareaText,CloseTag(StartCloseTag,TagName,EndTag))))",
            "",
        ),
        (
            "<script>This is not an entity: &lt;</script>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),ScriptText,CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            "<html>a&b</html>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Text,InvalidEntity,Text,CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            "<div>foo</p>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Text,CloseTag(StartCloseTag,TagName,EndTag)))",
            "noMatch",
        ),
        (
            "</strong>",
            "Document(CloseTag(StartCloseTag,TagName,EndTag))",
            "noMatch",
        ),
        (
            r#"<img src="foo&bar">"#,
            "Document(Element(SelfClosingTag(StartTag,TagName,Attribute(AttributeName,Is,AttributeValue(InvalidEntity)),EndTag)))",
            "",
        ),
        (
            "<section><image id=i2 /></section>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(SelfClosingTag(StartTag,TagName,Attribute(AttributeName,Is,UnquotedAttributeValue),SelfClosingEndTag)),CloseTag(StartCloseTag,TagName,EndTag)))",
            "selfClosing",
        ),
        (
            "<div><svg><circle/></svg></div>",
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),Element(SelfClosingTag(StartTag,TagName,SelfClosingEndTag)),CloseTag(StartCloseTag,TagName,EndTag)),CloseTag(StartCloseTag,TagName,EndTag)))",
            "",
        ),
        (
            r#"<div
<div
<div"#,
            "Document(Element(OpenTag(StartTag,TagName,⚠),Element(OpenTag(StartTag,TagName,⚠),Element(OpenTag(StartTag,TagName,⚠),⚠),⚠),⚠))",
            "",
        ),
        (
            r#"<body>
  <br/>
  <textarea/>
  <script/>
  <style/>
</body>"#,
            "Document(Element(OpenTag(StartTag,TagName,EndTag),Text,Element(SelfClosingTag(StartTag,TagName,SelfClosingEndTag)),Text,Element(SelfClosingTag(StartTag,TagName,SelfClosingEndTag)),Text,Element(SelfClosingTag(StartTag,TagName,SelfClosingEndTag)),Text,Element(SelfClosingTag(StartTag,TagName,SelfClosingEndTag)),Text,CloseTag(StartCloseTag,TagName,EndTag)))",
            "selfClosing",
        ),
        ("< div>x", "Document(IncompleteTag,Text)", ""),
    ] {
        let doc = HtmlDocument::parse_with_options(
            Text::new(source),
            HtmlOptions {
                match_closing_tags: dialect != "noMatch",
                self_closing_tags: dialect == "selfClosing",
            },
        );
        assert_eq!(tree(doc.root()), expected, "{source:?}");
        assert_tree_ranges(&doc);
    }
}

fn assert_tree_ranges(doc: &HtmlDocument) {
    let source = doc.text().as_str();
    assert_eq!(doc.root().range(), 0..source.len());
    for node in doc.nodes() {
        let range = node.range();
        assert!(source.get(range.clone()).is_some(), "{source:?}: {node:?}");
        let mut previous = range.start;
        for child in node.children() {
            let child_range = child.range();
            assert_eq!(child.parent().unwrap().range(), range);
            assert!(
                previous <= child_range.start && child_range.end <= range.end,
                "{source:?}: {:?} {range:?} / {:?} {child_range:?}",
                node.kind(),
                child.kind()
            );
            previous = child_range.end;
        }
    }
}

#[test]
fn recovers_byte_ranges_for_crlf_unicode_and_incomplete_input() {
    for source in [
        "",
        "<",
        "</",
        "< div>",
        "</ div>",
        "<a",
        "<a\r\n  ",
        "<a></a\r\n ",
        "<a><b></a>",
        "<p><😀></p>",
        "<script>a < b\r\n",
        "<textarea></TEXTAREA>",
        "<!DOCTYPE>",
        "<!--comment",
        "<?instruction",
        "<x y=\"ö&bar",
        "<x = abc/ >",
        "<dl><dt>a<dd>b<dt>c</dl>",
        "<math><mrow /></math><a/>",
        "<!DOCTYPE html>\r\n<名 class=\"ö&amp;é\" id=外>😀</名>",
    ] {
        let doc = parse(source);
        assert_tree_ranges(&doc);
        let spans: Vec<_> = doc.highlights(0..source.len()).unwrap().collect();
        assert!(
            spans
                .iter()
                .all(|span| source.get(span.range.clone()).is_some())
        );
        assert!(
            spans
                .windows(2)
                .all(|pair| pair[0].range.end <= pair[1].range.start)
        );
    }
    let doc = parse("<a><b></a>");
    let error = doc
        .nodes()
        .find(|node| node.kind() == HtmlNodeKind::Error)
        .unwrap();
    assert_eq!(error.range(), 6..6);
    assert_eq!(error.parent().unwrap().range(), 3..6);
}

#[test]
fn highlights_markup_references_and_mismatches_and_clips_to_utf8() {
    let source =
        "<!DOCTYPE html>\r\n<?test?><a x=\"é&amp;中&#67;&stray\" id=外>😀<!--ok--></b></a><";
    let doc = parse(source);
    let highlights = |range| {
        doc.highlights(range)
            .unwrap()
            .map(|span| {
                (
                    &source[span.range],
                    span.tags
                        .iter()
                        .map(|tag| tag.name())
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        highlights(0..source.len()),
        [
            ("<!DOCTYPE html>", "documentMeta"),
            ("\r\n", "content"),
            ("<?test?>", "processingInstruction"),
            ("<", "angleBracket"),
            ("a", "tagName"),
            ("x", "attributeName"),
            ("=", "definitionOperator"),
            ("\"é", "attributeValue"),
            ("&amp;", "character"),
            ("中", "attributeValue"),
            ("&#67;", "character"),
            ("stray\"", "attributeValue"),
            ("id", "attributeName"),
            ("=", "definitionOperator"),
            ("外", "attributeValue"),
            (">", "angleBracket"),
            ("😀", "content"),
            ("<!--ok-->", "blockComment"),
            ("</", "angleBracket"),
            ("b", "tagName invalid"),
            (">", "angleBracket"),
            ("</", "angleBracket"),
            ("a", "tagName"),
            (">", "angleBracket"),
            ("<", "content"),
        ]
        .map(|(text, tags)| (text, tags.to_owned()))
    );
    let accent = source.find('é').unwrap();
    assert_eq!(
        highlights(accent..accent + 4),
        [
            ("é", "attributeValue".to_owned()),
            ("&a", "character".to_owned())
        ]
    );
    assert!(doc.highlights(accent + 1..accent + 3).is_none());
    assert!(highlights(accent..accent).is_empty());
    assert!(
        doc.highlights(Range {
            start: accent,
            end: 0
        })
        .is_none()
    );
}

#[test]
fn routes_and_mounts_nested_content_without_treating_it_as_html() {
    use HtmlEmbeddedLanguage::*;
    let source = concat!(
        "<style type='TEXT/X-CSS'>\r\n  a {\r\n    color: red\r\n  }\r\n</style>",
        "<style lang=scss>.a { color: $red; }</style>",
        "<script type='application/vnd.example+JSON'>{\"α\":1}</script>",
        "<script type='speculationrules'>{extend: true}</script>",
        "<script type=module>if (a < b) x = '</not-script>';</script>",
        "<script type=text/typescript>x: string</script>",
        "<script type=text/jsx><A/></script><script type=text/typescript-jsx><B/></script>",
        "<script lang=ts type=text/babel>let x: T</script>",
        "<script type=text/vb>not JavaScript</script>",
        "<textarea><b>&amp;</b></textarea>",
        "<p style=\"color: blue\" onclick='run()' STYLE='plain' style=color:red>",
    );
    let doc = parse(source);
    assert_tree_ranges(&doc);
    assert_eq!(
        doc.mounts()
            .iter()
            .map(|mount| mount.language())
            .collect::<Vec<_>>(),
        [
            Css, Json, Json, JavaScript, TypeScript, Jsx, Tsx, TypeScript, CssStyles, JavaScript,
            CssStyles
        ]
    );
    let css = &doc.mounts()[0];
    let css_source = &source[css.range()];
    let HtmlEmbeddedDocument::Css(sheet) = css.document() else {
        panic!("expected CSS stylesheet")
    };
    assert_eq!(sheet.root().kind(), CssNodeKind::StyleSheet);
    assert_eq!(sheet.text().as_str(), css_source);
    for mount in &doc.mounts()[1..3] {
        let HtmlEmbeddedDocument::Javascript(expression) = mount.document() else {
            panic!("expected a JavaScript expression for a JSON script");
        };
        assert_eq!(mount.language(), Json);
        assert_eq!(expression.root().kind().name(), "SingleExpression");
        assert_eq!(
            expression.root().children().next().unwrap().kind().name(),
            "ObjectExpression"
        );
        assert!(!expression.is_recovered());
        assert_eq!(expression.text().as_str(), &source[mount.range()]);
    }
    let HtmlEmbeddedDocument::Css(inline) = doc.mounts()[8].document() else {
        panic!("expected inline CSS")
    };
    assert_eq!(inline.root().kind(), CssNodeKind::Styles);
    assert_eq!(inline.text().as_str(), "color: blue");
    assert_eq!(&source[doc.mounts()[9].range()], "run()");
    for mount in doc
        .mounts()
        .iter()
        .filter(|mount| matches!(mount.language(), JavaScript | TypeScript | Jsx | Tsx))
    {
        let HtmlEmbeddedDocument::Javascript(script) = mount.document() else {
            panic!("expected native script")
        };
        assert_eq!(script.text().as_str(), &source[mount.range()]);
    }
    let literal = "'</not-script>'";
    let string = source.find(literal).unwrap();
    let span = doc
        .highlights(string..string + literal.len())
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(span.range, string..string + literal.len());
    assert_eq!(span.tags[0].name(), "string");
    assert!(
        doc.nodes()
            .any(|node| node.kind() == HtmlNodeKind::TextareaText
                && &source[node.range()] == "<b>&amp;</b>")
    );
    assert!(
        doc.nodes()
            .any(|node| node.kind() == HtmlNodeKind::ScriptText
                && &source[node.range()] == "if (a < b) x = '</not-script>';")
    );
    let color = source.find("color: red").unwrap();
    let blue = source.find("color: blue").unwrap();
    for at in [color, blue] {
        let span = doc.highlights(at..at + 3).unwrap().next().unwrap();
        assert_eq!(span.range, at..at + 3);
        assert_eq!(span.tags[0].name(), "propertyName");
    }
    let quote = doc.highlights(blue - 1..blue).unwrap().next().unwrap();
    assert_eq!(quote.tags[0].name(), "attributeValue");
    let local_fold = doc
        .text()
        .line_at(source.find("a {").unwrap())
        .unwrap()
        .range;
    let fold = doc.foldable(local_fold.start, local_fold.end).unwrap();
    assert_eq!(&source[fold], "\r\n    color: red\r\n  ");
    let declaration = doc.text().line_at(color).unwrap().range.start;
    assert_eq!(
        doc.indentation(
            declaration,
            StreamSettings::default(),
            IndentOptions::default()
        ),
        Some(4)
    );
    let start_on_tag_line = parse("  <style>a {\ncolor: blue\n}\n</style>");
    assert_eq!(
        start_on_tag_line.indentation(13, StreamSettings::default(), IndentOptions::default()),
        Some(4)
    );
}

#[test]
fn provides_indent_fold_and_bracket_metadata() {
    assert_eq!(NAME, "html");
    assert_eq!(BLOCK_COMMENT, ("<!--", "-->"));
    assert_eq!(WORD_CHARS, "-_");
    let data = language_data();
    let comments = data.get::<CommentTokens<'static>>("commentTokens").unwrap();
    assert_eq!(comments.block, Some(BLOCK_COMMENT));
    assert!(comments.line.is_none());
    assert_eq!(data.get::<&str>("wordChars"), Some(&WORD_CHARS));
    assert!(data.get::<fn(&str) -> bool>("indentOnInput").unwrap()(
        "\t</div>"
    ));
    for source in ["\t</div>", "</div ", "\u{feff}</X1_>", "</x-", "</xα"] {
        assert!(indent_on_input(source), "{source:?}");
    }
    for source in [
        "</",
        "</div",
        "x</div>",
        "</foo-bar>",
        "</x😀",
        "\u{85}</x>",
    ] {
        assert!(!indent_on_input(source), "{source:?}");
    }
    let source = "<main>\r\n\t<div\r\n\t  x=\"😀\"\r\n\t>\r\n\t  hi\r\n\t</div>\r\n</main>\r\n";
    let doc = parse(source);
    let settings = StreamSettings::default();
    let options = IndentOptions::default();
    let starts = std::iter::once(0).chain(source.match_indices('\n').map(|(at, _)| at + 1));
    assert_eq!(
        starts
            .map(|at| doc.indentation(at, settings, options))
            .collect::<Vec<_>>(),
        [
            Some(0),
            Some(2),
            Some(6),
            Some(6),
            Some(6),
            Some(4),
            Some(0),
            None
        ]
    );
    assert_eq!(doc.foldable(0, 6), Some(6..source.find("</main>").unwrap()));
    let attr_end = source.find("\t>\r\n").unwrap();
    assert_eq!(
        doc.foldable(attr_end, attr_end + 2),
        Some(attr_end + 2..source.find("</div>").unwrap())
    );
    let handles: Vec<_> = doc
        .nodes()
        .filter_map(HtmlNode::bracket_matching_handle)
        .map(|node| &source[node.range()])
        .collect();
    assert_eq!(handles, ["main", "div", "div", "main"]);
    let override_indentation = |at| (at == attr_end).then_some(8);
    let options = IndentOptions {
        override_indentation: Some(&override_indentation),
        ..options
    };
    assert_eq!(doc.indentation(attr_end, settings, options), Some(6));
    let unclosed = parse("    <a>\n");
    assert_eq!(
        unclosed.indentation(unclosed.text().len(), settings, options),
        Some(6)
    );
    assert_eq!(unclosed.foldable(0, 7), Some(7..8));
    let incomplete = parse("  <a\n");
    assert_eq!(
        incomplete.indentation(incomplete.text().len(), settings, options),
        Some(4)
    );
}

#[test]
fn bounded_tables_and_iterative_recovery_handle_deep_or_repetitive_input() {
    let source = "<x>".repeat(512) + &"&".repeat(20_000) + "</x><br/>";
    let doc = parse(&source);
    assert_eq!(
        doc.nodes()
            .filter(|node| node.kind() == HtmlNodeKind::InvalidEntity)
            .count(),
        20_000
    );
    assert_eq!(
        doc.nodes()
            .filter(|node| node.kind() == HtmlNodeKind::SelfClosingTag)
            .count(),
        1
    );
    assert_tree_ranges(&doc);
}
