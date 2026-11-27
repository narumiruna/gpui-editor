//! Native Lezer grammar services based on `@codemirror/lang-lezer` 6.0.2 and
//! `@lezer/lezer` 1.1.3. See `docs/licenses/codemirror-lang-lezer.txt`.
//!
//! Ranges use UTF-8 bytes in the original document. The native LR runtime
//! parses the pinned grammar tables and reuses validated prefix checkpoints.
//! Malformed input uses bounded LR recovery, then a local parser if that fails.
//!
//! Differences from upstream:
//!
//! - This module exposes native syntax and language data, not the JavaScript
//!   `LRLanguage` or `LanguageSupport` APIs. The
//!   [state attachment](crate::language_data::lezer_extension) tracks syntax
//!   without compiling grammars, scheduling background parsing, or exposing
//!   partial editor trees.
//! - Strict parsing can resume from a verified prefix, including a prefix
//!   retained after recovery, and replay supported shifted suffix decisions.
//!   Each parse rebuilds the native node and highlight views. Recovery parses
//!   from the start and does not reuse recovered nodes. The tree omits anonymous
//!   repeat nodes, packed buffers, and general node properties. Bracket and
//!   indent queries use the pinned delimiter properties.
//! - Syntax and line queries recognize LF, CRLF, and lone CR. Original text and
//!   UTF-8 byte offsets do not change.
//! - Bounded recovery can choose different branches or leave partial output.
//!   A local parser supplies a tree if LR recovery cannot finish within the
//!   allowed work and stack limits.

use std::ops::Range;

use crate::{ChangeSet, EditError, Text, commands::CommentTokens};

use super::{
    HighlightSpan, IndentOptions, LanguageData, StreamSettings,
    lezer::{NormalizedEdges, ReuseStats},
    line_projection::editor_lines,
};

mod brackets;
mod highlight;
mod indent;
mod lexer;
mod lr;
mod parser;
mod tables;

/// The upstream language name.
pub const NAME: &str = "lezer";
pub const LINE_COMMENT: &str = "//";
pub const BLOCK_COMMENT: (&str, &str) = ("/*", "*/");

/// The document-wide data provided by the upstream language wrapper.
pub fn language_data() -> LanguageData {
    LanguageData::default()
        .with(
            "commentTokens",
            CommentTokens {
                line: Some(LINE_COMMENT),
                block: Some(BLOCK_COMMENT),
            },
        )
        .with("indentOnInput", indent_on_input as fn(&str) -> bool)
}

/// Whether the text from a line's start to the cursor triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    trim_js_space(line_prefix) == "}"
}

fn trim_js_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

macro_rules! node_kinds {
    ($($plain:ident,)*; $($named:ident => $name:literal,)*) => {
        /// The visible node types in the Lezer grammar.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum LezerNodeKind { $($plain,)* $($named,)* }

        impl LezerNodeKind {
            /// The node's name in the upstream syntax tree.
            pub const fn name(self) -> &'static str {
                match self { $(Self::$plain => stringify!($plain),)* $(Self::$named => $name,)* }
            }

            fn from_name(name: &str) -> Self {
                match name {
                    $(stringify!($plain) => Self::$plain,)*
                    $($name => Self::$named,)*
                    _ => unreachable!("unknown node in the pinned Lezer grammar: {name}"),
                }
            }
        }
    };
}

node_kinds! {
    Grammar, LineComment, BlockComment, RuleDeclaration, RuleName, Props, Prop, AtName, Name,
    Literal, PropEsc, ParamList, Body, CharSet, AnyChar, InvertedCharSet, ScopedName, Call, ArgList,
    CharClass, Optional, Repeat, Repeat1, InlineRule, ParenExpression, Specialization, Sequence,
    PrecedenceMarker, PrecedenceName, AmbiguityMarker, Choice, PrecedenceDeclaration,
    PrecedenceBody, Precedence, TokensDeclaration, TokensBody, TokenPrecedenceDeclaration,
    TokenConflictDeclaration, ConflictBody, LiteralTokenDeclaration, LocalTokensDeclaration,
    ElseToken, ExternalTokensDeclaration, Token, ExternalConflictDeclaration,
    ExternalPropDeclaration, ExternalPropSourceDeclaration, ExternalSpecializeDeclaration,
    ContextDeclaration, DialectsDeclaration, DialectBody, TopSkipDeclaration, SkipScope, SkipBody,
    DetectDelimDeclaration,
    ;
    Error => "⚠",
    OpenBrace => "{", CloseBrace => "}", OpenBracket => "[", CloseBracket => "]",
    OpenParen => "(", CloseParen => ")", Equal => "=", Dot => ".", Bar => "|", Bang => "!",
    Tilde => "~", Star => "*", Plus => "+", Question => "?",
    AtSpecialize => "@specialize", AtExtend => "@extend", AtTop => "@top",
    AtPrecedence => "@precedence", AtLeft => "@left", AtRight => "@right", AtCut => "@cut",
    AtTokens => "@tokens", AtConflict => "@conflict", AtLocal => "@local", AtElse => "@else",
    AtExternal => "@external", AtContext => "@context", AtDialects => "@dialects",
    AtSkip => "@skip", AtDetectDelim => "@detectDelim",
    Tokens => "tokens", From => "from", PropKeyword => "prop", As => "as",
    PropSource => "propSource", Extend => "extend", Specialize => "specialize",
}

