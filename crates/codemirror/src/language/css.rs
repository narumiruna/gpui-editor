//! CSS language services from `@codemirror/lang-css` 6.3.1 and `@lezer/css`
//! 1.3.9. See `docs/licenses/codemirror-lang-css.txt`.
//!
//! The native parser uses the published Lezer tables and bounded GLR, with
//! local recovery on errors. Strict LR parses can reuse prefix checkpoints.
//! Recovery is not incremental and does not reproduce Lezer's exact recovery.
//! Ranges are UTF-8 byte offsets into the original text, including CRLF.

use std::{ops::Range, sync::OnceLock};

use crate::{ChangeSet, EditError, Text};

use super::{HighlightSpan, IndentOptions, StreamSettings, lezer::ReuseStats};

mod brackets;
mod completion;
mod highlight;
mod indent;
pub(super) mod input;
mod lexer;
mod lr;
mod recovery;
mod tables;
mod tokens;

pub use completion::{css_completion_source, define_css_completion_source};

/// The name from the upstream language provider.
pub const NAME: &str = "css";
/// The block comment delimiters from the upstream language data.
pub const BLOCK_COMMENT: (&str, &str) = ("/*", "*/");
/// Extra characters that count as part of a word in CSS.
pub const WORD_CHARS: &str = "-";

/// Whether the text from a line's start to the cursor triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    trim_js_space(line_prefix) == "}"
}

fn trim_js_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

macro_rules! node_kinds {
    ($($plain:ident,)*; $($named:ident => $name:literal,)*) => {
        /// The visible node types of the CSS grammar.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum CssNodeKind { $($plain,)* $($named,)* }

        impl CssNodeKind {
            /// The name of the node in the upstream syntax tree.
            pub const fn name(self) -> &'static str {
                match self { $(Self::$plain => stringify!($plain),)* $(Self::$named => $name,)* }
            }

            fn from_name(name: &str) -> Self {
                match name {
                    $(stringify!($plain) => Self::$plain,)*
                    $($name => Self::$named,)*
                    _ => unreachable!("unknown node in the pinned CSS grammar: {name}"),
                }
            }
        }
    };
}

node_kinds! {
    Unit, VariableName, QueryCallee, Comment, StyleSheet, RuleSet, UniversalSelector,
    TagSelector, TagName, NamespacedTagSelector, NamespaceName, NestingSelector,
    ClassSelector, ClassName, PseudoClassSelector, PseudoClassName, ArgList, ValueName,
    ParenthesizedValue, AtKeyword, BracketedValue, BracedValue, ColorLiteral, NumberLiteral,
    StringLiteral, BinaryExpression, BinOp, CallExpression, Callee, IfExpression, IfBranch,
    KeywordQuery, FeatureQuery, FeatureName, BinaryQuery, LogicOp, ComparisonQuery, CompareOp,
    UnaryQuery, UnaryQueryOp, ParenthesizedQuery, SelectorQuery, ParenthesizedSelector,
    StyleQuery, ParenthesedQuery, CallQuery, PropertyName, StyleFeature, StyleRange, PseudoQuery,
    CallLiteral, CallTag, ParenthesizedContent, IdSelector, IdName, AttributeSelector,
    AttributeName, NamespacedAttribute, MatchOp, MatchFlag, ChildSelector, ChildOp,
    DescendantSelector, SiblingSelector, SiblingOp, Block, Declaration, Important,
    ImportStatement, Layer, LayerName, MediaStatement, CharsetStatement, NamespaceStatement,
    KeyframesStatement, KeyframeName, KeyframeList, KeyframeSelector, KeyframeRangeName,
    SupportsStatement, ScopeStatement, FontFeatureStatement, FontName, AtRule, Styles,
    ;
    Error => "⚠", Dot => ".", Colon => ":", DoubleColon => "::", Semicolon => ";", Comma => ",",
    CloseParen => ")", OpenParen => "(", CloseBracket => "]", OpenBracket => "[",
    CloseBrace => "}", OpenBrace => "{", Pipe => "|",
    If => "if", Selector => "selector", Style => "style", Import => "import", LayerKeyword => "layer",
    Media => "media", Charset => "charset", Namespace => "namespace", Keyframes => "keyframes",
    Supports => "supports", Scope => "scope", To => "to", FontFeatureValues => "font-feature-values",
}

