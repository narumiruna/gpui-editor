//! SQL language services from `@codemirror/lang-sql` 6.10.0 and its bundled
//! Lezer grammar. See `docs/licenses/codemirror-lang-sql.txt`.
//!
//! The parser uses the published grammar tables and a native dialect tokenizer.
//! It can reuse a verified strict LR prefix after an edit. For rejected input
//! it uses local recovery, not Lezer's error search. For example, recovery closes open
//! groups before a semicolon instead of trying deletion of the semicolon.
//! Ranges use original UTF-8 bytes. Fold endpoints inside a surrogate pair
//! move back to the start of that scalar. The module supplies keyword and
//! schema completion sources, an opt-in native schema registration, and
//! optional bidirectional isolation marks. It does not allow arbitrary parser
//! properties.

use std::{ops::Range, sync::Arc};

use crate::{Assoc, ChangeSet, EditError, EditorSelection, SelectionRange, Text};

use super::{
    HighlightSpan, IndentContext, IndentOptions, StreamSettings,
    lezer::{
        ExternalOptions, ExternalTokenizer, GrammarSpec, ParseError, Parser, ReuseStats,
        SyntaxNode, SyntaxTree, Tokenizer, TreeFragment,
    },
};

mod bidi;
mod completion;
mod dialect;
mod highlight;
mod recovery;
mod tables;
mod tokenizer;

pub use bidi::{SqlBidiIsolate, SqlBidiIsolateOptions};
pub use completion::{
    NativeSqlSchemaExtension, SqlKeywordCompletionSource, SqlNamespace, SqlSchemaCompletionSource,
    SqlSchemaConfig, SqlSchemaOption, keyword_completion_source, schema_completion_source,
};
pub use dialect::{SqlDialect, SqlDialectSpec, SqlWord};

/// The language name and default language data shared by all dialects.
pub const NAME: &str = "sql";
pub const LINE_COMMENT: &str = "--";
pub const BLOCK_COMMENT: (&str, &str) = ("/*", "*/");
pub const CLOSE_BRACKETS: [char; 6] = ['(', '[', '{', '\'', '"', '`'];

macro_rules! node_kinds {
    ($($kind:ident => $name:literal),+ $(,)?) => {
        /// The visible node types of the bundled grammar.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum SqlNodeKind { $($kind),+ }

        impl SqlNodeKind {
            pub const fn name(self) -> &'static str {
                match self { $(Self::$kind => $name),+ }
            }

            fn from_term(term: u16) -> Self {
                match term {
                    31 => Self::Statement, // the optional final statement
                    $(x if x == Self::$kind as u16 => Self::$kind,)+
                    _ => unreachable!("unknown SQL grammar term {term}"),
                }
            }
        }
    };
}

// The order up to Brackets matches the grammar term IDs.
node_kinds! {
    Error => "⚠", LineComment => "LineComment", BlockComment => "BlockComment",
    String => "String", Number => "Number", Bool => "Bool", Null => "Null",
    OpenParen => "(", CloseParen => ")", OpenBrace => "{", CloseBrace => "}",
    OpenBracket => "[", CloseBracket => "]", Semicolon => ";", Dot => ".", Operator => "Operator",
    Punctuation => "Punctuation", SpecialVar => "SpecialVar", Identifier => "Identifier",
    QuotedIdentifier => "QuotedIdentifier", Keyword => "Keyword", Type => "Type", Bits => "Bits",
    Bytes => "Bytes", Builtin => "Builtin", Script => "Script", Statement => "Statement",
    CompositeIdentifier => "CompositeIdentifier", Parens => "Parens", Braces => "Braces",
    Brackets => "Brackets",
}

impl SqlNodeKind {
    fn is_comment(self) -> bool {
        matches!(self, Self::LineComment | Self::BlockComment)
    }

    fn is_identifier_part(self) -> bool {
        matches!(
            self,
            Self::Identifier | Self::QuotedIdentifier | Self::SpecialVar
        )
    }
}

