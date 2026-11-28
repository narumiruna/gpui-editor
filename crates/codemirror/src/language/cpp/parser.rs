use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use super::{
    super::{
        lezer::{
            CLOSED_BY, FOLD, GROUP, GrammarSpec, ISOLATE, Isolation, LANGUAGE_DATA, LanguageFacet,
            LrFoldCallback, NodeTypes, OPENED_BY, ParseError, Parser, ReuseStats, SerializedTables,
            Specialization, Specializer, SyntaxTree, TreeFragment, stock_fold,
        },
        line_projection::NormalizedInput,
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

pub(crate) fn stock_data_facet() -> &'static LanguageFacet {
    static FACET: LazyLock<LanguageFacet> =
        LazyLock::new(|| LanguageFacet::new(Some(super::language_data())));
    &FACET
}

pub(crate) fn stock_node_types() -> &'static NodeTypes {
    static COMMENT_FOLD: LazyLock<LrFoldCallback> =
        LazyLock::new(|| Arc::new(|node, _| node.cpp_fold_range()));
    static TYPES: LazyLock<NodeTypes> = LazyLock::new(|| {
        let top = tables::TOP_RULE[1];
        // Structural properties from SHA-512-pinned @lezer/cpp 1.1.6.
        NodeTypes::new(
            tables::NODE_NAMES,
            tables::REPEAT_NODE_COUNT,
            top,
            &[
                0, 3, 4, 5, 6, 7, 10, 297, 298, 299, 300, 301, 302, 303, 304, 305, 306, 308, 349,
                350,
            ],
            |term, _, props| {
                if term == top {
                    props.insert(&LANGUAGE_DATA, Some(stock_data_facet().clone()));
                }
                // Exact fold rows from SHA-512-pinned @codemirror/lang-cpp 6.0.3.
                match term {
                    5 => props.insert(&FOLD, Some(COMMENT_FOLD.clone())),
                    78 | 126 | 136 | 260 => props.insert(&FOLD, Some(stock_fold())),
                    _ => {}
                }
                let group = match term {
                    1
                    | 8
                    | 11
                    | 15..=17
                    | 19
                    | 71
                    | 72
                    | 100..=102
                    | 104
                    | 191
                    | 208
                    | 229
                    | 242
                    | 243
                    | 270..=272
                    | 277
                    | 280..=282
                    | 284..=287
                    | 290
                    | 292..=296 => Some("Expression"),
                    18 | 25..=27 | 43 | 255..=258 | 262 | 263 | 265 | 266 => Some("Type"),
                    126 | 129 | 147 | 150 | 152 | 153 | 158 | 160 | 163 | 164 | 166 | 168 | 170
                    | 172 | 174 | 176 | 178 | 179 | 188 => Some("Statement"),
                    _ => None,
                };
                if let Some(group) = group {
                    props.insert(&GROUP, vec![group.into()]);
                }
                match term {
                    4 | 5 | 8 | 10 => props.insert(&ISOLATE, Some(Isolation::Auto)),
                    12 => props.insert(&OPENED_BY, vec!["(".into()]),
                    13 => props.insert(&CLOSED_BY, vec![")".into()]),
                    51 => props.insert(&CLOSED_BY, vec!["}".into()]),
                    52 => props.insert(&OPENED_BY, vec!["{".into()]),
                    53 => props.insert(&CLOSED_BY, vec!["]".into()]),
                    54 => props.insert(&OPENED_BY, vec!["[".into()]),
                    _ => {}
                }
            },
        )
    });
    &TYPES
}

const MAX_RECOVERY_SEED_BYTES: usize = 16 * 1024;

#[derive(Debug)]
pub(super) struct Cache {
    tree: SyntaxTree,
    pub(super) reuse: ReuseStats,
}

pub(super) fn parse(
    source: &str,
    prefix: Option<(&Cache, usize)>,
) -> (Vec<Node>, bool, Option<Cache>) {
    let normalized = NormalizedInput::new(source);
    let byte = |at| normalized.original_byte(at);
    let fragment = prefix.map(|(cache, mut end)| {
        // The edit can join or split CRLF at the prefix boundary.
        if end > 0 && source.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        let end = normalized.normalized_byte(end);
        TreeFragment {
            tree: &cache.tree,
            old_range: 0..end,
            new_range: 0..end,
        }
    });
    let input = normalized.text.as_ref();
    let failure = match PARSER.parse_incremental(input, fragment.clone()) {
        Ok(tree) => {
            let mut reuse = tree.reuse_stats();
            reuse.resumed_at = byte(reuse.resumed_at);
            let nodes = project(&tree, input.len(), source.len(), None, byte);
            return (nodes, false, Some(Cache { tree, reuse }));
        }
        Err(error) => match try_glr(input, error) {
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
        // The size limit applies to repeating strict LR for checkpoints,
        // not to bounded recovery. Large inputs keep no recovery seed.
        let recovered = if input.len() <= MAX_RECOVERY_SEED_BYTES {
            PARSER.parse_incremental_recovering(input, fragment.as_slice())
        } else {
            PARSER.parse_recovering(input)
        };
        if let Ok(tree) = recovered {
            let nodes = project(&tree, input.len(), source.len(), None, byte);
            let cache = tree.has_incremental_state().then_some(Cache {
                tree,
                reuse: ReuseStats::default(),
            });
            return (nodes, true, cache);
        }
        if let Some((tree, cutoff, hole)) = recovery::recover(input, offset) {
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

pub(crate) fn parse_raw(source: &str, strict: bool) -> Result<SyntaxTree, ParseError> {
    match parse_tree(source) {
        Err(ParseError::UnexpectedToken { .. }) if !strict => PARSER.parse_recovering(source),
        result => result,
    }
}

pub(super) fn parse_editor_strict(source: &str) -> Result<Vec<Node>, ParseError> {
    let input = NormalizedInput::new(source);
    let byte = |at| input.original_byte(at);
    let tree = parse_tree(&input.text).map_err(|error| error.map_offset(byte))?;
    Ok(project(&tree, input.text.len(), source.len(), None, byte))
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
