//! Less language services from `@codemirror/lang-less` 6.0.2.
//!
//! The package ships its own Lezer grammar, with no mounted languages. This
//! port uses its published tables with bounded GLR and local repair. If repair
//! cannot finish within its limits, the stylesheet contains one error node.
//! Strict LR parses can reuse prefix checkpoints. Recovery is not incremental
//! and does not reproduce Lezer's exact recovery.
//! See `docs/licenses/codemirror-lang-less.txt`.

use std::ops::Range;

use crate::{ChangeSet, EditError, Text};

use super::{
    HighlightSpan, IndentOptions, StreamSettings,
    lezer::{ReuseStats, SyntaxTree},
};

mod completion;
mod highlight;
mod indent;
mod parser;
mod recovery;
mod tables;

pub use completion::{less_completion_source, less_completion_source_with_properties};

pub const NAME: &str = "less";
pub const BLOCK_COMMENT: (&str, &str) = ("/*", "*/");
pub const LINE_COMMENT: &str = "//";
pub const WORD_CHARS: &str = "@-";

/// Whether the text from the start of a line to the cursor triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    trim_js_space(line_prefix) == "}"
}

fn trim_js_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

macro_rules! node_kinds {
    ($($plain:ident,)*; $($named:ident => $name:literal,)*) => {
        /// The visible node types of the published Less grammar.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum LessNodeKind { $($plain,)* $($named,)* }

        impl LessNodeKind {
            pub const fn name(self) -> &'static str {
                match self { $(Self::$plain => stringify!($plain),)* $(Self::$named => $name,)* }
            }

            fn from_name(name: &str) -> Self {
                match name {
                    $(stringify!($plain) => Self::$plain,)*
                    $($name => Self::$named,)*
                    _ => unreachable!("unknown node in the pinned Less grammar: {name}"),
                }
            }
        }
    };
}

node_kinds! {
    Unit, Comment, LineComment, StyleSheet, RuleSet, UniversalSelector, TagSelector, TagName,
    NestingSelector, ClassSelector, ClassName, PseudoClassSelector, PseudoClassName,
    ArgList, VariableName, AtKeyword, PropertyVariable, ValueName, ParenthesizedValue,
    ColorLiteral, NumberLiteral, StringLiteral, Escape, Interpolation, BinaryExpression,
    BinOp, LogicOp, UnaryExpression, UnaryQueryOp, CallExpression, SubscriptExpression,
    CallLiteral, CallTag, ParenthesizedContent, IdSelector, IdName, AttributeSelector,
    AttributeName, MatchOp, ChildSelector, ChildOp, DescendantSelector, SiblingSelector,
    SiblingOp, InterpolatedSelector, Block, ImportStatement, KeywordQuery, FeatureQuery,
    FeatureName, BinaryQuery, UnaryQuery, ParenthesizedQuery, SelectorQuery, CallQuery,
    SubscriptQuery, MediaStatement, CharsetStatement, NamespaceStatement, NamespaceName,
    KeyframesStatement, KeyframeName, KeyframeList, SupportsStatement, DetachedRuleSet,
    PropertyName, Declaration, Important, Inclusion,
    ;
    Error => "⚠", Colon => ":", DoubleColon => "::", Semicolon => ";", Comma => ",",
    CloseParen => ")", OpenParen => "(", CloseBracket => "]", OpenBracket => "[",
    CloseBrace => "}", OpenBrace => "{", Hash => "#",
    When => "when", Import => "import", Selector => "selector", Media => "media",
    Charset => "charset", Namespace => "namespace", Keyframes => "keyframes", From => "from",
    To => "to", Supports => "supports",
}

#[derive(Debug)]
struct Node {
    kind: LessNodeKind,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
    // The grammar has two `(` terms, but only the regular one has `closedBy`.
    closing: Option<LessNodeKind>,
}

/// A borrowed Less syntax node. All ranges refer to UTF-8 bytes in the source.
#[derive(Clone, Copy, Debug)]
pub struct LessNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> LessNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    pub fn kind(self) -> LessNodeKind {
        self.data().kind
    }

    pub fn range(self) -> Range<usize> {
        self.data().range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    /// Direct children in source order, including comments but not whitespace.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }

    /// Fold the content of a block, but not a keyframe list.
    pub fn fold_range(self) -> Option<Range<usize>> {
        if self.kind() != LessNodeKind::Block {
            return None;
        }
        let first = self.children().next()?;
        let last = self.children().next_back()?;
        let end = if last.kind() == LessNodeKind::Error {
            self.data().range.end
        } else {
            last.data().range.start
        };
        let start = first.data().range.end;
        (start < last.data().range.start).then_some(start..end)
    }
}

/// A Less stylesheet and its syntax tree.
#[derive(Debug)]
pub struct LessDocument {
    text: Text,
    nodes: Vec<Node>,
    highlights: Vec<HighlightSpan<'static>>,
    strict: Option<Box<SyntaxTree>>,
}

impl LessDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_prefix(text, None)
    }

    /// Apply UTF-8 byte edits and parse the result. Invalid edits leave this
    /// document unchanged. Only strict LR prefixes can be reused.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let end = changes
            .changes()
            .next()
            .map_or(text.len(), |(range, _)| range.start);
        let prefix = self.strict.as_deref().map(|tree| (tree, end));
        Ok(Self::parse_with_prefix(text, prefix))
    }

    /// Report LR checkpoint reuse, not reuse of the flattened editor tree.
    /// The resume offset is in logical input bytes (CRLF counts as one byte).
    /// GLR and recovery report no reuse.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.strict
            .as_deref()
            .map_or_else(ReuseStats::default, SyntaxTree::reuse_stats)
    }

    fn parse_with_prefix(text: Text, prefix: Option<(&SyntaxTree, usize)>) -> Self {
        let (nodes, strict) = parser::parse(text.as_str(), prefix);
        let highlights = highlight::collect(&nodes);
        Self {
            text,
            nodes,
            highlights,
            strict: strict.map(Box::new),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// The root covers the full document, including whitespace.
    pub fn root(&self) -> LessNode<'_> {
        LessNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// All nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = LessNode<'_>> {
        (0..self.nodes.len()).map(|id| LessNode { id, ..self.root() })
    }

    /// Return spans clipped to the requested range or `None` for invalid endpoints.
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
        self.path(line_end, Side::Right)
            .filter_map(LessNode::fold_range)
            .find(|range| (line_start..=line_end).contains(&range.start) && line_end < range.end)
    }

    /// Compute indentation in columns at a valid UTF-8 byte position.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        indent::indentation(self, position, settings, options)
    }

    fn path(&self, position: usize, side: Side) -> impl Iterator<Item = LessNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let i = children.partition_point(|&id| {
                let end = self.nodes[id].range.end;
                end < position || (side != Side::Left && end == position)
            });
            next = children.get(i).and_then(|&id| {
                let start = self.nodes[id].range.start;
                (start < position || (side == Side::Right && start == position))
                    .then_some(LessNode { id, ..node })
            });
            Some(node)
        })
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Left,
    Inner,
    Right,
}
