//! Syntax-aware indentation commands adapted from `@codemirror/commands` 6.11.2.
//! See `docs/licenses/codemirror-commands.txt`.

use std::{cell::RefCell, collections::HashMap};

use super::{lines::LineEditText, *};
use crate::{
    Assoc,
    language::{IndentOptions, StreamSettings, line_projection::command_line_projection},
};

/// Replace the selection with a newline and the indentation given by `indentation_at`.
/// The provider receives a UTF-8 byte position and options for a simulated break
/// in the original document. If it returns `None`, use the source line's indentation.
///
/// For a caret between matching brackets, insert a second break before the
/// closing bracket. Adjacent `()`, `[]`, and `{}` need no syntax information.
/// Otherwise `between_brackets` can return the same-line whitespace gap between
/// matching syntax tokens, or `None` (also use `|_| None` without a syntax tree).
///
/// Unlike CodeMirror's state-wide line separator, use the ending after the source
/// line, then the ending before it, then LF. Edits do not split CRLF pairs.
pub fn insert_newline_and_indent(
    state: &EditorState,
    indent: Indentation,
    indentation_at: impl FnMut(usize, StreamSettings, IndentOptions<'_>) -> Option<usize>,
    between_brackets: impl FnMut(usize) -> Option<Range<usize>>,
) -> CommandResult {
    insert_newline_and_indent_with_ending(state, indent, indentation_at, between_brackets, "\n")
}

/// Like [`insert_newline_and_indent`], but use the enclosing line ending when
/// the source has no local line ending. Only LF and CRLF are supported.
pub fn insert_newline_and_indent_with_ending(
    state: &EditorState,
    indent: Indentation,
    indentation_at: impl FnMut(usize, StreamSettings, IndentOptions<'_>) -> Option<usize>,
    between_brackets: impl FnMut(usize) -> Option<Range<usize>>,
    enclosing_ending: &str,
) -> CommandResult {
    insert_indented_break(
        state,
        indent,
        indentation_at,
        between_brackets,
        enclosing_ending,
        false,
        command_line_projection(state).as_ref(),
    )
}

/// Use the editor's logical line bounds, including lone CR, without replacing
/// existing line endings. Decline the edit if it joins two separate breaks.
pub fn insert_newline_and_indent_with_projection(
    state: &EditorState,
    lines: &Text,
    indent: Indentation,
    indentation_at: impl FnMut(usize, StreamSettings, IndentOptions<'_>) -> Option<usize>,
    between_brackets: impl FnMut(usize) -> Option<Range<usize>>,
    enclosing_ending: &str,
) -> CommandResult {
    insert_indented_break(
        state,
        indent,
        indentation_at,
        between_brackets,
        enclosing_ending,
        false,
        Some(lines),
    )
}

/// Insert an indented line below the last line of each selection. Do not split
/// bracket pairs. Clear leading whitespace only on a single selected line.
/// Use `enclosing_ending` (LF or CRLF) when the source has no local ending.
pub fn insert_blank_line(
    state: &EditorState,
    indent: Indentation,
    indentation_at: impl FnMut(usize, StreamSettings, IndentOptions<'_>) -> Option<usize>,
    enclosing_ending: &str,
) -> CommandResult {
    insert_indented_break(
        state,
        indent,
        indentation_at,
        |_| None,
        enclosing_ending,
        true,
        command_line_projection(state).as_ref(),
    )
}

/// Like [`insert_blank_line`], with the same projection as
/// [`insert_newline_and_indent_with_projection`].
pub fn insert_blank_line_with_projection(
    state: &EditorState,
    lines: &Text,
    indent: Indentation,
    indentation_at: impl FnMut(usize, StreamSettings, IndentOptions<'_>) -> Option<usize>,
    enclosing_ending: &str,
) -> CommandResult {
    insert_indented_break(
        state,
        indent,
        indentation_at,
        |_| None,
        enclosing_ending,
        true,
        Some(lines),
    )
}

fn insert_indented_break(
    state: &EditorState,
    indent: Indentation,
    mut indentation_at: impl FnMut(usize, StreamSettings, IndentOptions<'_>) -> Option<usize>,
    mut between_brackets: impl FnMut(usize) -> Option<Range<usize>>,
    enclosing_ending: &str,
    blank_below: bool,
    projection: Option<&Text>,
) -> CommandResult {
    let enclosing_ending = if enclosing_ending == "\r\n" {
        "\r\n"
    } else {
        "\n"
    };
    if state.read_only() {
        return Ok(None);
    }
    let edit_text = LineEditText::new(state.text(), projection)?;
    let text = edit_text.lines;
    let source = edit_text.raw;
    let settings = settings(indent);
    let transaction = state.change_by_range(|selection| {
        let mut range = include_crlf(source, selection.range());
        let first = line_at(text, range.start);
        let first_number = first.number;
        let line = if blank_below {
            let last = line_at(text, range.end);
            range = last.range.end..last.range.end;
            last
        } else {
            first
        };
        let original_indent = leading_space(line.text);
        let gap = if !blank_below && range.is_empty() {
            bracket_gap(source, &line.range, range.start, &mut between_brackets)
        } else {
            None
        };
        let position = range.start;
        let column = indentation_at(
            position,
            settings,
            IndentOptions {
                simulate_break: Some(position),
                simulate_double_break: gap.is_some(),
                ..IndentOptions::default()
            },
        )
        .unwrap_or_else(|| indent.columns(original_indent));
        let ending = edit_text.preferred_break(&line.range, enclosing_ending);
        let mut insert = String::from(ending);
        insert.push_str(&indent.at_column(column));
        let caret_offset = insert.len();
        if let Some(gap) = gap {
            range = gap;
            let closing_column = if position == line.range.start {
                0 // A double break puts an empty logical line here.
            } else {
                indent.columns(original_indent)
            };
            insert.push_str(ending);
            insert.push_str(&indent.at_column(closing_column));
        } else {
            if range.end < line.range.end {
                range.end += leading_space(&source[range.end..line.range.end]).len();
            }
            if first_number == line.number && clear_prefix(text, &line, position, original_indent) {
                range.start = line.range.start;
            }
        }
        let caret = range.start + caret_offset;
        replace_range(range, insert, caret)
    })?;
    if !edit_text.keeps_breaks(transaction.changes()) {
        return Ok(None);
    }
    Ok(Some(
        transaction.with_user_event("input").with_scroll_into_view(),
    ))
}

/// Normalize indentation on selected hard lines. Query each line once in
/// document order, with overrides for earlier replacements. The provider sees
/// the original text and can return `None` to keep a line's indentation.
/// A nonempty range that ends at a line start does not select that line.
/// A read-only state returns `None`. If no edit is needed, return a handled
/// empty transaction with no selection or user event (upstream does not dispatch).
pub fn indent_selection(
    state: &EditorState,
    indent: Indentation,
    indentation_at: impl FnMut(usize, StreamSettings, IndentOptions<'_>) -> Option<usize>,
) -> CommandResult {
    indent_selected_lines(
        state,
        indent,
        indentation_at,
        command_line_projection(state).as_ref(),
    )
}

/// Like [`indent_selection`], but use the editor's logical line bounds.
/// The same-width projection must replace all lone CR bytes with LF.
/// Decline the edit if it joins any separate line breaks.
pub fn indent_selection_with_projection(
    state: &EditorState,
    lines: &Text,
    indent: Indentation,
    indentation_at: impl FnMut(usize, StreamSettings, IndentOptions<'_>) -> Option<usize>,
) -> CommandResult {
    indent_selected_lines(state, indent, indentation_at, Some(lines))
}

fn indent_selected_lines(
    state: &EditorState,
    indent: Indentation,
    mut indentation_at: impl FnMut(usize, StreamSettings, IndentOptions<'_>) -> Option<usize>,
    projection: Option<&Text>,
) -> CommandResult {
    if state.read_only() {
        return Ok(None);
    }
    let edit_text = LineEditText::new(state.text(), projection)?;
    let text = edit_text.lines;
    let settings = settings(indent);
    let mut last_line = 0;
    let updated = RefCell::new(HashMap::new());
    let override_indentation = |start| updated.borrow().get(&start).copied();
    let options = IndentOptions {
        override_indentation: Some(&override_indentation),
        ..IndentOptions::default()
    };
    let changes = state.change_by_range(|selection| {
        let range = selection.range();
        let first = line_at(text, range.start).number;
        let last = line_at(text, range.end);
        let mut changes = Vec::new();
        for number in first.max(last_line + 1)..=last.number {
            let line = text.line(number).expect("selected line exists");
            let start = line.range.start;
            if !range.is_empty() && range.end == start {
                break;
            }
            last_line = number;
            let Some(column) = indentation_at(start, settings, options) else {
                continue;
            };
            let current = leading_space(line.text);
            let column = if current == line.text { 0 } else { column };
            let normalized = indent.at_column(column);
            let end = start + current.len();
            if current != normalized || range.start < end {
                updated.borrow_mut().insert(start, column);
                changes.push(Change::new(start..end, normalized));
            }
        }
        let changes = ChangeSet::new(changes)?;
        let map = |position| changes.map_pos(position, Assoc::After);
        let selection = SelectionRange::new(map(selection.anchor()), map(selection.head()));
        Ok(range_edit(changes, selection))
    })?;
    if !edit_text.keeps_breaks(changes.changes()) {
        return Ok(None);
    }
    Ok(Some(if changes.changes().is_empty() {
        Transaction::default()
    } else {
        changes.with_user_event("indent")
    }))
}

fn settings(indent: Indentation) -> StreamSettings {
    StreamSettings {
        tab_size: indent.tab_size,
        indent_unit: indent.width(),
    }
}

fn bracket_gap(
    source: &str,
    line: &Range<usize>,
    position: usize,
    between_brackets: impl FnOnce(usize) -> Option<Range<usize>>,
) -> Option<Range<usize>> {
    if position > 0
        && matches!(
            source.as_bytes().get(position - 1..position + 1),
            Some(b"()" | b"[]" | b"{}")
        )
    {
        return Some(position..position);
    }
    between_brackets(position).filter(|gap| {
        line.start <= gap.start
            && gap.start <= position
            && position <= gap.end
            && gap.end <= line.end
            && source
                .get(gap.clone())
                .is_some_and(|s| s.chars().all(space))
    })
}

fn clear_prefix(text: &Text, line: &Line<'_>, from: usize, indent: &str) -> bool {
    let offset = from - line.range.start;
    if offset == 0
        || offset > indent.len()
        || line.text[..offset].encode_utf16().take(100).count() == 100
    {
        return false;
    }
    if indent == line.text {
        return true;
    }
    // The pinned command tests a line prefix sliced at the *document* offset.
    // Compare with that UTF-16 offset, counting each hard break as one unit.
    let mut remaining = indent.encode_utf16().count();
    text.iter_range(0..from)
        .expect("selection positions are valid")
        .all(|chunk| {
            let units = if chunk == "\r\n" {
                1
            } else {
                chunk.encode_utf16().take(remaining + 1).count()
            };
            if let Some(left) = remaining.checked_sub(units) {
                remaining = left;
                true
            } else {
                false
            }
        })
}
