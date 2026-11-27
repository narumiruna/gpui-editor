//! Run the published HTML grammar with native element and token contexts.

use std::sync::LazyLock;

use super::{
    super::lezer::{
        ExternalOptions, ExternalToken, ExternalTokenizer, ParseError, Parser, StackView,
        SyntaxTree, TokenInput, Tokenizer, TreeFragment,
    },
    HtmlNodeKind, HtmlOptions, Node, tables,
    tag_context::{self, Token},
};

pub(super) fn parse(
    source: &str,
    options: HtmlOptions,
    fragment: Option<TreeFragment<'_>>,
) -> Option<(Vec<Node>, Option<Box<SyntaxTree>>)> {
    static PARSERS: [LazyLock<Parser>; 4] = [
        LazyLock::new(|| parser(false, false)),
        LazyLock::new(|| parser(false, true)),
        LazyLock::new(|| parser(true, false)),
        LazyLock::new(|| parser(true, true)),
    ];
    let parser = &PARSERS
        [usize::from(options.self_closing_tags) * 2 + usize::from(options.match_closing_tags)];
    match parser.parse_incremental(source, fragment) {
        Ok(tree) => Some((collect(&tree, source.len()), Some(Box::new(tree)))),
        Err(ParseError::Ambiguous { .. }) => {
            let tree = parser.parse_glr(source).ok()?;
            Some((collect(&tree, source.len()), None))
        }
        Err(_) => None,
    }
}

fn collect(tree: &SyntaxTree, len: usize) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut stack = vec![(tree.root(), None::<usize>)];
    while let Some((syntax, parent)) = stack.pop() {
        let id = nodes.len();
        if let Some(parent) = parent {
            nodes[parent].children.push(id);
        }
        nodes.push(Node {
            kind: HtmlNodeKind::from_name(syntax.name()),
            range: syntax.range(),
            parent,
            children: Vec::new(),
        });
        stack.extend(syntax.children().rev().map(|child| (child, Some(id))));
    }
    // Skipped input cannot truncate the root range.
    nodes[0].range.end = len;
    nodes
}

fn parser(self_closing: bool, match_closing: bool) -> Parser {
    tag_context::install(tables::parser(tokenizers(self_closing, match_closing)))
}

fn tokenizers(self_closing: bool, match_closing: bool) -> [Tokenizer; 12] {
    fn external(
        contextual: bool,
        scan: impl Fn(&mut TokenInput<'_>, &StackView<'_>) -> Token + Send + Sync + 'static,
    ) -> Tokenizer {
        // The scans read metered input and immutable tag contexts only.
        Tokenizer::External(ExternalTokenizer::new_deterministic(
            ExternalOptions {
                contextual,
                ..ExternalOptions::default()
            },
            scan,
        ))
    }
    [
        external(false, |input, _| raw(input, b"script", 55, 1)),
        external(false, |input, _| raw(input, b"style", 56, 2)),
        external(false, |input, _| raw(input, b"textarea", 57, 3)),
        external(false, move |input, stack| {
            tag_context::end(input, stack, self_closing)
        }),
        external(true, move |input, stack| {
            tag_context::start(input, stack, match_closing)
        }),
        external(false, |input, _| comment(input)),
        0.into(),
        1.into(),
        2.into(),
        3.into(),
        4.into(),
        5.into(),
    ]
}

fn comment(input: &mut TokenInput<'_>) -> Token {
    let start = input.position();
    let mut dashes = 0;
    while let Some(ch) = input.peek(0)? {
        if ch == 45 {
            dashes += 1;
        } else if ch == 62 && dashes >= 2 {
            let end = input.position() - 2;
            return Ok((end > start).then_some(ExternalToken { term: 59, end }));
        } else {
            dashes = 0;
        }
        input.advance(1)?;
    }
    Ok((input.position() > start).then(|| input.token(59)))
}

