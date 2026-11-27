use std::sync::LazyLock;

use super::{
    super::lezer::{
        GrammarSpec, ParseError, Parser, SerializedTables, Specializer, SyntaxTree, Tokenizer,
        TreeFragment,
    },
    Node, tables,
};

static PROGRAM: LazyLock<Parser> = LazyLock::new(|| build(tables::PROGRAM));
static CLASS_CONTENT: LazyLock<Parser> = LazyLock::new(|| build(tables::CLASS_CONTENT));
static NAMES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| tables::NODE_NAMES.split(' ').collect());

/// The strict parser for the upstream `Program` top rule. Use
/// [`Parser::parse_glr`] for input with ambiguous LR actions.
pub fn parser() -> &'static Parser {
    &PROGRAM
}

/// The parser for the upstream `ClassContent` top rule. Use
/// [`Parser::parse_glr`] for ambiguity or [`Parser::parse_recovering`] for
/// malformed input. Checkpoints from this root cannot seed `Program` parses.
pub fn class_content_parser() -> &'static Parser {
    &CLASS_CONTENT
}

pub(super) fn name(term: u16) -> &'static str {
    NAMES[usize::from(term)]
}

fn build(top_rule: [u16; 2]) -> Parser {
    Parser::deserialize_with_hooks(
        GrammarSpec {
            tables: SerializedTables {
                version: 14,
                states: tables::STATES,
                state_data: tables::STATE_DATA,
                goto: tables::GOTO,
                token_data: tables::TOKEN_DATA,
            },
            node_names: tables::NODE_NAMES,
            max_term: tables::MAX_TERM,
            repeat_node_count: tables::REPEAT_NODE_COUNT,
            tokenizers: &[0, 1, 2, 3].map(Tokenizer::Internal),
            top_rule,
            token_prec: tables::TOKEN_PREC,
        },
        &[Specializer::context_free(
            tables::IDENTIFIER,
            tables::KEYWORDS,
        )],
    )
    .and_then(|parser| parser.with_dynamic_precedence(tables::DYNAMIC_PRECEDENCE))
    .expect("valid tables for the pinned Java grammar")
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Mode {
    Program,
    ClassContent,
}

impl Mode {
    fn parser(self) -> &'static Parser {
        match self {
            Self::Program => parser(),
            Self::ClassContent => class_content_parser(),
        }
    }

    fn root(self) -> u16 {
        match self {
            Self::Program => tables::PROGRAM[1],
            Self::ClassContent => tables::CLASS_CONTENT[1],
        }
    }
}

fn parse(parser: &Parser, source: &str) -> Result<SyntaxTree, ParseError> {
    parser
        .parse(source)
        .or_else(|error| try_glr(parser, source, error))
}

fn try_glr(parser: &Parser, source: &str, error: ParseError) -> Result<SyntaxTree, ParseError> {
    match error {
        ParseError::Ambiguous { .. } => parser.parse_glr(source),
        _ => Err(error),
    }
}

pub(super) fn document_nodes(
    mode: Mode,
    source: &str,
    prefix: Option<(&SyntaxTree, usize)>,
) -> (Vec<Node>, Option<ParseError>, Option<SyntaxTree>) {
    let fragment = prefix.map(|(tree, end)| TreeFragment {
        tree,
        old_range: 0..end,
        new_range: 0..end,
    });
    let parser = mode.parser();
    let error = match parser.parse_incremental(source, fragment.clone()) {
        Ok(tree) => return (collect(&tree, None), None, Some(tree)),
        Err(error) => match try_glr(parser, source, error) {
            Ok(tree) => return (collect(&tree, None), None, None),
            Err(error) => error,
        },
    };
    let mut nodes = vec![Node {
        term: mode.root(),
        range: 0..source.len(),
        parent: None,
        children: Vec::new(),
    }];
    // Keep recovery within the existing size cap. Do not retry work-limit
    // errors. Repeat the strict pass to keep only its validated checkpoints.
    if source.len() <= 16 * 1024
        && let ParseError::UnexpectedToken { offset, .. } = error
    {
        if let Ok(tree) = parser.parse_incremental_recovering(source, fragment.as_slice()) {
            let nodes = collect(&tree, None);
            return (
                nodes,
                Some(error),
                tree.has_incremental_state().then_some(tree),
            );
        }
        // If bounded recovery hits a limit, try only end punctuation or a
        // strict prefix. These fallback trees cannot seed the next parse.
        if offset == source.len()
            && let Some(tree) = close_at_end(parser, source)
        {
            return (collect(&tree, Some(offset)), Some(error), None);
        }
        if offset < source.len()
            && let Some(prefix) = source.get(..offset)
            && let Ok(tree) = parse(parser, prefix)
        {
            nodes = collect(&tree, None);
            nodes[0].range.end = source.len();
            add_error(&mut nodes, 0, offset..source.len());
            return (nodes, Some(error), None);
        }
    }
    add_error(&mut nodes, 0, 0..source.len());
    (nodes, Some(error), None)
}

fn close_at_end(parser: &Parser, source: &str) -> Option<SyntaxTree> {
    let mut probe = String::with_capacity(source.len() + 9);
    probe.push_str(source);
    // A closing token must not become part of a trailing line comment.
    probe.push('\n');
    for _ in 0..8 {
        let mut advanced = false;
        for token in ['}', ')', ']', ';'] {
            probe.push(token);
            match parse(parser, &probe) {
                Ok(tree) => return Some(tree),
                Err(ParseError::UnexpectedToken { offset, .. }) if offset == probe.len() => {
                    advanced = true;
                    break;
                }
                Err(_) => {
                    probe.pop();
                }
            }
        }
        if !advanced {
            break;
        }
    }
    None
}

fn add_error(nodes: &mut Vec<Node>, parent: usize, range: std::ops::Range<usize>) {
    let id = nodes.len();
    nodes[parent].children.push(id);
    nodes.push(Node {
        term: 0,
        range,
        parent: Some(parent),
        children: Vec::new(),
    });
}

pub(super) fn collect(tree: &SyntaxTree, cutoff: Option<usize>) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut pending = vec![(Some(tree.root()), None)];
    while let Some((syntax, parent)) = pending.pop() {
        let Some(syntax) = syntax else {
            let end = cutoff.expect("clipped trailing token");
            add_error(&mut nodes, parent.expect("error parent"), end..end);
            continue;
        };
        let id = nodes.len();
        if let Some(parent) = parent {
            nodes[parent].children.push(id);
        }
        let range = syntax.range();
        if let Some(end) = cutoff
            && range.end > end
            && syntax
                .children()
                .next_back()
                .is_some_and(|last| last.range().start >= end)
        {
            pending.push((None, Some(id)));
        }
        pending.extend(syntax.children().rev().filter_map(|child| {
            cutoff
                .is_none_or(|end| child.range().start < end)
                .then_some((Some(child), Some(id)))
        }));
        nodes.push(Node {
            term: syntax.term(),
            range: range.start..cutoff.map_or(range.end, |end| range.end.min(end)),
            parent,
            children: Vec::new(),
        });
    }
    nodes
}
