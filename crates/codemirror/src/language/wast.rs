//! Native WAT and WAST support based on `@codemirror/lang-wast` 6.0.2 and its
//! bundled Lezer grammar. See `docs/licenses/codemirror-lang-wast.txt`.
//!
//! Ranges use UTF-8 bytes in the original [`Text`]. Indentation treats LF,
//! CRLF, and lone CR as line breaks. The parser uses the published Lezer
//! tables, with a local fallback when the runtime cannot finish. Error nodes
//! can differ from upstream. This grammar describes expressions, not valid
//! WebAssembly modules.

use std::{ops::Range, sync::LazyLock};

use crate::{Assoc, ChangeSet, EditError, EditorState, Text};

use super::{
    HighlightSpan, HighlightTag,
    lezer::{NormalizedEdges, ReuseStats, SyntaxTree},
    line_projection::{editor_lines, splits_crlf},
    markdown::MarkdownSourceMap,
};

mod brackets;
mod data;
mod indent;
mod isolation;
mod job;
mod lexer;
mod node_props;
mod parser;
mod publication;
mod style_tags;
mod tables;

use super::bidi::IsolateRanges;
use super::lezer::ConfiguredHighlights;
use data::WastDataNodes;
pub use data::{WastDataFacet, WastSublanguage, WastSublanguageMode};
pub use indent::{WastIndentContext, WastIndentStrategy};
pub use job::{WastParseJob, WastParseOutput, WastParseStep};
pub(crate) use node_props::WastNodeTypes;
pub use node_props::{
    BRACKET_MATCHING_HANDLE, CLOSED_BY, FOLD, GROUP, INDENT, ISOLATE, LANGUAGE_DATA, NodeProp,
    OPENED_BY, SUBLANGUAGES, WastBracketHandle, WastFoldCallback, WastIsolation, WastNodeType,
    WastPropSource,
};
pub use parser::parse_raw;
pub(crate) use parser::parse_raw_with_node_types;
pub(crate) use publication::WastPublication;
pub use style_tags::{WastStyleSelectorError, WastStyleTags, style_tags};

/// Line comment marker from upstream language data.
pub const LINE_COMMENT: &str = ";;";
/// Block comment delimiters from upstream language data.
pub const BLOCK_COMMENT: (&str, &str) = ("(;", ";)");
/// Opening characters that upstream asks the editor to close automatically.
pub const CLOSE_BRACKETS: [char; 2] = ['(', '"'];

/// Visible node types of the upstream grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WastNodeKind {
    Module,
    App,
    OpenParen,
    CloseParen,
    Identifier,
    Type,
    Keyword,
    Number,
    String,
    LineComment,
    BlockComment,
    Error,
}

impl WastNodeKind {
    /// The node's name in the upstream syntax tree.
    pub const fn name(self) -> &'static str {
        use WastNodeKind::*;
        match self {
            Module => "Module",
            App => "App",
            OpenParen => "(",
            CloseParen => ")",
            Identifier => "Identifier",
            Type => "Type",
            Keyword => "Keyword",
            Number => "Number",
            String => "String",
            LineComment => "LineComment",
            BlockComment => "BlockComment",
            Error => "⚠",
        }
    }

    /// Whether this kind belongs to the stock grammar's `Expression` group.
    pub const fn is_expression(self) -> bool {
        use WastNodeKind::*;
        matches!(self, App | Identifier | Type | Keyword | Number | String)
    }

    /// The grammar's matching delimiter type, if this type opens or closes a pair.
    pub const fn matching_delimiter(self) -> Option<Self> {
        match self {
            Self::OpenParen => Some(Self::CloseParen),
            Self::CloseParen => Some(Self::OpenParen),
            _ => None,
        }
    }

    /// Whether the stock grammar specifies automatic bidirectional text isolation.
    pub const fn isolates_direction(self) -> bool {
        matches!(self, Self::LineComment | Self::BlockComment | Self::String)
    }

    fn highlight(self) -> Option<&'static [HighlightTag]> {
        static TAGS: LazyLock<[HighlightTag; 8]> = LazyLock::new(|| {
            [
                "paren",
                "variableName",
                "typeName",
                "keyword",
                "number",
                "string",
                "lineComment",
                "blockComment",
            ]
            .map(HighlightTag::new)
        });
        use WastNodeKind::*;
        let index = match self {
            OpenParen | CloseParen => 0,
            Identifier => 1,
            Type => 2,
            Keyword => 3,
            Number => 4,
            String => 5,
            LineComment => 6,
            BlockComment => 7,
            _ => return None,
        };
        Some(std::slice::from_ref(&TAGS[index]))
    }
}

