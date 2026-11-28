use std::{
    cell::Cell,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, EditorState,
    language::{
        IndentContext,
        markdown::{MarkdownCodeParser, MarkdownMount, MarkdownSourceMap},
        rust::{RustDocument, RustNode, indent as rust_indent},
    },
    language_data::NativeSyntax,
};

use super::{
    CLOSED_BY, NodeProp,
    service_node::{Coordinates, LrSyntaxNode, Syntax},
};

const STRATEGY_LIMIT: usize = 64;

/// A typed LR indentation rule. Return `None` to stop without
/// a column, or call [`LrIndentContext::continue_indent`] to consult ancestors.
pub type LrIndentStrategy = Arc<dyn Fn(&LrIndentContext<'_>) -> Option<usize> + Send + Sync>;

/// The indentation rule associated with a grammar row. An explicit `None`
/// removes a previous rule and allows the default delimiter strategy.
pub static INDENT: LazyLock<NodeProp<Option<LrIndentStrategy>>> = LazyLock::new(NodeProp::new);

#[derive(Clone, Copy)]
enum Resolver<'a> {
    Lr {
        root: LrSyntaxNode<'a>,
        fallback: Option<usize>,
    },
    Rust {
        doc: &'a RustDocument,
        map: Option<MarkdownSourceMap<'a>>,
    },
}

/// The state, line context, and grammar node for an indentation query.
pub struct LrIndentContext<'a> {
    resolver: Resolver<'a>,
    node: LrSyntaxNode<'a>,
    state: Option<&'a EditorState>,
    lines: &'a IndentContext<'a>,
    remaining: &'a Cell<usize>,
}

impl<'a> LrIndentContext<'a> {
    pub fn node(&self) -> LrSyntaxNode<'a> {
        self.node
    }

    /// The current state, or `None` for a syntax-only query.
    pub fn state(&self) -> Option<&'a EditorState> {
        self.state
    }

    /// The query position, settings, and logical lines in host byte coordinates.
    pub fn line_context(&self) -> &IndentContext<'a> {
        self.lines
    }

    /// Indentation at the reference line, with line overrides applied.
    /// If an earlier sibling covers the start of that line, use its line instead.
    pub fn base_indent(&self) -> Option<usize> {
        let root = match self.resolver {
            Resolver::Lr { root, .. } => root,
            Resolver::Rust { doc, map } => {
                return rust_indent::callback_base_indent(doc, self.rust_node(), self.lines, map);
            }
        };
        let text = self.lines.text();
        let start = self.node.range().start;
        let mut reference = text.line_at(start)?.range.start;
        loop {
            let mut at_break = root.resolve_at(reference, false);
            while let Some(parent) = at_break
                .parent()
                .filter(|parent| parent.range().start == at_break.range().start)
            {
                at_break = parent;
            }
            let range = at_break.range();
            if range.end > start || range.start >= reference {
                return self.lines.line_indent(reference, Assoc::After);
            }
            reference = text.line_at(range.start)?.range.start;
        }
    }

    /// Get the next ancestor strategy's result. At most 64 custom strategies
    /// run per query, including repeated requests to continue.
    pub fn continue_indent(&self) -> Option<usize> {
        match self.resolver {
            Resolver::Lr { fallback, .. } => self.indent_from(self.node.parent(), fallback),
            Resolver::Rust { doc, map } => self.rust_from(doc, self.rust_node().parent(), map),
        }
    }

    fn rust_node(&self) -> RustNode<'a> {
        let Syntax::Rust(node) = self.node.syntax else {
            unreachable!("Rust resolver requires a Rust node");
        };
        node
    }

    fn rust_from(
        &self,
        doc: &'a RustDocument,
        start: Option<RustNode<'a>>,
        map: Option<MarkdownSourceMap<'a>>,
    ) -> Option<usize> {
        rust_indent::indentation_from(
            doc,
            start,
            self.lines,
            map,
            Some(&|node, strategy| {
                Self {
                    node: LrSyntaxNode {
                        syntax: Syntax::Rust(node),
                        ..self.node
                    },
                    ..*self
                }
                .run(strategy)
            }),
        )
    }

    fn run(&self, strategy: &LrIndentStrategy) -> Option<usize> {
        self.remaining.set(self.remaining.get().checked_sub(1)?);
        strategy(self)
    }

    fn indent_from(
        &self,
        start: Option<LrSyntaxNode<'a>>,
        fallback: Option<usize>,
    ) -> Option<usize> {
        for node in std::iter::successors(start, |node| node.parent()) {
            let context = Self { node, ..*self };
            if let Some(strategy) = node
                .node_type()
                .and_then(|row| row.prop(&INDENT))
                .and_then(Option::as_ref)
            {
                return context.run(strategy);
            }
            if let Some(open) = node.children().next()
                && let Some(closers) = closers(open)
            {
                return context.delimited(open, closers);
            }
        }
        fallback
    }

    fn delimited(&self, open: LrSyntaxNode<'a>, closers: &[String]) -> Option<usize> {
        let lines = self.lines;
        let pos = lines.position();
        let after = lines.text_after_pos(pos, Assoc::After)?;
        let leading = after.len() - trim_space(after).len();
        let last = self.node.children().next_back()?;
        let closed = !lines.at_double_break()
            && last.range().start == pos + leading
            && closers.iter().any(|name| name == last.name());
        if let Some(end) = self.alignment(open) {
            let at = if closed { open.range().start } else { end };
            return Some(lines.column(at, Assoc::After)?.max(0) as usize);
        }
        Some(
            self.base_indent()?
                .saturating_add(if closed { 0 } else { lines.unit() }),
        )
    }

    fn alignment(&self, open: LrSyntaxNode<'a>) -> Option<usize> {
        let lines = self.lines;
        let open = open.range();
        let line = lines.text().line_at(open.start)?;
        let end = lines
            .simulated_break()
            .filter(|&at| at > line.range.start)
            .map_or(line.range.end, |at| at.min(line.range.end));
        // Pinned Lezer returns distinct wrappers from `childAfter` and `lastChild`,
        // so an eligible last child can supply alignment, even if it is the closer.
        let next = self.node.children().skip(1).find(|node| {
            node.range().end > open.end
                && !node.node_type().map_or_else(
                    || matches!(node.name(), "LineComment" | "BlockComment" | "⚠"),
                    |row| row.is_skipped(),
                )
        })?;
        if next.range().start >= end {
            return None;
        }
        let spaces = lines
            .text()
            .slice(open.end..line.range.end)?
            .bytes()
            .take_while(|&ch| ch == b' ')
            .count();
        Some(open.end + spaces)
    }
}

