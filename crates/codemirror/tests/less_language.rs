use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Text,
    autocomplete::session::SourceResult,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        css::{CssDocument, css_completion_source},
        less::{self, LessDocument, LessNodeKind::*, less_completion_source},
    },
};

// Expected trees, tags, columns, folds, and completion contexts come from
// lang-less 6.0.2 and the pinned CodeMirror APIs, run without the editor view.
const LESS_SOURCE: &str = concat!(
    "// 😀 comment\n",
    "@base: #abc;\n",
    "@prop: color;\n",
    "@detached: { width: 12px; };\n",
    ".mixin(@x: 2px; @rest...) when (@x > 0) {\n",
    "  @{prop}: @base;\n",
    "  color: $other;\n",
    "  margin: (2 + 3) * 1px;\n",
    "  content: ~\"😀\";\n",
    "  .nested > .child:hover {\n",
    "    #ns.mixin(@x);\n",
    "    @detached();\n",
    "  }\n",
    "}",
);

fn parses(source: &str) -> LessDocument {
    let doc = LessDocument::parse(Text::new(source));
    assert!(!doc.nodes().any(|node| node.kind() == Error), "{source:?}");
    doc
}

#[test]
fn parses_less_variables_guards_interpolation_and_mixins() {
    let doc = parses(LESS_SOURCE);
    assert_eq!(doc.root().kind(), StyleSheet);
    assert_eq!(doc.root().range(), 0..LESS_SOURCE.len());
    assert_eq!(
        doc.root()
            .children()
            .map(|node| node.kind())
            .collect::<Vec<_>>(),
        [
            LineComment,
            Declaration,
            Semicolon,
            Declaration,
            Semicolon,
            DetachedRuleSet,
            RuleSet
        ]
    );
    let picked: Vec<_> = doc
        .nodes()
        .filter_map(|node| match node.kind() {
            When | Interpolation | PropertyVariable | Escape | CallExpression | Inclusion => {
                Some((node.kind(), &LESS_SOURCE[node.range()]))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        picked,
        [
            (When, "when"),
            (Interpolation, "@{prop}"),
            (PropertyVariable, "$other"),
            (Escape, "~\"😀\""),
            (Inclusion, "#ns.mixin(@x)"),
            (CallExpression, "@detached()"),
        ]
    );
    let plus = doc
        .nodes()
        .find(|node| node.kind() == BinOp && &LESS_SOURCE[node.range()] == "+")
        .unwrap();
    assert_eq!(plus.parent().unwrap().kind(), BinaryExpression);
    let rest = doc
        .nodes()
        .find(|node| node.kind() == AtKeyword && &LESS_SOURCE[node.range()] == "@rest")
        .unwrap();
    assert_eq!(rest.parent().unwrap().kind(), RuleSet);
}

#[test]
fn parses_imports_queries_and_keyframes_without_mounted_languages() {
    let source = concat!(
        "@import (reference) \"file.less\";\n",
        "@plugin \"x.js\";\n",
        "@supports selector(div:focus) { a { color: red; } }\n",
        "@media screen and (width: 12em) { a { x: url(img.png) } }\n",
        "@keyframes fade {\n  from { opacity: 0 }\n  to { opacity: 1 }\n}",
    );
    let doc = parses(source);
    let picked: Vec<_> = doc
        .nodes()
        .filter_map(|node| match node.kind() {
            Import | StringLiteral | Selector | FeatureName | Unit | CallTag
            | ParenthesizedContent | From | To => Some((node.kind(), &source[node.range()])),
            _ => None,
        })
        .collect();
    assert_eq!(
        picked,
        [
            (Import, "@import"),
            (StringLiteral, "\"file.less\""),
            (Import, "@plugin"),
            (StringLiteral, "\"x.js\""),
            (Selector, "selector"),
            (FeatureName, "width"),
            (Unit, "em"),
            (CallTag, "url"),
            (ParenthesizedContent, "img.png"),
            (From, "from"),
            (To, "to"),
        ]
    );
    let line = doc.text().line(5).unwrap();
    assert_eq!(doc.foldable(line.range.start, line.range.end), None);
}

#[test]
fn matches_less_highlight_tags_and_utf8_byte_ranges() {
    let source = LESS_SOURCE.replace('\n', "\r\n");
    let doc = parses(&source);
    assert_eq!(doc.text().as_str(), source);
    let spans: Vec<_> = doc.highlights(0..source.len()).unwrap().collect();
    let tag = |token: &str| {
        let span = spans
            .iter()
            .find(|span| &source[span.range.clone()] == token)
            .unwrap();
        assert_eq!(span.tags.len(), 1);
        (span.tags[0].name(), span.tags[0].modifiers())
    };
    assert_eq!(tag("// 😀 comment"), ("blockComment", &[][..]));
    assert_eq!(
        tag("@{prop}"),
        ("variableName", &[TagModifier::Special][..])
    );
    assert_eq!(tag("$other"), ("propertyName", &[][..]));
    assert_eq!(tag("#abc"), ("color", &[][..]));
    assert_eq!(tag("#"), ("derefOperator", &[][..]));
    assert_eq!(tag("hover"), ("className", &[TagModifier::Constant][..]));
    assert_eq!(tag("~\"😀\""), ("string", &[TagModifier::Special][..]));
    let emoji = source.find('😀').unwrap();
    let clipped = doc.highlights(emoji..emoji + 4).unwrap().next().unwrap();
    assert_eq!(clipped.range, emoji..emoji + 4);
    assert!(doc.highlights(emoji + 1..source.len()).is_none());
    assert!(doc.highlights(emoji..emoji).unwrap().next().is_none());
    assert!(
        doc.highlights(Range {
            start: emoji,
            end: 0
        })
        .is_none()
    );
    assert!(
        doc.indentation(
            emoji + 1,
            StreamSettings::default(),
            IndentOptions::default()
        )
        .is_none()
    );
    assert!(doc.foldable(0, emoji + 1).is_none());
    assert!(less_completion_source(&doc, emoji + 1, true).is_none());
}

#[test]
fn distinguishes_css_like_contexts_using_less_grammar() {
    let source = ".a + .b ~ .c > .d .e, #foo[attr~=word] { foo: a and b; w: calc(1px + 2px); f: calc (3px); }";
    let doc = parses(source);
    assert_eq!(
        doc.nodes()
            .filter(|node| node.kind() == CallExpression)
            .count(),
        1
    );
    assert_eq!(
        doc.nodes()
            .filter(|node| node.kind() == ParenthesizedValue)
            .count(),
        1
    );
    let logic: Vec<_> = doc
        .highlights(0..source.len())
        .unwrap()
        .filter(|span| span.tags[0].name() == "logicOperator")
        .map(|span| &source[span.range])
        .collect();
    assert_eq!(logic, [">", "and"]);
    assert!(doc.nodes().any(|node| node.kind() == DescendantSelector));
    assert!(doc.nodes().any(|node| node.kind() == AttributeSelector));
}

#[test]
fn indents_continuations_and_folds_blocks() {
    let settings = StreamSettings::default();
    let doc = parses("  a { color:\nred;\n    .nested { top: 1px;\nleft: 2px\n    }\n  }");
    let actual = (1..=doc.text().line_count())
        .map(|n| {
            let line = doc.text().line(n).unwrap().range;
            (
                doc.indentation(line.start, settings, IndentOptions::default())
                    .unwrap(),
                doc.foldable(line.start, line.end),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        [
            (0, Some(5..60)),
            (4, None),
            (6, Some(31..56)),
            (14, None),
            (12, None),
            (4, None)
        ]
    );

    // Function argument parentheses are external tokens without `closedBy`.
    // They use continued declaration indentation rather than column alignment.
    let doc = parses("  a {\n    x: calc(1px,\n2px\n    );\n  }");
    let line = doc.text().line(3).unwrap();
    assert_eq!(
        doc.indentation(line.range.start, settings, IndentOptions::default()),
        Some(6)
    );
    assert_eq!(
        doc.indentation(
            line.range.start,
            settings,
            IndentOptions {
                override_indentation: Some(&|line| (line == 6).then_some(7)),
                ..IndentOptions::default()
            }
        ),
        Some(9)
    );

    let doc = parses("a { }");
    for (simulate_double_break, expected) in [(false, 0), (true, 2)] {
        let options = IndentOptions {
            simulate_break: Some(4),
            simulate_double_break,
            ..IndentOptions::default()
        };
        assert_eq!(doc.indentation(4, settings, options), Some(expected));
    }
}

fn complete(input: &str, explicit: bool) -> Option<SourceResult> {
    let at = input.find('§').unwrap();
    let doc = LessDocument::parse(Text::new(input.replace('§', "")));
    less_completion_source(&doc, at, explicit)
}

#[test]
fn completes_at_variables_and_reuses_css_lists() {
    let result = complete(
        "@var: 1; @variant: 2; @var: 3; --thing: 4; .a {\n color: @va§;\n}",
        false,
    )
    .unwrap();
    assert_eq!((result.from, result.to), (56, None));
    assert_eq!(
        result
            .options
            .iter()
            .map(|o| (o.label.as_str(), o.kind.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("@var", Some("variable")),
            ("@variant", Some("variable")),
            ("--thing", Some("variable"))
        ]
    );
    assert_eq!(
        complete("@var: 1; @va§", true).unwrap().options[0].label,
        "@var"
    );
    assert_eq!(
        complete("@var: 1; .a { color: @§; }", false)
            .unwrap()
            .options[0]
            .label,
        "@var"
    );
    let property = complete(".a { co§l", false).unwrap();
    let css = css_completion_source(&CssDocument::parse(Text::new("a { color: red }")), 8, false)
        .unwrap();
    assert_eq!(property.from, 5);
    assert!(Arc::ptr_eq(&property.options, &css.options));
    assert_eq!(
        property
            .options
            .iter()
            .find(|o| o.label == "color")
            .unwrap()
            .insert
            .as_deref(),
        Some("color: ")
    );
    assert_eq!(complete("bo§dy {}", false).unwrap().options.len(), 78);
    assert_eq!(complete(".a:§ {}", true).unwrap().options.len(), 63);
    assert_eq!(
        complete(".a { col: §red; }", true).unwrap().options.len(),
        561
    );
    assert_eq!(
        complete("--foo: 1; .a { c: var(§) }", true)
            .unwrap()
            .options
            .len(),
        561
    );
    assert!(complete(".a::§ {}", true).is_none());
    assert!(complete(".a { col:§ }", true).is_none());
    assert!(complete(".a { c: $c§olor }", false).is_none());
}

#[test]
fn repairs_missing_delimiters_and_recovers_later_rules() {
    let source = ".a {\r\n  col: @;\r\n  ???😀\r\n}\r\n.b { x: 2px }";
    let doc = LessDocument::parse(Text::new(source));
    assert!(
        doc.nodes()
            .any(|node| node.kind() == Error && &source[node.range()] == "😀")
    );
    assert!(
        doc.nodes()
            .any(|node| node.kind() == Unit && &source[node.range()] == "px")
    );
    assert_tree_ranges(&doc);

    let source = ".a { x: rgb(1; width: 2px; }\n.b { x: 2px; }";
    let doc = LessDocument::parse(Text::new(source));
    let properties: Vec<_> = doc
        .nodes()
        .filter(|node| node.kind() == PropertyName)
        .map(|node| &source[node.range()])
        .collect();
    assert_eq!(properties, ["x", "width", "x"]);
    assert!(
        doc.nodes()
            .any(|node| node.kind() == Error && node.parent().unwrap().kind() == ArgList)
    );
    assert_tree_ranges(&doc);

    let source = "a { image: url(http://x);\r\n color: @red";
    let doc = LessDocument::parse(Text::new(source));
    assert!(
        doc.nodes()
            .any(|node| node.kind() == ParenthesizedContent && &source[node.range()] == "http://x")
    );
    assert!(
        doc.nodes()
            .any(|node| node.kind() == AtKeyword && &source[node.range()] == "@red")
    );
    assert_tree_ranges(&doc);

    let source = ".a {\n .b {\n color: red\n}\n";
    let doc = LessDocument::parse(Text::new(source));
    let last = doc.nodes().last().unwrap();
    assert_eq!(
        (last.kind(), last.range()),
        (Error, source.len()..source.len())
    );
    assert_eq!(doc.foldable(0, 4), Some(4..source.len()));
    assert_eq!(
        doc.indentation(
            source.len(),
            StreamSettings::default(),
            IndentOptions::default()
        ),
        Some(2)
    );
}

fn assert_tree_ranges(doc: &LessDocument) {
    assert_eq!(doc.root().range(), 0..doc.text().len());
    for node in doc.nodes() {
        let range = node.range();
        assert!(doc.text().slice(range.clone()).is_some());
        let mut previous_end = range.start;
        for child in node.children() {
            let r = child.range();
            assert!(previous_end <= r.start && r.end <= range.end, "{child:?}");
            assert_eq!(child.parent().unwrap().range(), range);
            previous_end = r.end;
        }
    }
    let mut previous_end = 0;
    for span in doc.highlights(0..doc.text().len()).unwrap() {
        assert!(previous_end <= span.range.start);
        assert!(doc.text().slice(span.range.clone()).is_some());
        previous_end = span.range.end;
    }
}

#[test]
fn bounds_repair_depth_and_exposes_language_data() {
    let broken = ".a {\n".repeat(200);
    let doc = LessDocument::parse(Text::new(&broken));
    assert_tree_ranges(&doc);
    assert!(doc.nodes().any(|node| node.kind() == Error));
    assert_eq!(less::NAME, "less");
    assert_eq!(less::WORD_CHARS, "@-");
    assert_eq!(less::LINE_COMMENT, "//");
    assert_eq!(less::BLOCK_COMMENT, ("/*", "*/"));
    assert!(less::indent_on_input("\u{feff}\t}"));
    assert!(!less::indent_on_input("\u{85}}"));
    assert!(!less::indent_on_input("  } "));
}
