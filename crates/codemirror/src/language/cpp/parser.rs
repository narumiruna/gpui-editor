use std::{borrow::Cow, ops::Range, sync::LazyLock};

use super::{
    super::lezer::{
        GrammarSpec, ParseError, Parser, ReuseStats, SerializedTables, Specialization, Specializer,
        SyntaxTree, TreeFragment,
    },
    Node, recovery, tables, tokens,
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
        &[
            Specializer::context_free(362, tables::IDENTIFIERS),
            Specializer::context_free(33, &[("<", Specialization::Extend(383))]),
            Specializer::context_free(66, &[(">", Specialization::Extend(67))]),
            Specializer::context_free(369, tables::SCOPED_IDENTIFIERS),
        ],
    )
    .and_then(|parser| parser.with_dynamic_precedence(tables::DYNAMIC_PRECEDENCES))
    .expect("valid tables for the pinned C++ grammar")
});

#[derive(Debug)]
pub(super) struct Cache {
    tree: SyntaxTree,
    pub(super) reuse: ReuseStats,
}

pub(super) fn parse(
    source: &str,
    prefix: Option<(&Cache, usize)>,
) -> (Vec<Node>, bool, Option<Cache>) {
    // CodeMirror normalizes line endings before it parses the document.
    let breaks: Vec<_> = source
        .match_indices("\r\n")
        .enumerate()
        .map(|(i, (at, _))| (at, at - i))
        .collect();
    let input = if source.contains('\r') {
        Cow::Owned(source.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(source)
    };
    let byte = |at| at + breaks.partition_point(|&(_, lf)| lf < at);
    let fragment = prefix.map(|(cache, mut end)| {
        // The edit can join or split CRLF at the prefix boundary.
        if end > 0 && source.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        let end = end - breaks.partition_point(|&(cr, _)| cr < end);
        TreeFragment {
            tree: &cache.tree,
            old_range: 0..end,
            new_range: 0..end,
        }
    });
    let failure = match PARSER.parse_incremental(&input, fragment.clone()) {
        Ok(tree) => {
            let mut reuse = tree.reuse_stats();
            reuse.resumed_at = byte(reuse.resumed_at);
            let nodes = project(&tree, input.len(), source.len(), None, byte);
            return (nodes, false, Some(Cache { tree, reuse }));
        }
        Err(error) => match try_glr(&input, error) {
            Ok(tree) => {
                return (
                    project(&tree, input.len(), source.len(), None, byte),
                    false,
                    None,
                );
            }
            Err(error) => error,
        },
    };
    if let ParseError::UnexpectedToken { offset, .. } = failure {
        // Repeat strict LR only for small malformed documents. The recovery
        // tree retains strict checkpoints, not any of its recovered nodes.
        if input.len() <= 16 * 1024
            && let Ok(tree) = PARSER.parse_incremental_recovering(&input, fragment.as_slice())
        {
            let nodes = project(&tree, input.len(), source.len(), None, byte);
            let cache = tree.has_incremental_state().then_some(Cache {
                tree,
                reuse: ReuseStats::default(),
            });
            return (nodes, true, cache);
        }
        if let Some((tree, cutoff, hole)) = recovery::recover(&input, offset) {
            return (project(&tree, cutoff, source.len(), hole, byte), true, None);
        }
    }
    let mut nodes = vec![Node {
        term: tables::TOP_RULE[1],
        range: 0..source.len(),
        parent: None,
        children: Vec::new(),
    }];
    add_node(&mut nodes, 0, 0..source.len(), Some(0));
    (nodes, true, None)
}

pub(super) fn parse_tree(source: &str) -> Result<SyntaxTree, ParseError> {
    PARSER.parse(source).or_else(|error| try_glr(source, error))
}

fn try_glr(source: &str, error: ParseError) -> Result<SyntaxTree, ParseError> {
    match error {
        ParseError::Ambiguous { .. } => PARSER.parse_glr_pruned(source),
        other => Err(other),
    }
}

fn project(
    tree: &SyntaxTree,
    cutoff: usize,
    source_len: usize,
    hole: Option<Range<usize>>,
    byte: impl Fn(usize) -> usize,
) -> Vec<Node> {
    let hole = hole.map(|range| byte(range.start)..byte(range.end));
    let mut pending = vec![(tree.root(), None::<usize>)];
    let mut nodes = Vec::new();
    let mut missing = Vec::new();
    while let Some((syntax, parent)) = pending.pop() {
        let range = syntax.range();
        let synthesized = parent.is_some() && range.start >= cutoff && range.end > cutoff;
        let term = if synthesized { 0 } else { syntax.term() };
        let start = byte(range.start.min(cutoff));
        let end = byte(range.end.min(cutoff));
        let id = add_node(&mut nodes, term, start..end, parent);
        if !synthesized {
            // A synthetic anonymous terminator is not in the shared tree.
            if parent.is_some()
                && range.end > cutoff
                && !syntax
                    .children()
                    .any(|child| child.range().end == range.end)
            {
                missing.push((id, end));
            }
            pending.extend(syntax.children().rev().map(|child| (child, Some(id))));
        }
    }
    for (id, end) in missing {
        add_node(&mut nodes, 0, end..end, Some(id));
    }
    nodes[0].range = 0..source_len;
    let parsed_end = byte(cutoff);
    if parsed_end < source_len {
        add_node(&mut nodes, 0, parsed_end..source_len, Some(0));
    }
    if let Some(hole) = hole {
        let mut parent = 0;
        while let Some(inner) = nodes[parent]
            .children
            .iter()
            .copied()
            .find(|&id| nodes[id].range.start < hole.start && hole.end < nodes[id].range.end)
        {
            parent = inner;
        }
        let at = nodes[parent]
            .children
            .partition_point(|&id| nodes[id].range.start < hole.start);
        let id = add_node(&mut nodes, 0, hole, None);
        nodes[id].parent = Some(parent);
        nodes[parent].children.insert(at, id);
    }
    nodes
}

fn add_node(nodes: &mut Vec<Node>, term: u16, range: Range<usize>, parent: Option<usize>) -> usize {
    let id = nodes.len();
    nodes.push(Node {
        term,
        range,
        parent,
        children: Vec::new(),
    });
    if let Some(parent) = parent {
        nodes[parent].children.push(id);
    }
    id
}
