use unicode_segmentation::UnicodeSegmentation;

use super::*;
use crate::{CharCategory, language::line_projection::command_line_projection};

#[derive(Clone, Copy)]
pub(super) enum GroupStyle {
    Motion,
    Delete,
    Windows,
}

pub(super) fn group_boundary(
    state: &EditorState,
    pos: usize,
    direction: Direction,
    style: GroupStyle,
) -> usize {
    use GroupStyle::{Delete, Motion, Windows};

    let text = state.text();
    let source = text.as_str();
    let edge = match style {
        Delete => {
            let projection = command_line_projection(state);
            direction.edge(&line_at(projection.as_ref().unwrap_or(text), pos).range)
        }
        _ => direction.edge(&(0..source.len())),
    };
    if matches!(style, Delete) && (pos == edge || inside_crlf(source, pos)) {
        return direction.grapheme(source, pos);
    }
    let categorize = state.char_categorizer(pos);
    let mut cursor = GraphemeCursor::new(pos, source.len(), true);
    let mut category = None;
    let mut skipped_space = false;
    let mut at = pos;
    while at != edge {
        let next = direction.step(source, &mut cursor);
        let cluster = &source[at.min(next)..at.max(next)];
        let current = categorize.category(cluster);
        if !match style {
            Motion => category.is_none_or(|c| c == CharCategory::Space || c == current),
            Delete => category.is_none_or(|c| c == current),
            Windows => {
                if current == CharCategory::Space {
                    skipped_space = true;
                    true
                } else {
                    !skipped_space && category.is_none_or(|c| c == current)
                }
            }
        } {
            break;
        }
        if !matches!(style, Delete) || at != pos || cluster != " " {
            category = Some(current);
        }
        at = next;
    }
    at
}

fn move_selection(
    state: &EditorState,
    direction: Direction,
    extend: bool,
    collapse: bool,
    to: impl Fn(usize) -> usize,
) -> CommandResult {
    let selection = selection_by(state, |range| {
        if extend {
            SelectionRange::new(range.anchor(), to(range.head()))
        } else if collapse && !range.range().is_empty() {
            caret(direction.edge(&range.range()))
        } else {
            caret(to(range.head()))
        }
    })?;
    Ok((&selection != state.selection()).then(|| select(selection, true)))
}

fn by_char(state: &EditorState, direction: Direction, extend: bool) -> CommandResult {
    move_selection(state, direction, extend, true, |head| {
        direction.grapheme(state.text().as_str(), head)
    })
}

/// Move forward by one grapheme. Collapse nonempty ranges to their end.
pub fn cursor_char_forward_logical(state: &EditorState) -> CommandResult {
    by_char(state, Forward, false)
}

/// Move backward by one grapheme. Collapse nonempty ranges to their start.
pub fn cursor_char_backward_logical(state: &EditorState) -> CommandResult {
    by_char(state, Backward, false)
}

/// Move each selection head forward by one grapheme.
pub fn select_char_forward_logical(state: &EditorState) -> CommandResult {
    by_char(state, Forward, true)
}

/// Move each selection head backward by one grapheme.
pub fn select_char_backward_logical(state: &EditorState) -> CommandResult {
    by_char(state, Backward, true)
}

fn by_group(
    state: &EditorState,
    direction: Direction,
    extend: bool,
    style: GroupStyle,
) -> CommandResult {
    move_selection(state, direction, extend, true, |head| {
        group_boundary(state, head, direction, style)
    })
}

/// Move forward past whitespace and the next group of word or punctuation graphemes.
pub fn cursor_group_forward(state: &EditorState) -> CommandResult {
    by_group(state, Forward, false, GroupStyle::Motion)
}

/// Move backward past whitespace and the previous group of word or punctuation graphemes.
pub fn cursor_group_backward(state: &EditorState) -> CommandResult {
    by_group(state, Backward, false, GroupStyle::Motion)
}

/// Extend each selection forward by a group.
pub fn select_group_forward(state: &EditorState) -> CommandResult {
    by_group(state, Forward, true, GroupStyle::Motion)
}

/// Extend each selection backward by a group.
pub fn select_group_backward(state: &EditorState) -> CommandResult {
    by_group(state, Backward, true, GroupStyle::Motion)
}

/// Move forward past one group and the whitespace after it.
pub fn cursor_group_forward_win(state: &EditorState) -> CommandResult {
    by_group(state, Forward, false, GroupStyle::Windows)
}

/// Extend each selection past one group and the whitespace after it.
pub fn select_group_forward_win(state: &EditorState) -> CommandResult {
    by_group(state, Forward, true, GroupStyle::Windows)
}

fn by_boundary(
    state: &EditorState,
    direction: Direction,
    extend: bool,
    smart_start: bool,
) -> CommandResult {
    let projection = command_line_projection(state);
    let lines = projection.as_ref().unwrap_or(state.text());
    move_selection(state, direction, extend, false, |head| {
        let line = line_at(lines, head);
        if smart_start {
            let indent = line.range.start
                + line
                    .text
                    .chars()
                    .take(100)
                    .take_while(|&ch| space(ch))
                    .map(char::len_utf8)
                    .sum::<usize>();
            if head != indent {
                return indent;
            }
        }
        direction.edge(&line.range)
    })
}

