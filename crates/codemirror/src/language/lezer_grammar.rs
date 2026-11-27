//! Native Lezer grammar services based on `@codemirror/lang-lezer` 6.0.2 and
//! `@lezer/lezer` 1.1.3. See `docs/licenses/codemirror-lang-lezer.txt`.
//!
//! Ranges use UTF-8 bytes in the original document. The native LR runtime
//! parses the pinned grammar tables and reuses validated prefix checkpoints.
//! Malformed input uses bounded LR recovery, then a local parser if that fails.
//!
//! Differences from upstream:
//!
//! - Use [`parse_raw`] for direct parsing. A [language handle](crate::language_data::lezer_language)
//!   configures the synchronous editor attachment. These are native APIs, not
//!   the JavaScript `LRLanguage` or `LanguageSupport` APIs. They do not compile
//!   grammars, schedule background parsing, or expose partial editor trees.
//! - Strict parsing can resume from a verified prefix, including a prefix
//!   retained after recovery, and replay supported shifted suffix decisions.
//!   Each parse rebuilds the native node and highlight views. Recovery parses
//!   from the start and does not reuse recovered nodes. The tree omits anonymous
//!   repeat nodes and packed buffers. Configured grammar rows are available on
//!   LR nodes. Bracket and indent queries still use the stock grammar rules.
//! - Syntax and line queries recognize LF, CRLF, and lone CR. Original text and
//!   UTF-8 byte offsets do not change.
//! - Bounded recovery can choose different branches or leave partial output.
//!   A local parser supplies a tree if LR recovery cannot finish within the
//!   allowed work and stack limits.

use std::{
    ops::Range,
    sync::{Arc, OnceLock},
};

use crate::{
    ChangeSet, EditError, RangeSet, Text,
    commands::CommentTokens,
    view::{MarkDecoration, Viewport},
};

use super::{
    BidiIsolateOptions, HighlightSpan, IndentOptions, LanguageData, StreamSettings,
    bidi::IsolateRanges,
    lezer::{
        ConfiguredHighlights, ISOLATE, NodeType, NodeTypes, NormalizedEdges, ParseError,
        ReuseStats, StyleTags, StyledNode, select_tags,
    },
    line_projection::editor_lines,
};

mod brackets;
mod highlight;
mod indent;
mod lexer;
mod lr;
mod parser;
mod tables;

pub use lr::parse_raw;
pub(crate) use lr::{stock_data_facet, stock_node_types};

/// The upstream language name.
pub const NAME: &str = "lezer";
pub const LINE_COMMENT: &str = "//";
pub const BLOCK_COMMENT: (&str, &str) = ("/*", "*/");

/// The document-wide data provided by the upstream language wrapper.
pub fn language_data() -> LanguageData {
    LanguageData::default()
        .with(
            "commentTokens",
            CommentTokens {
                line: Some(LINE_COMMENT),
                block: Some(BLOCK_COMMENT),
            },
        )
        .with("indentOnInput", indent_on_input as fn(&str) -> bool)
}

/// Whether the text from a line's start to the cursor triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    trim_js_space(line_prefix) == "}"
}

fn trim_js_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

macro_rules! node_kinds {
    ($($plain:ident,)*; $($named:ident => $name:literal,)*) => {
        /// The visible node types in the Lezer grammar.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum LezerNodeKind { $($plain,)* $($named,)* }

        impl LezerNodeKind {
            /// The node's name in the upstream syntax tree.
            pub const fn name(self) -> &'static str {
                match self { $(Self::$plain => stringify!($plain),)* $(Self::$named => $name,)* }
            }

            fn from_name(name: &str) -> Self {
                match name {
                    $(stringify!($plain) => Self::$plain,)*
                    $($name => Self::$named,)*
                    _ => unreachable!("unknown node in the pinned Lezer grammar: {name}"),
                }
            }
        }
    };
}

node_kinds! {
    Grammar, LineComment, BlockComment, RuleDeclaration, RuleName, Props, Prop, AtName, Name,
    Literal, PropEsc, ParamList, Body, CharSet, AnyChar, InvertedCharSet, ScopedName, Call, ArgList,
    CharClass, Optional, Repeat, Repeat1, InlineRule, ParenExpression, Specialization, Sequence,
    PrecedenceMarker, PrecedenceName, AmbiguityMarker, Choice, PrecedenceDeclaration,
    PrecedenceBody, Precedence, TokensDeclaration, TokensBody, TokenPrecedenceDeclaration,
    TokenConflictDeclaration, ConflictBody, LiteralTokenDeclaration, LocalTokensDeclaration,
    ElseToken, ExternalTokensDeclaration, Token, ExternalConflictDeclaration,
    ExternalPropDeclaration, ExternalPropSourceDeclaration, ExternalSpecializeDeclaration,
    ContextDeclaration, DialectsDeclaration, DialectBody, TopSkipDeclaration, SkipScope, SkipBody,
    DetectDelimDeclaration,
    ;
    Error => "⚠",
    OpenBrace => "{", CloseBrace => "}", OpenBracket => "[", CloseBracket => "]",
    OpenParen => "(", CloseParen => ")", Equal => "=", Dot => ".", Bar => "|", Bang => "!",
    Tilde => "~", Star => "*", Plus => "+", Question => "?",
    AtSpecialize => "@specialize", AtExtend => "@extend", AtTop => "@top",
    AtPrecedence => "@precedence", AtLeft => "@left", AtRight => "@right", AtCut => "@cut",
    AtTokens => "@tokens", AtConflict => "@conflict", AtLocal => "@local", AtElse => "@else",
    AtExternal => "@external", AtContext => "@context", AtDialects => "@dialects",
    AtSkip => "@skip", AtDetectDelim => "@detectDelim",
    Tokens => "tokens", From => "from", PropKeyword => "prop", As => "as",
    PropSource => "propSource", Extend => "extend", Specialize => "specialize",
}

