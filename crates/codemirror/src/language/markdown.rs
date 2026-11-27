//! Native source services for `@codemirror/lang-markdown` 6.5.2.
//! See `docs/licenses/codemirror-lang-markdown.txt` for pins and differences.
//!
//! CommonMark is the default, as in `markdown()`. The extended dialect adds
//! GFM and Pandoc-style scripts and emoji. All positions are original UTF-8
//! byte offsets, including in documents with CRLF or lone CR line breaks.

use std::ops::Range;

use crate::{Assoc, ChangeSet, EditError, Text};

use super::{
    HighlightSpan, HighlightTag, LanguageData,
    line_projection::{NormalizedInput, editor_lines},
};

mod commands;
mod completion;
mod highlight;
mod incremental;
mod inline;
mod install;
mod mounts;
mod parser;
mod paste_link;
mod table;

pub use mounts::{
    MarkdownCodeLanguages, MarkdownCodeParser, MarkdownCodeParserResolver, MarkdownCodeResolver,
    MarkdownMount, MarkdownParserConfig,
};

pub use commands::{MarkdownEdit, MarkdownMarkupCommand, markdown_keymap};
pub use completion::{markdown_completion_source, markdown_completion_start};
pub use install::{
    MarkdownCompletionSource, MarkdownConfig, configured_markdown_parser, markdown,
    markdown_completions, markdown_keymap_for, markdown_snapshot, markdown_with_parser,
    plan_markdown_markup, plan_markdown_paste,
};
pub use paste_link::paste_url_as_link;

pub const NAME: &str = "markdown";
pub const BLOCK_COMMENT: (&str, &str) = ("<!--", "-->");

pub fn language_data() -> LanguageData {
    LanguageData::default().with("commentTokens", BLOCK_COMMENT)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MarkdownDialect {
    #[default]
    CommonMark,
    Extended,
}

macro_rules! node_kinds {
    ($($node:ident),+ $(,)?) => {
        /// A named syntax node from the Lezer Markdown grammar.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum MarkdownNodeKind { $($node),+ }

        impl MarkdownNodeKind {
            pub const fn name(self) -> &'static str {
                match self { $(Self::$node => stringify!($node)),+ }
            }
        }
    };
}

node_kinds!(
    Document,
    Paragraph,
    Blockquote,
    BulletList,
    OrderedList,
    ListItem,
    ATXHeading1,
    ATXHeading2,
    ATXHeading3,
    ATXHeading4,
    ATXHeading5,
    ATXHeading6,
    SetextHeading1,
    SetextHeading2,
    FencedCode,
    CodeBlock,
    HorizontalRule,
    HTMLBlock,
    CommentBlock,
    ProcessingInstructionBlock,
    LinkReference,
    Escape,
    Entity,
    HardBreak,
    Emphasis,
    StrongEmphasis,
    Link,
    Image,
    InlineCode,
    HTMLTag,
    Comment,
    ProcessingInstruction,
    Autolink,
    URL,
    HeaderMark,
    QuoteMark,
    ListMark,
    LinkMark,
    EmphasisMark,
    CodeMark,
    CodeText,
    CodeInfo,
    LinkLabel,
    LinkTitle,
    Strikethrough,
    StrikethroughMark,
    Table,
    TableHeader,
    TableRow,
    TableCell,
    TableDelimiter,
    Task,
    TaskMarker,
    Subscript,
    SubscriptMark,
    Superscript,
    SuperscriptMark,
    Emoji,
);

use MarkdownNodeKind::*;

impl MarkdownNodeKind {
    pub const fn heading_level(self) -> Option<u8> {
        Some(match self {
            ATXHeading1 | SetextHeading1 => 1,
            ATXHeading2 | SetextHeading2 => 2,
            ATXHeading3 => 3,
            ATXHeading4 => 4,
            ATXHeading5 => 5,
            ATXHeading6 => 6,
            _ => return None,
        })
    }

    const fn fold_block(self) -> bool {
        matches!(
            self,
            Paragraph
                | Blockquote
                | ListItem
                | FencedCode
                | CodeBlock
                | HorizontalRule
                | HTMLBlock
                | CommentBlock
                | ProcessingInstructionBlock
                | LinkReference
                | Table
                | Task
        )
    }
}

/// One syntax node with source-ordered children. Plain text is not a node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownNode {
    kind: MarkdownNodeKind,
    range: Range<usize>,
    children: Vec<Self>,
}

impl MarkdownNode {
    fn new(kind: MarkdownNodeKind, range: Range<usize>, children: Vec<Self>) -> Self {
        Self {
            kind,
            range,
            children,
        }
    }

    fn leaf(kind: MarkdownNodeKind, range: Range<usize>) -> Self {
        Self::new(kind, range, Vec::new())
    }

