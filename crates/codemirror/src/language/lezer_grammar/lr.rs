use std::sync::LazyLock;

use super::{
    super::{
        lezer::{NormalizedEdges, Parser, ReuseStats, SyntaxTree},
        line_projection::NormalizedInput,
    },
    LezerNodeKind, Node, parser, tables,
};

static PARSER: LazyLock<Parser> = LazyLock::new(tables::parser);

#[derive(Debug)]
pub(super) struct Cache {
    pub(super) tree: SyntaxTree,
    pub(super) reuse: ReuseStats,
}

pub(super) fn parse(
    source: &str,
    reuse: Option<NormalizedEdges<'_>>,
) -> (Vec<Node>, Option<Cache>) {
    let input = NormalizedInput::new(source);
    let fragments = reuse.map_or_else(Vec::new, |edges| {
        edges.fragments(source, input.text.len(), |at| input.normalized_byte(at))
    });
    if let Ok(tree) = PARSER.parse_incremental_recovering(&input.text, &fragments) {
        let mut reuse = tree.reuse_stats();
        reuse.resumed_at = input.original_byte(reuse.resumed_at);
        let nodes = nodes(&tree, &input);
        (
            nodes,
            tree.has_incremental_state()
                .then_some(Cache { tree, reuse }),
        )
    } else {
        let mut nodes = parser::parse(&input.text);
        for node in &mut nodes {
            node.range = input.original_byte(node.range.start)..input.original_byte(node.range.end);
        }
        (nodes, None)
    }
}

fn nodes(tree: &SyntaxTree, input: &NormalizedInput<'_>) -> Vec<Node> {
    let mut stack = vec![(tree.root(), None::<usize>)];
    let mut nodes: Vec<Node> = Vec::new();
    while let Some((syntax, parent)) = stack.pop() {
        let id = nodes.len();
        let range = syntax.range();
        if let Some(parent) = parent {
            nodes[parent].children.push(id);
        }
        nodes.push(Node {
            kind: LezerNodeKind::from_name(syntax.name()),
            term: Some(syntax.term()),
            range: input.original_byte(range.start)..input.original_byte(range.end),
            parent,
            children: Vec::new(),
        });
        stack.extend(syntax.children().rev().map(|child| (child, Some(id))));
    }
    nodes
}

#[cfg(test)]
mod tests {
    #[test]
    fn published_tables_parse_specialized_tokens() {
        use super::LezerNodeKind::*;
        let source = "// 😀\r\n@top Main { @digit+ }\r\n@external propSource p from 'src'";
        let (nodes, cache) = super::parse(source, None);
        assert!(cache.is_some());
        let leaves: Vec<_> = nodes
            .iter()
            .filter(|node| node.children.is_empty())
            .map(|node| (node.kind, &source[node.range.clone()]))
            .collect();
        assert_eq!(
            leaves,
            [
                (LineComment, "// 😀"),
                (AtTop, "@top"),
                (RuleName, "Main"),
                (OpenBrace, "{"),
                (CharClass, "@digit"),
                (Plus, "+"),
                (CloseBrace, "}"),
                (AtExternal, "@external"),
                (PropSource, "propSource"),
                (Name, "p"),
                (From, "from"),
                (Literal, "'src'"),
            ]
        );
        assert_eq!(nodes[0].range, 0..source.len());
    }
}