fn closers(node: LrSyntaxNode<'_>) -> Option<&[String]> {
    if let Some(row) = node.node_type() {
        return row.prop(&CLOSED_BY).map(Vec::as_slice);
    }
    // Local Lezer recovery may erase the term, but retains delimiter names.
    static FALLBACK: LazyLock<[Vec<String>; 3]> =
        LazyLock::new(|| [vec!["}".into()], vec!["]".into()], vec![")".into()]]);
    match node.name() {
        "{" => Some(&FALLBACK[0]),
        "[" => Some(&FALLBACK[1]),
        "(" => Some(&FALLBACK[2]),
        _ => None,
    }
}

pub(in crate::language) fn stock_json_indent() -> [LrIndentStrategy; 2] {
    static STOCK: LazyLock<[LrIndentStrategy; 2]> = LazyLock::new(|| {
        ['}', ']'].map(|closer| {
            let strategy: LrIndentStrategy = Arc::new(move |cx| {
                let after = cx.lines.text_after_pos(cx.lines.position(), Assoc::After)?;
                let extra = if trim_space(after).starts_with(closer) {
                    0
                } else {
                    cx.lines.unit()
                };
                Some(cx.base_indent()?.saturating_add(extra))
            });
            strategy
        })
    });
    STOCK.clone()
}

fn trim_space(text: &str) -> &str {
    text.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
}

pub(in crate::language) fn indentation(
    root: LrSyntaxNode<'_>,
    state: Option<&EditorState>,
    lines: &IndentContext<'_>,
    fallback: Option<usize>,
) -> Option<usize> {
    let pos = lines.position();
    let node = root.resolve_at(pos, false).unfinished_before(pos);
    let context = LrIndentContext {
        resolver: Resolver::Lr { root, fallback },
        node,
        state,
        lines,
        remaining: &Cell::new(STRATEGY_LIMIT),
    };
    context.indent_from(Some(node), fallback)
}

pub(in crate::language) fn rust_indentation<'a>(
    doc: &'a RustDocument,
    state: &'a EditorState,
    lines: &'a IndentContext<'a>,
) -> Option<usize> {
    rust_indentation_at(doc, state, lines, lines.position(), None)
}

fn rust_indentation_at<'a>(
    doc: &'a RustDocument,
    state: &'a EditorState,
    lines: &'a IndentContext<'a>,
    local: usize,
    map: Option<MarkdownSourceMap<'a>>,
) -> Option<usize> {
    let start = rust_indent::indentation_node(doc, local)?;
    let node = LrSyntaxNode::rust(start);
    let context = LrIndentContext {
        resolver: Resolver::Rust { doc, map },
        node: map.map_or(node, |map| node.in_mount(map)),
        state: Some(state),
        lines,
        remaining: &Cell::new(STRATEGY_LIMIT),
    };
    context.rust_from(doc, Some(start), map)
}

impl MarkdownMount {
    pub(crate) fn has_rust_indentation(&self) -> bool {
        matches!(self.code_parser(), Some(MarkdownCodeParser::Rust(_)))
    }

    pub(crate) fn code_indentation(
        &self,
        state: &EditorState,
        lines: &IndentContext<'_>,
    ) -> Option<usize> {
        let map = self.source_map();
        if let Some(wast) = self.wast() {
            return wast.indentation_in_markdown(map, state, lines);
        }
        if let NativeSyntax::Rust(doc) = self.syntax()
            && self.has_rust_indentation()
        {
            let position = lines.position();
            let (segment, start) = map.segment_at(position, Assoc::After)?;
            return rust_indentation_at(
                doc,
                state,
                lines,
                start + (position - segment.start),
                Some(map),
            );
        }
        indentation(
            self.syntax().lr_root()?.in_mount(map),
            Some(state),
            lines,
            None,
        )
    }
}

impl NativeSyntax {
    pub(in crate::language) fn lr_indentation_in_body(
        &self,
        origin: usize,
        state: &EditorState,
        lines: &IndentContext<'_>,
    ) -> Option<usize> {
        let root = LrSyntaxNode {
            coordinates: Coordinates::Linear(origin),
            ..self.lr_root()?
        };
        indentation(root, Some(state), lines, Some(0))
    }
}
