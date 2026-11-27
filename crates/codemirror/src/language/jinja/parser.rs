use std::{ops::Range, sync::LazyLock};

use super::{
    super::{
        css::input::Input,
        lezer::{GrammarSpec, ParseError, Parser, SerializedTables, Specializer, SyntaxTree},
    },
    tables, tokens,
};

static PARSER: LazyLock<Parser> = LazyLock::new(|| {
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
            max_term: 173,
            repeat_node_count: 13,
            tokenizers: &tokens::tokenizers(),
            top_rule: [0, 5],
            token_prec: 3602,
        },
        &[
            Specializer::context_free(161, tables::IDENTIFIERS),
            Specializer::context_free(55, tables::TAG_NAMES),
        ],
    )
    .expect("valid bundled Jinja parser tables")
});

static NAMES: LazyLock<Vec<&str>> = LazyLock::new(|| tables::NODE_NAMES.split(' ').collect());

#[derive(Debug)]
pub(super) struct Node {
    term: u16,
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
    prefix: Option<(&SyntaxTree, usize)>,
) -> (Vec<Node>, bool, Option<SyntaxTree>) {
    // CodeMirror sends LF to Lezer. Translate normalized byte offsets back.
    let input = Input::new(source, true);
    let fragment = prefix.map(|(tree, end)| input.fragment(tree, end));
    let byte = |at| input.byte(at);
    let input = input.text.as_ref();
    let failure = match PARSER.parse_incremental(input, fragment) {
        Ok(tree) => return (collect(&tree, 0..0, source.len(), byte), false, Some(tree)),
        Err(ParseError::Ambiguous { .. }) => match PARSER.parse_glr(input) {
            Ok(tree) => return (collect(&tree, 0..0, source.len(), byte), false, None),
            Err(error) => error,
        },
        Err(error) => error,
    };
    if let ParseError::UnexpectedToken { offset, .. } = failure
        && input.len() <= 64 * 1024
    {
        // Only try a fixed number of strict parses. Do not retry a parse
        // that stopped at the runtime's work limit.
        let rest = &input[offset..];
        let try_insert = |at, insert: &str| {
            let mut repaired = input.to_string();
            repaired.insert_str(at, insert);
            let tree = parse_tree(&repaired).ok()?;
            Some(collect(&tree, at..at + insert.len(), source.len(), byte))
        };
        let repaired = if rest.starts_with("}}") || rest.starts_with("%}") {
            try_insert(offset, "x ")
        } else if rest.trim().is_empty() {
            let blocks = missing_blocks(input);
            let closer = missing_delimiter(input);
            let at = input.len();
            let attempt = |pending: &str| {
                (!pending.is_empty() || !blocks.is_empty())
                    .then(|| try_insert(at, &format!("{pending}{blocks}")))
                    .flatten()
            };
            attempt(closer)
                .or_else(|| {
                    if closer.is_empty() {
                        None
                    } else {
                        attempt(&format!(" x {closer}"))
                    }
                })
                // An apparent delimiter inside raw text can be ignored by the grammar.
                .or_else(|| if closer.is_empty() { None } else { attempt("") })
        } else {
            None
        };
        if let Some(nodes) = repaired {
            return (nodes, true, None);
        }
        let through = &input[..offset];
        let marker = |s: &str| matches!(s.as_bytes().get(1), Some(b'{' | b'%' | b'#'));
        let cuts = through
            .rmatch_indices('{')
            .filter(|&(at, _)| marker(&input[at..]))
            .take(3);
        for (end, _) in cuts {
            if let Ok(tree) = parse_tree(&input[..end]) {
                return (collect(&tree, 0..0, source.len(), byte), true, None);
            }
        }
    }
    let mut nodes = Vec::new();
    push(&mut nodes, 5, 0..source.len(), None);
    push(&mut nodes, 0, 0..source.len(), Some(0));
    (nodes, true, None)
}

fn parse_tree(input: &str) -> Result<SyntaxTree, ParseError> {
    PARSER.parse(input).or_else(|error| match error {
        ParseError::Ambiguous { .. } => PARSER.parse_glr(input),
        _ => Err(error),
    })
}

fn missing_delimiter(input: &str) -> &'static str {
    let Some((at, _)) = input
        .rmatch_indices('{')
        .find(|&(at, _)| matches!(input.as_bytes().get(at + 1), Some(b'{' | b'%' | b'#')))
    else {
        return "";
    };
    let tail = &input[at + 2..];
    match input.as_bytes()[at + 1] {
        b'{' if !tail.contains("}}") && tail.trim_end().ends_with('(') => ")}}",
        b'{' if !tail.contains("}}") => "}}",
        b'#' if !tail.contains("#}") => "#}",
        b'%' if !tail.contains("%}") => "%}",
        _ => "",
    }
}

fn missing_blocks(input: &str) -> String {
    const BLOCKS: &[&str] = &[
        "if",
        "for",
        "raw",
        "block",
        "macro",
        "call",
        "filter",
        "set",
        "trans",
        "with",
        "autoescape",
    ];
    let mut closed = [0; BLOCKS.len()];
    let mut suffix = String::new();
    let mut n = 0;
    // This reverse scan only proposes closing tags. A strict parse must
    // accept the result before any part of it can enter the source tree.
    for (at, _) in input.rmatch_indices("{%").take(16) {
        let tag = input[at + 2..].trim_start_matches('-').trim_start();
        let word = &tag[..tag
            .find(|ch: char| !ch.is_ascii_alphabetic())
            .unwrap_or(tag.len())];
        let (close, name) = word
            .strip_prefix("end")
            .map_or((false, word), |name| (true, name));
        let Some(index) = BLOCKS.iter().position(|&kind| kind == name) else {
            continue;
        };
        if close {
            closed[index] += 1;
        } else if name != "set"
            || !tag
                .split_once("%}")
                .map_or(tag, |(tag, _)| tag)
                .contains('=')
        {
            if closed[index] > 0 {
                closed[index] -= 1;
            } else {
                suffix.push_str("{% end");
                suffix.push_str(name);
                suffix.push_str(" %}");
                n += 1;
                if n == 3 {
                    break;
                }
            }
        }
    }
    suffix
}

fn collect(
    tree: &SyntaxTree,
    inserted: Range<usize>,
    source_end: usize,
    byte: impl Fn(usize) -> usize,
) -> Vec<Node> {
    let offset = |pos: usize| byte(pos - inserted.len().min(pos.saturating_sub(inserted.start)));
    let parsed_end = offset(tree.root().range().end);
    let mut nodes: Vec<Node> = Vec::new();
    let mut stack = vec![(tree.root(), None::<usize>)];
    while let Some((node, parent)) = stack.pop() {
        let at = node.range();
        let missing = !at.is_empty() && at.start >= inserted.start && at.end <= inserted.end;
        let range = offset(at.start)..offset(at.end);
        if missing
            && parent
                .and_then(|id| nodes.get(id))
                .and_then(|node| node.children.last())
                .is_some_and(|&id| nodes[id].term == 0 && nodes[id].range == range)
        {
            continue;
        }
        let id = push(
            &mut nodes,
            if missing { 0 } else { node.term() },
            range,
            parent,
        );
        if !missing {
            stack.extend(node.children().rev().map(|child| (child, Some(id))));
        }
    }
    nodes[0].range = 0..source_end;
    if parsed_end < source_end {
        push(&mut nodes, 0, parsed_end..source_end, Some(0));
    }
    nodes
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
