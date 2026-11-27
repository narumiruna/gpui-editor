use std::{borrow::Cow, ops::Range, sync::LazyLock};

use super::{
    super::lezer::{
        GrammarSpec, ParseError, Parser, ReuseStats, SerializedTables, Specializer, SyntaxTree,
        TreeFragment,
    },
    Node, fallback, tables, tokens,
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
            max_term: tables::MAX_TERM,
            repeat_node_count: tables::REPEAT_NODE_COUNT,
            tokenizers: &tokens::tokenizers(),
            top_rule: tables::TOP_RULE,
            token_prec: tables::TOKEN_PREC,
        },
        &[Specializer::context_free(
            tables::IDENTIFIER,
            tables::SPECIALIZATIONS,
        )],
    )
    .expect("valid tables for the pinned Python grammar")
    .with_deterministic_context_tracker(tokens::tracker())
});

static NAMES: LazyLock<Vec<&str>> = LazyLock::new(|| tables::NODE_NAMES.split(' ').collect());
const MAX_PARSE_BYTES: usize = 1 << 20;
const MAX_REPAIR_BYTES: usize = 64 * 1024;

pub(super) fn name(term: u16) -> &'static str {
    NAMES[usize::from(term)]
}

#[derive(Debug)]
pub(super) struct Cache {
    tree: SyntaxTree,
    pub(super) reuse: ReuseStats,
}

pub(super) struct Parsed {
    pub(super) nodes: Vec<Node>,
    pub(super) recovered: bool,
    pub(super) fallback_start: Option<usize>,
    pub(super) strict: Option<Cache>,
}

pub(super) fn parse(source: &str, prefix: Option<(&Cache, Option<usize>)>) -> Parsed {
    if source.len() > MAX_PARSE_BYTES {
        return result(None, 0, source.len(), |at| at, true);
    }
    let input = Input::new(source);
    let text = &input.text;
    let byte = |at| input.original_byte(at);
    let fragment = prefix.map(|(cache, first_edit)| {
        let end = first_edit.map_or(text.len(), |at| input.prefix_end(source, at));
        TreeFragment {
            tree: &cache.tree,
            old_range: 0..end,
            new_range: 0..end,
        }
    });
    let failure = match PARSER.parse_incremental(text, fragment) {
        Ok(tree) => {
            let mut parsed = result(Some(&tree), text.len(), source.len(), byte, false);
            let mut reuse = tree.reuse_stats();
            reuse.resumed_at = byte(reuse.resumed_at);
            parsed.strict = Some(Cache { tree, reuse });
            return parsed;
        }
        Err(error) => match try_glr(text, error) {
            Ok(tree) => return result(Some(&tree), text.len(), source.len(), byte, false),
            Err(error) => error,
        },
    };
    if let ParseError::UnexpectedToken { offset, .. } = failure
        && text.len() <= MAX_REPAIR_BYTES
    {
        let suffix = fallback::suffix(text);
        if !suffix.is_empty() {
            let mut repaired = text.to_string();
            repaired.push_str(&suffix);
            if let Ok(tree) = parse_tree(&repaired) {
                return result(Some(&tree), text.len(), source.len(), byte, true);
            }
        }
        let line = text[..offset.min(text.len())]
            .rfind('\n')
            .map_or(0, |at| at + 1);
        if line != 0
            && line < text.len()
            && let Ok(tree) = parse_tree(&text[..line])
        {
            return result(Some(&tree), line, source.len(), byte, true);
        }
    }
    result(None, 0, source.len(), byte, true)
}

fn parse_tree(source: &str) -> Result<SyntaxTree, ParseError> {
    PARSER.parse(source).or_else(|error| try_glr(source, error))
}

fn try_glr(source: &str, error: ParseError) -> Result<SyntaxTree, ParseError> {
    match error {
        ParseError::Ambiguous { .. } => PARSER.parse_glr(source),
        other => Err(other),
    }
}

struct Input<'a> {
    text: Cow<'a, str>,
    // Starts of CRLF pairs in the original and normalized text.
    breaks: Vec<(usize, usize)>,
}

impl<'a> Input<'a> {
    fn new(source: &'a str) -> Self {
        let text = if source.contains('\r') {
            Cow::Owned(source.replace("\r\n", "\n").replace('\r', "\n"))
        } else {
            Cow::Borrowed(source)
        };
        let breaks = source
            .match_indices("\r\n")
            .enumerate()
            .map(|(i, (at, _))| (at, at - i))
            .collect();
        Self { text, breaks }
    }

    fn original_byte(&self, at: usize) -> usize {
        at + self.breaks.partition_point(|&(_, lf)| lf < at)
    }

    fn prefix_end(&self, source: &str, mut end: usize) -> usize {
        // A CR at the edit boundary can join or split a CRLF pair. Do not
        // include its normalized LF in the prefix that the parser validates.
        if end > 0 && source.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        end - self.breaks.partition_point(|&(cr, _)| cr < end)
    }
}

fn result(
    tree: Option<&SyntaxTree>,
    cutoff: usize,
    end: usize,
    byte: impl Fn(usize) -> usize,
    recovered: bool,
) -> Parsed {
    let mut nodes = Vec::<Node>::new();
    if let Some(tree) = tree {
        let mut stack = vec![(Some(tree.root()), None::<usize>)];
        while let Some((syntax, parent)) = stack.pop() {
            let Some(syntax) = syntax else {
                let at = byte(cutoff);
                error_node(
                    &mut nodes,
                    parent.expect("missing closer has a parent"),
                    at..at,
                );
                continue;
            };
            let id = nodes.len();
            let range = syntax.range();
            if let Some(parent) = parent {
                nodes[parent].children.push(id);
            }
            let beyond = parent.is_some() && range.end > cutoff;
            let missing = beyond && range.start >= cutoff;
            let range = byte(range.start.min(cutoff))..byte(range.end.min(cutoff));
            nodes.push(Node {
                term: if missing { 0 } else { syntax.term() },
                range,
                parent,
                children: Vec::new(),
            });
            if !missing {
                if beyond && syntax.children().all(|child| child.range().end <= cutoff) {
                    // Anonymous synthetic closers do not appear as tree nodes.
                    stack.push((None, Some(id)));
                }
                stack.extend(syntax.children().rev().map(|child| (Some(child), Some(id))));
            }
        }
    } else {
        nodes.push(Node {
            term: tables::TOP_RULE[1],
            range: 0..end,
            parent: None,
            children: Vec::new(),
        });
    }
    let remainder = byte(cutoff);
    nodes[0].range = 0..end;
    let fallback_start = (remainder < end).then_some(remainder);
    if fallback_start.is_some() {
        error_node(&mut nodes, 0, remainder..end);
    }
    Parsed {
        nodes,
        recovered,
        fallback_start,
        strict: None,
    }
}

fn error_node(nodes: &mut Vec<Node>, parent: usize, range: Range<usize>) {
    let id = nodes.len();
    nodes[parent].children.push(id);
    nodes.push(Node {
        term: 0,
        range,
        parent: Some(parent),
        children: Vec::new(),
    });
}
