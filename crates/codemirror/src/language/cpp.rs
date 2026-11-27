//! Native C and C++ editor services from `@codemirror/lang-cpp` 6.0.3 and
//! `@lezer/cpp` 1.1.6. See `docs/licenses/codemirror-lang-cpp.txt`.
//!
//! The document uses the published grammar and shared bounded LR/GLR runtime.
//! Strict LR parses can reuse a validated prefix checkpoint. GLR and recovery
//! start from scratch. For small malformed documents, bounded Lezer recovery
//! can retain a private strict prefix checkpoint for the next edit. A local
//! fallback can close unfinished nodes, skip one character, or keep a prefix.
//! C and C++ use the same grammar; no preprocessor runs. All ranges are UTF-8
//! bytes in the original document.

use std::ops::Range;

use crate::{Assoc, ChangeSet, EditError, Text};

use super::{HighlightSpan, IndentOptions, StreamSettings, lezer::ReuseStats};

mod brackets;
mod highlight;
mod indent;
mod kind;
mod parser;
mod recovery;
mod tables;
mod tokens;

pub use kind::CppNodeKind;

pub const NAME: &str = "cpp";
pub const LINE_COMMENT: &str = "//";
pub const BLOCK_COMMENT: (&str, &str) = ("/*", "*/");
pub const STRING_PREFIXES: [&str; 9] = ["L", "u", "U", "u8", "LR", "UR", "uR", "u8R", "R"];

/// Whether the text from the start of the line to the cursor triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    matches!(trim_js_space(line_prefix), "case " | "default:" | "{" | "}")
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

/// A borrowed grammar node. An empty error node can mark missing syntax.
#[derive(Clone, Copy, Debug)]
pub struct CppNode<'a> {
    doc: &'a CppDocument,
    id: usize,
}

impl<'a> CppNode<'a> {
    fn data(self) -> &'a Node {
        &self.doc.nodes[self.id]
    }

    pub fn kind(self) -> CppNodeKind {
        CppNodeKind::from_term(self.data().term)
    }

    pub fn range(self) -> Range<usize> {
        self.data().range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    /// Direct children in source order. Whitespace and anonymous grammar terms are omitted.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }

    /// The fold range for a node with an upstream fold rule.
    pub fn fold_range(self) -> Option<Range<usize>> {
        use CppNodeKind::*;
        let range = &self.data().range;
        if self.kind() == BlockComment {
            // Upstream removes two UTF-16 units at the end, even when the
            // terminator is missing. Do not split CRLF or a surrogate pair.
            let start = range.start + 2;
            let mut end = range.end;
            let mut units = 2usize;
            let source = self.doc.text().as_str();
            while units != 0 && end > start {
                if source[..end].ends_with("\r\n") {
                    end -= 2;
                    units -= 1;
                } else {
                    let ch = source[..end].chars().next_back()?;
                    end -= ch.len_utf8();
                    units = units.saturating_sub(ch.len_utf16());
                }
            }
            return (units == 0).then_some(start..end);
        }
        if !(matches!(
            self.kind(),
            CompoundStatement | EnumeratorList | FieldDeclarationList | InitializerList
        ) || self.kind().name() == "DeclarationList")
        {
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

/// A C or C++ document and its native syntax tree.
#[derive(Debug)]
pub struct CppDocument {
    text: Text,
    editor_lines: Option<Text>,
    nodes: Vec<Node>,
    highlights: Vec<HighlightSpan<'static>>,
    recovered: bool,
    strict: Option<Box<parser::Cache>>,
}

impl CppDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_prefix(text, None)
    }

    /// Apply byte edits and parse the result. Reuse a validated LR checkpoint
    /// before the first edit when possible. A recovered tree can keep strict
    /// checkpoints but does not reuse recovered nodes. GLR keeps no checkpoints.
    /// Invalid edits leave this document unchanged.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let end = changes
            .changes()
            .next()
            .map_or(text.len(), |(range, _)| range.start);
        let prefix = self.strict.as_deref().map(|cache| (cache, end));
        Ok(Self::parse_with_prefix(text, prefix))
    }

    /// LR reuse in the last parse. The resume offset is in original UTF-8 bytes,
    /// including CRLF. GLR and recovery report no reuse. Native nodes and
    /// highlights are rebuilt.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.strict
            .as_deref()
            .map_or_else(ReuseStats::default, |cache| cache.reuse)
    }

    fn parse_with_prefix(text: Text, prefix: Option<(&parser::Cache, usize)>) -> Self {
        let (nodes, recovered, strict) = parser::parse(text.as_str(), prefix);
        let mut doc = Self {
            editor_lines: super::line_projection::editor_lines(text.as_str()),
            text,
            nodes,
            highlights: Vec::new(),
            recovered,
            strict: strict.map(Box::new),
        };
        doc.highlights = highlight::collect(doc.root());
        doc
    }

    /// Whether strict parsing and GLR failed to parse the document.
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

    /// The root covers all text, including skipped whitespace.
    pub fn root(&self) -> CppNode<'_> {
        CppNode { doc: self, id: 0 }
    }

    /// All nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl Iterator<Item = CppNode<'_>> {
        let mut pending = vec![self.root()];
        std::iter::from_fn(move || {
            let node = pending.pop()?;
            pending.extend(node.children().rev());
            Some(node)
        })
    }

    /// Highlight spans clipped to the byte range, or `None` for an invalid range.
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

    /// Find a fold that starts on this line and continues after its end.
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

    /// Return the indentation in columns, or `None` to preserve it or if the position is invalid.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        indent::indentation(self, position, settings, options)
    }

    fn path(&self, position: usize, side: Option<Assoc>) -> impl Iterator<Item = CppNode<'_>> {
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
                    .then_some(CppNode { id, ..node })
            });
            Some(node)
        })
    }
}
