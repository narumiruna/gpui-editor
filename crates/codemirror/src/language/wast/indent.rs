use std::{cell::Cell, sync::Arc};

use crate::{
    Assoc, EditorState,
    language::{IndentContext, IndentOptions, StreamSettings},
};

use super::{INDENT, WastDocument, WastNode};

// Bound both stack depth and work if callbacks call continue_indent more than once.
const STRATEGY_LIMIT: usize = 64;

/// A strategy returns an indentation column or `None` to stop without one.
/// Call [`WastIndentContext::continue_indent`] to consult ancestors instead.
pub type WastIndentStrategy = Arc<dyn Fn(&WastIndentContext<'_>) -> Option<usize> + Send + Sync>;

/// The configured WAST node and the line context for one indentation query.
pub struct WastIndentContext<'a> {
    root: WastNode<'a>,
    node: WastNode<'a>,
    state: Option<&'a EditorState>,
    fallback: Option<usize>,
    lines: &'a IndentContext<'a>,
    remaining: &'a Cell<usize>,
}

impl<'a> WastIndentContext<'a> {
    pub fn node(&self) -> WastNode<'a> {
        self.node
    }

    /// The current editor state, if this is a state indentation query.
    pub fn state(&self) -> Option<&'a EditorState> {
        self.state
    }

    /// The query position, settings, and logical lines in original byte coordinates.
    /// Line queries use the same simulated breaks and overrides as the query.
    pub fn line_context(&self) -> &IndentContext<'a> {
        self.lines
    }

    /// Indentation at the node's reference line. If an earlier node covers
    /// the start of that line, use its reference line instead.
    pub fn base_indent(&self) -> Option<usize> {
        let text = self.lines.text();
        let start = self.node.range().start;
        let mut reference = text.line_at(start)?.range.start;
        loop {
            let mut at_break = self.root.resolve_at(reference, None);
            // Treat nodes that share a start as one reference.
            while let Some(parent) = at_break
                .parent()
                .filter(|parent| parent.range().start == at_break.range().start)
            {
                at_break = parent;
            }
            if at_break.range().end > start {
                break;
            }
            let earlier = text.line_at(at_break.range().start)?.range.start;
            if earlier >= reference {
                break; // An empty node can have no earlier reference line.
            }
            reference = earlier;
        }
        let line = self.lines.line_at(reference, Assoc::After)?;
        Some(self.lines.overridden(line.range.start).unwrap_or_else(|| {
            let tab = self.lines.settings().tab_size.get();
            line.text
                .chars()
                .take_while(|&ch| is_js_space(ch))
                .fold(0_usize, |col, ch| {
                    col.saturating_add(if ch == '\t' { tab - col % tab } else { 1 })
                })
        }))
    }

    /// Get the next ancestor strategy's result. Without another strategy,
    /// return zero in a standalone tree or `None` in a Markdown fence.
    /// Return `None` if another call would exceed 64 strategies in this query.
    pub fn continue_indent(&self) -> Option<usize> {
        self.indent_from(self.node.parent())
    }

    fn indent_from(&self, start: Option<WastNode<'a>>) -> Option<usize> {
        for node in std::iter::successors(start, |node| node.parent()) {
            if let Some(strategy) = node.prop(&INDENT) {
                self.remaining.set(self.remaining.get().checked_sub(1)?);
                return strategy(&Self { node, ..*self });
            }
        }
        self.fallback
    }
}

impl WastDocument {
    /// Query configured indentation at a UTF-8 byte position without an editor state.
    /// Return `None` for an invalid position or a strategy that gives no column.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        let lines = IndentContext::new(self.syntax_lines(), position, settings, options)?;
        self.indentation_with_context(None, &lines)
    }

    pub(in crate::language) fn indentation_with_context(
        &self,
        state: Option<&EditorState>,
        lines: &IndentContext<'_>,
    ) -> Option<usize> {
        indentation_from_root(self.root(), state, lines, Some(0))
    }

    pub(crate) fn indentation_in_markdown(
        &self,
        map: super::MarkdownSourceMap<'_>,
        state: &EditorState,
        lines: &IndentContext<'_>,
    ) -> Option<usize> {
        indentation_from_root(self.root().in_markdown(map), Some(state), lines, None)
    }
}

fn indentation_from_root(
    root: WastNode<'_>,
    state: Option<&EditorState>,
    lines: &IndentContext<'_>,
    fallback: Option<usize>,
) -> Option<usize> {
    let position = lines.position();
    let node = root.resolve_at(position, None).unfinished_before(position);
    let context = WastIndentContext {
        root,
        node,
        state,
        fallback,
        lines,
        remaining: &Cell::new(STRATEGY_LIMIT),
    };
    context.indent_from(Some(node))
}

pub(super) fn app_indent(context: &WastIndentContext<'_>) -> Option<usize> {
    let base = context.base_indent()?;
    let lines = context.line_context();
    let after = lines.text_after_pos(lines.position(), Assoc::After)?;
    let closed = after.trim_start_matches(is_js_space).starts_with(')');
    Some(if closed {
        base
    } else {
        base.saturating_add(lines.unit())
    })
}

fn is_js_space(ch: char) -> bool {
    (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}'
}
