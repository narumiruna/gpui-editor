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
    document: &'a WastDocument,
    node: WastNode<'a>,
    state: Option<&'a EditorState>,
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
        let start = self.node.data().range.start;
        let mut reference = text.line_at(start)?.range.start;
        loop {
            let mut at_break = self.document.path(reference, false).last()?;
            // Treat nodes that share a start as one reference.
            while let Some(parent) = at_break
                .parent()
                .filter(|parent| parent.data().range.start == at_break.data().range.start)
            {
                at_break = parent;
            }
            if at_break.data().range.end > start {
                break;
            }
            let earlier = text.line_at(at_break.data().range.start)?.range.start;
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

    /// Get the result of the next ancestor strategy. If no strategy remains,
    /// return column zero. If another call would exceed the query limit of 64
    /// strategy calls (including repeated calls), return `None`.
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
        Some(0)
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
        let position = lines.position();
        let node = self
            .path(position, false)
            .last()?
            .unfinished_before(position);
        let context = WastIndentContext {
            document: self,
            node,
            state,
            lines,
            remaining: &Cell::new(STRATEGY_LIMIT),
        };
        context.indent_from(Some(node))
    }
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
