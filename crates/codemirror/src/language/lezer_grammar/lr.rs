use std::sync::LazyLock;

use super::{
    super::{
        lezer::{
            CLOSED_BY, FOLD, GROUP, LANGUAGE_DATA, LanguageFacet, NodeTypes, NormalizedEdges,
            OPENED_BY, ParseError, Parser, ReuseStats, SyntaxTree, stock_fold,
        },
        line_projection::NormalizedInput,
    },
    LezerNodeKind, Node, highlight, parser, tables,
};

pub(super) static PARSER: LazyLock<Parser> = LazyLock::new(tables::parser);

pub(crate) fn stock_data_facet() -> &'static LanguageFacet {
    static FACET: LazyLock<LanguageFacet> =
        LazyLock::new(|| LanguageFacet::new(Some(super::language_data())));
    &FACET
}

pub(crate) fn stock_node_types() -> &'static NodeTypes {
    static TYPES: LazyLock<NodeTypes> = LazyLock::new(|| {
        NodeTypes::new(
            tables::NODE_NAMES,
            tables::REPEAT_NODE_COUNT,
            tables::TOP_TERM,
            &[0, 1, 2],
            |term, name, props| match term {
                tables::TOP_TERM => props.insert(&LANGUAGE_DATA, Some(stock_data_facet().clone())),
                4 => props.insert(&GROUP, vec!["Declaration".into(), "Declaration".into()]),
                5 | 13 | 20..=24 | 26 | 28 | 30 | 32 | 33 | 36 | 37 | 40 | 46 => {
                    props.insert(&GROUP, vec!["Expression".into()]);
                }
                48 | 50 | 57 | 60 | 62 | 65 | 66 | 72 | 79 | 82 | 84 | 87 | 89 | 92 | 94 | 96 => {
                    props.insert(&GROUP, vec!["Declaration".into()]);
                }
                6 => props.insert(&OPENED_BY, vec!["[".into()]),
                7 => props.insert(&CLOSED_BY, vec!["]".into()]),
                15 => props.insert(&OPENED_BY, vec!["{".into()]),
                16 => props.insert(&CLOSED_BY, vec!["}".into()]),
                34 => props.insert(&OPENED_BY, vec!["(".into()]),
                35 => props.insert(&CLOSED_BY, vec![")".into()]),
                _ if matches!(name, "Body" | "TokensBody" | "SkipBody" | "PrecedenceBody") => {
                    props.insert(&FOLD, Some(stock_fold()));
                }
                _ => (),
            },
        )
        .extend(&[highlight::stock_styles()])
    });
    &TYPES
}

#[derive(Debug)]
pub(super) struct Cache {
    pub(super) tree: SyntaxTree,
    pub(super) reuse: ReuseStats,
}

/// Parse original grammar source without normalizing editor line breaks.
/// Ranges use the source's UTF-8 bytes. A line comment ends at LF or EOF.
/// Strict parsing skips recovery after a stall. Either mode can reach a
/// native parser limit. Unlike [`super::LezerDocument`], this API does not
/// fall back to a local parser.
pub fn parse_raw(source: &str, strict: bool) -> Result<SyntaxTree, ParseError> {
    if strict {
        PARSER.parse_editor_strict(source)
    } else {
        PARSER.parse_incremental_recovering(source, &[])
    }
}

pub(super) fn parse_strict(source: &str) -> Result<Vec<Node>, ParseError> {
    let input = NormalizedInput::new(source);
    let tree = parse_raw(&input.text, true)
        .map_err(|error| error.map_offset(|at| input.original_byte(at)))?;
    Ok(nodes(&tree, &input))
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
