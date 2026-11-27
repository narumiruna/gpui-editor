use gpui_editor_codemirror::{
    Text,
    language::markdown::{MarkdownDialect, MarkdownDocument},
};

#[test]
fn pinned_markdown_trees() {
    // @codemirror/lang-markdown 6.5.2 and @lezer/markdown 1.8.2.
    // Tree offsets from the UTF-16/LF input are mapped to UTF-8/CRLF.
    let cases: Vec<(String, String, String, String)> =
        serde_json::from_str(include_str!("fixtures/markdown_syntax.json")).unwrap();
    for (name, dialect, source, expected) in cases {
        let dialect = match dialect.as_str() {
            "extended" => MarkdownDialect::Extended,
            "commonmark" => MarkdownDialect::CommonMark,
            _ => panic!("unknown Markdown dialect: {dialect}"),
        };
        let doc = MarkdownDocument::parse_with_dialect(Text::new(source), dialect);
        let nodes = doc
            .nodes()
            .map(|node| {
                let range = node.range();
                format!("{}:{}..{}", node.kind().name(), range.start, range.end)
            })
            .collect::<Vec<_>>();
        assert_eq!(nodes.join(" "), expected, "{name}");
    }
}

#[test]
fn keeps_delimiter_depth_bounded_on_large_runs() {
    use gpui_editor_codemirror::language::markdown::MarkdownNodeKind;

    let run = "*".repeat(16_000);
    let source = format!("text {run}café{run} end");
    let doc = MarkdownDocument::parse(Text::new(source));
    assert!(
        doc.nodes()
            .any(|node| node.kind() == MarkdownNodeKind::StrongEmphasis)
    );
    assert!(doc.nodes().count() < 1_000);
    assert!(
        doc.nodes()
            .all(|node| doc.text().slice(node.range()).is_some())
    );
}
