use std::{borrow::Cow, sync::LazyLock};

use super::{
    super::lezer::{
        GrammarSpec, ParseError, Parser, ReuseStats, SerializedTables, SyntaxNode, SyntaxTree,
        TreeFragment,
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

const MAX_PARSE_BYTES: usize = 1 << 20;
const MAX_TREE_DEPTH: usize = 128;

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
    if source.len() <= MAX_PARSE_BYTES {
        let input = Input::new(source);
        if let Some(result) = parse_pinned(&input, prefix) {
            return result;
        }
    }
    Parsed {
        root: parser::parse(source),
        strict: None,
        recovered: true,
    }
}

fn parse_pinned(input: &Input<'_>, prefix: Option<(&Cache, Option<usize>)>) -> Option<Parsed> {
    let source = &input.text;
    let fragment = prefix.map(|(cache, first_edit)| {
        let end = first_edit.map_or(source.len(), |at| input.prefix_end(at));
        TreeFragment {
            tree: &cache.tree,
            old_range: 0..end,
            new_range: 0..end,
        }
    });
    let failure = match PARSER.parse_incremental(source, fragment) {
        Ok(tree) => {
            let root = convert(tree.root(), input, 0)?;
            let mut reuse = tree.reuse_stats();
            reuse.resumed_at = input.original_byte(reuse.resumed_at);
            return Some(Parsed {
                root,
                strict: Some(Cache { tree, reuse }),
                recovered: false,
            });
        }
        Err(ParseError::Ambiguous { .. }) => match PARSER.parse_glr(source) {
            Ok(tree) => {
                return Some(Parsed {
                    root: convert(tree.root(), input, 0)?,
                    strict: None,
                    recovered: false,
                });
            }
            Err(error) => error,
        },
        Err(error) => error,
    };
    if matches!(failure, ParseError::UnexpectedToken { .. }) {
        let tree = PARSER.parse_recovering(source).ok()?;
        return Some(Parsed {
            root: convert(tree.root(), input, 0)?,
            strict: None,
            recovered: true,
        });
    }
    None
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

fn convert(node: SyntaxNode<'_>, input: &Input<'_>, depth: usize) -> Option<Node> {
    if depth == MAX_TREE_DEPTH {
        return None;
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
    let range = node.range();
    let range = input.original_byte(range.start)..input.original_byte(range.end);
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
    Some(Node::new(kind, range, children))
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
