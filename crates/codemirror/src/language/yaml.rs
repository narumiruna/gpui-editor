//! Native YAML support based on `@codemirror/lang-yaml` 6.1.3 and
//! `@lezer/yaml` 1.0.4. See `docs/licenses/codemirror-lang-yaml.txt`.
//!
//! The parser uses the pinned LR tables and native contextual tokenizers.
//! Trees use UTF-8 byte offsets in the original text. Strict LR parses can
//! reuse validated prefix checkpoints. GLR and recovery parse from scratch.
//! The pinned path is limited to 1 MiB and 128 tree levels, subject to the
//! shared parser's work limits. Recovery and fallback trees can differ from
//! upstream.

use std::{
    ops::Range,
    sync::{LazyLock, OnceLock},
};

use crate::{Assoc, ChangeSet, EditError, Text};

use super::{
    HighlightSpan, HighlightTag, IndentContext, IndentOptions, StreamSettings, TagModifier,
    lezer::ReuseStats, line_projection::editor_lines,
};

pub mod frontmatter_language;
mod lexer;
mod lr;
mod parser;
mod tables;
mod tokens;

/// The YAML line comment marker.
pub const LINE_COMMENT: &str = "#";

/// Whether the line prefix triggers upstream reindentation on input.
pub fn indent_on_input(line_prefix: &str) -> bool {
    matches!(trim_space(line_prefix), "]" | "}")
}

/// Visible YAML syntax, including punctuation and recovered errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YamlNodeKind {
    Stream,
    Bom,
    Document,
    Directive,
    DirectiveName,
    DirectiveContent,
    DirectiveEnd,
    DocEnd,
    BlockMapping,
    BlockSequence,
    FlowMapping,
    FlowSequence,
    BlockLiteral,
    BlockLiteralHeader,
    BlockLiteralContent,
    Pair,
    Key,
    Item,
    Literal,
    QuotedLiteral,
    Anchored,
    Anchor,
    Alias,
    Tagged,
    Tag,
    Comment,
    Dash,
    Question,
    Colon,
    Comma,
    OpenBracket,
    CloseBracket,
    OpenBrace,
    CloseBrace,
    Error,
}

use YamlNodeKind::*;

impl YamlNodeKind {
    /// The node's name in the upstream grammar.
    pub const fn name(self) -> &'static str {
        match self {
            Stream => "Stream",
            Bom => "BOM",
            Document => "Document",
            Directive => "Directive",
            DirectiveName => "DirectiveName",
            DirectiveContent => "DirectiveContent",
            DirectiveEnd => "DirectiveEnd",
            DocEnd => "DocEnd",
            BlockMapping => "BlockMapping",
            BlockSequence => "BlockSequence",
            FlowMapping => "FlowMapping",
            FlowSequence => "FlowSequence",
            BlockLiteral => "BlockLiteral",
            BlockLiteralHeader => "BlockLiteralHeader",
            BlockLiteralContent => "BlockLiteralContent",
            Pair => "Pair",
            Key => "Key",
            Item => "Item",
            Literal => "Literal",
            QuotedLiteral => "QuotedLiteral",
            Anchored => "Anchored",
            Anchor => "Anchor",
            Alias => "Alias",
            Tagged => "Tagged",
            Tag => "Tag",
            Comment => "Comment",
            Dash => "-",
            Question => "?",
            Colon => ":",
            Comma => ",",
            OpenBracket => "[",
            CloseBracket => "]",
            OpenBrace => "{",
            CloseBrace => "}",
            Error => "⚠",
        }
    }

    fn highlight(self, key: bool) -> Option<&'static [HighlightTag]> {
        static TAGS: LazyLock<[HighlightTag; 14]> = LazyLock::new(|| {
            [
                HighlightTag::new("keyword"),
                HighlightTag::new("attributeValue"),
                HighlightTag::new("meta"),
                HighlightTag::new("string"),
                HighlightTag::new("string").with_modifier(TagModifier::Special),
                HighlightTag::new("content"),
                HighlightTag::new("propertyName").with_modifier(TagModifier::Definition),
                HighlightTag::new("labelName"),
                HighlightTag::new("typeName"),
                HighlightTag::new("lineComment"),
                HighlightTag::new("separator"),
                HighlightTag::new("squareBracket"),
                HighlightTag::new("brace"),
                HighlightTag::new("punctuation"),
            ]
        });
        let index = match self {
            DirectiveName => 0,
            DirectiveContent => 1,
            DirectiveEnd | DocEnd => 2,
            Literal | QuotedLiteral if key => 6,
            QuotedLiteral => 3,
            BlockLiteralHeader => 4,
            Literal | BlockLiteralContent => 5,
            Anchor | Alias => 7,
            Tag => 8,
            Comment => 9,
            Colon | Comma | Dash => 10,
            OpenBracket | CloseBracket => 11,
            OpenBrace | CloseBrace => 12,
            Question => 13,
            _ => return None,
        };
        Some(std::slice::from_ref(&TAGS[index]))
    }
}

