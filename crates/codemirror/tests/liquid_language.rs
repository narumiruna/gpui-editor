use gpui_editor_codemirror::{
    Assoc, Text,
    commands::CommentTokens,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        html::{HtmlEmbeddedLanguage, HtmlOptions},
        liquid::{
            HTML_CLOSE_BRACKETS, LiquidDocument, LiquidNode, LiquidNodeKind as Kind, LiquidOptions,
            html_language_data, indent_on_input, language_data,
        },
    },
};

fn parse(source: &str) -> LiquidDocument {
    LiquidDocument::parse(Text::new(source))
}

fn shape(node: LiquidNode<'_>) -> String {
    let children: Vec<_> = node.children().map(shape).collect();
    if children.is_empty() {
        node.kind().name().to_owned()
    } else {
        format!("{}({})", node.kind().name(), children.join(","))
    }
}

#[test]
fn bundled_grammar_matches_reference_directives_and_expressions() {
    // Reference from @codemirror/lang-liquid 6.3.4. Punctuation names are unquoted here.
    let source = "{% assign value = (1..3) %}{% for item in value reversed limit: 2 %}\n{{ item['name'].size | plus: 1 }}\n{% endfor %}\n{% comment %}<b>hidden</b>{{ y }}\n{% endcomment %}\n{% raw %}<em>{{ raw }}</em>\n{% endraw %}\n{% # 😀 %}";
    let doc = parse(source);
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(
        shape(doc.root()),
        concat!(
            "Template(Tag({%,assign,AssignmentExpression(VariableName,AssignOp,RangeExpression((,NumberLiteral,..,NumberLiteral,))),%}),",
            "ForDirective(Tag({%,for,VariableName,in,VariableName,Parameter(ParameterName),Parameter(ParameterName,:,NumberLiteral),%}),",
            "Text,Interpolation({{,MemberExpression(SubscriptExpression(VariableName,StringLiteral),.,PropertyName),Filter(|,FilterName,:,NumberLiteral),}}),Text,EndTag({%,endfor,%})),",
            "Text,Comment(Tag({%,comment,%}),CommentText,EndTag({%,endcomment,%})),Text,RawDirective(Tag({%,raw,%}),RawText,EndTag({%,endraw,%})),Text,Tag({%,InlineComment,%}))"
        ),
    );
    let highlight = |needle: &str| {
        let start = source.find(needle).unwrap();
        let end = start + needle.len();
        let spans: Vec<_> = doc.highlights(start..end).unwrap().collect();
        assert_eq!(spans.len(), 1, "{needle:?}: {spans:?}");
        assert_eq!(spans[0].range, start..end);
        &spans[0].tags[0]
    };
    assert_eq!(highlight("plus").name(), "variableName");
    assert_eq!(highlight("plus").modifiers(), &[TagModifier::Function]);
    assert_eq!(highlight("<b>hidden</b>{{ y }}\n").name(), "blockComment");
    assert_eq!(highlight("{{ raw }}").name(), "content");
    assert_eq!(highlight("# 😀 ").name(), "lineComment");
    assert_eq!(highlight("reversed").name(), "attributeName");
}

#[test]
fn html_overlay_uses_the_existing_parser_and_maps_gaps_and_crlf() {
    let source = concat!(
        "<{{ skip }}main title=\"hi 😀 {{ name }}!\">\r\n",
        "<script>const x = {{ val }}10;</script>\r\n",
        "<style>.class-{{ id }} { color: red }</style>\r\n",
        "</main>"
    );
    let doc = parse(source);
    let html = doc.html().unwrap();
    assert_eq!(
        html.document().text().as_str(),
        "<main title=\"hi 😀 !\">\r\n<script>const x = 10;</script>\r\n<style>.class- { color: red }</style>\r\n</main>"
    );
    assert_eq!(
        html.document()
            .mounts()
            .iter()
            .map(|mount| mount.language())
            .collect::<Vec<_>>(),
        [HtmlEmbeddedLanguage::JavaScript, HtmlEmbeddedLanguage::Css]
    );
    let gap = source.find("{{ skip }}").unwrap();
    assert_eq!(html.to_html(gap), Some(gap));
    assert_eq!(html.to_html(gap + 1), None);
    assert_eq!(html.to_html(gap + "{{ skip }}".len()), Some(gap));
    assert_eq!(html.to_source(gap, Assoc::Before), Some(gap));
    assert_eq!(
        html.to_source(gap, Assoc::After),
        Some(gap + "{{ skip }}".len())
    );
    let title = html.document().text().as_str().find("\"hi").unwrap();
    let pieces: Vec<_> = html
        .source_ranges(title..title + "\"hi 😀 !\"".len())
        .unwrap()
        .map(|range| &source[range])
        .collect();
    assert_eq!(pieces, ["\"hi 😀 ", "!\""]);
    let check = |needle: &str, tag: &str| {
        let at = source.find(needle).unwrap();
        let spans: Vec<_> = doc.highlights(at..at + needle.len()).unwrap().collect();
        assert_eq!(spans.len(), 1, "{needle:?}: {spans:?}");
        assert_eq!(spans[0].tags[0].name(), tag);
    };
    check("main", "tagName");
    check("hi 😀 ", "attributeValue");
    check("!\"", "attributeValue");
    check("10", "number");
    check("class-", "className");
    check("color", "propertyName");
    let name = source.find("{{ name").unwrap();
    assert_eq!(
        doc.highlights(name..name + "{{ name".len())
            .unwrap()
            .map(|s| (&source[s.range], s.tags[0].name()))
            .collect::<Vec<_>>(),
        [("{{", "brace"), ("name", "variableName")]
    );
    let crlf = source.find("\r\n").unwrap();
    assert_eq!(
        doc.highlights(crlf..crlf + 2)
            .unwrap()
            .next()
            .unwrap()
            .range,
        crlf..crlf + 2
    );
    let emoji = source.find('😀').unwrap();
    assert!(doc.highlights(emoji + 1..emoji + 3).is_none());
    assert!(doc.highlights(emoji..emoji).unwrap().next().is_none());
    assert!(doc.highlights(source.len()..source.len() + 1).is_none());
    assert_eq!(html.to_html(emoji + 1), None);
}