#[derive(Debug)]
struct Node {
    kind: LezerNodeKind,
    // Some LR node types have the same name but different IDs.
    term: Option<u16>,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A borrowed syntax node. A zero-width error can mark missing syntax.
#[derive(Clone, Copy, Debug)]
pub struct LezerNode<'a> {
    document: &'a LezerDocument,
    id: usize,
}

impl<'a> LezerNode<'a> {
    fn data(self) -> &'a Node {
        &self.document.nodes[self.id]
    }

    pub fn kind(self) -> LezerNodeKind {
        self.data().kind
    }

    /// Return the configured row for this node. Local fallback nodes other
    /// than the root have no verified LR term.
    pub fn node_type(self) -> Option<&'a NodeType> {
        let term = self
            .data()
            .term
            .or_else(|| (self.id == 0).then_some(tables::TOP_TERM))?;
        self.document.node_types().get(term)
    }

    /// Select the first matching rule for a node with a verified grammar row.
    /// Use the document's spans for inherited tags.
    pub fn style_tags(self) -> Option<StyleTags<'a>> {
        let ancestors = std::iter::successors(self.parent(), |node| node.parent())
            .map(|node| node.kind().name());
        select_tags(self.node_type()?, ancestors)
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

    /// Range inside a rule, token, skip, or precedence body.
    pub fn fold_range(self) -> Option<Range<usize>> {
        use LezerNodeKind::*;
        if !matches!(self.kind(), Body | TokensBody | SkipBody | PrecedenceBody) {
            return None;
        }
        let first = self.children().next()?;
        let last = self.children().next_back()?;
        let start = first.data().range.end;
        (start < last.data().range.start).then(|| {
            start..if last.kind() == Error {
                self.data().range.end
            } else {
                last.data().range.start
            }
        })
    }
}

impl<'a> StyledNode<'a> for LezerNode<'a> {
    fn child(self, index: usize) -> Option<Self> {
        self.children().nth(index)
    }

    fn range(self) -> Range<usize> {
        self.range()
    }

    fn name(self) -> &'a str {
        self.kind().name()
    }

    fn node_type(self) -> Option<&'a NodeType> {
        self.node_type()
    }
}

/// One Lezer grammar and its recovered syntax tree.
#[derive(Debug)]
pub struct LezerDocument {
    text: Text,
    editor_lines: Option<Text>,
    nodes: Vec<Node>,
    highlights: Vec<HighlightSpan<'static>>,
    published: Option<Box<lr::Cache>>,
    node_types: Option<NodeTypes>,
    isolate_ranges: OnceLock<IsolateRanges>,
    configured_highlights: OnceLock<Option<ConfiguredHighlights>>,
    configured_folds: OnceLock<bool>,
}

