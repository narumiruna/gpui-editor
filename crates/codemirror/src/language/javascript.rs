//! Native JavaScript, TypeScript, JSX, and TSX syntax from
//! `@codemirror/lang-javascript` 6.2.5 and `@lezer/javascript` 1.5.7.
//! See `docs/licenses/codemirror-lang-javascript.txt`.
//!
//! The parser uses the pinned tables and tracks newlines for automatic
//! semicolon insertion. Deterministic LR parses can reuse an unchanged prefix.
//! GLR and recovery run from scratch. The module provides completion, tag
//! input, and an adapter for host-supplied ESLint findings. Native ranges use
//! UTF-8 bytes in the original text.

use std::{ops::Range, sync::OnceLock};

use crate::{Assoc, ChangeSet, EditError, Text};

use super::{
    HighlightSpan, IndentOptions, StreamSettings,
    lezer::{ParseError, ReuseStats, SyntaxTree, TreeFragment},
};

mod brackets;
mod close_tags;
mod completion;
mod eslint;
mod highlight;
mod indent;
mod parser;
mod quotes;
mod recovery;
mod specialize;
mod tables;
mod tokens;

pub use close_tags::{auto_close_tags, auto_close_tags_in};
pub use completion::{
    CompletionPath, JavascriptScope, JavascriptScopeProperty, JavascriptScopeValue,
    JavascriptSnippet, SNIPPETS, TYPESCRIPT_SNIPPETS, completion_path,
    javascript_completion_snippet, javascript_completion_source, local_completion_source,
    scope_completion_source,
};
pub use eslint::{EslintFinding, EslintFix, es_lint};

pub const NAME: &str = "javascript";
pub const LINE_COMMENT: &str = "//";
pub const BLOCK_COMMENT: (&str, &str) = ("/*", "*/");
pub const JSX_BLOCK_COMMENT: (&str, &str) = ("{/*", "*/}");
pub const CLOSE_BRACKETS: [char; 6] = ['(', '[', '{', '\'', '"', '`'];
pub const WORD_CHARS: &str = "$";

/// Whether a line prefix triggers reindentation in the upstream language.
pub fn indent_on_input(line_prefix: &str) -> bool {
    matches!(
        trim_js_space(line_prefix),
        "case " | "default:" | "{" | "}" | "</"
    )
}

fn trim_js_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

/// The two independent dialect switches. The default accepts JavaScript.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct JavascriptOptions {
    pub typescript: bool,
    pub jsx: bool,
}

/// A visible syntax node type from the JavaScript grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JavascriptNodeKind(&'static str);

impl JavascriptNodeKind {
    pub const fn name(self) -> &'static str {
        self.0
    }

    pub fn is_error(self) -> bool {
        self.0 == "⚠"
    }
}

#[derive(Debug)]
struct Node {
    kind: JavascriptNodeKind,
    term: Option<u16>,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A borrowed JavaScript syntax node. An empty error range marks missing text.
#[derive(Clone, Copy, Debug)]
pub struct JavascriptNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> JavascriptNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    pub fn kind(self) -> JavascriptNodeKind {
        self.data().kind
    }

