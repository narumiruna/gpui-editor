//! Native JSON language services based on `@codemirror/lang-json` 6.0.2 and
//! `@lezer/json` 1.0.3. See `docs/licenses/codemirror-lang-json.txt`.
//!
//! All ranges use UTF-8 bytes into the original [`Text`]. The syntax tree uses
//! strict Lezer parsing first, then bounded Lezer recovery. If recovery cannot
//! finish within its limits, the local parser supplies the tree. The JSON parse
//! linter validates the input separately from the syntax tree.

use std::{
    ops::Range,
    sync::{Arc, LazyLock, OnceLock},
};

use crate::{
    Assoc, ChangeSet, EditError, RangeSet, Text,
    autocomplete::brackets::{CloseBracketConfig, CloseBracketSyntax},
    lint::Diagnostic,
    view::{MarkDecoration, Viewport},
};

use super::{
    BidiIsolateOptions, HighlightSpan, HighlightTag, IndentContext, IndentOptions, LanguageData,
    StreamSettings,
    bidi::isolate_marks,
    lezer::{ReuseStats, SyntaxTree, TreeFragment},
    line_projection::{NormalizedInput, editor_lines},
};

mod brackets;
mod lexer;
mod linter;
mod lr;
mod parser;

/// Brackets the upstream language asks the editor to close automatically.
pub const CLOSE_BRACKETS: [char; 3] = ['[', '{', '"'];

pub fn language_data() -> LanguageData {
    LanguageData::default()
        .with("closeBrackets", CLOSE_BRACKETS)
        .with("indentOnInput", indent_on_input as fn(&str) -> bool)
}

pub fn close_bracket_config() -> CloseBracketConfig<'static> {
    CloseBracketConfig {
        brackets: &["[", "{", "\""],
        ..CloseBracketConfig::default()
    }
}

/// Whether the text from a line's start to the cursor triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    matches!(trim_js_space(line_prefix), "}" | "]")
}

/// The visible node types in the JSON grammar, including punctuation and errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonNodeKind {
    JsonText,
    Object,
    Array,
    Property,
    PropertyName,
    String,
    Number,
    True,
    False,
    Null,
    OpenBrace,
    CloseBrace,
    OpenBracket,
    CloseBracket,
    Colon,
    Comma,
    Error,
}

impl JsonNodeKind {
    /// The node's name in the upstream syntax tree.
    pub const fn name(self) -> &'static str {
        use JsonNodeKind::*;
        match self {
            JsonText => "JsonText",
            Object => "Object",
            Array => "Array",
            Property => "Property",
            PropertyName => "PropertyName",
            String => "String",
            Number => "Number",
            True => "True",
            False => "False",
            Null => "Null",
            OpenBrace => "{",
            CloseBrace => "}",
            OpenBracket => "[",
            CloseBracket => "]",
            Colon => ":",
            Comma => ",",
            Error => "⚠",
        }
    }

    /// Whether the pinned grammar specifies automatic bidirectional text isolation.
    pub const fn isolates_direction(self) -> bool {
        matches!(self, Self::PropertyName | Self::String)
    }

    fn highlight(self) -> Option<&'static [HighlightTag]> {
        static TAGS: LazyLock<[HighlightTag; 8]> = LazyLock::new(|| {
            [
                "string",
                "propertyName",
                "number",
                "bool",
                "null",
                "separator",
                "squareBracket",
                "brace",
            ]
            .map(HighlightTag::new)
        });
        use JsonNodeKind::*;
        let index = match self {
            String => 0,
            PropertyName => 1,
            Number => 2,
            True | False => 3,
            Null => 4,
            Colon | Comma => 5,
            OpenBracket | CloseBracket => 6,
            OpenBrace | CloseBrace => 7,
            _ => return None,
        };
        Some(std::slice::from_ref(&TAGS[index]))
    }
}

#[derive(Debug)]
struct Node {
    kind: JsonNodeKind,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A borrowed syntax node. An empty error range can mark missing syntax. A
/// nonempty error may have a child for a recognized but unexpected token.
#[derive(Clone, Copy)]
pub struct JsonNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> JsonNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    pub fn kind(self) -> JsonNodeKind {
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