/// One syntax node and its children in parse order. Whitespace is not a child.
/// A leading skipped comment can overlap the range of the next mapping.
#[derive(Debug)]
pub struct YamlNode {
    kind: YamlNodeKind,
    range: Range<usize>,
    children: Vec<Self>,
}

impl YamlNode {
    fn new(kind: YamlNodeKind, range: Range<usize>, children: Vec<Self>) -> Self {
        Self {
            kind,
            range,
            children,
        }
    }

    fn leaf(kind: YamlNodeKind, range: Range<usize>) -> Self {
        Self {
            kind,
            range,
            children: Vec::new(),
        }
    }

    fn ending(kind: YamlNodeKind, start: usize, children: Vec<Self>) -> Self {
        let end = children.last().map_or(start, |node| node.range.end);
        Self::new(kind, start..end, children)
    }

    fn wrapping(kind: YamlNodeKind, child: Self) -> Self {
        Self::new(kind, child.range(), vec![child])
    }

    pub fn kind(&self) -> YamlNodeKind {
        self.kind
    }

    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    pub fn children(&self) -> &[Self] {
        &self.children
    }

    fn fold_range(&self, text: &Text) -> Option<Range<usize>> {
        let (start, end) = match self.kind {
            FlowMapping | FlowSequence => {
                let first = self.children.first()?;
                let last = self.children.last()?;
                (first.range.end, last.range.start)
            }
            Pair | Item | BlockLiteral => {
                (text.line_at(self.range.start)?.range.end, self.range.end)
            }
            _ => return None,
        };
        (start < end).then_some(start..end)
    }

    fn collect_highlights(
        &mut self,
        offset: usize,
        key: bool,
        out: &mut Vec<HighlightSpan<'static>>,
    ) {
        self.range.start += offset;
        self.range.end += offset;
        if !self.range.is_empty()
            && let Some(tags) = self.kind.highlight(key)
        {
            out.push(HighlightSpan {
                range: self.range(),
                tags,
            });
        }
        for child in &mut self.children {
            child.collect_highlights(offset, self.kind == Key, out);
        }
    }
}

/// A YAML document with a source tree and editor services.
#[derive(Debug)]
pub struct YamlDocument {
    text: Text,
    editor_lines: OnceLock<Option<Text>>,
    root: YamlNode,
    highlights: Vec<HighlightSpan<'static>>,
    strict: Option<Box<lr::Cache>>,
    recovered: bool,
}

impl YamlDocument {
    pub fn parse(text: Text) -> Self {
        let end = text.len();
        Self::parse_range(text, 0..end, None)
    }