    pub fn range(self) -> Range<usize> {
        self.data().range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    /// Direct syntax children in source order. Whitespace is not included.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }

    /// Get the content covered by the upstream fold rule for this node.
    pub fn fold_range(self) -> Option<Range<usize>> {
        let range = &self.data().range;
        let name = self.kind().name();
        if name == "BlockComment" {
            if range.len() < 4
                || self
                    .children()
                    .next_back()
                    .is_some_and(|n| n.kind().is_error())
            {
                return None;
            }
            return Some(range.start + 2..range.end - 2);
        }
        let mut children = self.children();
        let first = children.next()?;
        let last = children.next_back()?;
        let start = match name {
            "Block" | "ClassBody" | "SwitchBody" | "EnumBody" | "ObjectExpression"
            | "ArrayExpression" | "ObjectType" | "JSXElement" => first.data().range.end,
            "JSXSelfClosingTag" | "JSXOpenTag" => {
                // If there are no attributes, the tag name immediately
                // precedes the closing token and the fold may be empty.
                let next = children.next().unwrap_or(last);
                if next.kind().is_error() {
                    return None;
                }
                next.data().range.end
            }
            _ => return None,
        };
        let end = if last.kind().is_error() {
            range.end
        } else {
            last.data().range.start
        };
        (start < end || start == end && name.starts_with("JSX")).then_some(start..end)
    }
}

/// A parsed JavaScript document and its native syntax tree.
#[derive(Debug)]
pub struct JavascriptDocument {
    text: Text,
    nodes: Vec<Node>,
    highlights: Vec<HighlightSpan<'static>>,
    recovered: bool,
    options: JavascriptOptions,
    expression: bool,
    strict: Option<Box<SyntaxTree>>,
    bracket_projection: OnceLock<Option<Box<brackets::Projection>>>,
}

impl JavascriptDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_options(text, JavascriptOptions::default())
    }

    pub fn parse_with_options(text: Text, options: JavascriptOptions) -> Self {
        Self::parse_top(text, options, false, None)
    }

    /// Parse a single expression, as used by embedded template languages.
    pub fn parse_expression(text: Text, options: JavascriptOptions) -> Self {
        Self::parse_top(text, options, true, None)
    }

    /// Apply byte edits and parse with the same dialect and top rule.
    /// Reuse a validated LR checkpoint before the first edit when possible.
    /// Invalid edits leave this document unchanged. GLR and recovered trees
    /// do not keep checkpoints.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
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
        Ok(Self::parse_top(
            text,
            self.options,
            self.expression,
            fragment,
        ))
    }

    /// LR checkpoint reuse in the last parse. The resume offset is in the
    /// original UTF-8 text. Native nodes and highlights are rebuilt.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.strict
            .as_deref()
            .map_or_else(ReuseStats::default, SyntaxTree::reuse_stats)
    }

    fn parse_top(
        text: Text,
        options: JavascriptOptions,
        expression: bool,
        fragment: Option<TreeFragment<'_>>,
    ) -> Self {
        let (nodes, recovered, strict) =
            Self::parse_nodes(text.as_str(), options, expression, fragment);
        Self::from_nodes(text, nodes, recovered, options, expression, strict)
    }

    fn parse_nodes(
        source: &str,
        options: JavascriptOptions,
        expression: bool,
        fragment: Option<TreeFragment<'_>>,
    ) -> (Vec<Node>, bool, Option<Box<SyntaxTree>>) {
        let parser = parser::configured(options, expression);
        match parser.parse_incremental(source, fragment) {
            Ok(tree) => (
                parser::collect(&tree, source.len()),
                false,
                Some(Box::new(tree)),
            ),
            Err(error) => match parser::try_glr(parser, source, error) {
                Ok(tree) => (parser::collect(&tree, source.len()), false, None),
                Err(error) => (
                    recovery::parse(
                        source,
                        options,
                        expression,
                        matches!(error, ParseError::UnexpectedToken { .. }),
                    ),
                    true,
                    None,
                ),
            },
        }
    }

    fn parse_for_tag_input(text: Text, options: JavascriptOptions) -> Self {
        let source = text.as_str();
        let parser = parser::configured(options, false);
        let (nodes, recovered) = match parser::parse_fresh(parser, source) {
            Ok(tree) => (parser::collect(&tree, source.len()), false),
            Err(_) => (
                parser.parse_recovering(source).map_or_else(
                    |_| recovery::parse(source, options, false, false),
                    |tree| parser::collect(&tree, source.len()),
                ),
                true,
            ),
        };
        Self::from_nodes(text, nodes, recovered, options, false, None)
    }

    fn from_nodes(
        text: Text,
        nodes: Vec<Node>,
        recovered: bool,
        options: JavascriptOptions,
        expression: bool,
        strict: Option<Box<SyntaxTree>>,
    ) -> Self {
        let highlights = highlight::collect(JavascriptNode {
            nodes: &nodes,
            id: 0,
        });
        Self {
            text,
            nodes,
            highlights,
            recovered,
            options,
            expression,
            strict,
            bracket_projection: OnceLock::new(),
        }
    }

    /// Whether the parser needed recovery for incomplete or invalid syntax.
    pub fn is_recovered(&self) -> bool {
        self.recovered
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// The root covers the entire document, including skipped whitespace.
    pub fn root(&self) -> JavascriptNode<'_> {
        JavascriptNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// All nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = JavascriptNode<'_>> {
        (0..self.nodes.len()).map(|id| JavascriptNode { id, ..self.root() })
    }

    /// Highlight spans clipped to a byte range. Return `None` if the range is
    /// reversed or an endpoint is not a UTF-8 character boundary.
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

    /// Find an upstream fold that starts on this line and continues past it.
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

    /// Compute indentation in columns. `None` marks an invalid byte position
    /// or a node that preserves its current indentation.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        self.embedded_indentation(position, settings, options, || Some(0))
    }

    pub(super) fn embedded_indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
        outer: impl FnOnce() -> Option<usize>,
    ) -> Option<usize> {
        indent::indentation(self, position, settings, options, outer)
    }

    fn path(
        &self,
        position: usize,
        side: Option<Assoc>,
    ) -> impl Iterator<Item = JavascriptNode<'_>> {
        path(&self.nodes, position, side)
    }
}

fn path(
    nodes: &[Node],
    position: usize,
    side: Option<Assoc>,
) -> impl Iterator<Item = JavascriptNode<'_>> {
    std::iter::successors(Some(JavascriptNode { nodes, id: 0 }), move |&node| {
        let children = &node.data().children;
        let index = children.partition_point(|&id| {
            let end = nodes[id].range.end;
            end < position || (end == position && side != Some(Assoc::Before))
        });
        let &id = children.get(index)?;
        let range = &nodes[id].range;
        (!range.is_empty()
            && (range.start < position || (side == Some(Assoc::After) && range.start == position)))
            .then_some(JavascriptNode { id, ..node })
    })
}
