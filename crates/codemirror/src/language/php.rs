//! Native PHP language services from `@codemirror/lang-php` 6.0.2 and
//! `@lezer/php` 1.0.6. See `docs/licenses/codemirror-lang-php.txt`.
//!
//! The parser uses the published Lezer tables and native token hooks. Ranges
//! use UTF-8 byte offsets. Strict LR parses can reuse prefix checkpoints.
//! The shared LR and GLR work limits apply. Recovery can retain a complete
//! prefix but does not reproduce Lezer's full recovery. The HTML parse uses a
//! separate, mapped projection with optional HTML dialects and nesting rules.
//! Fold and indentation queries use both native trees.

use std::ops::Range;

use crate::{ChangeSet, EditError, Text, commands::CommentTokens};

use super::{
    HighlightSpan, IndentOptions, LanguageData, StreamSettings,
    html::{HtmlNesting, HtmlOptions},
    lezer::{ReuseStats, SyntaxTree},
};

mod highlight;
mod indent;
mod outer;
mod parser;
mod tables;
mod tokens;

pub use outer::PhpHtml;

pub const NAME: &str = "php";
pub const LINE_COMMENT: &str = "//";
pub const BLOCK_COMMENT: (&str, &str) = ("/*", "*/");
pub const WORD_CHARS: &str = "$";
pub const STRING_PREFIXES: [&str; 2] = ["b", "B"];

/// The PHP language data. Close brackets use the standard bracket list with
/// [`STRING_PREFIXES`] for strings.
pub fn language_data() -> LanguageData {
    LanguageData::default()
        .with(
            "commentTokens",
            CommentTokens {
                line: Some(LINE_COMMENT),
                block: Some(BLOCK_COMMENT),
            },
        )
        .with("wordChars", WORD_CHARS)
        .with("indentOnInput", indent_on_input as fn(&str) -> bool)
        .with(
            "closeBrackets",
            CloseBrackets {
                string_prefixes: &STRING_PREFIXES,
            },
        )
}

/// The PHP override for CodeMirror's close-bracket configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CloseBrackets {
    pub string_prefixes: &'static [&'static str],
}

/// Whether the line from its start to the cursor triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    matches!(
        trim_space(line_prefix),
        "case "
            | "default:"
            | "endif"
            | "endfor"
            | "endforeach"
            | "endswitch"
            | "endwhile"
            | "else"
            | "elseif"
            | "{"
            | "}"
    )
}

fn trim_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

/// Select the upstream parser top rule and whether to parse non-PHP text as HTML.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhpOptions {
    /// Use `Program` for PHP without opening or closing tags. The default
    /// `Template` mode recognizes `<?php`, `<?=`, and `<?` without a dialect flag.
    pub plain: bool,
    /// Parse template text using native HTML. Has no effect in plain mode.
    pub html: bool,
}

impl Default for PhpOptions {
    fn default() -> Self {
        Self {
            plain: false,
            html: true,
        }
    }
}

/// A borrowed node from the PHP syntax tree.
#[derive(Clone, Copy, Debug)]
pub struct PhpNode<'a> {
    doc: &'a PhpDocument,
    id: usize,
}

impl<'a> PhpNode<'a> {
    fn data(self) -> &'a parser::Node {
        &self.doc.nodes[self.id]
    }

    /// The name from the pinned grammar. `⚠` marks a native recovery region.
    pub fn name(self) -> &'static str {
        self.data().name()
    }

    pub fn range(self) -> Range<usize> {
        self.data().range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    /// Direct PHP tree children in source order, including template text.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }

    /// The fold range for a node with an upstream fold rule.
    pub fn fold_range(self) -> Option<Range<usize>> {
        let range = self.range();
        match self.name() {
            "Block" | "EnumBody" | "DeclarationList" | "SwitchBody" | "ArrayExpression"
            | "ValueList" => {
                let first = self.children().next()?.range();
                let last = self.children().next_back()?;
                (first.end < last.range().start).then(|| {
                    first.end..if last.name() == "⚠" {
                        range.end
                    } else {
                        last.range().start
                    }
                })
            }
            "ColonBlock" => Some(range.start + 1..range.end),
            "BlockComment" => Some(range.start + 2..range.end - 2),
            _ => None,
        }
    }
}

/// A PHP syntax tree and its editor services.
#[derive(Debug)]
pub struct PhpDocument {
    text: Text,
    nodes: Vec<parser::Node>,
    html: Option<PhpHtml>,
    highlights: Vec<HighlightSpan<'static>>,
    recovered: bool,
    options: PhpOptions,
    html_options: HtmlOptions,
    strict: Option<Box<SyntaxTree>>,
}

