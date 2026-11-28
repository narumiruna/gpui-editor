use std::sync::{Arc, LazyLock};

use super::{
    super::{
        lezer::{
            CLOSED_BY, FOLD, GROUP, GrammarSpec, INDENT, ISOLATE, Isolation, LANGUAGE_DATA,
            LanguageFacet, LrFoldCallback, NodeTypes, OPENED_BY, ParseError, Parser, ReuseStats,
            SerializedTables, Specializer, SyntaxTree, TreeFragment,
        },
        line_projection::NormalizedInput,
    },
    Node, RustNodeKind,
    indent::stock_strategy,
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
            max_term: tables::MAX_TERM,
            repeat_node_count: tables::REPEAT_NODE_COUNT,
            tokenizers: &tokens::tokenizers(),
            top_rule: tables::TOP_RULE,
            token_prec: tables::TOKEN_PREC,
        },
        &[Specializer::context_free(
            tables::IDENTIFIER,
            tables::KEYWORDS,
        )],
    )
    .expect("valid tables for the pinned Rust grammar")
});

pub(crate) fn stock_data_facet() -> &'static LanguageFacet {
    static FACET: LazyLock<LanguageFacet> =
        LazyLock::new(|| LanguageFacet::new(Some(super::language_data())));
    &FACET
}

