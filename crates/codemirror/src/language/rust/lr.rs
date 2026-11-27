use std::sync::LazyLock;

use super::{
    super::{
        lezer::{
            GrammarSpec, ParseError, Parser, ReuseStats, SerializedTables, Specializer, SyntaxTree,
            TreeFragment,
        },
        line_projection::NormalizedInput,
    },
    Node, RustNodeKind, tables, tokens,
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

const MAX_RECOVERY_BYTES: usize = 16 * 1024;

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

pub(super) fn parse(source: &str, prefix: Option<(&Cache, usize)>) -> Result<Parsed, ParseError> {
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
    let error = match PARSER.parse_incremental(&input.text, fragment.clone()) {
        Ok(tree) => {
            let mut reuse = tree.reuse_stats();
            reuse.resumed_at = byte(reuse.resumed_at);
            return Ok(Parsed {
                nodes: project(&tree, byte),
                cache: Some(Cache { tree, reuse }),
                recovered: false,
            });
        }
        Err(ParseError::Ambiguous { .. }) => match PARSER.parse_glr(&input.text) {
            Ok(tree) => {
                return Ok(Parsed {
                    nodes: project(&tree, byte),
                    cache: None,
                    recovered: false,
                });
            }
            Err(error) => error,
        },
        Err(error) => error,
    };
    if source.len() > MAX_RECOVERY_BYTES || !matches!(error, ParseError::UnexpectedToken { .. }) {
        return Err(error);
    }
    // Recovery starts fresh. Repeat the strict pass only to keep its validated
    // prefix checkpoints for a later edit. The recovered output cannot seed it.
    let tree = PARSER.parse_incremental_recovering(&input.text, fragment.as_slice())?;
    Ok(Parsed {
        nodes: project(&tree, byte),
        cache: tree.has_incremental_state().then_some(Cache {
            tree,
            reuse: ReuseStats::default(),
        }),
        recovered: true,
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
        let parsed = super::parse(source, None).unwrap();
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