#[test]
fn indentation_and_folds_cover_liquid_and_html() {
    let source = "{% if x %}\n<p>\n {{x}}\n</p>\n{% else %}\n<p>{{y}}</p>\n{% endif %}";
    let doc = parse(source);
    let folds: Vec<_> = source
        .lines()
        .scan(0, |start, line| {
            let result = doc.foldable(*start, *start + line.len());
            *start += line.len() + 1;
            Some(result.map(|range| &source[range]))
        })
        .collect();
    assert_eq!(
        folds,
        [
            Some("\n<p>\n {{x}}\n</p>\n{% else %}\n<p>{{y}}</p>\n"),
            Some("\n {{x}}\n"),
            None,
            None,
            None,
            None,
            None
        ]
    );

    let settings = StreamSettings::default();
    let lines = [
        "{% if x %}",
        "<p>\n",
        " {{x}}",
        "</p>",
        "{% else %}",
        "<p>{{y}}",
    ];
    let indentation: Vec<_> = lines
        .into_iter()
        .map(|needle| {
            doc.indentation(
                source.find(needle).unwrap(),
                settings,
                IndentOptions::default(),
            )
        })
        .collect();
    assert_eq!(
        indentation,
        [Some(0), Some(2), Some(2), Some(0), Some(0), Some(2)]
    );

    let src = "{% liquid\n  assign n = 2\n  for item in collection\n    echo item.name\n  endfor\n  # comment\n  echo n\n%}";
    let doc = parse(src);
    let columns = src
        .lines()
        .scan(0, |start, line| {
            let indent = doc.indentation(*start, settings, IndentOptions::default());
            *start += line.len() + 1;
            Some(indent)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        [
            Some(0),
            Some(3),
            Some(3),
            Some(4),
            Some(2),
            Some(3),
            Some(3),
            Some(0)
        ]
    );
}

#[test]
fn partial_tags_preserve_surrounding_trees_and_error_ranges() {
    let source = "<p>😀</p>\r\n{% if x %}\r\n  {{ product. }}\r\n  {{ value |\r\n  .";
    let doc = parse(source);
    assert_eq!(
        shape(doc.root()),
        "Template(Text,IfDirective(Tag({%,if,VariableName,%}),Text,Interpolation({{,MemberExpression(VariableName,.,⚠),}}),Text,Interpolation({{,VariableName,Filter(|,⚠),⚠),Text,⚠))"
    );
    for node in doc.nodes() {
        let range = node.range();
        assert!(doc.text().slice(range.clone()).is_some(), "{node:?}");
        if let Some(parent) = node.parent() {
            assert!(parent.range().start <= range.start && range.end <= parent.range().end);
        }
    }
    let expected_start = source.find("%}").unwrap() + 2;
    assert_eq!(
        doc.root().children().next_back().unwrap().fold_range(),
        Some(expected_start..source.len())
    );
    let emoji = source.find('😀').unwrap();
    assert_eq!(
        doc.indentation(
            emoji + 1,
            StreamSettings::default(),
            IndentOptions::default()
        ),
        None
    );

    let deep = "{% if x %}".repeat(1_024) + "{{ value |";
    let doc = parse(&deep);
    assert_eq!(doc.root().range(), 0..deep.len());
    assert!(doc.nodes().any(|node| node.kind() == Kind::Error));
}

#[test]
fn exposes_liquid_and_outer_html_language_data() {
    let data = language_data();
    let comments = data.get::<CommentTokens>("commentTokens").unwrap();
    assert_eq!(comments.line, Some("#"));
    assert_eq!(comments.block, None);
    assert_eq!(
        html_language_data().get::<[char; 1]>("closeBrackets"),
        Some(&HTML_CLOSE_BRACKETS)
    );
    assert!(indent_on_input("  {%- elsif"));
    assert!(indent_on_input("{%"));
    assert!(indent_on_input("{%end"));
    assert!(!indent_on_input("{%endif"));
    assert!(!indent_on_input("{% else "));

    let doc = LiquidDocument::parse_with_options(
        Text::new("<widget/>{{ x }}"),
        LiquidOptions {
            html: HtmlOptions {
                self_closing_tags: true,
                ..HtmlOptions::default()
            },
        },
    );
    assert!(
        doc.html()
            .unwrap()
            .document()
            .nodes()
            .any(|node| node.kind().name() == "SelfClosingTag")
    );
    assert!(parse("{{ x }}").html().is_none());
}