/// A parser configured for one SQL dialect. Use [`SqlDialect::language`] for
/// a bundled dialect or [`Self::define`] for a custom one.
#[derive(Debug)]
pub struct SqlLanguage {
    parser: Parser,
    config: Arc<dialect::LexConfig>,
    case_insensitive_identifiers: bool,
}

impl SqlLanguage {
    pub fn define(spec: SqlDialectSpec<'_>) -> Self {
        let config = Arc::new(dialect::LexConfig::new(spec));
        let lex = Arc::clone(&config);
        let tokens =
            ExternalTokenizer::new_deterministic(ExternalOptions::default(), move |input, _| {
                tokenizer::external(input, &lex)
            });
        let parser = Parser::deserialize_with_hooks(
            GrammarSpec {
                tables: tables::TABLES,
                node_names: tables::NODE_NAMES,
                max_term: 38,
                repeat_node_count: 3,
                tokenizers: &[Tokenizer::Internal(0), Tokenizer::External(tokens)],
                top_rule: [0, 25],
                token_prec: 0,
            },
            &[],
        )
        .expect("valid pinned SQL grammar tables");
        Self {
            parser,
            config,
            case_insensitive_identifiers: spec.case_insensitive_identifiers,
        }
    }

    /// The deduplicated word table in upstream keyword completion order.
    pub fn words(&self) -> &[SqlWord] {
        &self.config.words
    }

    pub fn case_insensitive_identifiers(&self) -> bool {
        self.case_insensitive_identifiers
    }

    pub fn parse(&self, text: Text) -> SqlDocument {
        self.parse_with_fragment(text, None)
    }

    /// Apply edits and parse with this dialect. Reuse the previous strict
    /// prefix if the parser and unchanged bytes match. GLR and recovered
    /// documents reparse from the start.
    pub fn reparse(
        &self,
        previous: &SqlDocument,
        changes: &ChangeSet,
    ) -> Result<SqlDocument, EditError> {
        let mut text = previous.text.clone();
        changes.apply_to(&mut text)?;
        let end = changes
            .changes()
            .next()
            .map_or(text.len(), |(range, _)| range.start);
        let fragment = previous.strict.as_deref().map(|tree| TreeFragment {
            tree,
            old_range: 0..end,
            new_range: 0..end,
        });
        Ok(self.parse_with_fragment(text, fragment))
    }

    fn parse_with_fragment(&self, text: Text, fragment: Option<TreeFragment<'_>>) -> SqlDocument {
        let source = text.as_str();
        let (nodes, error, strict) = match self.parser.parse_incremental(source, fragment) {
            Ok(tree) => (import_tree(tree.root()), None, Some(Box::new(tree))),
            Err(error) => match error {
                ParseError::Ambiguous { .. } => match self.parser.parse_glr(source) {
                    Ok(tree) => (import_tree(tree.root()), None, None),
                    Err(error) => (recovery::parse(source, &self.config), Some(error), None),
                },
                _ => (recovery::parse(source, &self.config), Some(error), None),
            },
        };
        let highlights = highlight::collect(&nodes);
        let isolate_ranges = bidi::isolate_ranges(&nodes);
        SqlDocument {
            text,
            nodes,
            highlights,
            isolate_ranges,
            error,
            strict,
        }
    }
}

#[derive(Debug)]
struct Node {
    kind: SqlNodeKind,
    range: Range<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
}

impl Node {
    fn new(kind: SqlNodeKind, range: Range<usize>, parent: Option<usize>) -> Self {
        Self {
            kind,
            range,
            parent,
            children: Vec::new(),
        }
    }
}

fn import_tree(root: SyntaxNode<'_>) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut pending = vec![(root, None::<usize>)];
    while let Some((node, parent)) = pending.pop() {
        let id = nodes.len();
        nodes.push(Node::new(
            SqlNodeKind::from_term(node.term()),
            node.range(),
            parent,
        ));
        if let Some(parent) = parent {
            nodes[parent].children.push(id);
        }
        pending.extend(node.children().rev().map(|child| (child, Some(id))));
    }
    nodes
}

/// A borrowed SQL syntax node. A zero-width error marks missing syntax.
#[derive(Clone, Copy, Debug)]
pub struct SqlNode<'a> {
    nodes: &'a [Node],
    id: usize,
}

