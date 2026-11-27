//! Native Liquid template services from `@codemirror/lang-liquid` 6.3.4.
//! The package contains its own Lezer grammar. See
//! `docs/licenses/codemirror-lang-liquid.txt` for the MIT notice.
//!
//! Liquid and HTML ranges use UTF-8 bytes. The HTML parser receives only the
//! text and raw text of the template. Its ranges are mapped back to the source.
//! Strict LR parses can reuse prefix checkpoints.

use std::ops::Range;

use crate::{ChangeSet, EditError, Text, commands::CommentTokens};

use super::{
    HighlightSpan, IndentOptions, LanguageData, StreamSettings,
    html::HtmlOptions,
    lezer::{ReuseStats, SyntaxTree},
};

mod completion;
mod highlight;
mod overlay;
mod parser;
mod recovery;
mod services;
mod tables;
mod tokens;

pub use super::jinja::{close_percent_brace, close_percent_brace_in};
pub(crate) use completion::has_recovered_end_tag_name;
pub use completion::{
    LiquidCompletionConfig, LiquidPropertyCompletion, liquid_completion_source,
    liquid_completion_source_with,
};
pub use overlay::{LiquidHtmlMount, LiquidHtmlRange};

pub const NAME: &str = "liquid";
pub const LINE_COMMENT: &str = "#";
/// The brackets to close automatically in the HTML overlay.
pub const HTML_CLOSE_BRACKETS: [char; 1] = ['{'];

/// Default data for a Liquid tag or expression, outside the HTML overlay.
pub fn language_data() -> LanguageData {
    LanguageData::default()
        .with(
            "commentTokens",
            CommentTokens {
                line: Some(LINE_COMMENT),
                block: None,
            },
        )
        .with("indentOnInput", indent_on_input as fn(&str) -> bool)
}

/// Language data for plain HTML in a Liquid template.
pub fn html_language_data() -> LanguageData {
    super::html::language_data().with("closeBrackets", HTML_CLOSE_BRACKETS)
}

/// Whether this line prefix matches the upstream Liquid indent trigger.
pub fn indent_on_input(prefix: &str) -> bool {
    let Some(after) = trim_space(prefix).strip_prefix("{%") else {
        return false;
    };
    matches!(
        trim_space(after.strip_prefix('-').unwrap_or(after)),
        "" | "end" | "elsif" | "else" | "when"
    )
}

fn trim_space(source: &str) -> &str {
    source.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

/// The native HTML dialect to use in the overlay.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LiquidOptions {
    pub html: HtmlOptions,
}

macro_rules! node_kinds {
    ($($kind:ident $(= $name:literal)?),+ $(,)?) => {
        /// The visible node types of the bundled grammar.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum LiquidNodeKind { $($kind,)+ }

        impl LiquidNodeKind {
            pub const fn name(self) -> &'static str {
                match self { $(Self::$kind => node_kinds!(@name $kind $($name)?)),+ }
            }

            fn from_name(name: &str) -> Self {
                match name {
                    $(node_kinds!(@name $kind $($name)?) => Self::$kind,)+
                    _ => unreachable!("unknown node in the pinned Liquid grammar: {name}"),
                }
            }
        }
    };
    (@name $kind:ident $name:literal) => { $name };
    (@name $kind:ident) => { stringify!($kind) };
}

node_kinds! {
    Template, Text, Interpolation, InterpolationStart = "{{", InterpolationEnd = "}}",
    Tag, EndTag, TagName, TagStart = "{%", TagEnd = "%}",
    IfDirective, UnlessDirective, CaseDirective, ForDirective, TableDirective,
    CaptureDirective, RawDirective, Comment, RawText, CommentText, InlineComment,
    VariableName, MemberExpression, Dot = ".", PropertyName, SubscriptExpression,
    BinaryExpression, Contains = "contains", CompareOp, LogicOp, AssignmentExpression,
    AssignOp, OpenParen = "(", CloseParen = ")", RangeExpression, Dots = "..",
    BooleanLiteral, Empty = "empty", ForLoop = "forloop", TableRowLoop = "tablerowloop",
    StringLiteral, NumberLiteral, Filter, Pipe = "|", FilterName, Colon = ":",
    NamedArgument, ParameterName, Comma = ",", Parameter, RenderParameter,
    If = "if", EndIf = "endif", Elsif = "elsif", Else = "else", Unless = "unless",
    EndUnless = "endunless", Case = "case", EndCase = "endcase", When = "when",
    For = "for", In = "in", EndFor = "endfor", TableRow = "tablerow", EndTableRow = "endtablerow",
    Break = "break", Continue = "continue", Cycle = "cycle", CommentKeyword = "comment",
    EndComment = "endcomment", Raw = "raw", EndRaw = "endraw", Echo = "echo", Render = "render",
    With = "with", As = "as", Include = "include", Assign = "assign", Capture = "capture",
    EndCapture = "endcapture", Increment = "increment", Decrement = "decrement", Liquid = "liquid",
    Error = "⚠",
}