#[derive(Debug)]
struct Node {
    kind: WastNodeKind,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A borrowed node in the recovered syntax tree. An empty error range marks
/// missing syntax. An error can contain an unexpected `)` token.
#[derive(Clone, Copy, Debug)]
pub struct WastNode<'a> {
    nodes: &'a [Node],
    node_types: &'a WastNodeTypes,
    id: usize,
    coordinates: Coordinates<'a>,
}

#[derive(Clone, Copy, Debug)]
enum Coordinates<'a> {
    Linear(usize),
    Segmented(MarkdownSourceMap<'a>),
}

impl Coordinates<'_> {
    fn source(self, local: usize) -> usize {
        match self {
            Self::Linear(origin) => origin + local,
            Self::Segmented(map) => map.source_offset(local),
        }
    }
}

impl<'a> WastNode<'a> {
    pub(crate) fn in_markdown(self, map: MarkdownSourceMap<'a>) -> Self {
        let coordinates = if map.is_contiguous() {
            Coordinates::Linear(map.bounds().start)
        } else {
            Coordinates::Segmented(map)
        };
        Self {
            coordinates,
            ..self
        }
    }

    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    pub fn kind(self) -> WastNodeKind {
        self.data().kind
    }

    /// The node-type row in this document's parser configuration.
    pub fn node_type(self) -> &'a WastNodeType {
        self.node_types.for_kind(self.kind())
    }

    /// Read a node-type property by key identity.
    pub fn prop<T: 'static>(self, key: &NodeProp<T>) -> Option<&'a T> {
        self.node_type().prop(key)
    }

    /// The node's UTF-8 byte range. In a Markdown callback this is a host
    /// range and can include gaps. Parent navigation stays in the WAST tree.
    pub fn range(self) -> Range<usize> {
        let range = &self.data().range;
        self.coordinates.source(range.start)..self.coordinates.source(range.end)
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    /// Direct children in source order, without whitespace.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }

    fn resolve_at(mut self, at: usize, side: Option<Assoc>) -> Self {
        loop {
            let children = &self.data().children;
            let i = children.partition_point(|&id| {
                let end = self.coordinates.source(self.nodes[id].range.end);
                end < at || (end == at && side != Some(Assoc::Before))
            });
            let Some(&id) = children.get(i) else {
                return self;
            };
            let start = self.coordinates.source(self.nodes[id].range.start);
            if start > at || (start == at && side != Some(Assoc::After)) {
                return self;
            }
            self.id = id;
        }
    }

    /// The stock range between the delimiters of an expression or block
    /// comment. An unfinished expression can fold to the end of the node.
    /// An empty block comment has an empty range.
    pub fn fold_range(self) -> Option<Range<usize>> {
        match self.kind() {
            WastNodeKind::App => self.fold_inside(),
            WastNodeKind::BlockComment => Some(self.fold_block_comment()),
            _ => None,
        }
    }

    fn fold_inside(self) -> Option<Range<usize>> {
        let mut children = self.children();
        let first = children.next()?;
        let last = children.next_back()?;
        let start = first.range().end;
        let last_start = last.range().start;
        (start < last_start).then(|| {
            let end = if last.kind() == WastNodeKind::Error {
                self.range().end
            } else {
                last_start
            };
            start..end
        })
    }

    fn fold_block_comment(self) -> Range<usize> {
        let range = self.range();
        (range.start + 2)..(range.end - 2)
    }

    fn foldable_by(
        self,
        line_start: usize,
        line_end: usize,
        fold_range: impl FnMut(Self) -> Option<Range<usize>>,
    ) -> Option<Range<usize>> {
        let node = self.resolve_at(line_end, Some(Assoc::After));
        super::fold::fold_on_line(
            std::iter::successors(Some(node), |node| node.parent()),
            line_start..line_end,
            Self::range,
            fold_range,
        )
    }

    fn unfinished_before(self, position: usize) -> Self {
        let children = &self.data().children;
        let before = children
            .partition_point(|&id| self.coordinates.source(self.nodes[id].range.start) < position);
        let mut scan = before.checked_sub(1).map(|i| Self {
            id: children[i],
            ..self
        });
        let mut inner = self;
        while let Some(node) = scan {
            let mut children = node.children();
            let Some(last) = children
                .next_back()
                .filter(|last| last.data().range.end == node.data().range.end)
            else {
                break;
            };
            if last.kind() == WastNodeKind::Error && last.data().range.is_empty() {
                inner = node;
                scan = children.next_back();
            } else {
                scan = Some(last);
            }
        }
        inner
    }
}