    fn shift(&mut self, offset: isize) {
        if offset == 0 {
            return;
        }
        let shift = |position: usize| {
            position
                .checked_add_signed(offset)
                .expect("shifted Markdown range")
        };
        self.range = shift(self.range.start)..shift(self.range.end);
        for child in &mut self.children {
            child.shift(offset);
        }
    }

    pub fn kind(&self) -> MarkdownNodeKind {
        self.kind
    }

    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    pub fn children(&self) -> impl DoubleEndedIterator<Item = &Self> + ExactSizeIterator {
        self.children.iter()
    }
}

/// Source ranges for an optional nested language. They exclude the host's
/// markup. Use [`MarkdownDocument::mounts`] to run the embedded parsers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkdownOverlay<'a> {
    Code {
        info: &'a str,
        ranges: Vec<Range<usize>>,
    },
    Html {
        ranges: Vec<Range<usize>>,
    },
}

/// A parsed Markdown document and its language services.
#[derive(Clone, Debug)]
pub struct MarkdownDocument {
    text: Text,
    editor_lines: Option<Text>,
    dialect: MarkdownDialect,
    root: MarkdownNode,
    highlights: Vec<(Range<usize>, Vec<HighlightTag>)>,
    parsed_bytes: usize,
    reused_blocks: usize,
}

