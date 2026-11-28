//! Native HTML language services from `@codemirror/lang-html` 6.4.12 and
//! `@lezer/html` 1.3.13. See `docs/licenses/codemirror-lang-html.txt`.
//!
//! Ranges use UTF-8 bytes in the original document. The parser uses the pinned
//! Lezer tables and native element contexts. When strict parsing fails it
//! uses local recovery.

use std::{ops::Range, sync::OnceLock};

use crate::{Assoc, ChangeSet, EditError, Text, commands::CommentTokens};

use super::{
    HighlightSpan, IndentOptions, LanguageData, StreamSettings,
    lezer::{ReuseStats, SyntaxTree, TreeFragment},
};

mod brackets;
mod close_tags;
mod completion;
mod highlight;
mod indent;
mod lexer;
mod lr;
mod nesting;
mod parser;
mod quotes;
mod tables;
mod tag_context;

pub use close_tags::{auto_close_tags, auto_close_tags_in};
pub(crate) use close_tags::{
    close_tags_after, html_input_is_active, tag_close_insert, xml_policy_end_tag_name,
};
pub(crate) use completion::html_schema_completion_source;
pub use completion::{
    HtmlAttributeSpec, HtmlCompletionConfig, HtmlTagSpec, html_completion_source,
    html_completion_source_with,
};
pub use nesting::{
    HtmlAttributes, HtmlCustomDocument, HtmlEmbeddedDocument, HtmlEmbeddedLanguage, HtmlMount,
    HtmlNestedAttribute, HtmlNestedLanguage, HtmlNestedParser, HtmlNesting,
};

/// Name of the upstream language provider.
pub const NAME: &str = "html";
/// HTML block comment delimiters.
pub const BLOCK_COMMENT: (&str, &str) = ("<!--", "-->");
/// Extra characters that count as part of a word in HTML.
pub const WORD_CHARS: &str = "-_";

pub(super) fn is_void_tag_name(name: &str) -> bool {
    lexer::is_void(name)
}

/// Language data for HTML outside a mounted region.
pub fn language_data() -> LanguageData {
    LanguageData::default()
        .with(
            "commentTokens",
            CommentTokens {
                line: None,
                block: Some(BLOCK_COMMENT),
            },
        )
        .with("wordChars", WORD_CHARS)
        .with("indentOnInput", indent_on_input as fn(&str) -> bool)
}

/// Whether this prefix matches the upstream indentation trigger `^\s*</\w+\W$`.
pub fn indent_on_input(line_prefix: &str) -> bool {
    let Some(after) = trim_js_space(line_prefix).strip_prefix("</") else {
        return false;
    };
    let word = |ch: char| ch.is_ascii_alphanumeric() || ch == '_';
    let mut chars = after.chars();
    if !chars.next().is_some_and(word) {
        return false;
    }
    chars
        .find(|&ch| !word(ch))
        .is_some_and(|ch| ch.len_utf16() == 1)
        && chars.next().is_none()
}

fn trim_js_space(source: &str) -> &str {
    source.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

/// HTML parser dialects. Defaults match the upstream `html()` provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HtmlOptions {
    pub match_closing_tags: bool,
    pub self_closing_tags: bool,
}

impl Default for HtmlOptions {
    fn default() -> Self {
        Self {
            match_closing_tags: true,
            self_closing_tags: false,
        }
    }
}

macro_rules! node_kinds {
    ($($kind:ident),+ $(,)?) => {
        /// The visible node types of the HTML grammar.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum HtmlNodeKind { $($kind,)+ Error }

        impl HtmlNodeKind {
            pub const fn name(self) -> &'static str {
                match self { $(Self::$kind => stringify!($kind),)+ Self::Error => "⚠" }
            }

            fn from_name(name: &str) -> Self {
                match name {
                    $(stringify!($kind) => Self::$kind,)+
                    "⚠" => Self::Error,
                    _ => unreachable!("unknown node in the pinned HTML grammar: {name}"),
                }
            }
        }
    };
}

node_kinds! {
    Document, Element, OpenTag, CloseTag, SelfClosingTag, MismatchedCloseTag,
    StartTag, StartCloseTag, EndTag, SelfClosingEndTag, TagName,
    Attribute, AttributeName, Is, AttributeValue, UnquotedAttributeValue,
    Text, ScriptText, StyleText, TextareaText, EntityReference, CharacterReference,
    InvalidEntity, Comment, ProcessingInst, DoctypeDecl, IncompleteTag, IncompleteCloseTag,
}

#[derive(Debug)]
struct Node {
    kind: HtmlNodeKind,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A borrowed syntax node. A zero-width error can mark missing syntax.
#[derive(Clone, Copy, Debug)]
pub struct HtmlNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> HtmlNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    pub fn kind(self) -> HtmlNodeKind {
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

    fn child(self, kind: HtmlNodeKind) -> Option<Self> {
        self.children().find(|node| node.kind() == kind)
    }

    /// The tag name used as the bracket matching handle for an open or close tag.
    pub fn bracket_matching_handle(self) -> Option<Self> {
        use HtmlNodeKind::*;
        matches!(self.kind(), OpenTag | CloseTag)
            .then(|| self.child(TagName))
            .flatten()
    }