impl LezerDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_reuse(text, None)
    }

    /// Apply byte edits and reparse. A valid prefix checkpoint and shifted
    /// suffix decisions can be reused. Invalid edits leave this document
    /// unchanged. If recovery is needed, keep only the preceding strict prefix
    /// as a seed for a later strict reparse.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let reuse = self
            .published
            .as_deref()
            .map(|cache| NormalizedEdges::new(&cache.tree, self.text.len(), changes));
        let mut document = Self::parse_with_reuse(text, reuse);
        document.node_types = self.node_types.clone();
        Ok(document)
    }

    /// LR checkpoint reuse in the last parse. The resume offset is in the
    /// original UTF-8 text, including CRLF. Native views are rebuilt.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.published
            .as_deref()
            .map_or_else(ReuseStats::default, |cache| cache.reuse)
    }

    /// Count the consuming token decisions reused in the shifted suffix.
    pub fn reused_suffix_actions(&self) -> usize {
        self.published
            .as_deref()
            .map_or(0, |cache| cache.tree.reused_suffix_actions())
    }

    pub(crate) fn parse_strict(text: Text) -> Result<Self, ParseError> {
        let nodes = lr::parse_strict(text.as_str())?;
        Ok(Self::from_nodes(text, nodes, None))
    }

    fn parse_with_reuse(text: Text, reuse: Option<NormalizedEdges<'_>>) -> Self {
        let (nodes, published) = lr::parse(text.as_str(), reuse);
        Self::from_nodes(text, nodes, published)
    }

    fn from_nodes(text: Text, nodes: Vec<Node>, published: Option<lr::Cache>) -> Self {
        let highlights = highlight::collect(&nodes);
        Self {
            editor_lines: editor_lines(text.as_str()),
            text,
            nodes,
            highlights,
            published: published.map(Box::new),
            node_types: None,
            isolate_ranges: OnceLock::new(),
            configured_highlights: OnceLock::new(),
            configured_folds: OnceLock::new(),
        }
    }

    pub(crate) fn with_node_types(mut self, node_types: NodeTypes) -> Self {
        self.node_types = Some(node_types);
        self
    }

    pub(super) fn has_configured_folds(&self) -> bool {
        *self
            .configured_folds
            .get_or_init(|| self.node_types().has_different_folds(stock_node_types()))
    }

    pub(crate) fn node_types(&self) -> &NodeTypes {
        match &self.node_types {
            Some(types) => types,
            None => stock_node_types(),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// Text for syntax line queries. Treat lone CR as a break without changing
    /// the original byte offsets. Use [`Self::text`] for original bytes.
    pub fn syntax_lines(&self) -> &Text {
        self.editor_lines.as_ref().unwrap_or(&self.text)
    }

    /// The root spans the full document, including skipped whitespace.
    pub fn root(&self) -> LezerNode<'_> {
        LezerNode {
            document: self,
            id: 0,
        }
    }

    /// All nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = LezerNode<'_>> {
        (0..self.nodes.len()).map(|id| LezerNode { id, ..self.root() })
    }

    /// Build bidi isolation marks for nodes with a configured direction. The
    /// stock grammar defines no isolates. Marks have full node ranges in
    /// original bytes. Invalid viewport endpoints return an error.
    pub fn bidi_isolate_marks(
        &self,
        viewport: &Viewport,
        options: BidiIsolateOptions,
    ) -> Result<Arc<RangeSet<MarkDecoration>>, EditError> {
        let ranges = self.isolate_ranges.get_or_init(|| {
            if self
                .node_types
                .as_ref()
                .is_none_or(|types| types.same_set(stock_node_types()))
            {
                return IsolateRanges::default();
            }
            IsolateRanges::new(self.nodes().filter_map(|node| {
                let direction = (*node.node_type()?.prop(&ISOLATE)?)?;
                Some((node.range(), direction))
            }))
        });
        ranges.marks(&self.text, self.syntax_lines(), viewport, options)
    }

    /// Highlight spans clipped to the supplied byte range. Return `None` if
    /// the range is reversed or either endpoint is invalid.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.text.slice(range.clone())?;
        let configured = self
            .configured_highlights
            .get_or_init(|| {
                // The local fallback has no LR term IDs for its children.
                self.root().data().term?;
                self.node_types
                    .as_ref()
                    .filter(|types| types.has_different_styles(stock_node_types()))
                    .map(|_| ConfiguredHighlights::build(self.root()))
            })
            .as_ref();
        let stock = configured
            .is_none()
            .then(|| self.stock_highlights(range.clone()));
        let configured = configured.map(|spans| spans.clipped(range));
        Some(
            stock
                .into_iter()
                .flatten()
                .chain(configured.into_iter().flatten()),
        )
    }

    fn stock_highlights(&self, range: Range<usize>) -> impl Iterator<Item = HighlightSpan<'_>> {
        let first = self
            .highlights
            .partition_point(|span| span.range.end <= range.start);
        let end = if range.is_empty() {
            first
        } else {
            self.highlights
                .partition_point(|span| span.range.start < range.end)
        };
        self.highlights[first..end]
            .iter()
            .map(move |span| HighlightSpan {
                range: span.range.start.max(range.start)..span.range.end.min(range.end),
                tags: span.tags,
            })
    }

    /// Find the outermost fold that starts on this line and continues past it.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        self.path(line_end, true)
            .filter_map(LezerNode::fold_range)
            .find(|range| (line_start..=line_end).contains(&range.start) && line_end < range.end)
    }

    /// Compute the indentation in columns, using detected braces, brackets,
    /// and parentheses. Return `None` for an invalid UTF-8 byte position.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        indent::indentation(self, position, settings, options)
    }

    fn path(&self, position: usize, include_start: bool) -> impl Iterator<Item = LezerNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let i = children.partition_point(|&id| self.nodes[id].range.end <= position);
            next = children.get(i).and_then(|&id| {
                let start = self.nodes[id].range.start;
                (start < position || (include_start && start == position))
                    .then_some(LezerNode { id, ..node })
            });
            Some(node)
        })
    }
}
