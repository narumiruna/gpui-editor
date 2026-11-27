//! Native XML language services based on `@codemirror/lang-xml` 6.1.0 and
//! `@lezer/xml` 1.0.6. See `docs/licenses/codemirror-lang-xml.txt`.
//!
//! All ranges use UTF-8 bytes into the original [`Text`]. The native Lezer
//! runtime parses the published grammar and can reuse deterministic prefixes.
//! Invalid input uses bounded LR recovery, with a local fallback. Recovery can
//! differ from upstream Lezer. The parser does not validate a schema, interpret
//! a DTD, or expand references.

use std::{ops::Range, sync::OnceLock};

use crate::{Assoc, ChangeSet, EditError, Text};

use super::{
    HighlightSpan, IndentContext, IndentOptions, StreamSettings,
    lezer::{ReuseStats, SyntaxTree},
};

mod brackets;
mod close_tags;
mod completion;
mod highlight;
mod lr;
mod native;
mod parser;
mod quotes;
mod tables;
mod tokens;

pub use close_tags::{auto_close_tags, auto_close_tags_in};
pub use completion::{
    XmlAttrSpec, XmlAttribute, XmlCompletion, XmlCompletionResult, XmlCompletionValidity,
    XmlElementSpec, XmlSchema, XmlValue, xml_completion_source,
};
pub use native::NativeXmlSchemaExtension;

/// Block comment delimiters from the upstream language data.
pub const BLOCK_COMMENT: (&str, &str) = ("<!--", "-->");

/// Whether the text from a line's start to the cursor triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    trim_js_space(line_prefix) == "</"
}

/// The visible node types of the XML grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XmlNodeKind {
    Document,
    Element,
    OpenTag,
    CloseTag,
    SelfClosingTag,
    MismatchedCloseTag,
    MissingCloseTag,
    StartTag,
    StartCloseTag,
    EndTag,
    SelfCloseEndTag,
    TagName,
    Attribute,
    AttributeName,
    AttributeValue,
    Is,
    Text,
    EntityReference,
    CharacterReference,
    Cdata,
    Comment,
    ProcessingInst,
    DoctypeDecl,
    Error,
}

impl XmlNodeKind {
    /// The node's name in the upstream syntax tree.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Document => "Document",
            Self::Element => "Element",
            Self::OpenTag => "OpenTag",
            Self::CloseTag => "CloseTag",
            Self::SelfClosingTag => "SelfClosingTag",
            Self::MismatchedCloseTag => "MismatchedCloseTag",
            Self::MissingCloseTag => "MissingCloseTag",
            Self::StartTag => "StartTag",
            Self::StartCloseTag => "StartCloseTag",
            Self::EndTag => "EndTag",
            Self::SelfCloseEndTag => "SelfCloseEndTag",
            Self::TagName => "TagName",
            Self::Attribute => "Attribute",
            Self::AttributeName => "AttributeName",
            Self::AttributeValue => "AttributeValue",
            Self::Is => "Is",
            Self::Text => "Text",
            Self::EntityReference => "EntityReference",
            Self::CharacterReference => "CharacterReference",
            Self::Cdata => "Cdata",
            Self::Comment => "Comment",
            Self::ProcessingInst => "ProcessingInst",
            Self::DoctypeDecl => "DoctypeDecl",
            Self::Error => "⚠",
        }
    }
}

#[derive(Debug)]
struct Node {
    kind: XmlNodeKind,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A borrowed syntax node. A zero-width error or `MissingCloseTag` marks
/// missing syntax without adding bytes to the document.
#[derive(Clone, Copy, Debug)]
pub struct XmlNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> XmlNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    pub fn kind(self) -> XmlNodeKind {
        self.data().kind
    }