/// Move to the start of each caret's hard line.
pub fn cursor_line_start(state: &EditorState) -> CommandResult {
    by_boundary(state, Backward, false, false)
}

/// Move to the end of each caret's hard line.
pub fn cursor_line_end(state: &EditorState) -> CommandResult {
    by_boundary(state, Forward, false, false)
}

/// Extend each selection to the start of its head's hard line.
pub fn select_line_start(state: &EditorState) -> CommandResult {
    by_boundary(state, Backward, true, false)
}

/// Extend each selection to the end of its head's hard line.
pub fn select_line_end(state: &EditorState) -> CommandResult {
    by_boundary(state, Forward, true, false)
}

/// Move to the end of each caret's hard line, without considering wraps.
pub fn cursor_line_boundary_forward(state: &EditorState) -> CommandResult {
    cursor_line_end(state)
}

/// Move to the end of indentation, or to the hard line start if already there.
pub fn cursor_line_boundary_backward(state: &EditorState) -> CommandResult {
    by_boundary(state, Backward, false, true)
}

/// Extend each selection to the end of its head's hard line, without considering wraps.
pub fn select_line_boundary_forward(state: &EditorState) -> CommandResult {
    select_line_end(state)
}

/// Extend each selection to the end of indentation or to the hard line start.
pub fn select_line_boundary_backward(state: &EditorState) -> CommandResult {
    by_boundary(state, Backward, true, true)
}

pub(super) fn hard_line_step(text: &Text, pos: usize, direction: Direction) -> usize {
    let line = line_at(text, pos);
    let source = text.as_str();
    let next = match direction {
        Backward => (line.range.start > 0)
            .then(|| line.range.start - break_before(source, line.range.start).len()),
        Forward => (line.range.end < text.len())
            .then(|| line.range.end + break_after(source, line.range.end).len()),
    };
    let Some(next) = next else {
        return direction.edge(&line.range);
    };
    let next = line_at(text, next);
    let column = line
        .text
        .grapheme_indices(true)
        .take_while(|(start, _)| *start < pos - line.range.start)
        .count();
    next.text
        .grapheme_indices(true)
        .nth(column)
        .map_or(next.range.end, |(start, _)| next.range.start + start)
}

fn by_line(state: &EditorState, direction: Direction, extend: bool) -> CommandResult {
    let projection = command_line_projection(state);
    let lines = projection.as_ref().unwrap_or(state.text());
    move_selection(state, direction, extend, true, |head| {
        hard_line_step(lines, head, direction)
    })
}

/// Move down one hard line at the current grapheme column. No goal column is stored.
pub fn cursor_line_down(state: &EditorState) -> CommandResult {
    by_line(state, Forward, false)
}

/// Move up one hard line at the current grapheme column. No goal column is stored.
pub fn cursor_line_up(state: &EditorState) -> CommandResult {
    by_line(state, Backward, false)
}

/// Extend each selection down one hard line at the head's grapheme column.
pub fn select_line_down(state: &EditorState) -> CommandResult {
    by_line(state, Forward, true)
}

/// Extend each selection up one hard line at the head's grapheme column.
pub fn select_line_up(state: &EditorState) -> CommandResult {
    by_line(state, Backward, true)
}

/// Set a single cursor at the start of the document.
pub fn cursor_doc_start(_state: &EditorState) -> CommandResult {
    Ok(Some(select(EditorSelection::single(0, 0), true)))
}

/// Set a single cursor at the end of the document.
pub fn cursor_doc_end(state: &EditorState) -> CommandResult {
    let end = state.text().len();
    Ok(Some(select(EditorSelection::single(end, end), true)))
}

/// Extend the main selection to the start of the document and discard the other ranges.
pub fn select_doc_start(state: &EditorState) -> CommandResult {
    Ok(Some(select(
        EditorSelection::single(state.selection().main().anchor(), 0),
        true,
    )))
}

/// Extend the main selection to the end of the document and discard the other ranges.
pub fn select_doc_end(state: &EditorState) -> CommandResult {
    Ok(Some(select(
        EditorSelection::single(state.selection().main().anchor(), state.text().len()),
        true,
    )))
}

/// Select the entire document without requesting a scroll.
pub fn select_all(state: &EditorState) -> CommandResult {
    Ok(Some(select(
        EditorSelection::single(0, state.text().len()),
        false,
    )))
}

/// Keep only the main range, or collapse a single nonempty range to its head.
pub fn simplify_selection(state: &EditorState) -> CommandResult {
    let main = state.selection().main();
    let selection = if state.selection().ranges().len() > 1 {
        EditorSelection::single(main.anchor(), main.head())
    } else if !main.range().is_empty() {
        EditorSelection::single(main.head(), main.head())
    } else {
        return Ok(None);
    };
    Ok(Some(select(selection, true)))
}