    // Lezer's enterUnfinishedNodesBefore: only an inserted, zero-width
    // error at the end marks an unfinished node. A skipped token does not.
    fn enter_unfinished_before(mut self, position: usize) -> Self {
        let children = &self.data().children;
        let before = children.partition_point(|&id| self.nodes[id].range.start < position);
        let mut scan = before.checked_sub(1).map(|i| children[i]);
        while let Some(id) = scan {
            let node = &self.nodes[id];
            let Some((&last, previous)) = node.children.split_last() else {
                break;
            };
            let end = &self.nodes[last];
            if end.range.end != node.range.end {
                break;
            }
            if end.kind == JsonNodeKind::Error && end.range.is_empty() {
                self.id = id;
                scan = previous.last().copied();
            } else {
                scan = Some(last);
            }
        }
        self
    }

    /// Range inside the first and last children of an object or array.
    /// For an unfinished container with a final error, fold to the node's end.
    pub fn fold_range(self) -> Option<Range<usize>> {
        if !matches!(self.kind(), JsonNodeKind::Array | JsonNodeKind::Object) {
            return None;
        }
        let first = self.children().next()?;
        let last = self.children().next_back()?;
        let start = first.range().end;
        (start < last.range().start).then(|| {
            let end = if last.kind() == JsonNodeKind::Error {
                self.range().end
            } else {
                last.range().start
            };
            start..end
        })
    }
}

/// One JSON document and its recovered syntax tree.
#[derive(Debug)]
pub struct JsonDocument {
    text: Text,
    nodes: Vec<Node>,
    diagnostic: OnceLock<Option<Diagnostic>>,
    incremental: Option<Box<SyntaxTree>>,
    editor_lines: OnceLock<Option<Text>>,
    isolate_ranges: OnceLock<Vec<Range<usize>>>,
}

impl JsonDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_fragments(text, &[])
    }

    /// Apply edits and parse the result. Reuse an unchanged strict prefix and
    /// eligible token decisions in a shifted suffix. Recovery publishes no
    /// reuse and retains only verified strict prefix checkpoints for later
    /// repairs. Invalid edits leave this document unchanged.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let old_len = self.text.len();
        let desc = changes.desc(old_len)?;
        let fragments = if let Some(tree) = &self.incremental {
            desc.gaps()
                .filter(|(old, new)| {
                    (old.start == 0 && new.start == 0)
                        || (old.end == old_len && new.end == text.len() && old.start != new.start)
                })
                .map(|(old_range, new_range)| TreeFragment {
                    tree,
                    old_range,
                    new_range,
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(Self::parse_with_fragments(text, &fragments))
    }

    pub fn reuse_stats(&self) -> ReuseStats {
        self.incremental
            .as_deref()
            .map_or_else(ReuseStats::default, SyntaxTree::reuse_stats)
    }

    /// Consuming token decisions reused in a shifted suffix. This does not
    /// count shared syntax nodes or skipped parser transitions.
    pub fn reused_suffix_actions(&self) -> usize {
        self.incremental
            .as_deref()
            .map_or(0, SyntaxTree::reused_suffix_actions)
    }

    fn parse_with_fragments(text: Text, fragments: &[TreeFragment<'_>]) -> Self {
        let source = text.as_str();
        let (nodes, diagnostic, incremental) =
            if let Ok(tree) = lr::PARSER.parse_incremental_json_recovering(source, fragments) {
                let nodes = lr::nodes(&tree);
                // Keep diagnostic positions independent of the recovery markers.
                let diagnostic = if nodes.iter().any(|node| node.kind == JsonNodeKind::Error) {
                    OnceLock::new()
                } else {
                    OnceLock::from(None)
                };
                let incremental = tree.has_incremental_state().then(|| Box::new(tree));
                (nodes, diagnostic, incremental)
            } else {
                let (nodes, diagnostic) = parser::parse(source);
                (nodes, OnceLock::from(diagnostic), None)
            };
        Self {
            text,
            nodes,
            diagnostic,
            incremental,
            editor_lines: OnceLock::new(),
            isolate_ranges: OnceLock::new(),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// Text for syntax line queries. Treat lone CR as a break without changing
    /// the original byte offsets. Parsing and linting use [`Self::text`].
    pub fn syntax_lines(&self) -> &Text {
        self.editor_lines
            .get_or_init(|| editor_lines(self.text.as_str()))
            .as_ref()
            .unwrap_or(&self.text)
    }

    /// The root spans the full document, including skipped whitespace.
    pub fn root(&self) -> JsonNode<'_> {
        JsonNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// All nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = JsonNode<'_>> {
        (0..self.nodes.len()).map(|id| JsonNode {
            nodes: &self.nodes,
            id,
        })
    }

    /// Resolve the deepest node at a byte position. Select a side to
    /// include nodes whose end or start is exactly at that position.
    pub fn node_at(&self, position: usize, side: Option<Assoc>) -> Option<JsonNode<'_>> {
        if !self.text.is_char_boundary(position) {
            return None;
        }
        self.path(position, |node| {
            let Range { start, end } = node.range;
            start < end
                && match side {
                    Some(Assoc::Before) => start < position && position <= end,
                    Some(Assoc::After) => start <= position && position < end,
                    None => start < position && position < end,
                }
        })
        .last()
    }

    /// Highlight spans clipped to the requested range. Return `None` if either
    /// endpoint is invalid or the range is reversed.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.text.slice(range.clone())?;
        // Only the last node that starts before the range can be a styled
        // leaf that overlaps its start. All other styled nodes start inside it.
        let first = self
            .nodes
            .partition_point(|node| node.range.start < range.start)
            .saturating_sub(1);
        let end = self
            .nodes
            .partition_point(|node| node.range.start < range.end);
        Some(self.nodes[first..end].iter().filter_map(move |node| {
            let tags = node.kind.highlight()?;
            let start = node.range.start.max(range.start);
            let end = node.range.end.min(range.end);
            (start < end).then_some(HighlightSpan {
                range: start..end,
                tags,
            })
        }))
    }

    /// Build automatic `cm-iso` marks for property names and strings that touch
    /// the visible ranges. The viewport selects full nodes without clipping the
    /// marks. Default options restrict marks to the ranges selected by the
    /// CodeMirror RTL line optimization. Empty recovery nodes have no text to
    /// isolate.
    ///
    /// Return [`EditError::InvalidRange`] for viewport endpoints outside this
    /// document or between UTF-8 bytes of a character.
    ///
    /// This method installs no extension. Use the result at lowest precedence
    /// in [`crate::view::bidi::BIDI_ISOLATED_RANGES`] and as outer decorations on
    /// the same text snapshot. Refresh after document, viewport, or direction changes.
    pub fn bidi_isolate_marks(
        &self,
        viewport: &Viewport,
        options: BidiIsolateOptions,
    ) -> Result<Arc<RangeSet<MarkDecoration>>, EditError> {
        let ranges = self.isolate_ranges.get_or_init(|| {
            self.nodes
                .iter()
                .filter(|node| node.kind.isolates_direction() && !node.range.is_empty())
                .map(|node| node.range.clone())
                .collect()
        });
        isolate_marks(&self.text, self.syntax_lines(), ranges, viewport, options)
    }

    /// Find a fold that begins on the supplied line and continues past its end.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        self.path(line_end, |node| line_end < node.range.end)
            .filter_map(JsonNode::fold_range)
            .find(|range| (line_start..=line_end).contains(&range.start) && range.end > line_end)
    }

    /// Indent a position using the innermost container or recovered opener.
    /// Use the reference line's indentation on a container's matching closer,
    /// or add one unit otherwise.
    /// Return `None` for an invalid UTF-8 byte position.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        let text = self.syntax_lines();
        let cx = IndentContext::new(text, position, settings, options)?;
        let inner = self
            .node_at(position, None)?
            .enter_unfinished_before(position);
        let strategy = std::iter::successors(Some(inner), |node| node.parent()).find_map(|node| {
            use JsonNodeKind::*;
            let closer = match node.kind() {
                Object => Some('}'),
                Array => Some(']'),
                // A recovered error can contain one skipped token. When it
                // is an opener, CodeMirror uses its default delimited rule.
                Error => match node.children().next()?.kind() {
                    OpenBrace | OpenBracket => None,
                    _ => return None,
                },
                _ => return None,
            };
            Some((node, closer))
        });
        let Some((node, closer)) = strategy else {
            return Some(0);
        };
        let after = cx.text_after_pos(position, Assoc::After)?;
        let from = node.data().range.start;
        let mut reference = text.line_at(from)?.range.start;
        loop {
            let mut at_break = self.node_at(reference, None)?;
            // Lezer treats nodes with the same start as one when choosing
            // the reference line, even if a recovered sibling ends here.
            while let Some(parent) = at_break
                .parent()
                .filter(|parent| parent.data().range.start == at_break.data().range.start)
            {
                at_break = parent;
            }
            let covering = &at_break.data().range;
            if covering.end > from {
                // The node covers the opener, so it is an ancestor.
                break;
            }
            reference = text.line_at(covering.start)?.range.start;
        }
        let base = cx.line_indent(reference, Assoc::After)?;
        Some(
            if closer.is_some_and(|closer| trim_js_space(after).starts_with(closer)) {
                base
            } else {
                base.saturating_add(cx.unit())
            },
        )
    }

    /// The first syntax diagnostic from the local parser, if any. It does not
    /// depend on where Lezer recovers. Its range is empty and uses UTF-8 byte
    /// offsets. Its position and message can differ from [`json_parse_linter`].
    pub fn diagnostic(&self) -> Option<&Diagnostic> {
        self.diagnostic
            .get_or_init(|| parser::diagnostic(self.text.as_str()))
            .as_ref()
    }

    fn path(
        &self,
        position: usize,
        contains: impl Fn(&Node) -> bool,
    ) -> impl Iterator<Item = JsonNode<'_>> {
        let mut next = Some(self.root());
        std::iter::from_fn(move || {
            let node = next?;
            let children = &node.data().children;
            let start = children.partition_point(|&id| self.nodes[id].range.end < position);
            next = children[start..]
                .iter()
                .map(|&id| JsonNode { id, ..node })
                .take_while(|child| child.data().range.start <= position)
                .find(|child| contains(child.data()));
            Some(node)
        })
    }
}

