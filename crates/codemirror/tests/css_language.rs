use gpui_editor_codemirror::{
    Text,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        css::{self, CssDocument, CssNodeKind},
    },
};
use serde_json::json;

#[test]
fn matches_pinned_upstream_syntax_highlights_and_editor_services() {
    let oracle: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../src/language/css/oracle.json")).unwrap();
    for expected in oracle {
        let source = expected["doc"].as_str().unwrap();
        let doc = CssDocument::parse(Text::new(source));
        let nodes: Vec<_> = doc
            .nodes()
            .map(|node| {
                let depth = std::iter::successors(node.parent(), |n| n.parent()).count();
                let range = node.range();
                json!([node.kind().name(), range.start, range.end, depth])
            })
            .collect();
        assert_eq!(json!(nodes), expected["nodes"], "tree for {source:?}");

        let styles: Vec<_> = doc
            .highlights(0..source.len())
            .unwrap()
            .map(|span| {
                let [tag] = span.tags else {
                    panic!("expected one CSS tag")
                };
                let name = if tag.modifiers() == [TagModifier::Constant] {
                    assert_eq!(tag.name(), "className");
                    "className.constant"
                } else {
                    assert!(tag.modifiers().is_empty());
                    tag.name()
                };
                json!([span.range.start, span.range.end, name])
            })
            .collect();
        assert_eq!(
            json!(styles),
            expected["highlights"],
            "highlights for {source:?}"
        );
        let mut start = 0;
        let lines: Vec<_> = source
            .split('\n')
            .map(|line| {
                let end = start + line.len();
                let indent =
                    doc.indentation(start, StreamSettings::default(), IndentOptions::default());
                let fold = doc
                    .foldable(start, end)
                    .map(|range| [range.start, range.end]);
                start = end + 1;
                json!([indent, fold])
            })
            .collect();
        assert_eq!(
            json!(lines),
            expected["lines"],
            "line services for {source:?}"
        );
    }
}

#[test]
fn preserves_crlf_and_utf8_byte_ranges() {
    let source = "/* alpha\r\n😀 */\r\n.é {\r\n  label: \"東京\";\r\n}";
    let doc = CssDocument::parse(Text::new(source));
    assert_eq!(doc.text().as_str(), source);
    assert_eq!(doc.root().range(), 0..source.len());
    let string = doc
        .nodes()
        .find(|node| node.kind() == CssNodeKind::StringLiteral)
        .unwrap();
    assert_eq!(doc.text().slice(string.range()), Some("\"東京\""));
    let city = source.find('東').unwrap();
    let spans: Vec<_> = doc
        .highlights(city..city + '東'.len_utf8())
        .unwrap()
        .collect();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].range, city..city + '東'.len_utf8());
    assert_eq!(spans[0].tags[0].name(), "string");
    assert!(doc.highlights(city + 1..source.len()).is_none());
    assert!(doc.highlights(city..city).unwrap().next().is_none());
    let line_start = source.find(".é").unwrap();
    let line_end = source.find('{').unwrap() + 1;
    assert_eq!(
        doc.foldable(line_start, line_end),
        Some(line_end..source.len() - 1)
    );
}

#[test]
fn parses_inline_styles_and_recovers_following_declarations() {
    let source = "color: red;\r\n/* 字 */ width: 4px;";
    let doc = CssDocument::parse_styles(Text::new(source));
    assert_eq!(doc.root().kind(), CssNodeKind::Styles);
    assert_eq!(doc.root().range(), 0..source.len());
    assert_eq!(
        doc.root()
            .children()
            .filter(|node| node.kind() == CssNodeKind::Declaration)
            .count(),
        2
    );
    assert!(
        doc.highlights(0..source.len())
            .unwrap()
            .any(|span| &source[span.range] == "/* 字 */")
    );

    let source = "color: ); width: 4px;";
    let doc = CssDocument::parse_styles(Text::new(source));
    assert_eq!(doc.root().kind(), CssNodeKind::Styles);
    assert!(
        doc.nodes().any(
            |node| node.kind() == CssNodeKind::PropertyName && &source[node.range()] == "width"
        )
    );
}

