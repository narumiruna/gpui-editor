//! Native Java syntax and editor services from `@codemirror/lang-java` 6.0.2
//! and `@lezer/java` 1.2.1. See `docs/licenses/codemirror-lang-java.txt`.
//!
//! Strict LR parses can reuse a validated prefix checkpoint. GLR and bounded
//! recovery start from scratch, but a recovered document can keep the strict
//! pass's checkpoints for later edits. On invalid input up to 16 KiB, if bounded
//! recovery fails, the fallback can add up to eight closing tokens at EOF or
//! keep a prefix that parses. Recovery can differ from upstream Lezer.
//! Positions and ranges are UTF-8 bytes in the original text. Legacy
//! [`JavaDocument`] constructors use raw input for syntax and highlights.
//! On documents with lone CR, their editor services use projected lines.
//! The synchronous [`JavaLanguage`](crate::language_data::JavaLanguage)
//! attachment parses normalized editor lines and maps the tree back to bytes.

use std::{
    ops::Range,
    sync::{Arc, OnceLock},
};

use crate::{
    Assoc, ChangeSet, EditError, RangeSet, Text,
    commands::CommentTokens,
    view::{MarkDecoration, Viewport},
};

use super::{
    BidiIsolateOptions, HighlightSpan, IndentOptions, LanguageData, StreamSettings,
    bidi::isolate_marks,
    lezer::{NodeType, NodeTypes, ParseError, ReuseStats, SyntaxTree},
    line_projection::{NormalizedInput, editor_lines},
};

mod brackets;
mod highlight;
mod indent;
mod parser;
mod tables;

#[cfg(test)]
mod tests;

pub use parser::{Mode as JavaTop, class_content_parser, parser};
pub(crate) use parser::{parse_raw, stock_data_facet, stock_node_types};

pub const NAME: &str = "java";
pub const LINE_COMMENT: &str = "//";
pub const BLOCK_COMMENT: (&str, &str) = ("/*", "*/");

pub(crate) fn language_data() -> LanguageData {
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

/// Whether the text from the line start to the cursor triggers reindentation.
pub fn indent_on_input(line_prefix: &str) -> bool {
    matches!(trim_js_space(line_prefix), "case " | "default:" | "{" | "}")
}

fn trim_js_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

#[derive(Debug)]
struct Node {
    term: u16,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A node in the Java document tree. `⚠` marks local recovery.
#[derive(Clone, Copy, Debug)]
pub struct JavaNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> JavaNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    /// The term ID in the pinned grammar.
    pub fn term(self) -> u16 {
        self.data().term
    }

    /// The node type name in the pinned grammar.
    pub fn name(self) -> &'static str {
        parser::name(self.term())
    }

    pub fn range(self) -> Range<usize> {
        self.data().range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    /// Direct children in source order, including named comments.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }

    /// The pinned grammar group, if this type is in one.
    pub fn group(self) -> Option<&'static str> {
        group(self.term())
    }

    /// Whether the grammar marks the type as skipped.
    pub fn is_skipped(self) -> bool {
        self.term() <= 2
    }

    /// The type of a closing delimiter from the grammar's `closedBy` property.
    pub fn closed_by(self) -> Option<&'static str> {
        closed_by(self.term())
    }

    /// The type of an opening delimiter from the grammar's `openedBy` property.
    pub fn opened_by(self) -> Option<&'static str> {
        opened_by(self.term())
    }

    /// Whether the grammar isolates the node using automatic bidi direction.
    pub fn isolates_direction(self) -> bool {
        isolates_direction(self.term())
    }

    /// The content range for a node with an upstream fold rule.
    pub fn fold_range(self) -> Option<Range<usize>> {
        let range = &self.data().range;
        match fold_rule(self.name())? {
            FoldRule::Comment => Some(range.start + 2..range.end - 2),
            FoldRule::Inside => {
                let mut children = self.children();
                let start = children.next()?.data().range.end;
                let last = children.next_back()?;
                (start < last.data().range.start).then_some(
                    start..if last.term() == 0 {
                        range.end
                    } else {
                        last.data().range.start
                    },
                )
            }
        }
    }
}

fn group(term: u16) -> Option<&'static str> {
    if tables::STATEMENTS.contains(&term) {
        Some("Statement")
    } else if tables::EXPRESSIONS.contains(&term) {
        Some("Expression")
    } else if tables::TYPES.contains(&term) {
        Some("Type")
    } else {
        None
    }
}

fn closed_by(term: u16) -> Option<&'static str> {
    match term {
        11 => Some(")"),
        53 => Some("}"),
        _ => None,
    }
}

fn opened_by(term: u16) -> Option<&'static str> {
    match term {
        10 => Some("("),
        52 => Some("{"),
        _ => None,
    }
}

fn isolates_direction(term: u16) -> bool {
    matches!(term, 1 | 2 | 80 | 81)
}