/// One WebAssembly text document and its recovered syntax tree.
#[derive(Debug)]
pub struct WastDocument {
    text: Text,
    editor_lines: Option<Text>,
    nodes: Vec<Node>,
    node_types: WastNodeTypes,
    isolate_ranges: IsolateRanges,
    configured_highlights: Option<ConfiguredHighlights>,
    published: Option<SyntaxTree>,
    reuse: ReuseStats,
}

impl WastDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_node_types(text, WastNodeTypes::stock())
    }

    pub(crate) fn parse_with_node_types(text: Text, node_types: WastNodeTypes) -> Self {
        Self::build(text, None, node_types)
    }

    /// Apply edits after line-ending normalization. Strict parsing can reuse
    /// validated prefix checkpoints, even if the last tree needed recovery.
    /// Only strict output can supply shifted suffix decisions.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let reuse = self
            .published
            .as_ref()
            .map(|tree| NormalizedEdges::new(tree, self.text.len(), changes));
        Ok(Self::build(text, reuse, self.node_types.clone()))
    }

    /// Published checkpoint statistics. The resume position uses original
    /// UTF-8 byte offsets. Recovery and local fallback report zero reuse.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.reuse
    }

    /// Count consuming token decisions reused in the shifted suffix.
    pub fn reused_suffix_actions(&self) -> usize {
        self.published
            .as_ref()
            .map_or(0, SyntaxTree::reused_suffix_actions)
    }

    pub(crate) fn parse_strict(
        text: Text,
        node_types: WastNodeTypes,
    ) -> Result<Self, super::lezer::ParseError> {
        let nodes = parser::parse_strict(text.as_str())?;
        Ok(Self::from_nodes(
            text,
            nodes,
            node_types,
            None,
            ReuseStats::default(),
        ))
    }

    fn build(
        text: Text,
        fragments: Option<NormalizedEdges<'_>>,
        node_types: WastNodeTypes,
    ) -> Self {
        let (nodes, published, reuse) = parser::parse(text.as_str(), fragments);
        Self::from_nodes(text, nodes, node_types, published, reuse)
    }

    fn from_nodes(
        text: Text,
        nodes: Vec<Node>,
        node_types: WastNodeTypes,
        published: Option<SyntaxTree>,
        reuse: ReuseStats,
    ) -> Self {
        let isolate_ranges = isolation::index(&nodes, &node_types);
        let configured_highlights = node_types.has_configured_styles().then(|| {
            ConfiguredHighlights::build(WastNode {
                nodes: &nodes,
                node_types: &node_types,
                id: 0,
                coordinates: Coordinates::Linear(0),
            })
        });
        Self {
            editor_lines: editor_lines(text.as_str()),
            text,
            nodes,
            node_types,
            isolate_ranges,
            configured_highlights,
            published,
            reuse,
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// Text for syntax line queries. Lone CR is projected to LF so that it
    /// ends a line, with no change to the original byte offsets.
    pub fn syntax_lines(&self) -> &Text {
        self.editor_lines.as_ref().unwrap_or(&self.text)
    }

    /// The root spans the full document, including skipped whitespace.
    pub fn root(&self) -> WastNode<'_> {
        WastNode {
            nodes: &self.nodes,
            node_types: &self.node_types,
            id: 0,
            coordinates: Coordinates::Linear(0),
        }
    }

    /// All nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = WastNode<'_>> {
        (0..self.nodes.len()).map(|id| WastNode { id, ..self.root() })
    }

    /// Highlight spans clipped to the requested range. Return `None` if the
    /// range is reversed or either endpoint is not a valid UTF-8 byte boundary.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.text.slice(range.clone())?;
        let configured = self.configured_highlights.as_ref();
        let stock = configured
            .is_none()
            .then(|| self.stock_highlights(range.clone()));
        let configured = configured.map(|runs| runs.clipped(range));
        Some(
            stock
                .into_iter()
                .flatten()
                .chain(configured.into_iter().flatten()),
        )
    }

    fn stock_highlights(&self, range: Range<usize>) -> impl Iterator<Item = HighlightSpan<'_>> {
        let first = self
            .nodes
            .partition_point(|node| node.range.start < range.start);
        let end = self
            .nodes
            .partition_point(|node| node.range.start < range.end);
        // Only the last earlier node can be a styled leaf that overlaps the range.
        self.nodes[first.saturating_sub(1)..end]
            .iter()
            .filter_map(move |node| {
                let tags = node.kind.highlight()?;
                let start = node.range.start.max(range.start);
                let end = node.range.end.min(range.end);
                (start < end).then_some(HighlightSpan {
                    range: start..end,
                    tags,
                })
            })
    }

    /// Find a stock fold that begins on the supplied line and continues past
    /// its end. This stateless query does not call configured callbacks.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        self.root()
            .foldable_by(line_start, line_end, WastNode::fold_range)
    }

    /// Ask the configured WAST nodes for a fold in a contiguous editor body.
    /// The body must contain this snapshot's exact source. The line bounds,
    /// callback node ranges, and returned fold all use parent-state byte offsets.
    /// Ignore proposals that leave the body or split a character or CRLF break.
    /// Use [`crate::language::fold::foldable`] to run editor-wide services first.
    pub fn foldable_in_scope(
        &self,
        state: &EditorState,
        body: Range<usize>,
        line_start: usize,
        line_end: usize,
    ) -> Option<Range<usize>> {
        if line_start < body.start
            || line_end > body.end
            || state.text().slice_owned(body.clone())? != self.text
        {
            return None;
        }
        let origin = body.start;
        self.text
            .slice(line_start - origin..line_end.checked_sub(origin)?)?;
        self.root()
            .foldable_by(line_start - origin, line_end - origin, |node| {
                let callback = node.prop(&FOLD)?.as_ref()?;
                let range = callback(
                    WastNode {
                        coordinates: Coordinates::Linear(origin),
                        ..node
                    },
                    state,
                )?;
                let local = range.start.checked_sub(origin)?..range.end.checked_sub(origin)?;
                (self.text.is_valid_range(local.clone())
                    && [local.start, local.end]
                        .into_iter()
                        .all(|at| !splits_crlf(self.text.as_str(), at)))
                .then_some(local)
            })
            .map(|range| origin + range.start..origin + range.end)
    }

    /// For a mount from the current Markdown snapshot, use host bounds to
    /// select candidates. A covering WAST node can span a gap at line_end.
    pub(crate) fn foldable_in_markdown(
        &self,
        state: &EditorState,
        map: MarkdownSourceMap<'_>,
        line_start: usize,
        line_end: usize,
    ) -> Option<Range<usize>> {
        let body = map.bounds();
        if !body.contains(&line_end) {
            return None;
        }
        if map.is_contiguous() && line_start >= body.start {
            return self.foldable_in_scope(state, body, line_start, line_end);
        }
        self.root()
            .in_markdown(map)
            .foldable_by(line_start, line_end, |node| {
                let range = node.prop(&FOLD)?.as_ref()?(node, state)?;
                let text = state.text();
                (text.is_valid_range(range.clone())
                    && [range.start, range.end]
                        .into_iter()
                        .all(|at| map.contains_endpoint(at) && !splits_crlf(text.as_str(), at)))
                .then_some(range)
            })
    }
}