impl PhpDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_options(text, PhpOptions::default())
    }

    pub fn parse_with_options(text: Text, options: PhpOptions) -> Self {
        Self::parse_with_html_nesting(
            text,
            options,
            HtmlOptions {
                match_closing_tags: false,
                ..HtmlOptions::default()
            },
            HtmlNesting::default(),
        )
    }

    /// Parse template text with the specified HTML dialect and nesting rules.
    /// Rules check tag names and attributes in the original source before the
    /// stock rules. Child parsers receive projected text without PHP gaps.
    /// The HTML settings have no effect in plain mode or when HTML is disabled.
    pub fn parse_with_html_nesting(
        text: Text,
        options: PhpOptions,
        html_options: HtmlOptions,
        nesting: HtmlNesting<'_>,
    ) -> Self {
        Self::parse_with_prefix(text, options, html_options, nesting, None)
    }

    /// Apply UTF-8 byte edits with the same HTML dialect and stock routes.
    /// To keep custom routes, use [`Self::reparse_with_html_nesting`].
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        self.reparse_with_html_nesting(changes, HtmlNesting::default())
    }

    /// Apply UTF-8 byte edits and rebuild the HTML projection with the supplied
    /// routes. The document does not retain route callbacks. The PHP parser
    /// can reuse strict checkpoints before the first edit.
    pub fn reparse_with_html_nesting(
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
        let prefix = self.strict.as_deref().map(|tree| (tree, end));
        Ok(Self::parse_with_prefix(
            text,
            self.options,
            self.html_options,
            nesting,
            prefix,
        ))
    }

    /// Report strict LR prefix reuse in logical bytes (CRLF counts as one byte).
    /// GLR, recovery, and the HTML projection do not reuse checkpoints.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.strict
            .as_deref()
            .map_or_else(ReuseStats::default, SyntaxTree::reuse_stats)
    }

    fn parse_with_prefix(
        text: Text,
        options: PhpOptions,
        html_options: HtmlOptions,
        nesting: HtmlNesting<'_>,
        prefix: Option<(&SyntaxTree, usize)>,
    ) -> Self {
        let (nodes, recovered, strict) = parser::parse(text.as_str(), options.plain, prefix);
        let mut doc = Self {
            text,
            nodes,
            html: None,
            highlights: Vec::new(),
            recovered,
            options,
            html_options,
            strict: strict.map(Box::new),
        };
        doc.highlights = highlight::collect(doc.root());
        if !options.plain && options.html {
            doc.html = PhpHtml::parse(&doc, html_options, nesting);
        }
        doc
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// Whether strict parsing failed or reached a shared parser limit.
    pub fn is_recovered(&self) -> bool {
        self.recovered
    }

    /// The root covers the complete source, including skipped whitespace.
    pub fn root(&self) -> PhpNode<'_> {
        PhpNode { doc: self, id: 0 }
    }

    /// The PHP tree nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = PhpNode<'_>> {
        (0..self.nodes.len()).map(|id| PhpNode { doc: self, id })
    }

    /// An independently parsed, mapped native HTML tree for template text.
    pub fn html(&self) -> Option<&PhpHtml> {
        self.html.as_ref()
    }

    /// Highlight spans clipped to a valid UTF-8 byte range in the source.
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
        if let Some(html) = self.html()
            && !range.is_empty()
        {
            html.highlights(&range, &mut spans);
            spans.sort_unstable_by_key(|span| span.range.start);
        }
        Some(spans.into_iter())
    }

    /// Find a fold that starts on this line and continues past its end.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        let mut found = None;
        for node in core::iter::successors(self.path(line_end, true).last(), |n| n.parent()) {
            if node.range().end <= line_end {
                continue;
            }
            if found.is_some() && node.range().start < line_start {
                break;
            }
            if let Some(range) = node.fold_range()
                && (line_start..=line_end).contains(&range.start)
                && line_end < range.end
            {
                found = Some(range);
            }
        }
        found.or_else(|| self.html()?.foldable(line_start, line_end))
    }

    /// Compute indentation in columns. Return `None` for an invalid position
    /// or when the syntax rule keeps the current indentation.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        if !self.text.as_str().is_char_boundary(position) {
            return None;
        }
        let php = indent::indentation(self, position, settings, options);
        let html = self
            .html()
            .and_then(|html| html.indentation(position, settings, options));
        if let Some((start, columns)) = html
            && php.is_none_or(|(php_start, _)| php_start < start)
        {
            return columns;
        }
        php.map_or(Some(0), |(_, columns)| columns)
    }

    fn path(&self, position: usize, include_start: bool) -> impl Iterator<Item = PhpNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let index = children.partition_point(|&id| self.nodes[id].range.end <= position);
            next = children.get(index).and_then(|&id| {
                let start = self.nodes[id].range.start;
                (start < position || (include_start && start == position))
                    .then_some(PhpNode { id, ..node })
            });
            Some(node)
        })
    }
}