enum FoldRule {
    Comment,
    Inside,
}

fn fold_rule(name: &str) -> Option<FoldRule> {
    Some(match name {
        "BlockComment" => FoldRule::Comment,
        "Block"
        | "SwitchBlock"
        | "ClassBody"
        | "ElementValueArrayInitializer"
        | "ModuleBody"
        | "EnumBody"
        | "ConstructorBody"
        | "InterfaceBody"
        | "ArrayInitializer" => FoldRule::Inside,
        _ => return None,
    })
}

/// One Java document with a syntax tree and editor services.
#[derive(Debug)]
pub struct JavaDocument {
    text: Text,
    mode: JavaTop,
    editor_lines: Option<Text>,
    editor_nodes: Option<Vec<Node>>,
    isolate_ranges: OnceLock<Vec<Range<usize>>>,
    nodes: Vec<Node>,
    node_types: NodeTypes,
    highlights: Vec<HighlightSpan<'static>>,
    parse_error: Option<ParseError>,
    incremental: Option<Box<SyntaxTree>>,
    // Attached documents keep normalized checkpoints and report raw offsets.
    editor_reuse: Option<ReuseStats>,
}

impl JavaDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_prefix(text, JavaTop::Program, None)
    }

    /// Parse Java members without an enclosing class. The same editor
    /// services apply, and edits keep the `ClassContent` top rule.
    pub fn parse_class_content(text: Text) -> Self {
        Self::parse_with_prefix(text, JavaTop::ClassContent, None)
    }

    /// Apply byte edits and parse the result. Reuse a validated LR checkpoint
    /// before the first edit when possible. Only strict LR checkpoints can
    /// seed later parses. Invalid edits leave this document unchanged.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        let mut text = self.text.clone();
        changes.apply_to(&mut text)?;
        let end = changes
            .changes()
            .next()
            .map_or(text.len(), |(range, _)| range.start);
        let prefix = self.incremental.as_deref().map(|tree| (tree, end));
        if self.editor_reuse.is_some() {
            return Ok(Self::parse_editor_permissive(
                text,
                self.mode,
                self.node_types.clone(),
                prefix,
            ));
        }
        Ok(Self::parse_with_prefix(text, self.mode, prefix))
    }

    /// LR reuse in the last parse. The resume offset is in original UTF-8 bytes.
    /// Native nodes and highlights are rebuilt. Recovered documents report
    /// no reuse, even if they keep a strict checkpoint for a later edit.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.editor_reuse.unwrap_or_else(|| {
            self.incremental
                .as_deref()
                .map_or_else(ReuseStats::default, SyntaxTree::reuse_stats)
        })
    }

    fn parse_with_prefix(text: Text, top: JavaTop, prefix: Option<(&SyntaxTree, usize)>) -> Self {
        let parsed = parser::document_nodes(top, text.as_str(), text.len(), prefix);
        let mut doc = Self::from_parse(text, top, stock_node_types().clone(), parsed, None);
        doc.editor_nodes = doc.editor_lines.as_ref().map(|_| {
            // Lezer's raw line comments stop only at LF. A lone CR can hide
            // following code in the raw tree but not in the editor document.
            let input = NormalizedInput::new(doc.text.as_str());
            let (mut nodes, _, _) =
                parser::document_nodes(top, &input.text, input.text.len(), None);
            project_nodes(&input, &mut nodes);
            nodes
        });
        doc
    }

    pub(crate) fn parse_editor(
        text: Text,
        top: JavaTop,
        strict: bool,
        node_types: NodeTypes,
    ) -> Result<Self, ParseError> {
        if !strict {
            return Ok(Self::parse_editor_permissive(text, top, node_types, None));
        }
        let input = NormalizedInput::new(text.as_str());
        let tree = parse_raw(top, &input.text, true)
            .map_err(|error| error.map_offset(|at| input.original_byte(at)))?;
        let mut nodes = parser::collect(&tree, None);
        project_nodes(&input, &mut nodes);
        Ok(Self::from_parse(
            text,
            top,
            node_types,
            (nodes, None, None),
            Some(ReuseStats::default()),
        ))
    }

    fn parse_editor_permissive(
        text: Text,
        top: JavaTop,
        node_types: NodeTypes,
        prefix: Option<(&SyntaxTree, usize)>,
    ) -> Self {
        let source = text.as_str();
        let input = NormalizedInput::new(source);
        let prefix = prefix.map(|(tree, mut end)| {
            // An edit can join or split CRLF at the prefix boundary.
            if end > 0 && source.as_bytes()[end - 1] == b'\r' {
                end -= 1;
            }
            (tree, input.normalized_byte(end))
        });
        let (mut nodes, error, incremental) =
            parser::document_nodes(top, &input.text, source.len(), prefix);
        project_nodes(&input, &mut nodes);
        let error = error.map(|error| error.map_offset(|at| input.original_byte(at)));
        let mut reuse = incremental
            .as_ref()
            .map_or_else(ReuseStats::default, SyntaxTree::reuse_stats);
        reuse.resumed_at = input.original_byte(reuse.resumed_at);
        Self::from_parse(
            text,
            top,
            node_types,
            (nodes, error, incremental),
            Some(reuse),
        )
    }

    fn from_parse(
        text: Text,
        top: JavaTop,
        node_types: NodeTypes,
        (nodes, parse_error, incremental): parser::Parsed,
        editor_reuse: Option<ReuseStats>,
    ) -> Self {
        let mut doc = Self {
            editor_lines: editor_lines(text.as_str()),
            text,
            mode: top,
            editor_nodes: None,
            isolate_ranges: OnceLock::new(),
            nodes,
            node_types,
            highlights: Vec::new(),
            parse_error,
            incremental: incremental.map(Box::new),
            editor_reuse,
        };
        doc.highlights = highlight::collect(&doc);
        doc
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// Logical editor lines with lone CR mapped to LF at the same byte offset.
    pub fn syntax_lines(&self) -> &Text {
        self.editor_lines.as_ref().unwrap_or(&self.text)
    }

    /// The strict parser error, if the document needed local recovery.
    pub fn parse_error(&self) -> Option<ParseError> {
        self.parse_error
    }

    pub fn is_recovered(&self) -> bool {
        self.parse_error.is_some()
    }

    /// The root spans the full document, including skipped whitespace.
    pub fn root(&self) -> JavaNode<'_> {
        JavaNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// Look up a grammar row by term, including configured properties.
    /// Legacy Java node services always use their stock rules.
    pub fn node_type(&self, term: u16) -> Option<&NodeType> {
        self.node_types.get(term)
    }

    pub(crate) fn node_types(&self) -> &NodeTypes {
        &self.node_types
    }

    /// All visible nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = JavaNode<'_>> {
        (0..self.nodes.len()).map(|id| JavaNode {
            nodes: &self.nodes,
            id,
        })
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

    /// Build opt-in automatic bidi marks for comments and quoted literals.
    /// The viewport selects full nodes. Ranges use original UTF-8 bytes.
    /// The host installs and renders the marks for the same text snapshot.
    pub fn bidi_isolate_marks(
        &self,
        viewport: &Viewport,
        options: BidiIsolateOptions,
    ) -> Result<Arc<RangeSet<MarkDecoration>>, EditError> {
        let ranges = self.isolate_ranges.get_or_init(|| {
            self.editor_nodes()
                .iter()
                .filter_map(|node| {
                    if !isolates_direction(node.term) || node.range.is_empty() {
                        return None;
                    }
                    let mut range = node.range.clone();
                    // Raw line comments can contain the CR of a CRLF break.
                    if parser::name(node.term) == "LineComment"
                        && self.text.as_str().as_bytes().get(range.end - 1..=range.end)
                            == Some(b"\r\n")
                    {
                        range.end -= 1;
                    }
                    (!range.is_empty()).then_some(range)
                })
                .collect()
        });
        isolate_marks(&self.text, self.syntax_lines(), ranges, viewport, options)
    }

    /// Find a fold that starts on the given line and continues past its end.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text.slice(line_start..line_end)?;
        let mut found = None;
        let inner = self.resolve(line_end, Some(Assoc::After));
        for node in std::iter::successors(Some(inner), |node| node.parent()) {
            if node.data().range.end <= line_end {
                continue;
            }
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

    /// Compute indentation in columns. LF, CRLF, and lone CR end lines.
    /// Return `None` at an invalid byte position or when the upstream rule
    /// preserves the existing indentation.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        indent::indentation(self, position, settings, options)
    }

    fn editor_nodes(&self) -> &[Node] {
        self.editor_nodes.as_deref().unwrap_or(&self.nodes)
    }

    fn resolve(&self, position: usize, side: Option<Assoc>) -> JavaNode<'_> {
        let nodes = self.editor_nodes();
        let mut node = JavaNode { nodes, id: 0 };
        loop {
            let children = &node.data().children;
            let index = children.partition_point(|&id| {
                let end = nodes[id].range.end;
                end < position || (end == position && side != Some(Assoc::Before))
            });
            let Some(&id) = children.get(index) else {
                break;
            };
            let range = &nodes[id].range;
            if range.is_empty()
                || range.start > position
                || (range.start == position && side != Some(Assoc::After))
            {
                break;
            }
            node.id = id;
        }
        node
    }
}

fn project_nodes(input: &NormalizedInput<'_>, nodes: &mut [Node]) {
    for node in nodes {
        node.range = input.original_byte(node.range.start)..input.original_byte(node.range.end);
    }
}