impl CloseBracketSyntax for JsonDocument {
    fn resolve(&self, pos: usize, side: Option<Assoc>) -> Option<usize> {
        self.node_at(pos, side).map(|node| node.id)
    }

    fn range(&self, node: usize) -> Range<usize> {
        self.nodes[node].range.clone()
    }

    fn parent(&self, node: usize) -> Option<usize> {
        self.nodes[node].parent
    }

    fn first_child(&self, node: usize) -> Option<usize> {
        self.nodes[node].children.first().copied()
    }
}

/// Validate strict JSON and report at most one error with an empty range.
///
/// The upstream linter forwards the host engine's `JSON.parse` error message.
/// This native linter uses deterministic wording based on common V8 errors,
/// not the exact wording of every engine or V8 version. Errors without a
/// reported position use `0`, as upstream does. Text quoted in a message
/// cannot supply a position. Unpaired UTF-16 units in excerpts become `�`.
///
/// The range is a UTF-8 byte offset in the original input. Messages print a
/// zero-based UTF-16 position and a one-based line and UTF-16 column. The input
/// keeps all line endings. A CRLF counts as two units in the position and one
/// line break. A lone CR also counts as a line break in messages. CodeMirror's
/// default state normalizes both forms to LF before it calls `JSON.parse`.
pub fn json_parse_linter(text: &Text) -> Vec<Diagnostic> {
    linter::diagnostic(text.as_str()).into_iter().collect()
}

/// Validate a default editor document with one LF per logical line break.
/// Keep diagnostic ranges in the original UTF-8 bytes. As in
/// [`json_parse_linter`], only the validator supplies a diagnostic position,
/// never source text quoted in a message.
pub fn json_editor_linter(text: &Text) -> Vec<Diagnostic> {
    let input = NormalizedInput::new(text.as_str());
    linter::diagnostic(&input.text)
        .map(|mut diagnostic| {
            let range = diagnostic.range;
            diagnostic.range = input.original_byte(range.start)..input.original_byte(range.end);
            diagnostic
        })
        .into_iter()
        .collect()
}

fn trim_js_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}