#[test]
fn malformed_declarations_do_not_hide_later_rules() {
    let source = ".a { color: var(--red; width: 3px; content: \"bad\n; }\n.b { color: #f08; }";
    let doc = CssDocument::parse(Text::new(source));
    let names: Vec<_> = doc
        .nodes()
        .filter(|node| node.kind() == CssNodeKind::PropertyName)
        .map(|node| &source[node.range()])
        .collect();
    assert_eq!(names, ["color", "width", "content", "color"]);
    assert!(doc.nodes().any(|node| node.kind() == CssNodeKind::Error));
    assert!(doc.nodes().any(|node| node.kind() == CssNodeKind::ColorLiteral && &source[node.range()] == "#f08"));
}

#[test]
fn recovers_unfinished_blocks_and_continuations() {
    let source = ".a {\n  color: red;\n";
    let doc = CssDocument::parse(Text::new(source));
    assert_eq!(doc.foldable(0, 4), Some(4..source.len()));
    assert_eq!(
        doc.indentation(
            source.len(),
            StreamSettings::default(),
            IndentOptions::default()
        ),
        Some(2)
    );
    let source = ".a {\n\tcolor: red\n    blue;\n}";
    let doc = CssDocument::parse(Text::new(source));
    let continuation = source.find("    blue").unwrap();
    assert_eq!(
        doc.indentation(
            continuation,
            StreamSettings::default(),
            IndentOptions::default()
        ),
        Some(6)
    );
}

#[test]
fn indent_at_simulated_breaks_and_overridden_lines() {
    let doc = CssDocument::parse(Text::new("  a {}"));
    let settings = StreamSettings::default();
    let options = IndentOptions {
        simulate_break: Some(5),
        override_indentation: Some(&|line| (line == 0).then_some(3)),
        ..IndentOptions::default()
    };
    assert_eq!(doc.indentation(5, settings, options), Some(3));
    assert_eq!(
        doc.indentation(
            5,
            settings,
            IndentOptions {
                simulate_double_break: true,
                ..options
            }
        ),
        Some(5)
    );
}

#[test]
fn malformed_input_keeps_ordered_byte_ranges() {
    let parts = [
        "p", ".", "--x", "1px", "@media", "/*é*/", "😀", "!", ":", "::", ";", "{", "}", "(", ")",
        "[", "]", "#ccc", "\"str\"", "\"bad", "\\é", " ", "\r\n", "url(",
    ];
    let mut state = 13_u32;
    for _ in 0..256 {
        let mut source = "p { color: ".to_owned();
        for _ in 0..28 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            source.push_str(parts[(state >> 16) as usize % parts.len()]);
        }
        let doc = CssDocument::parse(Text::new(&source));
        for node in doc.nodes() {
            let range = node.range();
            assert!(source.get(range.clone()).is_some(), "{source:?}, {range:?}");
            let mut end = range.start;
            for child in node.children() {
                let child_range = child.range();
                assert!(
                    end <= child_range.start && child_range.end <= range.end,
                    "{source:?}, {range:?}, {child_range:?}"
                );
                end = child_range.end;
            }
        }
        let mut end = 0;
        for span in doc.highlights(0..source.len()).unwrap() {
            assert!(end <= span.range.start && source.get(span.range.clone()).is_some());
            end = span.range.end;
        }
    }
}

#[test]
fn exposes_css_language_data() {
    assert_eq!(css::NAME, "css");
    assert_eq!(css::BLOCK_COMMENT, ("/*", "*/"));
    assert_eq!(css::WORD_CHARS, "-");
    for line in ["}", " \t}", "\u{feff}}"] {
        assert!(css::indent_on_input(line));
    }
    for line in ["", "a}", "} ", " \t]", "\u{85}}"] {
        assert!(!css::indent_on_input(line));
    }
}
