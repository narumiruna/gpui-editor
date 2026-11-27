use std::{ops::Range, sync::LazyLock};

use super::{
    Node,
    WastNodeKind::{self, *},
    WastNodeTypes,
    lexer::Lexer,
    tables,
};
use crate::language::{
    lezer::{NormalizedEdges, ParseError, Parser, ReuseStats, SyntaxTree, TreeFragment},
    line_projection::NormalizedInput,
};

pub(super) static PARSER: LazyLock<Parser> = LazyLock::new(tables::parser);

/// Parse a WAST string without editor line normalization. The tree uses UTF-8
/// byte ranges in `source`. A line comment ends at LF or EOF and includes any CR
/// before that point.
///
/// Both modes apply the LR stack cut. Strict mode does not try recovery when
/// parsing stalls. Either mode can return a native parser limit error.
/// Editor services use [`super::WastDocument`].
pub fn parse_raw(source: &str, strict: bool) -> Result<SyntaxTree, ParseError> {
    parse_raw_with_node_types(source, strict, WastNodeTypes::stock())
}

pub(crate) fn parse_raw_with_node_types(
    source: &str,
    strict: bool,
    node_types: WastNodeTypes,
) -> Result<SyntaxTree, ParseError> {
    parse_tree(source, &[], strict).map(|tree| tree.with_metadata(node_types))
}

