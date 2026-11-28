use std::{borrow::Cow, sync::LazyLock};

use super::{
    super::lezer::{
        GrammarSpec, LANGUAGE_DATA, LanguageFacet, NodeType, NodeTypes, ParseError, Parser,
        ReuseStats, SerializedTables, SyntaxNode, SyntaxTree, TreeFragment,
    },
    YamlNode as Node,
    YamlNodeKind::*,
    parser, tables, tokens,
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
        &[],
    )
    .expect("valid tables for the pinned YAML grammar")
    .with_deterministic_context_tracker(tokens::tracker())
});

pub(crate) fn stock_data_facet() -> &'static LanguageFacet {
    static FACET: LazyLock<LanguageFacet> =
        LazyLock::new(|| LanguageFacet::new(Some(super::language_data())));
    &FACET
}

pub(crate) fn stock_node_types() -> &'static NodeTypes {
    static TYPES: LazyLock<NodeTypes> = LazyLock::new(|| {
        let top = tables::TOP_RULE[1];
        NodeTypes::new(
            tables::NODE_NAMES,
            tables::REPEAT_NODE_COUNT,
            top,
            &[0],
            |term, _, props| {
                if term == top {
                    props.insert(&LANGUAGE_DATA, Some(stock_data_facet().clone()));
                }
            },
        )
    });
    &TYPES
}

pub(crate) fn stock_root_type() -> &'static NodeType {
    stock_node_types()
        .get(tables::TOP_RULE[1])
        .expect("YAML top term")
}

const MAX_PARSE_BYTES: usize = 1 << 20;
const MAX_TREE_DEPTH: usize = 128;
const LEZER_BUFFER_UNITS: usize = 1024;

#[derive(Debug)]
pub(super) struct Cache {
    tree: SyntaxTree,
    pub(super) reuse: ReuseStats,
}

pub(super) struct Parsed {
    pub(super) root: Node,
    pub(super) strict: Option<Cache>,
    pub(super) recovered: bool,
}

pub(super) fn parse(source: &str, prefix: Option<(&Cache, Option<usize>)>) -> Parsed {
    parse_pinned(source, prefix, false).unwrap_or_else(|_| Parsed {
        root: parser::parse(source),
        strict: None,
        recovered: true,
    })
}

pub(super) fn parse_strict(source: &str) -> Result<Parsed, ParseError> {
    parse_pinned(source, None, true)
}

pub(crate) fn parse_raw(source: &str, strict: bool) -> Result<SyntaxTree, ParseError> {
    check_size(source)?;
    let (mut tree, _) = finish_tree(source, PARSER.parse(source), strict)?;
    // Match the pinned unbuffered tree. The packed tree can wrap starts
    // that precede the buffer base. Keep the original byte ranges instead.
    tree.move_preceding_children(raw_comment_mapping);
    Ok(tree)
}

fn raw_comment_mapping<'a>(
    comment: SyntaxNode<'a>,
    mut node: SyntaxNode<'a>,
) -> Option<SyntaxNode<'a>> {
    if comment.name() != "Comment" {
        return None;
    }
    let comment = comment.range();
    // The zero-width implicit map start puts the last skipped comment
    // before the first Pair, inside the mapping, in the pinned tree.
    loop {
        let range = node.range();
        if range.start > comment.start || range.end < comment.end {
            return None;
        }
        let first = node.children().next()?;
        if node.name() == "BlockMapping"
            && first.name() == "Pair"
            && first.range().start >= comment.end
        {
            return Some(node);
        }
        node = first;
    }
}

fn check_size(source: &str) -> Result<(), ParseError> {
    if source.len() > MAX_PARSE_BYTES {
        return Err(ParseError::LimitExceeded { offset: 0 });
    }
    Ok(())
}

fn parse_pinned(
    original: &str,
    prefix: Option<(&Cache, Option<usize>)>,
    strict: bool,
) -> Result<Parsed, ParseError> {
    check_size(original)?;
    let input = Input::new(original);
    let source = &input.text;
    let fragment = prefix.map(|(cache, first_edit)| {
        let end = first_edit.map_or(source.len(), |at| input.prefix_end(at));
        TreeFragment {
            tree: &cache.tree,
            old_range: 0..end,
            new_range: 0..end,
        }
    });
    let lr = if strict {
        PARSER.parse(source)
    } else {
        PARSER.parse_incremental(source, fragment)
    };
    let (tree, recovered) = finish_tree(source, lr, strict)
        .map_err(|error| error.map_offset(|at| input.original_byte(at)))?;
    let root = convert(tree.root(), &input, 0)?;
    let cache = tree.has_incremental_state().then(|| {
        let mut reuse = tree.reuse_stats();
        reuse.resumed_at = input.original_byte(reuse.resumed_at);
        Cache { tree, reuse }
    });
    Ok(Parsed {
        root,
        strict: cache,
        recovered,
    })
}

