use std::num::NonZeroUsize;

use super::{lines::LineEditText, *};
use crate::{
    Assoc,
    language::{IndentStyle, indent_style, line_projection::command_line_projection},
};

/// The text added when indenting by one unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndentUnit {
    Spaces(NonZeroUsize),
    Tab,
    Configured(IndentStyle),
}

/// Indentation style and tab stop width. Defaults to two spaces and four columns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Indentation {
    pub unit: IndentUnit,
    pub tab_size: NonZeroUsize,
}

impl Default for Indentation {
    fn default() -> Self {
        Self {
            unit: IndentUnit::Spaces(NonZeroUsize::new(2).unwrap()),
            tab_size: NonZeroUsize::new(4).unwrap(),
        }
    }
}

impl Indentation {
    /// Use the state's indentation unit and tab stop facets.
    pub fn for_state(state: &EditorState) -> Self {
        Self {
            tab_size: state.tab_size(),
            unit: IndentUnit::Configured(indent_style(state)),
        }
    }

    /// Return the number of columns in one indent unit.
    pub fn width(self) -> usize {
        match self.unit {
            IndentUnit::Spaces(n) => n.get(),
            IndentUnit::Tab => self.tab_size.get(),
            IndentUnit::Configured(style) => style.width(self.tab_size),
        }
    }

    pub(super) fn columns(self, text: &str) -> usize {
        text.chars().fold(0usize, |column, ch| {
            let tab = self.tab_size.get();
            let width = if ch == '\t' { tab - column % tab } else { 1 };
            column.saturating_add(width)
        })
    }

    pub(crate) fn at_column(self, column: usize) -> String {
        match self.unit {
            IndentUnit::Configured(style) => style.at_column(self.tab_size, column),
            IndentUnit::Tab => {
                let width = self.tab_size.get();
                ["\t".repeat(column / width), " ".repeat(column % width)].concat()
            }
            IndentUnit::Spaces(_) => " ".repeat(column),
        }
    }
}

fn change_selected_lines(
    state: &EditorState,
    edit: impl Fn(usize, &str) -> Option<Change>,
) -> CommandResult {
    let projection = command_line_projection(state);
    let view = LineEditText::new(state.text(), projection.as_ref())?;
    let text = view.lines;
    let source = text.as_str();
    let mut next_unhandled = 0;
    let transaction = state.change_by_range(|selection| {
        let range = selection.range();
        let mut start = line_at(text, range.start).range.start.max(next_unhandled);
        let mut changes = Vec::new();
        while start < range.end || (range.is_empty() && start <= range.end) {
            let tail = &source[start..];
            let (line, next) = match tail.find('\n') {
                Some(newline) => {
                    let prefix = &tail[..newline];
                    (
                        prefix.strip_suffix('\r').unwrap_or(prefix),
                        start + newline + 1,
                    )
                }
                None => (tail, source.len() + 1),
            };
            changes.extend(edit(start, line));
            next_unhandled = next;
            start = next;
        }
        let changes = ChangeSet::new(changes)?;
        let anchor = changes.map_pos(selection.anchor(), Assoc::After);
        let head = changes.map_pos(selection.head(), Assoc::After);
        Ok(range_edit(changes, SelectionRange::new(anchor, head)))
    })?;
    Ok(view
        .keeps_breaks(transaction.changes())
        .then_some(transaction))
}

/// Insert one configured indentation unit at the start of each selected hard line.
pub fn indent_more(state: &EditorState) -> CommandResult {
    indent_more_with(state, Indentation::for_state(state))
}

/// Insert one indentation unit per selected hard line. A range that ends at
/// the start of a line does not select that line.
pub fn indent_more_with(state: &EditorState, indent: Indentation) -> CommandResult {
    if state.read_only() {
        return Ok(None);
    }
    let insert = indent.at_column(indent.width());
    let transaction =
        change_selected_lines(state, |start, _| Some(Change::new(start..start, &insert)))?;
    Ok(transaction.map(|transaction| transaction.with_user_event("input.indent")))
}

/// Remove a configured indentation unit from the lines the selection touches.
pub fn indent_less(state: &EditorState) -> CommandResult {
    indent_less_with(state, Indentation::for_state(state))
}

/// Remove up to one indentation unit from each selected hard line. Expand a
/// tab when necessary to keep the remaining columns. Handle unindented lines
/// without making edits, as in CodeMirror.
pub fn indent_less_with(state: &EditorState, indent: Indentation) -> CommandResult {
    if state.read_only() {
        return Ok(None);
    }
    let transaction = change_selected_lines(state, |start, line| {
        let space = leading_space(line);
        if space.is_empty() {
            return None;
        }
        let column = indent.columns(space).saturating_sub(indent.width());
        let insert = indent.at_column(column);
        let keep = space
            .chars()
            .zip(insert.chars())
            .take_while(|(a, b)| a == b)
            .map(|(ch, _)| ch.len_utf8())
            .sum::<usize>();
        Some(Change::new(
            start + keep..start + space.len(),
            &insert[keep..],
        ))
    })?;
    Ok(transaction.map(|transaction| transaction.with_user_event("delete.dedent")))
}

/// Insert tabs at carets, or indent the selected lines when any range is nonempty.
pub fn insert_tab(state: &EditorState) -> CommandResult {
    insert_tab_with(state, Indentation::for_state(state))
}

/// Insert tabs at carets, or indent the selected lines with the given style.
pub fn insert_tab_with(state: &EditorState, indent: Indentation) -> CommandResult {
    if state
        .selection()
        .ranges()
        .iter()
        .any(|r| !r.range().is_empty())
    {
        return indent_more_with(state, indent);
    }
    let source = state.text().as_str();
    let transaction = state.change_by_range(|selection| {
        let pos = selection.head() - usize::from(inside_crlf(source, selection.head()));
        replace_range(pos..pos, "\t", pos + 1)
    })?;
    Ok(Some(
        transaction.with_user_event("input").with_scroll_into_view(),
    ))
}