    pub fn range(self) -> Range<usize> {
        self.data().range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    /// Direct children in source order. Text nodes include whitespace outside tags.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }

    fn child_before(self, pos: usize) -> Option<Self> {
        let children = &self.data().children;
        let before = children.partition_point(|&id| self.nodes[id].range.start < pos);
        let id = *children.get(before.checked_sub(1)?)?;
        Some(Self { id, ..self })
    }

    fn ends_in_error(self) -> bool {
        self.children()
            .next_back()
            .is_some_and(|node| node.kind() == XmlNodeKind::Error)
    }

    /// The tag name used for bracket matching on an open or close tag.
    pub fn bracket_matching_handle(self) -> Option<Self> {
        if !matches!(self.kind(), XmlNodeKind::OpenTag | XmlNodeKind::CloseTag) {
            return None;
        }
        self.children()
            .find(|node| node.kind() == XmlNodeKind::TagName)
    }

    /// The range between an element's opening and closing tags, or between
    /// the opening tag and the end of an unfinished element.
    pub fn fold_range(self) -> Option<Range<usize>> {
        if self.kind() != XmlNodeKind::Element {
            return None;
        }
        let first = self.children().next()?;
        if first.kind() != XmlNodeKind::OpenTag {
            return None;
        }
        let last = self.children().next_back()?;
        let end = if last.kind() == XmlNodeKind::CloseTag {
            last.data().range.start
        } else {
            self.data().range.end
        };
        Some(first.data().range.end..end)
    }
}

/// One XML document and its recovered syntax tree.
#[derive(Debug)]
pub struct XmlDocument {
    text: Text,
    nodes: Vec<Node>,
    highlights: Vec<HighlightSpan<'static>>,
    strict: Option<Box<SyntaxTree>>,
    bracket_lines: OnceLock<Option<Text>>,
}

impl XmlDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_prefix(text, None)
    }

    /// Apply byte edits and parse the new document. Reuse a validated LR
    /// checkpoint before the first edit when possible. Invalid edits leave
    /// this document unchanged. Recovered trees do not keep checkpoints.
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

    /// LR checkpoint reuse in the last parse, in original UTF-8 bytes.
    /// XML node views and highlights are rebuilt, even when LR nodes are reused.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.strict
            .as_deref()
            .map_or_else(ReuseStats::default, SyntaxTree::reuse_stats)
    }

    fn parse_with_prefix(text: Text, prefix: Option<(&SyntaxTree, usize)>) -> Self {
        let (nodes, strict) = lr::parse(text.as_str(), prefix);
        let highlights = highlight::collect(&nodes);
        Self {
            text,
            nodes,
            highlights,
            strict: strict.map(Box::new),
            bracket_lines: OnceLock::new(),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// The root spans the entire document.
    pub fn root(&self) -> XmlNode<'_> {
        XmlNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// All nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = XmlNode<'_>> {
        (0..self.nodes.len()).map(|id| XmlNode { id, ..self.root() })
    }

    /// Highlight spans clipped to the supplied range. Return `None` for an
    /// invalid range or byte boundary.
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
    /// Prefer an enclosing element only if that element also starts on the line.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        let inner = self.path(line_end, Side::Right).last()?;
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

    /// Compute the indentation in columns. A line inside a tag uses the
    /// column of the opening `<` plus one unit. Element content uses the
    /// opening line's indentation plus one unit, except before `</`.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        use XmlNodeKind::*;
        let cx = IndentContext::new(&self.text, position, settings, options)?;
        let strict = self.path(position, Side::Strict).last()?;
        let mut inner = strict;
        let mut scan = strict.child_before(position);
        // At a boundary, enter an unfinished node only if its final child
        // marks missing syntax at the node's end.
        while let Some(node) = scan {
            let mut children = node.children();
            let Some(last) = children
                .next_back()
                .filter(|last| last.data().range.end == node.data().range.end)
            else {
                break;
            };
            if last.kind() == Error && last.data().range.is_empty() {
                inner = node;
                scan = children.next_back();
            } else {
                scan = Some(last);
            }
        }
        for node in std::iter::successors(Some(inner), |node| node.parent()) {
            let start = node.data().range.start;
            match node.kind() {
                Element => {
                    let line = cx.line_indent(start, Assoc::After)?;
                    let after = cx.text_after_pos(position, Assoc::After)?;
                    return Some(if trim_js_space(after).starts_with("</") {
                        line
                    } else {
                        line.saturating_add(cx.unit())
                    });
                }
                OpenTag | CloseTag | SelfClosingTag => {
                    let column = cx.column(start, Assoc::After)?;
                    return Some(if column < 0 {
                        cx.unit().saturating_sub(column.unsigned_abs())
                    } else {
                        (column as usize).saturating_add(cx.unit())
                    });
                }
                _ => {}
            }
        }
        Some(0)
    }

    fn path(&self, position: usize, side: Side) -> impl Iterator<Item = XmlNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let start = children.partition_point(|&id| {
                let end = self.nodes[id].range.end;
                end < position || (!matches!(side, Side::Left) && end == position)
            });
            next = children[start..].first().and_then(|&id| {
                let start = self.nodes[id].range.start;
                (start < position || (matches!(side, Side::Right) && start == position))
                    .then_some(XmlNode { id, ..node })
            });
            Some(node)
        })
    }
}

#[derive(Clone, Copy)]
enum Side {
    Left,
    Strict,
    Right,
}

fn trim_js_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}
