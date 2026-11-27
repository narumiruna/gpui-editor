//! Native Jinja template services from `@codemirror/lang-jinja` 6.0.1.
//! The package contains its own grammar. See `docs/licenses/codemirror-lang-jinja.txt`.
//!
//! Ranges use UTF-8 bytes. The pinned Lezer tables create the Jinja tree.
//! Strict LR parses can reuse prefix checkpoints. Bounded local repair can
//! keep incomplete syntax but does not implement Lezer's full recovery.
//! The crate also provides template completion and a percent-brace transaction builder.

use std::ops::Range;

use crate::{ChangeSet, EditError, Text};

use super::{
    HighlightSpan, IndentOptions, LanguageData, StreamSettings,
    html::HtmlOptions,
    lezer::{ReuseStats, SyntaxTree},
};

mod completion;
mod highlight;
mod overlay;
mod parser;
mod services;
mod tables;
mod tokens;

mod percent_brace;

pub use completion::{
    JinjaCompletionConfig, JinjaPropertyCompletion, jinja_completion_source,
    jinja_completion_source_with,
};
pub use overlay::JinjaHtml;
use parser::Node;
pub use percent_brace::{close_percent_brace, close_percent_brace_in};

pub const NAME: &str = "jinja";
/// Brackets that the upstream provider closes automatically in outer HTML.
pub const HTML_CLOSE_BRACKETS: [char; 1] = ['{'];

/// Upstream metadata for the template language, outside the HTML overlay.
pub fn language_data() -> LanguageData {
    LanguageData::default().with("indentOnInput", indent_on_input as fn(&str) -> bool)
}

/// Upstream metadata for outer HTML in a Jinja template.
pub fn html_language_data() -> LanguageData {
    super::html::language_data().with("closeBrackets", HTML_CLOSE_BRACKETS)
}

pub fn indent_on_input(prefix: &str) -> bool {
    let Some(after) = trim_space(prefix).strip_prefix("{%") else {
        return false;
    };
    matches!(
        trim_space(after.strip_prefix('-').unwrap_or(after)),
        "end" | "elif" | "else"
    )
}

fn trim_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

/// Select the native HTML dialect, or omit the HTML projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JinjaOptions {
    pub html: Option<HtmlOptions>,
}

impl Default for JinjaOptions {
    fn default() -> Self {
        Self {
            html: Some(HtmlOptions::default()),
        }
    }
}

/// A borrowed Jinja syntax node. Zero-width errors mark missing syntax.
#[derive(Clone, Copy, Debug)]
pub struct JinjaNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> JinjaNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    /// The visible node name from the upstream grammar.
    pub fn name(self) -> &'static str {
        self.data().name()
    }

    pub fn range(self) -> Range<usize> {
        self.data().range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }

    pub fn fold_range(self) -> Option<Range<usize>> {
        services::fold_range(self)
    }
}

/// A native Jinja tree and, by default, an HTML tree for its static content.
#[derive(Debug)]
pub struct JinjaDocument {
    text: Text,
    nodes: Vec<Node>,
    html: Option<JinjaHtml>,
    highlights: Vec<HighlightSpan<'static>>,
    recovered: bool,
    options: JinjaOptions,
    strict: Option<Box<SyntaxTree>>,
}

impl JinjaDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_options(text, JinjaOptions::default())
    }

    pub fn parse_with_options(text: Text, options: JinjaOptions) -> Self {
        Self::parse_with_prefix(text, options, None)
    }

    /// Apply UTF-8 byte edits and rebuild the HTML projection with the original options.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let end = changes
            .changes()
            .next()
            .map_or(text.len(), |(range, _)| range.start);
        let prefix = self.strict.as_deref().map(|tree| (tree, end));
        Ok(Self::parse_with_prefix(text, self.options, prefix))
    }

    /// Report strict LR prefix reuse in logical bytes (CRLF counts as one byte).
    /// GLR, local repair, and the HTML projection do not reuse checkpoints.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.strict
            .as_deref()
            .map_or_else(ReuseStats::default, SyntaxTree::reuse_stats)
    }

    fn parse_with_prefix(
        text: Text,
        options: JinjaOptions,
        prefix: Option<(&SyntaxTree, usize)>,
    ) -> Self {
        let (nodes, recovered, strict) = parser::parse(text.as_str(), prefix);
        let root = JinjaNode {
            nodes: &nodes,
            id: 0,
        };
        let highlights = highlight::collect(root);
        let html = options
            .html
            .and_then(|options| JinjaHtml::parse(&text, &nodes, options));
        Self {
            text,
            nodes,
            html,
            highlights,
            recovered,
            options,
            strict: strict.map(Box::new),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn root(&self) -> JinjaNode<'_> {
        JinjaNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// Jinja nodes in pre-order. HTML has a separate native syntax tree.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = JinjaNode<'_>> {
        (0..self.nodes.len()).map(|id| JinjaNode { id, ..self.root() })
    }

    pub fn is_recovered(&self) -> bool {
        self.recovered
    }

    pub fn html(&self) -> Option<&JinjaHtml> {
        self.html.as_ref()
    }

    /// Metadata for the template or outer HTML at this byte position.
    pub fn language_data_at(&self, position: usize) -> Option<LanguageData> {
        self.text.as_str().is_char_boundary(position).then(|| {
            if self
                .html()
                .and_then(|html| html.projected_offset(position))
                .is_some()
            {
                html_language_data()
            } else {
                language_data()
            }
        })
    }

    /// Highlight spans clipped to a valid UTF-8 byte range.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.text.slice(range.clone())?;
        let mut spans = Vec::new();
        if !range.is_empty() {
            let first = self
                .highlights
                .partition_point(|span| span.range.end <= range.start);
            spans.extend(
                self.highlights[first..]
                    .iter()
                    .take_while(|span| span.range.start < range.end)
                    .map(|span| HighlightSpan {
                        range: span.range.start.max(range.start)..span.range.end.min(range.end),
                        tags: span.tags,
                    }),
            );
            if let Some(html) = self.html() {
                html.highlights(&range, &mut spans);
                spans.sort_unstable_by_key(|span| span.range.start);
            }
        }
        Some(spans.into_iter())
    }

    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        services::foldable(self, line_start, line_end)
    }

    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        services::indentation(self, position, settings, options)
    }

    fn path(
        &self,
        position: usize,
        include_start: bool,
        include_unfinished: bool,
    ) -> impl Iterator<Item = JinjaNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let index = children.partition_point(|&id| self.nodes[id].range.end <= position);
            let candidate = children.get(index).filter(|&&id| {
                let start = self.nodes[id].range.start;
                start < position || (include_start && start == position)
            });
            let candidate = candidate.or_else(|| {
                let id = children.get(index.checked_sub(1)?)?;
                let previous = JinjaNode { id: *id, ..node };
                (include_unfinished
                    && previous.data().range.end <= position
                    && previous.children().next_back().is_some_and(|child| {
                        child.name() == "⚠" && child.data().range.end == previous.data().range.end
                    }))
                .then_some(id)
            });
            next = candidate.map(|&id| JinjaNode { id, ..node });
            Some(node)
        })
    }
}