fn finish_tree(
    source: &str,
    lr: Result<SyntaxTree, ParseError>,
    strict: bool,
) -> Result<(SyntaxTree, bool), ParseError> {
    let result = match lr {
        Err(ParseError::Ambiguous { .. }) => PARSER.parse_glr(source),
        result => result,
    };
    match result {
        Err(ParseError::UnexpectedToken { .. }) if !strict => {
            PARSER.parse_recovering(source).map(|tree| (tree, true))
        }
        result => result.map(|tree| (tree, false)),
    }
}

// CodeMirror's editor normalizes line breaks before passing input to Lezer.
// Only CRLF pairs change offsets; a bare CR maps to a one-byte LF.
struct Input<'a> {
    original: &'a str,
    text: Cow<'a, str>,
    breaks: Vec<(usize, usize)>,
}

impl<'a> Input<'a> {
    fn new(original: &'a str) -> Self {
        let text = if original.contains('\r') {
            Cow::Owned(original.replace("\r\n", "\n").replace('\r', "\n"))
        } else {
            Cow::Borrowed(original)
        };
        let breaks = original
            .match_indices("\r\n")
            .enumerate()
            .map(|(i, (at, _))| (at, at - i))
            .collect();
        Self {
            original,
            text,
            breaks,
        }
    }

    fn original_byte(&self, at: usize) -> usize {
        at + self.breaks.partition_point(|&(_, lf)| lf < at)
    }

    fn prefix_end(&self, mut end: usize) -> usize {
        // An edit boundary after CR can join or split a CRLF pair.
        if end > 0 && self.original.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        end - self.breaks.partition_point(|&(cr, _)| cr < end)
    }
}

fn convert(node: SyntaxNode<'_>, input: &Input<'_>, depth: usize) -> Result<Node, ParseError> {
    if depth == MAX_TREE_DEPTH {
        return Err(ParseError::LimitExceeded {
            offset: input.original_byte(node.range().start),
        });
    }
    let kind = match node.name() {
        "⚠" => Error,
        "DirectiveEnd" => DirectiveEnd,
        "DocEnd" => DocEnd,
        "-" => Dash,
        "?" => Question,
        "Literal" => Literal,
        "QuotedLiteral" => QuotedLiteral,
        "Anchor" => Anchor,
        "Alias" => Alias,
        "Tag" => Tag,
        "BlockLiteralContent" => BlockLiteralContent,
        "Comment" => Comment,
        "Stream" => Stream,
        "BOM" => Bom,
        "Document" => Document,
        "]" => CloseBracket,
        "[" => OpenBracket,
        "FlowSequence" => FlowSequence,
        "Item" => Item,
        "Tagged" => Tagged,
        "Anchored" => Anchored,
        "FlowMapping" => FlowMapping,
        "Pair" => Pair,
        "Key" => Key,
        ":" => Colon,
        "," => Comma,
        "}" => CloseBrace,
        "{" => OpenBrace,
        "BlockSequence" => BlockSequence,
        "BlockMapping" => BlockMapping,
        "BlockLiteral" => BlockLiteral,
        "BlockLiteralHeader" => BlockLiteralHeader,
        "Directive" => Directive,
        "DirectiveName" => DirectiveName,
        "DirectiveContent" => DirectiveContent,
        _ => unreachable!("named node from the pinned YAML grammar"),
    };
    let normalized = node.range();
    let range = input.original_byte(normalized.start)..input.original_byte(normalized.end);
    let mut children: Vec<Node> = Vec::new();
    for child in node.children() {
        let mut child = convert(child, input, depth + 1)?;
        if let Some(comment) = children.last().filter(|node| node.kind == Comment)
            && let Some(mapping) = leading_mapping(&mut child, comment)
        {
            // Lezer inserts a context record before trailing skipped nodes on
            // the zero-width map-start shift. This puts the last skipped
            // comment inside the mapping in the published tree.
            mapping.children.insert(0, children.pop().unwrap());
        }
        children.push(child);
    }
    let mut node = Node::new(kind, range, children);
    if kind != Stream
        && node.child_order == super::ChildOrder::Overlapping
        && input.text[normalized]
            .encode_utf16()
            .take(LEZER_BUFFER_UNITS + 1)
            .count()
            <= LEZER_BUFFER_UNITS
    {
        node.child_order = super::ChildOrder::BufferedOverlap;
    }
    Ok(node)
}

fn leading_mapping<'a>(mut node: &'a mut Node, comment: &Node) -> Option<&'a mut Node> {
    loop {
        if node.range.start > comment.range.start || node.range.end < comment.range.end {
            return None;
        }
        let first = node.children.first()?;
        if node.kind == BlockMapping && first.kind == Pair && first.range.start >= comment.range.end
        {
            return Some(node);
        }
        node = node.children.first_mut()?;
    }
}