    /// Apply byte edits and reuse a validated LR checkpoint when possible.
    /// For an embedded document, map its region without redetecting delimiters.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let old = self.root.range();
        let range =
            changes.map_pos(old.start, Assoc::Before)..changes.map_pos(old.end, Assoc::After);
        let first_edit = changes
            .changes()
            .next()
            .map(|(edit, _)| edit.start.saturating_sub(old.start).min(range.len()));
        let prefix = self.strict.as_deref().map(|cache| (cache, first_edit));
        Ok(Self::parse_range(text, range, prefix))
    }

    /// Prefix reuse in the last strict LR parse. The offset is in the
    /// original UTF-8 source, including CRLF. YAML nodes and spans are rebuilt.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.strict
            .as_deref()
            .map_or_else(ReuseStats::default, |cache| {
                let mut reuse = cache.reuse;
                if reuse.resumed_at != 0 {
                    reuse.resumed_at += self.root.range.start;
                }
                reuse
            })
    }

    /// Whether parsing required recovery or the local fallback parser.
    pub fn is_recovered(&self) -> bool {
        self.recovered
    }

    fn parse_range(
        text: Text,
        range: Range<usize>,
        prefix: Option<(&lr::Cache, Option<usize>)>,
    ) -> Self {
        let parsed = lr::parse(&text.as_str()[range.clone()], prefix);
        let mut root = parsed.root;
        let mut highlights = Vec::new();
        root.collect_highlights(range.start, false, &mut highlights);
        Self {
            text,
            editor_lines: OnceLock::new(),
            root,
            highlights,
            strict: parsed.strict.map(Box::new),
            recovered: parsed.recovered,
        }
    }

    /// The original source, including text outside an embedded YAML region.
    pub fn text(&self) -> &Text {
        &self.text
    }

    /// Logical line boundaries used by indentation and folding. Lone CR is
    /// a break, but ranges keep the original source byte offsets.
    pub fn syntax_lines(&self) -> &Text {
        self.editor_lines
            .get_or_init(|| editor_lines(self.text.as_str()))
            .as_ref()
            .unwrap_or(&self.text)
    }

    pub fn root(&self) -> &YamlNode {
        &self.root
    }

    /// Highlight spans clipped to a valid byte range in the source.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.text.slice(range.clone())?;
        let first = self
            .highlights
            .partition_point(|span| span.range.end <= range.start);
        let last = self
            .highlights
            .partition_point(|span| span.range.start < range.end)
            .max(first);
        Some(
            self.highlights[first..last]
                .iter()
                .filter_map(move |span| clip(span, &range)),
        )
    }

    /// Find a fold that starts on the supplied line and continues past its end.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        let lines = self.syntax_lines();
        let path = self.path(line_end, |node| {
            node.range.start <= line_end && line_end < node.range.end
        });
        let mut found = None;
        for node in path.into_iter().rev() {
            if found.is_some() && node.range.start < line_start {
                break;
            }
            if let Some(range) = node.fold_range(lines)
                && (line_start..=line_end).contains(&range.start)
                && range.end > line_end
            {
                found = Some(range);
            }
        }
        found
    }

    /// Get syntax-based indentation in columns, or `None` when no rule applies.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        let cx = IndentContext::new(self.syntax_lines(), position, settings, options)?;
        if !(self.root.range.start..=self.root.range.end).contains(&position) {
            return None;
        }
        let path = self.path(position, |node| {
            node.range.start < position && position <= node.range.end
        });
        for node in path.iter().rev() {
            if matches!(node.kind, FlowMapping | FlowSequence)
                && (position < node.range.end
                    || node
                        .children
                        .last()
                        .is_some_and(|child| child.kind == Error))
            {
                return self.flow_indent(&cx, node);
            }
        }
        for node in path.into_iter().rev() {
            let start = node.range.start;
            match node.kind {
                BlockLiteralContent if !node.range.is_empty() => {
                    return self.base_indent(&cx, start);
                }
                BlockLiteral => {
                    return Some(self.base_indent(&cx, start)?.saturating_add(cx.unit()));
                }
                BlockMapping | BlockSequence => {
                    return column(&cx, node.children.first()?.range.start);
                }
                QuotedLiteral => return None,
                Literal => {
                    let col = column(&cx, start)?;
                    if col == cx.line_indent(start, Assoc::After)? {
                        return Some(col);
                    }
                    if position < node.range.end {
                        return None;
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn flow_indent(&self, cx: &IndentContext<'_>, node: &YamlNode) -> Option<usize> {
        let closer = if node.kind == FlowMapping { '}' } else { ']' };
        let closed =
            trim_space(cx.text_after_pos(cx.position(), Assoc::After)?).starts_with(closer);
        // A flow pair in a sequence also has a FlowMapping node, but has no `{`.
        // Upstream uses its first child just like a real opening token.
        let (open, rest) = node.children.split_first()?;
        let line = cx.text().line_at(open.range.start)?;
        let limit = cx
            .simulated_break()
            .filter(|&at| at > line.range.start)
            .map_or(line.range.end, |at| line.range.end.min(at));
        if let Some(child) = rest.iter().find(|child| child.kind != Error)
            && !std::ptr::eq(child, node.children.last()?)
            && child.range.start < limit
        {
            let after = cx
                .text()
                .as_str()
                .as_bytes()
                .get(open.range.end..line.range.end)
                .unwrap_or_default();
            let spaces = after.iter().take_while(|&&ch| ch == b' ').count();
            return column(
                cx,
                if closed {
                    open.range.start
                } else {
                    open.range.end + spaces
                },
            );
        }
        let base = self.base_indent(cx, node.range.start)?;
        Some(if closed {
            base
        } else {
            base.saturating_add(cx.unit())
        })
    }

    fn base_indent(&self, cx: &IndentContext<'_>, node_start: usize) -> Option<usize> {
        let mut line = cx.text().line_at(node_start)?.range.start;
        while let Some(previous) = self
            .path(line, |node| {
                node.range.start < line && line < node.range.end
            })
            .into_iter()
            .rfind(|node| node.range.end <= node_start)
        {
            line = cx.text().line_at(previous.range.start)?.range.start;
        }
        cx.line_indent(line, Assoc::After)
    }

    fn path(&self, position: usize, contains: impl Fn(&YamlNode) -> bool) -> Vec<&YamlNode> {
        let mut result = Vec::new();
        let mut parent = &self.root;
        loop {
            let start = parent
                .children
                .partition_point(|child| child.range.end < position);
            let Some(child) = parent.children[start..]
                .iter()
                .take_while(|child| child.range.start <= position)
                .find(|child| contains(child))
            else {
                return result;
            };
            result.push(child);
            parent = child;
        }
    }
}

/// A YAML region at the start of a document, followed by an opaque body.
/// The caller selects the language for the body. All ranges are absolute.
#[derive(Debug)]
pub struct YamlFrontmatter {
    opening: Range<usize>,
    yaml: YamlDocument,
    closing: Option<Range<usize>>,
    body: Range<usize>,
}

impl YamlFrontmatter {
    /// Accept only a first line that contains exactly `---` and has text
    /// after its line ending. The same marker closes the region. Without a
    /// closing marker, the YAML region continues to EOF.
    pub fn parse(text: &Text) -> Option<Self> {
        Self::parse_with_prefix(text, None)
    }

    fn parse_with_prefix(text: &Text, previous: Option<(&Self, usize)>) -> Option<Self> {
        let source = lexer::Source(text.as_str());
        if !text.as_str().starts_with("---") {
            return None;
        }
        let start = source.after_break(3);
        if start == 3 || start == text.len() {
            return None;
        }
        let mut end = start;
        let mut closing = None;
        while end < text.len() {
            let line_end = source.line_end(end);
            let next = source.after_break(line_end);
            if &text.as_str()[end..line_end] == "---" {
                closing = Some(end..next);
                break;
            }
            end = next;
        }
        let body = closing.as_ref().map_or(text.len(), |range| range.end)..text.len();
        let prefix = previous.and_then(|(old, first_edit)| {
            let old_range = old.yaml.root.range();
            let cache = old.yaml.strict.as_deref()?;
            (old_range.start == start && first_edit >= start)
                .then(|| (cache, Some(first_edit.min(old_range.end).min(end) - start)))
        });
        Some(Self {
            opening: 0..start,
            yaml: YamlDocument::parse_range(text.clone(), start..end, prefix),
            closing,
            body,
        })
    }

    /// The opening delimiter and its line ending, if present.
    pub fn opening(&self) -> Range<usize> {
        self.opening.clone()
    }

    pub fn yaml(&self) -> &YamlDocument {
        &self.yaml
    }

    /// The closing delimiter and its line ending, if present.
    pub fn closing(&self) -> Option<Range<usize>> {
        self.closing.clone()
    }

    pub fn body(&self) -> Range<usize> {
        self.body.clone()
    }

    /// Highlight the front matter (including delimiters), not the body.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        let yaml = self.yaml.highlights(range.clone())?;
        let marker = |dash: &Range<usize>| {
            clip(
                &HighlightSpan {
                    range: dash.clone(),
                    tags: DirectiveEnd.highlight(false).unwrap(),
                },
                &range,
            )
        };
        Some(
            marker(&self.opening)
                .into_iter()
                .chain(yaml)
                .chain(self.closing.as_ref().and_then(marker)),
        )
    }
}

fn clip<'a>(span: &HighlightSpan<'a>, range: &Range<usize>) -> Option<HighlightSpan<'a>> {
    let start = span.range.start.max(range.start);
    let end = span.range.end.min(range.end);
    (start < end).then_some(HighlightSpan {
        range: start..end,
        tags: span.tags,
    })
}

fn column(cx: &IndentContext<'_>, at: usize) -> Option<usize> {
    Some(cx.column(at, Assoc::After)?.max(0) as usize)
}

fn trim_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}