#[derive(Debug)]
struct Node {
    kind: CssNodeKind,
    term: Option<u16>,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A borrowed CSS syntax node. A zero-width error can mark missing syntax.
#[derive(Clone, Copy, Debug)]
pub struct CssNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> CssNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    pub fn kind(self) -> CssNodeKind {
        self.data().kind
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

    /// The range between the braces of a block or keyframe list.
    pub fn fold_range(self) -> Option<Range<usize>> {
        use CssNodeKind::*;
        if !matches!(self.kind(), Block | KeyframeList) {
            return None;
        }
        let first = self.children().next()?;
        let last = self.children().next_back()?;
        let from = first.data().range.end;
        let to = last.data().range.start;
        (from < to).then(|| {
            from..if last.kind() == Error {
                self.data().range.end
            } else {
                to
            }
        })
    }
}

/// One CSS stylesheet and its syntax tree.
#[derive(Debug)]
pub struct CssDocument {
    text: Text,
    nodes: Vec<Node>,
    bracket_lines: OnceLock<Option<Text>>,
    highlights: Vec<HighlightSpan<'static>>,
    mode: lr::Mode,
    strict: Option<Box<lr::Strict>>,
}

impl CssDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_prefix(text, lr::Mode::Sheet, None)
    }

    /// Parse a CSS declaration list, such as an HTML `style` attribute.
    pub fn parse_styles(text: Text) -> Self {
        Self::parse_with_prefix(text, lr::Mode::Styles, None)
    }

    /// Apply UTF-8 byte edits and parse with the same start rule. Invalid edits
    /// leave this document unchanged. Only strict LR prefixes can be reused.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let end = changes
            .changes()
            .next()
            .map_or(text.len(), |(range, _)| range.start);
        let prefix = self.strict.as_deref().map(|strict| (strict, end));
        Ok(Self::parse_with_prefix(text, self.mode, prefix))
    }

    /// Report LR checkpoint reuse, not reuse of the flattened editor tree.
    /// The resume offset is in logical input bytes (CRLF counts as one byte).
    /// GLR, recovery, and a new color specialization table report no reuse.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.strict
            .as_deref()
            .map_or_else(ReuseStats::default, lr::Strict::reuse_stats)
    }

    fn parse_with_prefix(text: Text, mode: lr::Mode, prefix: Option<(&lr::Strict, usize)>) -> Self {
        let source = text.as_str();
        let (nodes, strict) = lr::parse_incremental(source, mode, prefix).unwrap_or_else(|_| {
            let nodes = match mode {
                lr::Mode::Sheet => recovery::parse(source),
                lr::Mode::Styles => recovery::parse_inline(source),
            };
            (nodes, None)
        });
        let highlights = highlight::collect(&nodes);
        Self {
            text,
            nodes,
            bracket_lines: OnceLock::new(),
            highlights,
            mode,
            strict: strict.map(Box::new),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// The root spans the full document, including skipped whitespace.
    pub fn root(&self) -> CssNode<'_> {
        CssNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// All nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = CssNode<'_>> {
        (0..self.nodes.len()).map(|id| CssNode { id, ..self.root() })
    }

    /// Highlight spans clipped to the requested byte range. Return `None`
    /// when the range is reversed or an endpoint is invalid.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.text.slice(range.clone())?;
        let start = self
            .highlights
            .partition_point(|span| span.range.end <= range.start);
        let end = if range.is_empty() {
            start
        } else {
            self.highlights
                .partition_point(|span| span.range.start < range.end)
        };
        Some(
            self.highlights[start..end]
                .iter()
                .map(move |span| HighlightSpan {
                    range: span.range.start.max(range.start)..span.range.end.min(range.end),
                    tags: span.tags,
                }),
        )
    }

    /// Find the outermost fold that starts on this line and continues past it.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        self.path(line_end, true)
            .filter_map(CssNode::fold_range)
            .find(|range| (line_start..=line_end).contains(&range.start) && line_end < range.end)
    }

    /// Compute the indentation in columns. Return `None` when the position
    /// is not a valid byte boundary.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        indent::indentation(self, position, settings, options)
    }

    fn path(&self, position: usize, include_start: bool) -> impl Iterator<Item = CssNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let i = children.partition_point(|&id| self.nodes[id].range.end <= position);
            next = children.get(i).and_then(|&id| {
                let start = self.nodes[id].range.start;
                (start < position || (include_start && start == position))
                    .then_some(CssNode { id, ..node })
            });
            Some(node)
        })
    }
}
