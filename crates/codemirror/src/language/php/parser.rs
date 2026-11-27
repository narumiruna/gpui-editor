use std::{ops::Range, slice, sync::LazyLock};

use super::{
    super::{
        css::input::Input,
        lezer::{GrammarSpec, ParseError, Parser, SerializedTables, Specializer, SyntaxTree},
    },
    tables, tokens,
};

static TEMPLATE: LazyLock<Parser> = LazyLock::new(|| parser(tables::TEMPLATE));
static PROGRAM: LazyLock<Parser> = LazyLock::new(|| parser(tables::PROGRAM));
static NAMES: LazyLock<Vec<&str>> = LazyLock::new(|| tables::NODE_NAMES.split(' ').collect());
static KEYWORDS: LazyLock<Specializer> = LazyLock::new(tokens::keyword_specializer);

fn parser(top_rule: [u16; 2]) -> Parser {
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
            tokenizers: &tokens::tokenizers(),
            top_rule,
            token_prec: tables::TOKEN_PREC,
        },
        slice::from_ref(&*KEYWORDS),
    )
    .and_then(|parser| parser.with_dynamic_precedence(tables::DYNAMIC_PRECEDENCE))
    .expect("valid tables for the pinned PHP grammar")
}

#[derive(Debug)]
pub(super) struct Node {
    pub(super) term: u16,
    pub(super) range: Range<usize>,
    pub(super) parent: Option<usize>,
    pub(super) children: Vec<usize>,
}

impl Node {
    pub(super) fn name(&self) -> &'static str {
        NAMES[usize::from(self.term)]
    }
}

pub(super) fn parse(
    source: &str,
    plain: bool,
    prefix: Option<(&SyntaxTree, usize)>,
) -> (Vec<Node>, bool, Option<SyntaxTree>) {
    let (parser, root) = if plain {
        (&*PROGRAM, tables::PROGRAM[1])
    } else {
        (&*TEMPLATE, tables::TEMPLATE[1])
    };
    let input = Input::new(source, true);
    let fragment = prefix.map(|(tree, end)| input.fragment(tree, end));
    let byte = |at| input.byte(at);
    let input = input.text.as_ref();
    let failure = match parser.parse_incremental(input, fragment) {
        Ok(tree) => return (flatten(&tree, source.len(), byte), false, Some(tree)),
        Err(ParseError::Ambiguous { .. }) => match parser.parse_glr(input) {
            Ok(tree) => return (flatten(&tree, source.len(), byte), false, None),
            Err(error) => error,
        },
        Err(error) => error,
    };
    if let ParseError::UnexpectedToken { offset, .. } = failure
        && input.len() <= 64 * 1024
    {
        // Keep only a prefix that the pinned grammar accepts on its own.
        // Retry on up to four previous line starts; do not invent nodes.
        for (at, _) in input[..offset].match_indices('\n').rev().take(4) {
            let end = at + 1;
            if let Ok(tree) = parse_tree(parser, &input[..end]) {
                let mut nodes = flatten(&tree, source.len(), byte);
                push(&mut nodes, 0, byte(end)..source.len(), Some(0));
                return (nodes, true, None);
            }
        }
    }
    let mut nodes = Vec::new();
    push(&mut nodes, root, 0..source.len(), None);
    if !source.is_empty() {
        push(&mut nodes, 0, 0..source.len(), Some(0));
    }
    (nodes, !source.is_empty(), None)
}

fn flatten(tree: &SyntaxTree, source_end: usize, byte: impl Fn(usize) -> usize) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut stack = vec![(tree.root(), None)];
    while let Some((node, parent)) = stack.pop() {
        let range = node.range();
        let id = push(
            &mut nodes,
            node.term(),
            byte(range.start)..byte(range.end),
            parent,
        );
        stack.extend(node.children().rev().map(|child| (child, Some(id))));
    }
    nodes[0].range = 0..source_end;
    nodes
}

fn parse_tree(parser: &Parser, text: &str) -> Result<SyntaxTree, ParseError> {
    parser.parse(text).or_else(|error| match error {
        ParseError::Ambiguous { .. } => parser.parse_glr(text),
        _ => Err(error),
    })
}

fn push(nodes: &mut Vec<Node>, term: u16, range: Range<usize>, parent: Option<usize>) -> usize {
    let id = nodes.len();
    if let Some(parent) = parent {
        nodes[parent].children.push(id);
    }
    nodes.push(Node {
        term,
        range,
        parent,
        children: Vec::new(),
    });
    id
}
