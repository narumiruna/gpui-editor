//! Native Go language services based on `@codemirror/lang-go` 6.0.1 and
//! `@lezer/go` 1.0.1. See `docs/licenses/codemirror-lang-go.txt`.
//!
//! The parser uses published tables, bounded GLR, and a shift context on
//! each branch to insert semicolons. Deterministic LR parses can reuse an
//! unchanged prefix, including after bounded recovery. GLR and recovery run
//! from scratch. If bounded recovery fails, a local fallback keeps a parseable
//! prefix and marks missing syntax or the unparsed suffix.
//! Ranges use UTF-8 bytes. Completion includes the fixed snippets and
//! keywords and syntax-scoped local names.

use std::ops::Range;

use crate::{
    Assoc, ChangeSet, EditError, Text, commands::CommentTokens, language_data::CloseBracketData,
};

use super::{
    HighlightSpan, IndentOptions, LanguageData, StreamSettings,
    lezer::{NodeType, NodeTypes, ParseError, ReuseStats},
};

mod brackets;
mod completion;
mod highlight;
mod indent;
mod kind;
mod parser;
mod tables;
mod tokens;

pub use completion::{
    GoSnippet, SNIPPETS, go_completion_snippet, go_local_completion, go_snippet_completion,
};
pub use kind::GoNodeKind;
pub(crate) use parser::{parse_raw, stock_data_facet, stock_node_types};

/// The upstream language name.
pub const NAME: &str = "go";
pub const LINE_COMMENT: &str = "//";
pub const BLOCK_COMMENT: (&str, &str) = ("/*", "*/");
pub const CLOSE_BRACKETS: [char; 6] = ['(', '[', '{', '\'', '"', '`'];

pub(crate) fn language_data() -> LanguageData {
    LanguageData::default()
        .with(
            "commentTokens",
            CommentTokens {
                line: Some(LINE_COMMENT),
                block: Some(BLOCK_COMMENT),
            },
        )
        .with("indentOnInput", indent_on_input as fn(&str) -> bool)
        .with(
            "closeBrackets",
            CloseBracketData {
                brackets: Some(&["(", "[", "{", "'", "\"", "`"]),
                ..Default::default()
            },
        )
}

/// Whether the line from its start to the cursor triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    matches!(trim_js_space(line_prefix), "case" | "default" | "}")
}

fn trim_js_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

#[derive(Debug)]
struct Node {
    term: u16,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A borrowed Go syntax node. An empty error node can mark missing syntax.
#[derive(Clone, Copy, Debug)]
pub struct GoNode<'a> {
    nodes: &'a [Node],
    node_types: Option<&'a NodeTypes>,
    source: &'a str,
    id: usize,
}

impl<'a> GoNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    pub fn kind(self) -> GoNodeKind {
        GoNodeKind::from_term(self.term())
    }

    /// The grammar-local term ID, including distinct IDs for repeated names.
    pub fn term(self) -> u16 {
        self.data().term
    }

    /// The immutable type row for this term.
    pub fn node_type(self) -> &'a NodeType {
        self.node_types
            .unwrap_or_else(|| stock_node_types())
            .get(self.term())
            .expect("valid Go node term")
    }

    pub fn range(self) -> Range<usize> {
        self.data().range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    /// Direct children in source order. Whitespace is not included.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }

    /// The fold range for a node with an upstream fold rule.
    pub fn fold_range(self) -> Option<Range<usize>> {
        use GoNodeKind::*;
        let range = &self.data().range;
        if self.kind() == BlockComment {
            let comment = &self.source[range.clone()];
            // The property subtracts two UTF-16 units, even for an unfinished
            // comment. Keep the result on a valid byte boundary.
            let mut before_end = comment;
            let mut units = 2usize;
            while units != 0 {
                if let Some(prefix) = before_end.strip_suffix("\r\n") {
                    before_end = prefix;
                    units -= 1;
                } else {
                    let last = before_end.chars().next_back()?;
                    before_end = &before_end[..before_end.len() - last.len_utf8()];
                    units = units.checked_sub(last.len_utf16())?;
                }
            }
            return (before_end.len() >= 2)
                .then(|| range.start + 2..range.start + before_end.len());
        }
        if !matches!(
            self.kind(),
            Block
                | SwitchBlock
                | SelectBlock
                | LiteralValue
                | InterfaceType
                | StructType
                | SpecList
        ) {
            return None;
        }
        let first = self.children().next()?;
        let last = self.children().next_back()?;
        let start = first.data().range.end;
        (start < last.data().range.start).then(|| {
            start..if last.kind() == Error {
                range.end
            } else {
                last.data().range.start
            }
        })
    }
}

/// One Go document and its native syntax tree.
#[derive(Debug)]
pub struct GoDocument {
    text: Text,
    nodes: Vec<Node>,
    highlights: Vec<HighlightSpan<'static>>,
    recovered: bool,
    cache: Option<Box<parser::Cache>>,
    editor_lines: Option<Text>,
    // Standalone and mounted parses keep their existing data routing.
    node_types: Option<NodeTypes>,
}