impl<'a> SqlNode<'a> {
    fn data(self) -> &'a Node {
        &self.nodes[self.id]
    }

    pub fn kind(self) -> SqlNodeKind {
        self.data().kind
    }

    pub fn range(self) -> Range<usize> {
        self.data().range.clone()
    }

    pub fn parent(self) -> Option<Self> {
        self.data().parent.map(|id| Self { id, ..self })
    }

    fn ancestors(self) -> impl Iterator<Item = Self> {
        std::iter::successors(Some(self), |node| node.parent())
    }

    /// Direct children in source order, excluding whitespace.
    pub fn children(self) -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator + 'a {
        self.data()
            .children
            .iter()
            .map(move |&id| Self { id, ..self })
    }
}

/// One SQL document and its native syntax tree.
#[derive(Debug)]
pub struct SqlDocument {
    text: Text,
    nodes: Vec<Node>,
    highlights: Vec<HighlightSpan<'static>>,
    isolate_ranges: Vec<Range<usize>>,
    error: Option<ParseError>,
    strict: Option<Box<SyntaxTree>>,
}

impl SqlDocument {
    /// Parse with StandardSQL.
    pub fn parse(text: Text) -> Self {
        Self::parse_with_dialect(text, SqlDialect::StandardSql)
    }

    pub fn parse_with_dialect(text: Text, dialect: SqlDialect) -> Self {
        dialect.language().parse(text)
    }

    pub fn reuse_stats(&self) -> ReuseStats {
        self.strict
            .as_deref()
            .map_or_else(ReuseStats::default, SyntaxTree::reuse_stats)
    }

    /// The strict parse error that caused local recovery, if any.
    pub fn parse_error(&self) -> Option<ParseError> {
        self.error
    }

    pub fn is_recovered(&self) -> bool {
        self.error.is_some()
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// The root covers all bytes, including skipped whitespace.
    pub fn root(&self) -> SqlNode<'_> {
        SqlNode {
            nodes: &self.nodes,
            id: 0,
        }
    }

