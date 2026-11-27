//! Native Python language services based on `@codemirror/lang-python` 6.2.1
//! and `@lezer/python` 1.1.20. See `docs/licenses/codemirror-lang-python.txt`.
//!
//! The parser uses the published tables and native contextual tokenizers.
//! Positions are UTF-8 byte offsets in the original text. The strict parser
//! runs on documents up to 1 MiB and tracks up to 256 nested contexts, subject
//! to the shared LR and GLR work limits. For unexpected tokens in documents up
//! to 64 KiB, it can retry with closing syntax or retain a complete prefix.
//! Lexical highlights cover any unparsed text; they do not produce syntax
//! nodes or the same contextual tags as the full parse.
//!
//! Deterministic LR parses can reuse checkpoints in an unchanged prefix.
//! GLR and recovery run from scratch. Recovery does not match upstream exactly.
//! Completion includes syntax-scoped local names, global names, and snippets.

use std::ops::Range;

use crate::{Assoc, ChangeSet, EditError, Text};

use super::{HighlightSpan, IndentOptions, StreamSettings, lezer::ReuseStats};

mod brackets;
mod completion;
mod fallback;
mod highlight;
mod indent;
mod parser;
mod quotes;
mod tables;
mod tokens;

pub use completion::{
    PythonSnippet, SNIPPETS, global_completion_source, local_completion_source,
    python_completion_snippet,
};

/// The upstream language name.
pub const NAME: &str = "python";
pub const LINE_COMMENT: &str = "#";
pub const CLOSE_BRACKETS: [&str; 7] = ["(", "[", "{", "'", "\"", "'''", "\"\"\""];
pub const STRING_PREFIXES: [&str; 16] = [
    "f", "fr", "rf", "r", "u", "b", "br", "rb", "F", "FR", "RF", "R", "U", "B", "BR", "RB",
];

/// Whether the line from its start to the cursor triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    let line = trim_space(line_prefix);
    matches!(
        line,
        "}" | "]" | ")" | "else:" | "elif " | "except " | "finally:"
    ) || line.strip_prefix("case").is_some_and(|tail| {
        tail.starts_with(js_space) && !tail.strip_suffix(':').unwrap_or(tail).contains(':')
    })
}

fn js_space(ch: char) -> bool {
    (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}'
}

fn trim_space(text: &str) -> &str {
    text.trim_start_matches(js_space)
}

#[derive(Debug)]
struct Node {
    term: u16,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A node in the native Python syntax tree.
#[derive(Clone, Copy, Debug)]
pub struct PythonNode<'a> {
    doc: &'a PythonDocument,
    id: usize,
}

impl<'a> PythonNode<'a> {
    fn data(self) -> &'a Node {
        &self.doc.nodes[self.id]
    }

    /// The name from the pinned grammar. `⚠` marks a native recovery region.
    pub fn name(self) -> &'static str {
        parser::name(self.data().term)
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

    /// The fold range for a node with an upstream fold rule.
    pub fn fold_range(self) -> Option<Range<usize>> {
        let range = &self.data().range;
        let result = match self.name() {
            "ArrayExpression" | "DictionaryExpression" | "SetExpression" | "TupleExpression" => {
                let first = self.children().next()?;
                let last = self.children().next_back()?;
                first.data().range.end..last.data().range.start
            }
            "Body" => {
                let text = self.doc.text.as_str();
                let end = if range.end == text.len() {
                    range.end
                } else {
                    let prefix = &text[..range.end];
                    prefix.strip_suffix("\r\n").map_or_else(
                        || prefix.char_indices().next_back().map_or(0, |(at, _)| at),
                        str::len,
                    )
                };
                (range.start + 1)..end
            }
            "String" | "FormatString" => self.doc.text.line_at(range.start)?.range.end..range.end,
            _ => return None,
        };
        (result.start < result.end).then_some(result)
    }
}

/// One Python document with a native syntax tree and highlight spans.
#[derive(Debug)]
pub struct PythonDocument {
    text: Text,
    editor_lines: Option<Text>,
    nodes: Vec<Node>,
    highlights: Vec<HighlightSpan<'static>>,
    recovered: bool,
    strict: Option<Box<parser::Cache>>,
}

impl PythonDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_prefix(text, None)
    }

    /// Apply byte edits and parse the new document. Reuse a validated LR
    /// checkpoint before the first edit when possible. Invalid edits leave
    /// this document unchanged. GLR and recovered trees do not keep checkpoints.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let first_edit = changes.changes().next().map(|(range, _)| range.start);
        let prefix = self.strict.as_deref().map(|cache| (cache, first_edit));
        Ok(Self::parse_with_prefix(text, prefix))
    }

    /// LR checkpoint reuse in the last parse. The resume offset is in the
    /// original UTF-8 text, including CRLF. Native nodes and highlights are rebuilt.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.strict
            .as_deref()
            .map_or_else(ReuseStats::default, |cache| cache.reuse)
    }

    fn parse_with_prefix(text: Text, prefix: Option<(&parser::Cache, Option<usize>)>) -> Self {
        let parsed = parser::parse(text.as_str(), prefix);
        let mut doc = Self {
            editor_lines: super::line_projection::editor_lines(text.as_str()),
            text,
            nodes: parsed.nodes,
            highlights: Vec::new(),
            recovered: parsed.recovered,
            strict: parsed.strict.map(Box::new),
        };
        doc.highlights = highlight::collect(doc.root());
        if let Some(start) = parsed.fallback_start {
            fallback::highlights(&mut doc.highlights, doc.text.as_str(), start);
        }
        doc
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// Whether strict table parsing failed or reached a limit.
    pub fn is_recovered(&self) -> bool {
        self.recovered
    }

    /// The root covers the full document.
    pub fn root(&self) -> PythonNode<'_> {
        PythonNode { doc: self, id: 0 }
    }

    /// Nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = PythonNode<'_>> {
        (0..self.nodes.len()).map(|id| PythonNode { doc: self, id })
    }

    /// Highlight spans clipped to a valid UTF-8 byte range.
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

    /// Find a fold that starts on the given line and continues past its end.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        std::iter::successors(self.path(line_end).last(), |node| node.parent())
            .filter_map(PythonNode::fold_range)
            .filter(|range| (line_start..=line_end).contains(&range.start) && line_end < range.end)
            .last()
    }

    /// Compute indentation in columns. Return `None` at an invalid byte
    /// offset or where upstream preserves the current indentation.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        indent::indentation(self, position, settings, options)
    }

    fn resolve_at(&self, at: usize, side: Option<Assoc>) -> Option<PythonNode<'_>> {
        if !self.text.is_char_boundary(at) {
            return None;
        }
        let mut node = self.root();
        loop {
            let children = &node.data().children;
            let index = children.partition_point(|&child| {
                let end = self.nodes[child].range.end;
                end < at || (end == at && side != Some(Assoc::Before))
            });
            let Some(&child) = children.get(index) else {
                return Some(node);
            };
            let range = &self.nodes[child].range;
            if range.is_empty()
                || range.start > at
                || (range.start == at && side != Some(Assoc::After))
            {
                return Some(node);
            }
            // A recovery region marks unparsed text, not a resolved token.
            if self.nodes[child].term == 0 {
                return None;
            }
            node.id = child;
        }
    }

    fn path(&self, position: usize) -> impl Iterator<Item = PythonNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let index = children.partition_point(|&id| self.nodes[id].range.end <= position);
            next = children.get(index).and_then(|&id| {
                (self.nodes[id].range.start < position).then_some(PythonNode { doc: self, id })
            });
            Some(node)
        })
    }
}