impl GoDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_prefix(text, None)
    }

    pub(crate) fn parse_editor_strict(text: Text) -> Result<Self, ParseError> {
        let nodes = parser::parse_editor_strict(text.as_str())?;
        Ok(Self::from_nodes(text, nodes, false, None))
    }

    pub(crate) fn with_node_types(mut self, node_types: NodeTypes) -> Self {
        self.node_types = Some(node_types);
        self
    }

    pub(crate) fn configured_node_types(&self) -> Option<&NodeTypes> {
        self.node_types.as_ref()
    }

    /// Apply byte edits and parse the new document. Reuse a validated LR
    /// checkpoint before the first edit when possible. Invalid edits leave
    /// this document unchanged. Recovery can retain checked strict prefix
    /// checkpoints for a later repair; it does not reuse recovered nodes.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let end = changes
            .changes()
            .next()
            .map_or(text.len(), |(range, _)| range.start);
        let prefix = self.cache.as_deref().map(|cache| (cache, end));
        let mut next = Self::parse_with_prefix(text, prefix);
        next.node_types = self.node_types.clone();
        Ok(next)
    }

    /// LR checkpoint reuse in the last parse. The resume offset is in the
    /// original UTF-8 text, including CRLF. GLR and recovered trees report
    /// zero reuse. Native nodes and highlights are rebuilt.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.cache
            .as_deref()
            .map_or_else(ReuseStats::default, |cache| cache.reuse)
    }

    fn parse_with_prefix(text: Text, prefix: Option<(&parser::Cache, usize)>) -> Self {
        let (nodes, recovered, cache) = parser::parse(text.as_str(), prefix);
        Self::from_nodes(text, nodes, recovered, cache)
    }

    fn from_nodes(
        text: Text,
        nodes: Vec<Node>,
        recovered: bool,
        cache: Option<parser::Cache>,
    ) -> Self {
        let highlights = highlight::collect(GoNode {
            nodes: &nodes,
            node_types: None,
            source: text.as_str(),
            id: 0,
        });
        Self {
            editor_lines: super::line_projection::editor_lines(text.as_str()),
            text,
            nodes,
            highlights,
            recovered,
            cache: cache.map(Box::new),
            node_types: None,
        }
    }

    /// Whether the native LR and GLR parser needed recovery.
    pub fn is_recovered(&self) -> bool {
        self.recovered
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// Logical editor lines with lone CR mapped to LF at the same byte offset.
    pub fn syntax_lines(&self) -> &Text {
        self.editor_lines.as_ref().unwrap_or(&self.text)
    }

    /// The root covers the full document, including skipped whitespace.
    pub fn root(&self) -> GoNode<'_> {
        GoNode {
            nodes: &self.nodes,
            node_types: self.node_types.as_ref(),
            source: self.text.as_str(),
            id: 0,
        }
    }

    /// All nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = GoNode<'_>> {
        (0..self.nodes.len()).map(|id| GoNode { id, ..self.root() })
    }

    /// Highlight spans clipped to a UTF-8 byte range. Return `None` for an
    /// invalid endpoint or reversed range.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.text.slice(range.clone())?;
        let first = self
            .highlights
            .partition_point(|span| span.range.end <= range.start);
        let end = if range.is_empty() {
            first
        } else {
            self.highlights
                .partition_point(|span| span.range.start < range.end)
        };
        Some(
            self.highlights[first..end]
                .iter()
                .map(move |span| HighlightSpan {
                    range: span.range.start.max(range.start)..span.range.end.min(range.end),
                    tags: span.tags,
                }),
        )
    }

    /// Find a fold that starts on this line and continues past its end.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        let inner = self.path(line_end, Some(Assoc::After)).last()?;
        let mut found = None;
        for node in std::iter::successors(Some(inner), |node| node.parent()) {
            if found.is_some() && node.data().range.start < line_start {
                break;
            }
            if let Some(range) = node.fold_range()
                && (line_start..=line_end).contains(&range.start)
                && line_end < range.end
            {
                found = Some(range);
            }
        }
        found
    }

    /// Compute the indentation in columns. Return `None` for invalid byte
    /// positions or where upstream preserves the current indentation.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        indent::indentation(self, position, settings, options)
    }

    /// Candidate indentation gap between adjacent matching syntax tokens.
    /// The newline command still checks that the gap is whitespace on one line.
    pub fn bracket_gap_for_indent(&self, position: usize) -> Option<Range<usize>> {
        use GoNodeKind::*;
        if !self.text.is_char_boundary(position) {
            return None;
        }
        let node = self.path(position, None).last()?;
        let open = node
            .children()
            .rev()
            .find(|child| child.range().start < position)?;
        let close = node
            .children()
            .find(|child| child.range().start >= position)?;
        (open.range().end <= position
            && matches!(
                (open.kind(), close.kind()),
                (OpenBrace, CloseBrace) | (OpenBracket, CloseBracket) | (OpenParen, CloseParen)
            ))
        .then(|| open.range().end..close.range().start)
    }

    fn path(&self, position: usize, side: Option<Assoc>) -> impl Iterator<Item = GoNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let index = children.partition_point(|&id| {
                let end = self.nodes[id].range.end;
                end < position || (end == position && side != Some(Assoc::Before))
            });
            next = children.get(index).and_then(|&id| {
                let range = &self.nodes[id].range;
                (!range.is_empty()
                    && (range.start < position
                        || (side == Some(Assoc::After) && range.start == position)))
                    .then_some(GoNode { id, ..node })
            });
            Some(node)
        })
    }
}