    /// All nodes in pre-order, with each parent before its children.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = SqlNode<'_>> {
        (0..self.nodes.len()).map(move |id| SqlNode { id, ..self.root() })
    }

    /// Return highlight spans clipped to a valid UTF-8 byte range.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        if !self.text.is_valid_range(range.clone()) {
            return None;
        }
        let start = self
            .highlights
            .partition_point(|span| span.range.end <= range.start);
        let end = if range.is_empty() {
            start
        } else {
            self.highlights
                .partition_point(|span| span.range.start < range.end)
        };
        Some(
            self.highlights[start..end]
                .iter()
                .map(move |span| HighlightSpan {
                    range: span.range.start.max(range.start)..span.range.end.min(range.end),
                    tags: span.tags,
                }),
        )
    }

    /// Expand one valid byte range to the next larger non-root node, as in
    /// `selectParentSyntax`. Return `None` if there is no larger node.
    pub fn parent_selection(&self, range: Range<usize>) -> Option<Range<usize>> {
        if !self.text.is_valid_range(range.clone()) {
            return None;
        }
        let mut node = self.resolve(range.start, Bias::Right);
        if range.is_empty() {
            let left = self.resolve(range.start, Bias::Left);
            if left.data().range.start >= node.data().range.start
                && left.data().range.end <= node.data().range.end
            {
                node = left;
            }
        }
        node.ancestors()
            .take_while(|node| node.id != 0)
            .find_map(|node| {
                let span = &node.data().range;
                (span.start <= range.start && span.end >= range.end && *span != range)
                    .then(|| span.clone())
            })
    }

    /// Expand each range and normalize the result. Return `None` for invalid
    /// selections or when no range can expand.
    pub fn select_parent_syntax(&self, selection: &EditorSelection) -> Option<EditorSelection> {
        selection
            .check(|pos| self.text.is_char_boundary(pos))
            .ok()?;
        let ranges = selection
            .ranges()
            .iter()
            .map(|&old| {
                self.parent_selection(old.range())
                    .map_or(old, |range| SelectionRange::new(range.start, range.end))
            })
            .collect();
        let expanded = EditorSelection::new(ranges, selection.main_index()).ok()?;
        (&expanded != selection).then_some(expanded)
    }

    /// Find a statement or block comment fold beginning on this line.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        if !self.text.is_valid_range(line_start..line_end) {
            return None;
        }
        let mut found = None;
        for node in self.resolve(line_end, Bias::Right).ancestors() {
            let span = &node.data().range;
            if found.is_some() && span.start < line_start {
                break;
            }
            if span.end <= line_end {
                continue;
            }
            let fold = match node.kind() {
                SqlNodeKind::Statement => {
                    let line = self.text.line_at(span.start)?;
                    let first = self.text.slice(span.start..line.range.end)?;
                    span.start + utf16_prefix_in_bytes(first, 100)..span.end
                }
                SqlNodeKind::BlockComment if span.len() >= 4 => {
                    let comment = self.text.slice(span.clone())?;
                    // Even for an unclosed comment, upstream subtracts two units.
                    span.start + 2..span.start + before_utf16_suffix(comment, 2)
                }
                _ => continue,
            };
            if (line_start..=line_end).contains(&fold.start)
                && line_end < fold.end
                && self.text.is_valid_range(fold.clone())
            {
                found = Some(fold);
            }
        }
        found
    }

    /// Compute continued statement indentation. Bracket groups have no
    /// separate indentation rule in the published SQL grammar.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        let context = IndentContext::new(&self.text, position, settings, options)?;
        let mut statement = self
            .resolve(position, Bias::Inside)
            .ancestors()
            .find(|node| node.kind() == SqlNodeKind::Statement);
        if statement.is_none() {
            let before = self.resolve(position, Bias::Left);
            statement = before.ancestors().find(|node| {
                node.kind() == SqlNodeKind::Statement && incomplete_at_end(*node, position)
            });
        }
        let Some(node) = statement else {
            return Some(0);
        };
        let mut line = self.text.line_at(node.data().range.start)?;
        // A preceding statement or its child can cover the start of this line.
        loop {
            let at_start = self.resolve(line.range.start, Bias::Inside);
            if node.ancestors().any(|ancestor| ancestor.id == at_start.id) {
                break;
            }
            let previous = self.text.line_at(at_start.data().range.start)?;
            if previous.range.start >= line.range.start {
                break;
            }
            line = previous;
        }
        Some(
            context
                .line_indent(line.range.start, Assoc::Before)?
                .saturating_add(context.unit()),
        )
    }

    fn resolve(&self, pos: usize, bias: Bias) -> SqlNode<'_> {
        let mut node = self.root();
        loop {
            let children = &node.data().children;
            let at = children.partition_point(|&id| {
                let end = self.nodes[id].range.end;
                end < pos || (end == pos && bias != Bias::Left)
            });
            let Some(&id) = children.get(at) else { break };
            let span = &self.nodes[id].range;
            if span.is_empty() || span.start > pos || (span.start == pos && bias != Bias::Right) {
                break;
            }
            node = SqlNode { id, ..node };
        }
        node
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Bias {
    Left,
    Inside,
    Right,
}

fn incomplete_at_end(mut node: SqlNode<'_>, position: usize) -> bool {
    while let Some(last) = node
        .children()
        .next_back()
        .filter(|last| last.data().range.end == position)
    {
        if last.kind() == SqlNodeKind::Error && last.data().range.is_empty() {
            return true;
        }
        node = last;
    }
    false
}

fn utf16_prefix_in_bytes(source: &str, mut count: usize) -> usize {
    for (byte, ch) in source.char_indices() {
        if count < ch.len_utf16() {
            return byte;
        }
        count -= ch.len_utf16();
    }
    source.len()
}

fn before_utf16_suffix(source: &str, mut count: usize) -> usize {
    for (byte, ch) in source.char_indices().rev() {
        count = count.saturating_sub(ch.len_utf16());
        if count == 0 {
            return byte;
        }
    }
    0
}