    /// The content between the opening and closing tags, or the end of the
    /// element for implicitly closed or unfinished elements.
    pub fn fold_range(self) -> Option<Range<usize>> {
        use HtmlNodeKind::*;
        if self.kind() != Element {
            return None;
        }
        let first = self
            .children()
            .next()
            .filter(|node| node.kind() == OpenTag)?;
        let last = self.children().next_back()?;
        let end = if last.kind() == CloseTag {
            last.data().range.start
        } else {
            self.data().range.end
        };
        Some(first.data().range.end..end)
    }
}

/// HTML syntax tree and the native parsers mounted in its content.
#[derive(Debug)]
pub struct HtmlDocument {
    text: Text,
    nodes: Vec<Node>,
    mounts: Vec<HtmlMount>,
    highlights: Vec<HighlightSpan<'static>>,
    options: HtmlOptions,
    strict: Option<Box<SyntaxTree>>,
    bracket_lines: OnceLock<Option<Text>>,
}

impl HtmlDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_options(text, HtmlOptions::default())
    }

    pub fn parse_with_options(text: Text, options: HtmlOptions) -> Self {
        Self::parse_with_nesting(text, options, HtmlNesting::default())
    }

    /// Parse HTML with additional routes checked before the stock routes.
    pub fn parse_with_nesting(text: Text, options: HtmlOptions, nesting: HtmlNesting<'_>) -> Self {
        Self::parse_top(text, options, nesting, None, None)
    }

    pub(super) fn parse_projected(
        text: Text,
        options: HtmlOptions,
        nesting: HtmlNesting<'_>,
        original: nesting::OriginalSource<'_>,
    ) -> Self {
        Self::parse_top(text, options, nesting, None, Some(original))
    }

    /// Apply byte edits with the same HTML dialect. Rebuild mounts with stock
    /// routes. To keep custom routes, use [`Self::reparse_with_nesting`].
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        self.reparse_with_nesting(changes, HtmlNesting::default())
    }

    /// Apply byte edits and rebuild mounts with the supplied routes. Supply
    /// custom routes on each call; the document does not retain callbacks.
    /// The strict LR parser can resume before the first edit. Invalid edits
    /// leave this document unchanged.
    pub fn reparse_with_nesting(
        &self,
        changes: &ChangeSet,
        nesting: HtmlNesting<'_>,
    ) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let end = changes
            .changes()
            .next()
            .map_or(text.len(), |(range, _)| range.start);
        let fragment = self.strict.as_deref().map(|tree| TreeFragment {
            tree,
            old_range: 0..end,
            new_range: 0..end,
        });
        Ok(Self::parse_top(text, self.options, nesting, fragment, None))
    }

    /// Strict HTML LR checkpoint reuse in the last parse, in UTF-8 bytes and
    /// shared LR node records. GLR and local recovery report zero. Native
    /// nodes, highlights, and mounts are rebuilt.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.strict
            .as_deref()
            .map_or_else(ReuseStats::default, SyntaxTree::reuse_stats)
    }

    fn parse_top(
        text: Text,
        options: HtmlOptions,
        nesting: HtmlNesting<'_>,
        fragment: Option<TreeFragment<'_>>,
        original: Option<nesting::OriginalSource<'_>>,
    ) -> Self {
        let source = text.as_str();
        let (nodes, strict) = lr::parse(source, options, fragment)
            .unwrap_or_else(|| (parser::parse(source, options), None));
        let mut doc = Self {
            text,
            nodes,
            mounts: Vec::new(),
            highlights: Vec::new(),
            options,
            strict,
            bracket_lines: OnceLock::new(),
        };
        doc.mounts = nesting::collect(&doc, nesting, original);
        doc.highlights = highlight::collect(&doc);
        doc
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn root(&self) -> HtmlNode<'_> {
        HtmlNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// HTML nodes in pre-order, including raw content nodes even if mounted.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = HtmlNode<'_>> {
        (0..self.nodes.len()).map(|id| HtmlNode { id, ..self.root() })
    }

    /// Routed regions in source order. Each native child tree uses offsets
    /// relative to the start of its region.
    pub fn mounts(&self) -> &[HtmlMount] {
        &self.mounts
    }

    /// Highlight spans clipped to the supplied range, including mounted spans.
    /// Return `None` for an invalid range or byte boundary.
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
        let mut spans: Vec<_> = self.highlights[first..end]
            .iter()
            .map(|span| HighlightSpan {
                range: span.range.start.max(range.start)..span.range.end.min(range.end),
                tags: span.tags,
            })
            .collect();
        if !range.is_empty() {
            nesting::highlights(self, &range, &mut spans);
            spans.sort_unstable_by_key(|span| span.range.start);
        }
        Some(spans.into_iter())
    }

    /// Find the outermost fold that starts on this line and continues past it.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        self.path(line_end, Some(Assoc::After))
            .filter_map(HtmlNode::fold_range)
            .find(|range| (line_start..=line_end).contains(&range.start) && line_end < range.end)
            .or_else(|| nesting::foldable(self, line_start, line_end))
    }

    /// Compute indentation in columns. Return `None` if no indentation applies
    /// or if the position is not a UTF-8 byte boundary.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        indent::indentation(self, position, settings, options)
    }

    fn path(&self, position: usize, side: Option<Assoc>) -> impl Iterator<Item = HtmlNode<'_>> {
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
                    .then_some(HtmlNode { id, ..node })
            });
            Some(node)
        })
    }
}