impl MarkdownDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_dialect(text, MarkdownDialect::CommonMark)
    }

    pub fn parse_with_dialect(text: Text, dialect: MarkdownDialect) -> Self {
        let lines = editor_lines(text.as_str());
        let root = parse_root(lines.as_ref().unwrap_or(&text).as_str(), dialect);
        let parsed_bytes = text.len();
        Self::from_root(text, lines, dialect, root, parsed_bytes, 0)
    }

    /// Apply the changes and reuse unchanged root blocks at safe boundaries.
    /// The dialect does not change. Some edits require a full parse.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut ranges = changes.changes().map(|(range, _)| range);
        let Some(first) = ranges.next() else {
            let mut doc = self.clone();
            doc.parsed_bytes = 0;
            doc.reused_blocks = self.root.children.len();
            return Ok(doc);
        };
        let changed = first.start..ranges.last().unwrap_or(first).end;
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let lines = editor_lines(text.as_str());
        let source = lines.as_ref().unwrap_or(&text).as_str();
        let (root, parsed_bytes, reused_blocks) =
            if incremental::changes_line_seam(self.text.as_str(), changes) {
                (parse_root(source, self.dialect), text.len(), 0)
            } else {
                incremental::reparse(
                    &self.root,
                    self.syntax_lines().as_str(),
                    source,
                    changes,
                    changed,
                    self.dialect,
                )
            };
        Ok(Self::from_root(
            text,
            lines,
            self.dialect,
            root,
            parsed_bytes,
            reused_blocks,
        ))
    }

    fn from_root(
        text: Text,
        editor_lines: Option<Text>,
        dialect: MarkdownDialect,
        root: MarkdownNode,
        parsed_bytes: usize,
        reused_blocks: usize,
    ) -> Self {
        let highlights = highlight::collect(&root);
        Self {
            text,
            editor_lines,
            dialect,
            root,
            highlights,
            parsed_bytes,
            reused_blocks,
        }
    }

    /// Original source bytes sent to the parser, including a failed attempt to reuse the suffix.
    pub fn parsed_bytes(&self) -> usize {
        self.parsed_bytes
    }

    /// Root blocks copied from the previous tree without parsing their text.
    pub fn reused_blocks(&self) -> usize {
        self.reused_blocks
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    fn syntax_lines(&self) -> &Text {
        self.editor_lines.as_ref().unwrap_or(&self.text)
    }

    pub fn root(&self) -> &MarkdownNode {
        &self.root
    }

    /// All syntax nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl Iterator<Item = &MarkdownNode> {
        let mut stack = vec![std::slice::from_ref(&self.root).iter()];
        std::iter::from_fn(move || {
            loop {
                if let Some(node) = stack.last_mut()?.next() {
                    if !node.children.is_empty() {
                        stack.push(node.children.iter());
                    }
                    return Some(node);
                }
                stack.pop();
            }
        })
    }

    /// Return the deepest node covering this position. At a boundary, prefer
    /// the node ending there when `left` is true, and the node starting there otherwise.
    pub fn node_at(&self, position: usize, left: bool) -> Option<&MarkdownNode> {
        self.text
            .as_str()
            .is_char_boundary(position)
            .then(|| self.path(position, left).last().unwrap().0)
    }

    /// Get source highlights clipped to a valid UTF-8 byte range.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.text.slice(range.clone())?;
        let first = self
            .highlights
            .partition_point(|(span, _)| span.end <= range.start);
        let end = if range.is_empty() {
            first
        } else {
            self.highlights
                .partition_point(|(span, _)| span.start < range.end)
        };
        Some(
            self.highlights[first..end]
                .iter()
                .map(move |(span, tags)| HighlightSpan {
                    range: span.start.max(range.start)..span.end.min(range.end),
                    tags,
                }),
        )
    }

    /// Find a section or block fold starting on this line and ending after it.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        self.heading_fold(line_start, line_end)
            .or_else(|| self.block_fold(line_start, line_end))
    }

    fn heading_fold(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        let heading_path = self.path(line_end, true).collect::<Vec<_>>();
        for (node, following) in heading_path
            .into_iter()
            .rev()
            .take_while(|(node, _)| node.range.start >= line_start)
        {
            if let Some(level) = node.kind.heading_level() {
                let last = following
                    .iter()
                    .take_while(|next| next.kind.heading_level().is_none_or(|l| l > level))
                    .last()
                    .unwrap_or(node);
                if last.range.end > line_end {
                    return Some(line_end..last.range.end);
                }
            }
        }
        None
    }

    fn block_fold(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        let mut fold = None;
        let path = self.path(line_end, false).collect::<Vec<_>>();
        for (node, _) in path.into_iter().rev() {
            if fold.is_some() && node.range.start < line_start {
                break;
            }
            if node.kind.fold_block() && node.range.end > line_end {
                let start = self.syntax_lines().line_at(node.range.start)?.range.end;
                if (line_start..=line_end).contains(&start) {
                    fold = Some(start..node.range.end);
                }
            }
        }
        fold
    }

    /// Markdown's root indent rule preserves the existing indentation.
    /// Return `None` also for invalid positions. Use the markup edit service
    /// to continue an existing list or blockquote on a new line.
    pub fn indentation(&self, _position: usize) -> Option<usize> {
        None
    }

    /// Return the code and HTML source regions available to nested parsers.
    pub fn overlays(&self) -> impl Iterator<Item = MarkdownOverlay<'_>> {
        self.nodes().filter_map(|node| match node.kind {
            FencedCode | CodeBlock => {
                let info = node
                    .children()
                    .find(|child| child.kind == CodeInfo)
                    .and_then(|child| self.text.slice(child.range()))
                    .unwrap_or("");
                let ranges = node
                    .children()
                    .filter(|child| child.kind == CodeText)
                    .map(MarkdownNode::range)
                    .collect();
                Some(MarkdownOverlay::Code { info, ranges })
            }
            HTMLBlock | HTMLTag | CommentBlock => {
                let mut from = node.range.start;
                let mut ranges = Vec::new();
                for child in node.children() {
                    if from < child.range.start {
                        ranges.push(from..child.range.start);
                    }
                    from = child.range.end;
                }
                if from < node.range.end {
                    ranges.push(from..node.range.end);
                }
                Some(MarkdownOverlay::Html { ranges })
            }
            _ => None,
        })
    }

    // The side must reach the owning fence, even if it points out of the
    // segment. At the end of an unclosed fence, only Before reaches it.
    pub(crate) fn fenced_segment_endpoint(
        &self,
        position: usize,
        side: Assoc,
    ) -> Option<Range<usize>> {
        let (fence, _) = self
            .path(position, side == Assoc::Before)
            .find(|(node, _)| node.kind == FencedCode)?;
        fence
            .children()
            .filter(|node| node.kind == CodeText && !node.range.is_empty())
            .map(MarkdownNode::range)
            .find(|range| match side {
                Assoc::Before => position == range.start,
                Assoc::After => position == range.end,
            })
    }

    fn path(
        &self,
        position: usize,
        left: bool,
    ) -> impl Iterator<Item = (&MarkdownNode, &[MarkdownNode])> {
        let mut next = Some((&self.root, &[][..]));
        std::iter::from_fn(move || {
            let (node, following) = next?;
            let i = node.children.partition_point(|child| {
                child.range.end < position || (!left && child.range.end == position)
            });
            next = node
                .children
                .get(i)
                .filter(|child| {
                    child.range.start < position || (!left && child.range.start == position)
                })
                .map(|child| (child, &node.children[i + 1..]));
            Some((node, following))
        })
    }
}

fn parse_root(source: &str, dialect: MarkdownDialect) -> MarkdownNode {
    // Use one LF per logical break and restore the original CRLF byte offsets.
    let input = NormalizedInput::new(source);
    let mut root = parser::parse(&input.text, dialect);
    if input.text.len() != source.len() {
        restore_ranges(&mut root, &input);
    }
    root
}

fn restore_ranges(node: &mut MarkdownNode, input: &NormalizedInput<'_>) {
    node.range = input.original_byte(node.range.start)..input.original_byte(node.range.end);
    for child in &mut node.children {
        restore_ranges(child, input);
    }
}
