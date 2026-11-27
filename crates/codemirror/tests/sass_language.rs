use gpui_editor_codemirror::{
    Text,
    autocomplete::session::SourceResult,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        sass::{self, SassDialect, SassDocument, sass_completion_source},
    },
};
use serde_json::{Value, json};

fn completion(result: Option<SourceResult>) -> Value {
    result.map_or(Value::Null, |result| {
        let items: Vec<_> = result
            .options
            .iter()
            .map(|item| {
                json!([
                    item.label,
                    item.kind,
                    item.insert.as_deref().unwrap_or_default()
                ])
            })
            .collect();
        json!([result.from, result.to, items])
    })
}

#[test]
fn matches_pinned_external_oracle_for_both_dialects() {
    // Captured from the published npm packages with EditorState, syntaxTree,
    // highlightTree, getIndentation, foldable, and Sass's completion source.
    // The property options use the same Chrome style names as the CSS oracle.
    let oracle: Value =
        serde_json::from_str(include_str!("../src/language/sass/oracle.json")).unwrap();
    for expected in oracle["cases"].as_array().unwrap() {
        let source = expected["doc"].as_str().unwrap();
        let dialect = match expected["dialect"].as_str().unwrap() {
            "scss" => SassDialect::Scss,
            "indented" => SassDialect::Indented,
            _ => unreachable!(),
        };
        let doc = SassDocument::parse(Text::new(source), dialect);
        assert_eq!(doc.text().as_str(), source);
        assert_eq!(doc.dialect(), dialect);
        let nodes: Vec<_> = doc
            .nodes()
            .map(|node| {
                let depth = std::iter::successors(node.parent(), |parent| parent.parent()).count();
                let range = node.range();
                json!([node.name(), range.start, range.end, depth])
            })
            .collect();
        let wants = expected["nodes"].as_array().unwrap();
        for (idx, (got, want)) in nodes.iter().zip(wants).enumerate() {
            assert_eq!(got, want, "node {idx} for {dialect:?} {source:?}");
        }
        assert_eq!(nodes.len(), wants.len(), "node count for {source:?}");
        let highlights: Vec<_> = doc
            .highlights(0..source.len())
            .unwrap()
            .map(|span| {
                let [tag] = span.tags else {
                    panic!("expected one Sass tag")
                };
                let name = match (tag.name(), tag.modifiers()) {
                    ("className", [TagModifier::Constant]) => "className.constant",
                    ("variableName", [TagModifier::Special]) => "variableName.special",
                    (name, []) => name,
                    (_, modifiers) => panic!("unexpected modifiers: {modifiers:?}"),
                };
                json!([span.range.start, span.range.end, name])
            })
            .collect();
        let wants = expected["highlights"].as_array().unwrap();
        for (idx, (got, want)) in highlights.iter().zip(wants).enumerate() {
            assert_eq!(got, want, "highlight {idx} for {source:?}");
        }
        assert_eq!(
            highlights.len(),
            wants.len(),
            "highlight count for {source:?}"
        );
        let lines: Vec<_> = (1..=doc.text().line_count())
            .map(|number| {
                let line = doc.text().line(number).unwrap();
                let start = line.range.start;
                let indentation =
                    doc.indentation(start, StreamSettings::default(), IndentOptions::default());
                let folded = doc
                    .foldable(start, line.range.end)
                    .map(|range| [range.start, range.end]);
                json!([indentation, folded])
            })
            .collect();
        assert_eq!(
            json!(lines),
            expected["lines"],
            "line services for {source:?}"
        );
        for test in expected["completion"].as_array().unwrap() {
            let at = usize::try_from(test[0].as_u64().unwrap()).unwrap();
            for (index, explicit) in [(1, false), (2, true)] {
                let result = completion(sass_completion_source(&doc, at, explicit));
                let expected = &test[index];
                let expected = if let Some(option_set) = expected[2].as_u64() {
                    let list = &oracle["optionSets"][usize::try_from(option_set).unwrap()];
                    json!([expected[0], expected[1], list])
                } else {
                    Value::Null
                };
                assert_eq!(
                    result, expected,
                    "completion at {at}, explicit {explicit} for {source:?}"
                );
            }
        }
    }
}

#[test]
fn reports_language_data_and_validates_byte_boundaries() {
    assert_eq!(sass::NAME, "sass");
    assert_eq!(sass::LINE_COMMENT, "//");
    assert_eq!(sass::BLOCK_COMMENT, ("/*", "*/"));
    assert_eq!(sass::WORD_CHARS, "$-");
    assert!(sass::indent_on_input(" \t}"));
    assert!(!sass::indent_on_input(" $}"));
    assert!(!sass::indent_on_input("} "));
    let source = ".é { label: \"東京😀\"; }";
    let doc = SassDocument::parse(Text::new(source), SassDialect::Scss);
    let at = source.find('東').unwrap();
    let span = doc
        .highlights(at..at + '東'.len_utf8())
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(span.range, at..at + '東'.len_utf8());
    assert_eq!(span.tags[0].name(), "string");
    assert!(doc.highlights(at..at).unwrap().next().is_none());
    assert!(doc.highlights(at + 1..source.len()).is_none());
    assert!(doc.foldable(at + 1, source.len()).is_none());
    assert!(
        doc.indentation(at + 1, StreamSettings::default(), IndentOptions::default())
            .is_none()
    );
    assert!(sass_completion_source(&doc, at + 1, true).is_none());
}

#[test]
fn preserves_the_prefix_of_an_incomplete_document() {
    let source = ".a {\n  color: red;\n  width: fn(1px";
    let doc = SassDocument::parse(Text::new(source), SassDialect::Scss);
    assert!(doc.nodes().any(|node| node.is_error()));
    assert!(doc.nodes().any(|node| node.name() == "CallExpression"));
    assert!(doc.nodes().any(
        |node| node.name() == "PropertyName" && doc.text().slice(node.range()) == Some("color")
    ));
    assert!(
        doc.nodes()
            .all(|node| doc.text().slice(node.range()).is_some())
    );
    assert_eq!(doc.foldable(0, 4), Some(4..source.len()));
    let source = "$tone: red\n.a\n  color: ???";
    let doc = SassDocument::parse(Text::new(source), SassDialect::Indented);
    assert!(doc.nodes().any(|node| node.name() == "SassVariableName"));
    assert!(doc.nodes().any(|node| node.is_error()));
}

#[test]
fn dialect_selection_controls_indented_tokens() {
    let text = Text::new("=foo\n  display: block\n.a\n  +foo\n");
    let indented = SassDocument::parse(text.clone(), SassDialect::Indented);
    assert!(indented.nodes().any(|node| node.name() == "IndentedMixin"));
    assert!(
        indented
            .nodes()
            .any(|node| node.name() == "IndentedInclude")
    );
    assert!(!indented.nodes().any(|node| node.is_error()));
    let scss = SassDocument::parse(text, SassDialect::Scss);
    assert!(!scss.nodes().any(|node| node.name() == "IndentedMixin"));
    assert!(scss.nodes().any(|node| node.is_error()));
}

#[test]
fn reports_an_error_when_the_input_limit_is_exceeded() {
    let source = " ".repeat((1 << 20) + 1);
    let doc = SassDocument::parse(Text::new(&source), SassDialect::Scss);
    let mut children = doc.root().children();
    let error = children.next().unwrap();
    assert!(error.is_error());
    assert_eq!(error.range(), 0..source.len());
    assert!(children.next().is_none());
}