#[derive(Debug)]
struct Node {
    kind: LezerNodeKind,
    // Some LR node types have the same name but different IDs.
    term: Option<u16>,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A borrowed syntax node. A zero-width error can mark missing syntax.
#[derive(Clone, Copy, Debug)]
pub struct LezerNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> LezerNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    pub fn kind(self) -> LezerNodeKind {
        self.data().kind
    }

    pub fn range(self) -> Range<usize> {
        self.data().range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    /// Direct children in source order. Whitespace is not included.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }

    /// Range inside a rule, token, skip, or precedence body.
    pub fn fold_range(self) -> Option<Range<usize>> {
        use LezerNodeKind::*;
        if !matches!(self.kind(), Body | TokensBody | SkipBody | PrecedenceBody) {
            return None;
        }
        let first = self.children().next()?;
        let last = self.children().next_back()?;
        let start = first.data().range.end;
        (start < last.data().range.start).then(|| {
            start..if last.kind() == Error {
                self.data().range.end
            } else {
                last.data().range.start
            }
        })
    }
}

/// One Lezer grammar and its recovered syntax tree.
#[derive(Debug)]
pub struct LezerDocument {
    text: Text,
    editor_lines: Option<Text>,
    nodes: Vec<Node>,
    highlights: Vec<HighlightSpan<'static>>,
    published: Option<Box<lr::Cache>>,
}

impl LezerDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_reuse(text, None)
    }

    /// Apply byte edits and reparse. A valid prefix checkpoint and shifted
    /// suffix decisions can be reused. Invalid edits leave this document
    /// unchanged. If recovery is needed, keep only the preceding strict prefix
    /// as a seed for a later strict reparse.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let reuse = self
            .published
            .as_deref()
            .map(|cache| NormalizedEdges::new(&cache.tree, self.text.len(), changes));
        Ok(Self::parse_with_reuse(text, reuse))
    }

    /// LR checkpoint reuse in the last parse. The resume offset is in the
    /// original UTF-8 text, including CRLF. Native views are rebuilt.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.published
            .as_deref()
            .map_or_else(ReuseStats::default, |cache| cache.reuse)
    }

    /// Count the consuming token decisions reused in the shifted suffix.
    pub fn reused_suffix_actions(&self) -> usize {
        self.published
            .as_deref()
            .map_or(0, |cache| cache.tree.reused_suffix_actions())
    }

    fn parse_with_reuse(text: Text, reuse: Option<NormalizedEdges<'_>>) -> Self {
        let (nodes, published) = lr::parse(text.as_str(), reuse);
        let highlights = highlight::collect(&nodes);
        Self {
            editor_lines: editor_lines(text.as_str()),
            text,
            nodes,
            highlights,
            published: published.map(Box::new),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// Text for syntax line queries. Treat lone CR as a break without changing
    /// the original byte offsets. Use [`Self::text`] for original bytes.
    pub fn syntax_lines(&self) -> &Text {
        self.editor_lines.as_ref().unwrap_or(&self.text)
    }

    /// The root spans the full document, including skipped whitespace.
    pub fn root(&self) -> LezerNode<'_> {
        LezerNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// All nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = LezerNode<'_>> {
        (0..self.nodes.len()).map(|id| LezerNode { id, ..self.root() })
    }

    /// Highlight spans clipped to the supplied byte range. Return `None` if
    /// the range is reversed or either endpoint is invalid.
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

    /// Find the outermost fold that starts on this line and continues past it.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        self.path(line_end, true)
            .filter_map(LezerNode::fold_range)
            .find(|range| (line_start..=line_end).contains(&range.start) && line_end < range.end)
    }

    /// Compute the indentation in columns, using detected braces, brackets,
    /// and parentheses. Return `None` for an invalid UTF-8 byte position.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        indent::indentation(self, position, settings, options)
    }

    fn path(&self, position: usize, include_start: bool) -> impl Iterator<Item = LezerNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let i = children.partition_point(|&id| self.nodes[id].range.end <= position);
            next = children.get(i).and_then(|&id| {
                let start = self.nodes[id].range.start;
                (start < position || (include_start && start == position))
                    .then_some(LezerNode { id, ..node })
            });
            Some(node)
        })
    }
}