fn parse_tree(
    source: &str,
    fragments: &[TreeFragment<'_>],
    strict: bool,
) -> Result<SyntaxTree, ParseError> {
    if strict {
        PARSER.parse_editor_strict(source)
    } else {
        PARSER.parse_incremental_wast_recovering(source, fragments)
    }
}

pub(super) fn parse_strict(source: &str) -> Result<Vec<Node>, ParseError> {
    let input = NormalizedInput::new(source);
    let tree = parse_tree(&input.text, &[], true)
        .map_err(|error| error.map_offset(|at| input.original_byte(at)))?;
    Ok(convert(&tree, |at| input.original_byte(at)))
}

pub(super) fn parse(
    source: &str,
    reuse: Option<NormalizedEdges<'_>>,
) -> (Vec<Node>, Option<SyntaxTree>, ReuseStats) {
    parse_published(source, reuse)
        .unwrap_or_else(|| (parse_locally(source), None, ReuseStats::default()))
}

fn parse_published(
    source: &str,
    reuse: Option<NormalizedEdges<'_>>,
) -> Option<(Vec<Node>, Option<SyntaxTree>, ReuseStats)> {
    let input = NormalizedInput::new(source);
    let fragments = reuse.map_or_else(Vec::new, |edges| {
        edges.fragments(source, input.text.len(), |at| input.normalized_byte(at))
    });
    let tree = parse_tree(&input.text, &fragments, false).ok()?;
    let mut stats = tree.reuse_stats();
    stats.resumed_at = input.original_byte(stats.resumed_at);
    let nodes = convert(&tree, |at| input.original_byte(at));
    Some((nodes, tree.has_incremental_state().then_some(tree), stats))
}

pub(super) fn convert(tree: &SyntaxTree, original_byte: impl Fn(usize) -> usize) -> Vec<Node> {
    let mut pending = vec![(tree.root(), None::<usize>)];
    let mut nodes: Vec<Node> = Vec::new();
    while let Some((syntax, parent)) = pending.pop() {
        let id = nodes.len();
        let kind = match syntax.name() {
            "Module" => Module,
            "App" => App,
            "(" => OpenParen,
            ")" => CloseParen,
            "Identifier" => Identifier,
            "Type" => Type,
            "Keyword" => Keyword,
            "Number" => Number,
            "String" => String,
            "LineComment" => LineComment,
            "BlockComment" => BlockComment,
            "⚠" => Error,
            _ => unreachable!("unknown WAST node"),
        };
        let range = syntax.range();
        nodes.push(Node::new(
            kind,
            original_byte(range.start)..original_byte(range.end),
            parent,
        ));
        if let Some(parent) = parent {
            nodes[parent].children.push(id);
        }
        pending.extend(syntax.children().rev().map(|child| (child, Some(id))));
    }
    nodes
}

// Keep a complete tree when the LR runtime reaches a work or recovery limit.
fn parse_locally(source: &str) -> Vec<Node> {
    let end = source.len();
    let mut nodes = vec![Node::new(Module, 0..end, None)];
    let mut parent = 0;
    for (kind, range) in Lexer::new(source) {
        match kind {
            OpenParen => {
                parent = push(&mut nodes, parent, App, range.start..end);
                push(&mut nodes, parent, kind, range);
            }
            CloseParen if parent != 0 => {
                nodes[parent].range.end = range.end;
                push(&mut nodes, parent, kind, range);
                parent = nodes[parent].parent.unwrap();
            }
            CloseParen => {
                let error = push(&mut nodes, parent, Error, range.clone());
                push(&mut nodes, error, kind, range);
            }
            Error => {
                // Merge contiguous unrecognized bytes, including ones that
                // immediately follow an unexpected closing token.
                if let Some(last) = last_error(&mut nodes, parent)
                    && last.range.end == range.start
                {
                    last.range.end = range.end;
                } else {
                    push(&mut nodes, parent, Error, range);
                }
            }
            _ => {
                push(&mut nodes, parent, kind, range);
            }
        }
    }
    // Each still-open expression reaches EOF. Omit a missing closing marker
    // when its previous child already records a syntax error.
    while parent != 0 {
        if last_error(&mut nodes, parent).is_none() {
            push(&mut nodes, parent, Error, end..end);
        }
        parent = nodes[parent].parent.unwrap();
    }
    nodes
}

impl Node {
    fn new(kind: WastNodeKind, range: Range<usize>, parent: Option<usize>) -> Self {
        Self {
            kind,
            range,
            parent,
            children: Vec::new(),
        }
    }
}

fn push(nodes: &mut Vec<Node>, parent: usize, kind: WastNodeKind, range: Range<usize>) -> usize {
    let id = nodes.len();
    nodes.push(Node::new(kind, range, Some(parent)));
    nodes[parent].children.push(id);
    id
}

fn last_error(nodes: &mut [Node], parent: usize) -> Option<&mut Node> {
    let &id = nodes[parent].children.last()?;
    let node = &mut nodes[id];
    (node.kind == Error).then_some(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_tables_handle_complete_and_recovered_input() {
        let complete = "(module ;; 🦀\r\n  (func (result funcref)))";
        let input = NormalizedInput::new(complete);
        assert!(PARSER.parse(&input.text).is_ok());
        let (nodes, _, _) = parse_published(complete, None).unwrap();
        assert_eq!(nodes[0].range, 0..complete.len());
        assert!(
            nodes
                .iter()
                .any(|node| { node.kind == Type && &complete[node.range.clone()] == "funcref" })
        );

        let incomplete = ")(module\r\n  (func $f\r\n";
        let input = NormalizedInput::new(incomplete);
        assert!(PARSER.parse(&input.text).is_err());
        let (nodes, _, _) = parse_published(incomplete, None).unwrap();
        let end = incomplete.len();
        assert_eq!(nodes[0].range, 0..end);
        assert_eq!(nodes[1].kind, Error);
        assert_eq!(nodes[2].kind, CloseParen);
        assert_eq!(
            nodes.iter().filter(|node| node.range == (end..end)).count(),
            2
        );
    }

    #[test]
    fn published_recovery_closes_deep_open_expressions_at_the_pinned_cut() {
        let source = "(".repeat(4_000);
        let (nodes, tree, reuse) = parse_published(&source, None).unwrap();
        assert!(tree.is_some());
        assert_eq!(reuse, ReuseStats::default());
        assert_eq!(nodes[0].range, 0..source.len());
        assert_eq!(nodes.len(), 1 + 3 * source.len());
        let app = nodes
            .iter()
            .find(|node| node.kind == App && node.range.start == 1_999)
            .unwrap();
        assert_eq!(app.range, 1_999..2_800);
        let error = nodes.iter().find(|node| node.kind == Error).unwrap();
        assert_eq!(error.range, 2_800..2_800);

        // Only the first checkpoint is shallow enough to retain. Completing
        // the expression restores that strict prefix, not the recovery cut.
        let repaired = format!("{source}{}", ")".repeat(source.len()));
        let prefix = 0..source.len();
        let tree = tree.unwrap();
        let parsed = PARSER
            .parse_incremental(
                &repaired,
                Some(crate::language::lezer::TreeFragment {
                    tree: &tree,
                    old_range: prefix.clone(),
                    new_range: prefix,
                }),
            )
            .unwrap();
        let reuse = parsed.reuse_stats();
        let nodes = convert(&parsed, std::convert::identity);
        assert_eq!(reuse.resumed_at, 1);
        assert_eq!(reuse.nodes_reused, 1);
        assert_eq!(nodes.len(), 1 + 3 * source.len());
        assert!(nodes.iter().all(|node| node.kind != Error));
    }

    #[test]
    fn raw_checkpoints_keep_only_strict_output_before_recovery() {
        for (depth, cut) in [(2_799, false), (2_800, true)] {
            let source = "(".repeat(depth) + &")".repeat(depth);
            let raw = parse_raw(&source, false).unwrap();
            assert_eq!(raw.reuse_stats(), ReuseStats::default());
            assert_eq!(raw.reused_suffix_actions(), 0);

            let reparse = |tree: &SyntaxTree, new_source: &str, start| {
                PARSER
                    .parse_incremental(
                        new_source,
                        Some(TreeFragment {
                            tree,
                            old_range: 0..source.len(),
                            new_range: start..new_source.len(),
                        }),
                    )
                    .unwrap()
            };
            let shifted = format!(" {source}");
            let reuse = reparse(&raw, &source, 0).reuse_stats();
            let suffix = reparse(&raw, &shifted, 1).reused_suffix_actions();
            if cut {
                assert_eq!((reuse.resumed_at, reuse.nodes_reused), (1, 1));
                assert_eq!(suffix, 0);
            } else {
                let baseline = PARSER.parse_incremental_recovering(&source, &[]).unwrap();
                assert!(reuse.resumed_at > depth);
                assert!(suffix > 0);
                assert_eq!(reuse, reparse(&baseline, &source, 0).reuse_stats());
                assert_eq!(
                    suffix,
                    reparse(&baseline, &shifted, 1).reused_suffix_actions()
                );
            }
        }
    }

    #[test]
    fn local_fallback_finishes_deep_input_without_recursion() {
        // Test the fallback directly. The pinned recovery can now parse
        // this input without reaching the work limit.
        let source = "(".repeat(4_000);
        let nodes = parse_locally(&source);
        assert_eq!(nodes[0].range, 0..source.len());
        assert_eq!(nodes.len(), 1 + 3 * source.len());
        let error = nodes.iter().find(|node| node.kind == Error).unwrap();
        assert_eq!(error.range, source.len()..source.len());
    }
}