#[derive(Debug)]
struct Node {
    kind: LiquidNodeKind,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A borrowed Liquid syntax node. A zero-width error can mark missing syntax.
#[derive(Clone, Copy, Debug)]
pub struct LiquidNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> LiquidNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    pub fn kind(self) -> LiquidNodeKind {
        self.data().kind
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

    /// The content between directive tags, or to the end of an unfinished directive.
    pub fn fold_range(self) -> Option<Range<usize>> {
        services::fold_range(self)
    }
}

/// A Liquid source tree with an HTML overlay that can contain native child languages.
#[derive(Debug)]
pub struct LiquidDocument {
    text: Text,
    nodes: Vec<Node>,
    html: Option<LiquidHtmlMount>,
    highlights: Vec<HighlightSpan<'static>>,
    options: LiquidOptions,
    strict: Option<Box<SyntaxTree>>,
}

impl LiquidDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_options(text, LiquidOptions::default())
    }

    pub fn parse_with_options(text: Text, options: LiquidOptions) -> Self {
        Self::parse_with_prefix(text, options, None)
    }

    /// Apply UTF-8 byte edits and rebuild the HTML projection with the original options.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let end = changes
            .changes()
            .next()
            .map_or(text.len(), |(range, _)| range.start);
        let prefix = self.strict.as_deref().map(|tree| (tree, end));
        Ok(Self::parse_with_prefix(text, self.options, prefix))
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
        options: LiquidOptions,
        prefix: Option<(&SyntaxTree, usize)>,
    ) -> Self {
        let source = text.as_str();
        let (nodes, strict) = parser::parse(source, prefix);
        let html = LiquidHtmlMount::new(source, &nodes, options.html);
        let highlights = highlight::collect(&nodes);
        Self {
            text,
            nodes,
            html,
            highlights,
            options,
            strict: strict.map(Box::new),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn root(&self) -> LiquidNode<'_> {
        LiquidNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// Liquid nodes in pre-order. The HTML overlay has a separate tree.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = LiquidNode<'_>> {
        (0..self.nodes.len()).map(|id| LiquidNode { id, ..self.root() })
    }

    /// The HTML document and its mapping, if the template contains outer text.
    pub fn html(&self) -> Option<&LiquidHtmlMount> {
        self.html.as_ref()
    }

    /// Liquid and HTML spans clipped to a valid original byte range.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.text.slice(range.clone())?;
        let mut spans = Vec::new();
        if !range.is_empty() {
            let first = self
                .highlights
                .partition_point(|span| span.range.end <= range.start);
            spans.extend(
                self.highlights[first..]
                    .iter()
                    .take_while(|span| span.range.start < range.end)
                    .map(|span| HighlightSpan {
                        range: span.range.start.max(range.start)..span.range.end.min(range.end),
                        tags: span.tags,
                    }),
            );
            if let Some(html) = &self.html {
                html.highlights(&range, &mut spans);
                spans.sort_unstable_by_key(|span| span.range.start);
            }
        }
        Some(spans.into_iter())
    }

    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        services::foldable(self, line_start, line_end)
    }

    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        services::indentation(self, position, settings, options)
    }

    fn path(&self, position: usize, include_start: bool) -> impl Iterator<Item = LiquidNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let index = children.partition_point(|&id| self.nodes[id].range.end <= position);
            next = children.get(index).and_then(|&id| {
                let start = self.nodes[id].range.start;
                (start < position || (include_start && start == position))
                    .then_some(LiquidNode { id, ..node })
            });
            Some(node)
        })
    }
}