fn raw(input: &mut TokenInput<'_>, name: &[u8], text: u16, close: u16) -> Token {
    let start = input.position();
    let mut matched = 0;
    let end_state = name.len() + 2;
    while let Some(ch) = input.peek(0)? {
        let expected = match matched {
            0 => b'<',
            1 => b'/',
            end if end == end_state => b'>',
            n => name[n - 2],
        };
        if ch == u16::from(expected) {
            if matched == end_state {
                let end = input.position() - matched;
                return Ok(Some(if end == start {
                    ExternalToken {
                        term: close,
                        end: start + 2,
                    }
                } else {
                    ExternalToken { term: text, end }
                }));
            }
            matched += 1;
        } else if matches!(ch, 10 | 13) && input.position() > start {
            input.advance(1)?;
            return Ok(Some(input.token(text)));
        } else {
            matched = 0;
        }
        input.advance(1)?;
    }
    Ok((input.position() > start).then(|| input.token(text)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use HtmlNodeKind::*;

    fn parse(source: &str, options: HtmlOptions) -> Option<Vec<Node>> {
        super::parse(source, options, None).map(|(nodes, _)| nodes)
    }

    #[test]
    fn uses_published_tables_for_matching_tags_comments_and_raw_bodies() {
        let source = "<!doctype html><main><!--😀---><br/><script>x < 0\r\n&n;</script><style>p{}</style><textarea></textarea></main>";
        let nodes = parse(source, HtmlOptions::default()).expect("strict table parse");
        assert!(
            nodes
                .iter()
                .any(|node| node.kind == Comment && &source[node.range.clone()] == "<!--😀--->")
        );
        assert!(
            nodes.iter().any(
                |node| node.kind == ScriptText && &source[node.range.clone()] == "x < 0\r\n&n;"
            )
        );
        assert!(
            nodes
                .iter()
                .any(|node| node.kind == TextareaText && node.range.is_empty())
        );
        assert!(
            parse(
                "<custom/>",
                HtmlOptions {
                    self_closing_tags: true,
                    ..HtmlOptions::default()
                }
            )
            .is_some()
        );
    }

    fn shape(nodes: &[Node], id: usize) -> String {
        let node = &nodes[id];
        if node.children.is_empty() {
            return node.kind.name().into();
        }
        let children = node
            .children
            .iter()
            .map(|&child| shape(nodes, child))
            .collect::<Vec<_>>()
            .join(",");
        format!("{}({children})", node.kind.name())
    }

    #[test]
    fn strict_element_context_matches_published_dialect_trees() {
        let defaults = HtmlOptions::default();
        let no_match = HtmlOptions {
            match_closing_tags: false,
            ..defaults
        };
        let self_closing = HtmlOptions {
            self_closing_tags: true,
            ..defaults
        };
        for (source, options, expected) in [
            (
                "<a></b>",
                defaults,
                "Document(Element(OpenTag(StartTag,TagName,EndTag),MismatchedCloseTag(StartCloseTag,TagName,EndTag)))",
            ),
            (
                "<ul><li>a<li>b</ul>",
                defaults,
                "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),Text),Element(OpenTag(StartTag,TagName,EndTag),Text),CloseTag(StartCloseTag,TagName,EndTag)))",
            ),
            (
                "<svg><path/></svg>",
                defaults,
                "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(SelfClosingTag(StartTag,TagName,SelfClosingEndTag)),CloseTag(StartCloseTag,TagName,EndTag)))",
            ),
            (
                "<div></i><p>bar</p></div>",
                no_match,
                "Document(Element(OpenTag(StartTag,TagName,EndTag),CloseTag(StartCloseTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),Text,CloseTag(StartCloseTag,TagName,EndTag)),CloseTag(StartCloseTag,TagName,EndTag)))",
            ),
            (
                "<div></i><",
                no_match,
                "Document(Element(OpenTag(StartTag,TagName,EndTag),CloseTag(StartCloseTag,TagName,EndTag),IncompleteTag))",
            ),
            (
                "<main><p>text<div/></main>",
                self_closing,
                "Document(Element(OpenTag(StartTag,TagName,EndTag),Element(OpenTag(StartTag,TagName,EndTag),Text),Element(SelfClosingTag(StartTag,TagName,SelfClosingEndTag)),CloseTag(StartCloseTag,TagName,EndTag)))",
            ),
        ] {
            let nodes = parse(source, options)
                .unwrap_or_else(|| panic!("{source:?}: expected a strict parse"));
            assert_eq!(shape(&nodes, 0), expected, "{source:?}");
        }
        assert!(parse("<section><span></section>", defaults).is_none());
    }
}
