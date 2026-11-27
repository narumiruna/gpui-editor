//! Native Rust language support based on `@codemirror/lang-rust` 6.0.2 and
//! `@lezer/rust` 1.0.3. See `docs/licenses/codemirror-lang-rust.txt`.
//!
//! Strict LR parses of the pinned grammar can reuse a validated prefix
//! checkpoint. GLR and bounded recovery start from scratch. For syntax errors
//! in inputs of at most 16 KiB, recovery can keep strict prefix checkpoints for
//! a later edit. A local token tree serves as the fallback. Recovery can differ
//! from upstream. Ranges are UTF-8 bytes in the original text.

use std::ops::Range;

use crate::{Assoc, ChangeSet, EditError, Text};

use super::{HighlightSpan, IndentOptions, StreamSettings, lezer::ReuseStats};

mod brackets;
mod highlight;
mod indent;
mod kind;
mod lr;
mod recovery;
mod tables;
mod tokens;

pub use kind::RustNodeKind;

/// The upstream language name.
pub const NAME: &str = "rust";
pub const LINE_COMMENT: &str = "//";
pub const BLOCK_COMMENT: (&str, &str) = ("/*", "*/");
pub const STRING_PREFIXES: [&str; 3] = ["b", "r", "br"];

/// Whether the line from its start to the cursor triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    matches!(trim_js_space(line_prefix), "{" | "}")
}

fn trim_js_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

#[derive(Debug)]
struct Node {
    kind: RustNodeKind,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
    statement: bool,
}

/// A borrowed Rust syntax node. An empty error range marks missing syntax.
#[derive(Clone, Copy, Debug)]
pub struct RustNode<'a> {
    source: &'a str,
    nodes: &'a [Node],
    id: usize,
}

impl<'a> RustNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    pub fn kind(self) -> RustNodeKind {
        self.data().kind
    }

    pub fn range(self) -> Range<usize> {
        self.data().range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    /// Direct children in source order, excluding whitespace.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }

    /// The fold range for a node with an upstream fold rule.
    pub fn fold_range(self) -> Option<Range<usize>> {
        let name = self.kind().name();
        if self.kind() == RustNodeKind::BlockComment {
            // The pinned language subtracts two logical UTF-16 units even
            // on an unfinished comment. Do not expose half of a scalar.
            let range = &self.data().range;
            let start = range.start + 2;
            let mut end = range.end;
            let mut units = 2usize;
            while units != 0 {
                let ch = self.source.get(range.start..end)?.chars().next_back()?;
                units = units.checked_sub(ch.len_utf16())?;
                end -= ch.len_utf8();
                if ch == '\n' && end > range.start && self.source.as_bytes()[end - 1] == b'\r' {
                    end -= 1;
                }
            }
            return (start <= end).then_some(start..end);
        }
        if !(name.ends_with("Block") || name.ends_with("edTokens") || name.ends_with("List")) {
            return None;
        }
        let first = self.children().next()?;
        let last = self.children().next_back()?;
        let start = first.data().range.end;
        (start < last.data().range.start).then(|| {
            start..if last.kind() == RustNodeKind::Error {
                self.data().range.end
            } else {
                last.data().range.start
            }
        })
    }
}

/// A Rust document and its native syntax tree.
#[derive(Debug)]
pub struct RustDocument {
    text: Text,
    editor_lines: Option<Text>,
    nodes: Vec<Node>,
    highlights: Vec<HighlightSpan<'static>>,
    recovered: bool,
    checkpoint: Option<Box<lr::Cache>>,
}

impl RustDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_prefix(text, None)
    }

    /// Apply byte edits and parse the result. Reuse a validated LR checkpoint
    /// before the first edit when possible. Only strict LR checkpoints can
    /// seed a later parse. Invalid edits leave this document unchanged.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let end = changes
            .changes()
            .next()
            .map_or(text.len(), |(range, _)| range.start);
        let prefix = self.checkpoint.as_deref().map(|cache| (cache, end));
        Ok(Self::parse_with_prefix(text, prefix))
    }

    /// LR reuse in the last parse. The resume offset is in original UTF-8 bytes,
    /// including CRLF. Recovery reports no reuse. Native nodes and highlights
    /// are rebuilt.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.checkpoint
            .as_deref()
            .map_or_else(ReuseStats::default, |cache| cache.reuse)
    }

    fn parse_with_prefix(text: Text, prefix: Option<(&lr::Cache, usize)>) -> Self {
        let source = text.as_str();
        let (parsed, local) = match lr::parse(source, prefix) {
            Ok(parsed) => (parsed, false),
            Err(_) => (
                lr::Parsed {
                    nodes: recovery::parse(source),
                    cache: None,
                    recovered: true,
                },
                true,
            ),
        };
        let highlights = highlight::collect(&parsed.nodes, local);
        Self {
            editor_lines: super::line_projection::editor_lines(text.as_str()),
            text,
            nodes: parsed.nodes,
            highlights,
            recovered: parsed.recovered,
            checkpoint: parsed.cache.map(Box::new),
        }
    }

    /// Whether the parser used bounded syntax recovery or the local token fallback.
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

    /// The root includes all document text, including skipped whitespace.
    pub fn root(&self) -> RustNode<'_> {
        RustNode {
            source: self.text.as_str(),
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// All nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = RustNode<'_>> {
        (0..self.nodes.len()).map(|id| RustNode { id, ..self.root() })
    }

    /// Highlight spans clipped to a byte range. Return `None` if either
    /// endpoint is invalid or if the range is reversed.
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

    /// Find a fold that starts on this line and continues past it.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        let node = self.path(line_end, Some(Assoc::After)).last()?;
        let mut found = None;
        for node in std::iter::successors(Some(node), |node| node.parent()) {
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
    /// positions or where upstream preserves existing indentation.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        indent::indentation(self, position, settings, options)
    }

    fn path(&self, position: usize, side: Option<Assoc>) -> impl Iterator<Item = RustNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let i = children.partition_point(|&id| {
                let end = self.nodes[id].range.end;
                end < position || (end == position && side != Some(Assoc::Before))
            });
            next = children.get(i).and_then(|&id| {
                let range = &self.nodes[id].range;
                (!range.is_empty()
                    && (range.start < position
                        || (side == Some(Assoc::After) && range.start == position)))
                    .then_some(RustNode { id, ..node })
            });
            Some(node)
        })
    }
}