pub(crate) fn stock_node_types() -> &'static NodeTypes {
    static FOLD_CALLBACK: LazyLock<LrFoldCallback> =
        LazyLock::new(|| Arc::new(|node, _| node.rust_fold_range()));
    static TYPES: LazyLock<NodeTypes> = LazyLock::new(|| {
        let top = tables::TOP_RULE[1];
        NodeTypes::new(
            tables::NODE_NAMES,
            tables::REPEAT_NODE_COUNT,
            top,
            &[0, 6, 7, 264],
            |term, name, props| {
                if term == top {
                    props.insert(&LANGUAGE_DATA, Some(stock_data_facet().clone()));
                }
                if let Some(rule) = stock_strategy(term) {
                    props.insert(&INDENT, Some(rule.clone()));
                }
                // Fold rows from @codemirror/lang-rust 6.0.2 dist/index.js.
                if name == "BlockComment"
                    || name.ends_with("Block")
                    || name.ends_with("edTokens")
                    || name.ends_with("List")
                {
                    props.insert(&FOLD, Some(FOLD_CALLBACK.clone()));
                }
                // Structural properties from @lezer/rust 1.0.3 dist/index.js.
                let group = match term {
                    4
                    | 5
                    | 13..=18
                    | 35
                    | 37..=39
                    | 43
                    | 54
                    | 56
                    | 58
                    | 59
                    | 61
                    | 64
                    | 112
                    | 118
                    | 122
                    | 124
                    | 125
                    | 134
                    | 135
                    | 137
                    | 139
                    | 140
                    | 142
                    | 144..=146
                    | 148
                    | 150..=155
                    | 159
                    | 160
                    | 166
                    | 168
                    | 170 => Some("Expression"),
                    21 | 23..=26 | 28 | 240 | 252..=260 | 262 => Some("Type"),
                    52 | 53 | 69 | 71..=74 | 77 | 83..=84 | 86..=88 | 90 | 91 | 93 | 94 => {
                        Some("Pattern")
                    }
                    _ if tables::STATEMENTS.contains(&term) => Some("Statement"),
                    _ => None,
                };
                if let Some(group) = group {
                    props.insert(&GROUP, vec![group.into()]);
                }
                match term {
                    4 | 6 | 7 | 35 => props.insert(&ISOLATE, Some(Isolation::Auto)),
                    10 => props.insert(&OPENED_BY, vec!["[".into()]),
                    41 => props.insert(&OPENED_BY, vec!["{".into()]),
                    42 => props.insert(&CLOSED_BY, vec!["}".into()]),
                    48 => props.insert(&CLOSED_BY, vec![")".into()]),
                    50 => props.insert(&OPENED_BY, vec!["(".into()]),
                    85 => props.insert(&CLOSED_BY, vec!["]".into()]),
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

pub(super) struct Parsed {
    pub(super) nodes: Vec<Node>,
    pub(super) cache: Option<Cache>,
    pub(super) recovered: bool,
}

pub(crate) fn parse_raw(source: &str, strict: bool) -> Result<SyntaxTree, ParseError> {
    match finish_strict(source, PARSER.parse(source)) {
        Err(ParseError::UnexpectedToken { .. }) if !strict => PARSER.parse_recovering(source),
        result => result,
    }
}

fn finish_strict(
    source: &str,
    lr: Result<SyntaxTree, ParseError>,
) -> Result<SyntaxTree, ParseError> {
    match lr {
        Err(ParseError::Ambiguous { .. }) => PARSER.parse_glr(source),
        result => result,
    }
}

pub(super) fn parse(
    source: &str,
    prefix: Option<(&Cache, usize)>,
    strict: bool,
) -> Result<Parsed, ParseError> {
    let input = NormalizedInput::new(source);
    let byte = |at| input.original_byte(at);
    let fragment = prefix.map(|(cache, mut end)| {
        // The edit can join or split CRLF at the prefix boundary.
        if end > 0 && source.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        let end = input.normalized_byte(end);
        TreeFragment {
            tree: &cache.tree,
            old_range: 0..end,
            new_range: 0..end,
        }
    });
    let lr = if strict {
        PARSER.parse(&input.text)
    } else {
        PARSER.parse_incremental(&input.text, fragment.clone())
    };
    let (tree, recovered) = match finish_strict(&input.text, lr) {
        Err(ParseError::UnexpectedToken { .. }) if !strict => {
            let tree = if source.len() <= MAX_RECOVERY_SEED_BYTES {
                // Repeat strict LR to keep validated checkpoints for a later edit.
                PARSER.parse_incremental_recovering(&input.text, fragment.as_slice())
            } else {
                PARSER.parse_recovering(&input.text)
            };
            tree.map(|tree| (tree, true))
        }
        result => result.map(|tree| (tree, false)),
    }
    .map_err(|error| error.map_offset(byte))?;
    Ok(Parsed {
        nodes: project(&tree, byte),
        cache: tree.has_incremental_state().then(|| {
            let mut reuse = if recovered {
                ReuseStats::default()
            } else {
                tree.reuse_stats()
            };
            reuse.resumed_at = byte(reuse.resumed_at);
            Cache { tree, reuse }
        }),
        recovered,
    })
}

fn project(tree: &SyntaxTree, byte: impl Fn(usize) -> usize) -> Vec<Node> {
    let mut stack = vec![(tree.root(), None::<usize>)];
    let mut nodes: Vec<Node> = Vec::new();
    while let Some((syntax, parent)) = stack.pop() {
        let id = nodes.len();
        let range = syntax.range();
        if let Some(parent) = parent {
            nodes[parent].children.push(id);
        }
        nodes.push(Node {
            kind: RustNodeKind::from_name(syntax.name()),
            term: Some(syntax.term()),
            range: byte(range.start)..byte(range.end),
            parent,
            children: Vec::new(),
            statement: tables::STATEMENTS.contains(&syntax.term()),
        });
        stack.extend(syntax.children().rev().map(|child| (child, Some(id))));
    }
    nodes
}

#[cfg(test)]
mod tests {
    #[test]
    fn glr_parses_an_unparenthesized_if_condition() {
        let source = "fn f() { if left && right { 1 } else { 0 } }";
        let parsed = super::parse(source, None, false).unwrap();
        assert!(!parsed.recovered);
        assert!(parsed.cache.is_none());
        let nodes = parsed.nodes;
        use super::RustNodeKind::{BinaryExpression, IfExpression};
        assert!(nodes.iter().any(|node| node.kind == IfExpression));
        assert!(
            nodes.iter().any(|node| node.kind == BinaryExpression
                && &source[node.range.clone()] == "left && right")
        );
    }
}
