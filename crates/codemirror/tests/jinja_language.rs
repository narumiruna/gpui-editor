// Tree, tag, indent, and fold expectations were checked with the published
// @codemirror/lang-jinja 6.0.1 and pinned dependencies in a scratch directory.

use gpui_editor_codemirror::{
    Text,
    commands::CommentTokens,
    language::{
        IndentOptions, StreamSettings, TagModifier,
        html::{HtmlEmbeddedLanguage, HtmlNodeKind},
        jinja::{JinjaDocument, JinjaNode, JinjaOptions, indent_on_input, language_data},
    },
};

fn children(node: JinjaNode<'_>) -> Vec<&'static str> {
    node.children().map(JinjaNode::name).collect()
}

fn styles(doc: &JinjaDocument) -> Vec<(&str, String)> {
    doc.highlights(0..doc.text().len())
        .unwrap()
        .map(|span| {
            let style = span
                .tags
                .iter()
                .map(|tag| {
                    let modifier = match tag.modifiers() {
                        [TagModifier::Definition] => ".definition",
                        [TagModifier::Special] => ".special",
                        [TagModifier::Standard] => ".standard",
                        [] => "",
                        mods => panic!("unexpected modifiers: {mods:?}"),
                    };
                    format!("{}{modifier}", tag.name())
                })
                .collect::<Vec<_>>()
                .join(" ");
            (doc.text().slice(span.range).unwrap(), style)
        })
        .collect()
}

#[test]
fn parses_recursive_for_with_filter_and_whitespace_trim() {
    let source =
        "{%- for person in people recursive -%}{{ person.name|escape }}{% else %}Empty{% endfor %}";
    let doc = JinjaDocument::parse(Text::new(source));
    assert!(!doc.is_recovered());
    assert_eq!(doc.root().range(), 0..source.len());
    assert_eq!(doc.root().name(), "Template");
    let statement = doc.root().children().next().unwrap();
    assert_eq!(statement.name(), "ForStatement");
    assert_eq!(
        children(statement),
        ["Tag", "Interpolation", "Tag", "Text", "EndTag"]
    );
    let tag = statement.children().next().unwrap();
    assert_eq!(
        children(tag),
        [
            "{%",
            "for",
            "Definition",
            "in",
            "VariableName",
            "recursive",
            "%}"
        ]
    );
    let interpolation = statement.children().nth(1).unwrap();
    assert_eq!(interpolation.parent().unwrap().name(), "ForStatement");
    assert_eq!(children(interpolation), ["{{", "FilterExpression", "}}"]);
    let filter = interpolation.children().nth(1).unwrap();
    assert_eq!(
        children(filter),
        ["MemberExpression", "FilterOp", "FilterName"]
    );
    assert_eq!(
        statement.fold_range(),
        Some(source.find("{{").unwrap()..source.rfind("{%").unwrap())
    );
    assert_eq!(
        styles(&doc),
        [
            ("{%-", "brace"),
            ("for", "controlKeyword"),
            ("person", "variableName.definition"),
            ("in", "operatorKeyword"),
            ("people", "variableName"),
            ("recursive", "modifier"),
            ("-%}", "brace"),
            ("{{", "brace"),
            ("person", "variableName"),
            (".", "derefOperator"),
            ("name", "propertyName"),
            ("|", "operator"),
            ("escape", "variableName.special"),
            ("}}", "brace"),
            ("{%", "brace"),
            ("else", "controlKeyword"),
            ("%}", "brace"),
            ("Empty", "content"),
            ("{%", "brace"),
            ("endfor", "controlKeyword"),
            ("%}", "brace"),
        ]
        .map(|(text, tag)| (text, tag.to_owned()))
    );
}

#[test]
fn keeps_expression_precedence_and_anonymous_delimiters() {
    let doc = JinjaDocument::parse(Text::new(
        "{{ x.y is not false and self is loop or super }}",
    ));
    assert!(!doc.is_recovered());
    assert!(doc.html().is_none());
    let or = doc
        .root()
        .children()
        .next()
        .unwrap()
        .children()
        .nth(1)
        .unwrap();
    assert_eq!(children(or), ["BinaryExpression", "or", "super"]);
    let and = or.children().next().unwrap();
    assert_eq!(
        children(and),
        ["BinaryExpression", "and", "BinaryExpression"]
    );
    let is = and.children().next().unwrap();
    assert_eq!(children(is), ["MemberExpression", "is", "NotExpression"]);
    let styles = styles(&doc);
    for (text, tag) in [
        ("false", "bool"),
        ("is", "operatorKeyword"),
        ("and", "logicOperator"),
        ("self", "self"),
        ("loop", "variableName.standard"),
    ] {
        assert!(
            styles.contains(&(text, tag.to_owned())),
            "missing {text}: {tag}"
        );
    }

    let doc = JinjaDocument::parse(Text::new(r#"{{{ "a": 1, "b": [2, 3] }}}"#));
    assert!(!doc.is_recovered());
    let dict = doc
        .nodes()
        .find(|node| node.name() == "DictExpression")
        .unwrap();
    assert_eq!(children(dict), ["Entry", ",", "Entry"]);
    assert_eq!(
        children(dict.children().next_back().unwrap()),
        ["StringLiteral", ":", "ArrayExpression"]
    );
    assert_eq!(
        doc.text().slice(dict.range()),
        Some(r#"{ "a": 1, "b": [2, 3] }"#)
    );
}

#[test]
fn maps_html_ranges_across_gaps_and_keeps_raw_text_literal() {
    let source =
        "🙂<a title=\"pre {{ v }} post\">é</a>\r\n{% raw %}{{ literal }}<em>hi</em>{% endraw %}";
    let doc = JinjaDocument::parse(Text::new(source));
    assert!(!doc.is_recovered());
    assert_eq!(
        doc.nodes()
            .filter(|node| node.name() == "Interpolation")
            .count(),
        1
    );
    let raw = doc.nodes().find(|node| node.name() == "RawText").unwrap();
    assert_eq!(
        doc.text().slice(raw.range()),
        Some("{{ literal }}<em>hi</em>")
    );
    let html = doc.html().unwrap();
    assert_eq!(
        html.document().text().as_str(),
        "🙂<a title=\"pre  post\">é</a>\r\n{{ literal }}<em>hi</em>"
    );
    let value = html
        .document()
        .nodes()
        .find(|node| node.kind() == HtmlNodeKind::AttributeValue)
        .unwrap();
    let parts = html.source_ranges(value.range()).unwrap();
    assert_eq!(
        parts
            .iter()
            .map(|part| &source[part.clone()])
            .collect::<Vec<_>>(),
        ["\"pre ", " post\""]
    );
    let pos = source.find("{{ v").unwrap();
    assert_eq!(html.projected_offset(pos), None);
    assert_eq!(html.projected_offset(1), None);
    assert_eq!(html.source_ranges(1..4), None);
    assert!(doc.highlights(1..4).is_none());
    assert!(
        doc.highlights(std::ops::Range {
            start: pos + 1,
            end: pos
        })
        .is_none()
    );
    assert_eq!(doc.highlights(pos..pos).unwrap().count(), 0);
    let clipped = doc
        .highlights(pos - 2..pos + 1)
        .unwrap()
        .map(|span| (&source[span.range], span.tags[0].name()))
        .collect::<Vec<_>>();
    assert_eq!(clipped, [("e ", "attributeValue"), ("{", "brace")]);
    assert!(styles(&doc).contains(&("\r\n", "content".into())));

    let plain = JinjaDocument::parse_with_options(Text::new(source), JinjaOptions { html: None });
    assert!(plain.html().is_none());
    assert_eq!(plain.nodes().count(), doc.nodes().count());
    assert!(
        !styles(&plain)
            .iter()
            .any(|(_, style)| matches!(style.as_str(), "content" | "tagName"))
    );
}

#[test]
fn native_html_keeps_its_script_tree_across_a_template_gap() {
    let source = "<script>\nconst suffix = \"hello {{ who }}!\";\n</script>";
    let doc = JinjaDocument::parse(Text::new(source));
    let mounts = doc.html().unwrap().document().mounts();
    assert_eq!(mounts.len(), 1);
    assert_eq!(mounts[0].language(), HtmlEmbeddedLanguage::JavaScript);
    let tags = styles(&doc);
    assert!(tags.contains(&("\"hello ", "string".into())));
    assert!(tags.contains(&("who", "variableName".into())));
    assert!(tags.contains(&("!\"", "string".into())));
}

#[test]
fn applies_mixed_indent_and_fold_rules() {
    let source = "{% if x %}\n  <b>\n   {{ 1 }}\n  </b>\n{% elif y %}\n{% else %}\n{% endif %}";
    let doc = JinjaDocument::parse(Text::new(source));
    let settings = StreamSettings::default();
    let query = |line| {
        let l = doc.text().line(line).unwrap();
        (
            doc.indentation(l.range.start, settings, IndentOptions::default()),
            doc.foldable(l.range.start, l.range.end),
        )
    };
    assert_eq!(
        (1..=7).map(query).collect::<Vec<_>>(),
        [
            (Some(0), Some(10..59)),
            (Some(2), Some(16..30)),
            (Some(4), None),
            (Some(2), None),
            (Some(0), None),
            (Some(0), None),
            (None, None),
        ]
    );

    let doc = JinjaDocument::parse(Text::new(
        "{% set a = fn(\n    1,\n    2\n) %}\n{% with\n    a=1\n%}\ntext\n{% endwith %}",
    ));
    let indent = |line| {
        doc.indentation(
            doc.text().line(line).unwrap().range.start,
            settings,
            IndentOptions::default(),
        )
    };
    assert_eq!(
        (1..=9).map(indent).collect::<Vec<_>>(),
        [
            Some(0),
            Some(2),
            Some(2),
            Some(0),
            Some(0),
            Some(3),
            Some(0),
            Some(2),
            None
        ]
    );
}

#[test]
fn comment_fold_and_highlights_keep_utf8_boundaries() {
    let source = "  {# start\n  café + 🐈\n  #}\n";
    let doc = JinjaDocument::parse(Text::new(source));
    let head = doc.text().line(1).unwrap().range;
    assert_eq!(
        doc.foldable(head.start, head.end),
        Some(4..source.find("#}").unwrap())
    );
    assert_eq!(
        styles(&doc),
        [
            ("  ", "content"),
            ("{#", "blockComment brace"),
            (" start\n  café + 🐈\n  ", "blockComment"),
            ("#}", "blockComment brace"),
            ("\n", "content")
        ]
        .map(|(text, tag)| (text, tag.to_owned()))
    );
    let invalid = source.find('🐈').unwrap() + 1;
    assert!(
        doc.indentation(invalid, StreamSettings::default(), IndentOptions::default())
            .is_none()
    );
    assert!(doc.foldable(0, invalid).is_none());
    assert!(doc.language_data_at(invalid).is_none());
}

#[test]
fn recovers_unfinished_nodes_and_retains_later_valid_syntax() {
    for (source, expected) in [
        ("a {{ name", "Interpolation"),
        ("a {# unfinished é", "Comment"),
        ("{% raw %}\n{{ name", "RawStatement"),
        ("{% block canvas %}\n  {% if x %}\nhello", "IfStatement"),
        ("  {% if x %}\r\n<main>\r\n{{ name", "Interpolation"),
    ] {
        let doc = JinjaDocument::parse(Text::new(source));
        assert!(doc.is_recovered(), "{source}");
        assert!(doc.nodes().any(|n| n.name() == expected), "{source}");
        assert!(
            doc.nodes()
                .any(|n| n.name() == "⚠" && n.range() == (source.len()..source.len())),
            "{source}"
        );
        for node in doc.nodes() {
            assert!(doc.text().slice(node.range()).is_some());
        }
    }

    let source = "é {{ x + }} b {{ ok }}";
    let doc = JinjaDocument::parse(Text::new(source));
    assert!(doc.is_recovered());
    assert_eq!(
        children(doc.root()),
        ["Text", "Interpolation", "Text", "Interpolation"]
    );
    let error = doc.nodes().find(|node| node.name() == "⚠").unwrap();
    let close = source.find("}}").unwrap();
    assert_eq!(error.range(), close..close);
    assert_eq!(error.parent().unwrap().name(), "BinaryExpression");
    assert!(styles(&doc).contains(&("ok", "variableName".into())));

    let invalid = format!("{{{{+ {}", "🙂".repeat(17_000));
    let large = JinjaDocument::parse(Text::new(&invalid));
    assert!(large.is_recovered());
    assert_eq!(children(large.root()), ["⚠"]);
    assert_eq!(
        large.root().children().next().unwrap().range(),
        0..invalid.len()
    );
}

#[test]
fn reports_template_and_outer_html_metadata() {
    let trigger = *language_data()
        .get::<fn(&str) -> bool>("indentOnInput")
        .unwrap();
    for prefix in ["{% end", "\t{%-  elif", " {%else"] {
        assert!(trigger(prefix));
    }
    for prefix in ["{%", "{% endif", "{% -else", "{% else ", "{{ end"] {
        assert!(!indent_on_input(prefix));
    }
    let doc = JinjaDocument::parse(Text::new("<p>{{ who }}</p>"));
    let html = doc.language_data_at(0).unwrap();
    assert_eq!(html.get::<[char; 1]>("closeBrackets"), Some(&['{']));
    assert_eq!(
        html.get::<CommentTokens>("commentTokens").unwrap().block,
        Some(("<!--", "-->"))
    );
    assert!(
        doc.language_data_at(5)
            .unwrap()
            .get::<CommentTokens>("commentTokens")
            .is_none()
    );
}
