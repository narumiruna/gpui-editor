//! Native language services from `@codemirror/lang-sass` 6.0.2 and
//! `@lezer/sass` 1.1.0. See `docs/licenses/codemirror-lang-sass.txt`.
//!
//! Both dialects use the published LR tables and native token hooks. Positions
//! are UTF-8 bytes into the original text. Strict LR parses can reuse prefix
//! checkpoints. On invalid input, bounded repairs can preserve a prefix. The
//! remaining text is an error node. This does not reproduce Lezer's recovery. The
//! parser limits input to 1 MiB, repairs to 64 KiB, and indentation nesting
//! to 256 levels.

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
mod tables;
mod tokens;

pub use completion::{sass_completion_source, sass_completion_source_with_properties};

pub const NAME: &str = "sass";
pub const LINE_COMMENT: &str = "//";
pub const BLOCK_COMMENT: (&str, &str) = ("/*", "*/");
pub const WORD_CHARS: &str = "$-";

/// Whether typing the line prefix triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    super::css::indent_on_input(line_prefix)
}

/// The upstream Sass parser dialect. The default uses braces and semicolons.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SassDialect {
    #[default]
    Scss,
    Indented,
}

#[derive(Debug)]
struct Node {
    term: u16,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A borrowed node of the pinned Sass grammar.
#[derive(Clone, Copy, Debug)]
pub struct SassNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> SassNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    /// The published grammar's visible name, including punctuation.
    pub fn name(self) -> &'static str {
        parser::name(self.data().term)
    }

    pub fn is_error(self) -> bool {
        self.data().term == 0
    }

    pub fn range(self) -> Range<usize> {
        self.data().range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    /// The direct children in source order. Whitespace is not included.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }
}

/// A Sass or SCSS stylesheet, its tree, and its editor services.
#[derive(Debug)]
pub struct SassDocument {
    text: Text,
    dialect: SassDialect,
    nodes: Vec<Node>,
    highlights: Vec<HighlightSpan<'static>>,
    strict: Option<Box<SyntaxTree>>,
}

impl SassDocument {
    pub fn parse(text: Text, dialect: SassDialect) -> Self {
        Self::parse_with_prefix(text, dialect, None)
    }

    /// Apply UTF-8 byte edits and parse with the same dialect. Invalid edits
    /// leave this document unchanged. Only strict LR prefixes can be reused.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let end = changes
            .changes()
            .next()
            .map_or(text.len(), |(range, _)| range.start);
        let prefix = self.strict.as_deref().map(|tree| (tree, end));
        Ok(Self::parse_with_prefix(text, self.dialect, prefix))
    }

    /// Report LR checkpoint reuse, not reuse of the flattened editor tree.
    /// The resume offset is in logical input bytes (CRLF counts as one byte).
    /// GLR and recovery report no reuse.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.strict
            .as_deref()
            .map_or_else(ReuseStats::default, SyntaxTree::reuse_stats)
    }

    fn parse_with_prefix(
        text: Text,
        dialect: SassDialect,
        prefix: Option<(&SyntaxTree, usize)>,
    ) -> Self {
        let (nodes, strict) = parser::parse(text.as_str(), dialect, prefix);
        let highlights = highlight::collect(&nodes);
        Self {
            text,
            dialect,
            nodes,
            highlights,
            strict: strict.map(Box::new),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn dialect(&self) -> SassDialect {
        self.dialect
    }

    /// The root spans the full source, including skipped whitespace.
    pub fn root(&self) -> SassNode<'_> {
        SassNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// All visible nodes in pre-order, including the root.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = SassNode<'_>> {
        (0..self.nodes.len()).map(|id| SassNode { id, ..self.root() })
    }

    /// Highlight spans clipped to a valid range in the original text.
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

    /// The outermost block or comment fold that starts on this line.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        self.path(line_end, 1)
            .filter_map(|node| self.fold_range(node))
            .find(|range| (line_start..=line_end).contains(&range.start) && line_end < range.end)
    }

    /// Compute the indentation in columns at a valid source byte boundary.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        indent::indentation(self, position, settings, options)
    }

    fn fold_range(&self, node: SassNode<'_>) -> Option<Range<usize>> {
        match node.name() {
            "Block" if self.dialect == SassDialect::Indented => Some(node.range()),
            "Block" => {
                let start = node.children().next()?.range().end;
                let last = node.children().next_back()?;
                (start < last.range().start).then(|| {
                    start..if last.is_error() {
                        node.range().end
                    } else {
                        last.range().start
                    }
                })
            }
            "Comment" => {
                let range = node.range();
                let closed = self.text.slice(range.clone())?.ends_with("*/");
                let start = range.start + 2;
                let end = range.end - if closed { 2 } else { 0 };
                (start < end).then_some(start..end)
            }
            _ => None,
        }
    }

    fn path(&self, position: usize, side: i8) -> impl Iterator<Item = SassNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let i = children.partition_point(|&id| {
                let end = self.nodes[id].range.end;
                end < position || side >= 0 && end == position
            });
            next = children.get(i).and_then(|&id| {
                let start = self.nodes[id].range.start;
                (start < position || side > 0 && start == position)
                    .then_some(SassNode { id, ..node })
            });
            Some(node)
        })
    }
}
