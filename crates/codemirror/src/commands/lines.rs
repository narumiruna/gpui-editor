use super::motion::hard_line_step;
use super::*;
use crate::{
    Assoc,
    language::line_projection::{
        command_line_projection, is_editor_line_view, keeps_separate_breaks,
    },
};

#[derive(Debug)]
struct LineBlock {
    lines: Range<usize>,
    selections: Range<usize>,
}

fn selected_blocks(state: &EditorState, text: &Text) -> Vec<LineBlock> {
    let source = text.as_str();
    let mut blocks: Vec<LineBlock> = Vec::new();
    for (i, selection) in state.selection().ranges().iter().enumerate() {
        let range = selection.range();
        let start = line_at(text, range.start).range.start;
        let last = line_at(text, range.end);
        // Exclude the next line when a nonempty selection ends at its start.
        let end = if !range.is_empty() && range.end == last.range.start {
            line_at(text, range.end - 1).range.end
        } else {
            last.range.end
        };
        if let Some(block) = blocks.last_mut()
            && start <= block.lines.end + break_after(source, block.lines.end).len()
        {
            block.lines.end = end.max(block.lines.end);
            block.selections.end = i + 1;
        } else {
            blocks.push(LineBlock {
                lines: start..end,
                selections: i..i + 1,
            });
        }
    }
    blocks
}

/// Select the touched hard lines and their trailing line breaks, if any.
/// Merge adjacent lines into blocks and make the first block primary.
pub fn select_line(state: &EditorState) -> CommandResult {
    let projection = command_line_projection(state);
    let text = LineEditText::new(state.text(), projection.as_ref())?;
    let ranges = selected_blocks(state, text.lines)
        .into_iter()
        .map(|block| {
            let range = block.lines;
            EditorSelection::undirectional_range(
                range.start,
                range.end + text.break_after(range.end).len(),
            )
        })
        .collect();
    Ok(Some(select(EditorSelection::new(ranges, 0)?, false)))
}

/// Delete the touched hard lines. Move heads down one hard line before mapping
/// them through the deletions. Use grapheme columns, not view coordinates.
pub fn delete_line(state: &EditorState) -> CommandResult {
    if state.read_only() {
        return Ok(None);
    }
    let projection = command_line_projection(state);
    let text = LineEditText::new(state.text(), projection.as_ref())?;
    let changes = selected_blocks(state, text.lines)
        .into_iter()
        .map(|block| {
            let mut range = block.lines;
            if range.start > 0 {
                range.start -= text.break_before(range.start).len();
            } else {
                range.end += text.break_after(range.end).len();
            }
            Change::new(range, "")
        })
        .collect();
    let changes = ChangeSet::new(changes)?;
    if !text.keeps_breaks(&changes) {
        return Ok(None);
    }
    let selection = selection_by(state, |r| {
        caret(hard_line_step(text.lines, r.head(), Forward))
    })?;
    let selection = selection.map_desc(&changes.desc(text.raw.len())?)?;
    Ok(Some(
        Transaction::new(changes)
            .with_selection(selection)
            .with_user_event("delete.line")
            .with_scroll_into_view(),
    ))
}

fn preferred_break(source: &str, line: &Range<usize>) -> &'static str {
    let after = break_after(source, line.end);
    if after == "\r\n" || (after.is_empty() && break_before(source, line.start) == "\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn newline(state: &EditorState, keep_indent: bool, split: bool) -> CommandResult {
    if split && state.read_only() {
        return Ok(None);
    }
    let projection = command_line_projection(state);
    let text = LineEditText::new(state.text(), projection.as_ref())?;
    let transaction = state.change_by_range(|selection| {
        let range = include_crlf(text.raw, selection.range());
        let line = line_at(text.lines, range.start);
        let ending = if projection.is_some() {
            text.preferred_break(&line.range, "\n")
        } else {
            preferred_break(text.raw, &line.range)
        };
        let indent = if keep_indent {
            leading_space(line.text)
        } else {
            ""
        };
        let insert = [ending, indent].concat();
        let target = range.start + if split { 0 } else { insert.len() };
        replace_range(range, insert, target)
    })?;
    if !text.keeps_breaks(transaction.changes()) {
        return Ok(None);
    }
    Ok(Some(
        transaction.with_user_event("input").with_scroll_into_view(),
    ))
}

/// Replace each selection with a local line break and leave the caret before it.
pub fn split_line(state: &EditorState) -> CommandResult {
    newline(state, false, true)
}

/// Replace each selection with a local line break and move the caret after it.
pub fn insert_newline(state: &EditorState) -> CommandResult {
    newline(state, false, false)
}

/// Insert a line break and copy the leading whitespace of its source line.
pub fn insert_newline_keep_indent(state: &EditorState) -> CommandResult {
    newline(state, true, false)
}

/// Swap the graphemes before and after each caret. A line break moves as one
/// unit. Skip nonempty selections and carets at a document edge or inside CRLF.
pub fn transpose_chars(state: &EditorState) -> CommandResult {
    if state.read_only() {
        return Ok(None);
    }
    let source = state.text().as_str();
    let transaction = state.change_by_range(|selection| {
        let pos = selection.head();
        if !selection.range().is_empty()
            || pos == 0
            || pos == source.len()
            || inside_crlf(source, pos)
        {
            return Ok(unchanged(selection));
        }
        let from = Backward.grapheme(source, pos);
        let to = Forward.grapheme(source, pos);
        replace_range(
            from..to,
            [&source[pos..to], &source[from..pos]].concat(),
            to,
        )
    })?;
    Ok((!transaction.changes().is_empty()).then(|| {
        transaction
            .with_user_event("move.character")
            .with_scroll_into_view()
    }))
}

pub(super) struct LineEditText<'a> {
    pub(super) lines: &'a Text,
    pub(super) raw: &'a str,
    projected: bool,
}

impl<'a> LineEditText<'a> {
    pub(super) fn new(raw: &'a Text, projection: Option<&'a Text>) -> Result<Self, EditError> {
        let lines = projection.unwrap_or(raw);
        let raw = raw.as_str();
        if projection.is_some_and(|lines| !is_editor_line_view(raw, lines)) {
            return Err(EditError::InvalidRange);
        }
        Ok(Self {
            lines,
            raw,
            projected: projection.is_some(),
        })
    }

    pub(super) fn break_after(&self, end: usize) -> &'a str {
        let len = break_after(self.lines.as_str(), end).len();
        &self.raw[end..end + len]
    }

    pub(super) fn break_before(&self, start: usize) -> &'a str {
        let len = break_before(self.lines.as_str(), start).len();
        &self.raw[start - len..start]
    }

    pub(super) fn preferred_break<'b>(&'b self, line: &Range<usize>, fallback: &'b str) -> &'b str {
        match self.break_after(line.end) {
            "" => match self.break_before(line.start) {
                "" => fallback,
                ending => ending,
            },
            ending => ending,
        }
    }

    // Edits at logical line bounds do not split existing CRLF pairs. Check
    // that joining the raw fragments does not turn two breaks into one.
    pub(super) fn keeps_breaks(&self, changes: &ChangeSet) -> bool {
        !self.projected || keeps_separate_breaks(self.raw, changes)
    }
}

fn move_line(
    state: &EditorState,
    direction: Direction,
    projection: Option<&Text>,
) -> CommandResult {
    if state.read_only() {
        return Ok(None);
    }
    let edit_text = LineEditText::new(state.text(), projection)?;
    let text = edit_text.lines;
    let source = edit_text.raw;
    let mut ranges = state.selection().ranges().to_vec();
    let mut changes = Vec::new();
    for block in selected_blocks(state, text) {
        let lines = block.lines;
        let (remove, insert_at, insert, moved_start, next_break_len) = match direction {
            Backward if lines.start > 0 => {
                let separator = edit_text.break_before(lines.start);
                let previous = line_at(text, lines.start - separator.len());
                (
                    previous.range.start..lines.start,
                    lines.end,
                    [separator, &source[previous.range.clone()]],
                    previous.range.start,
                    separator.len(),
                )
            }
            Forward if lines.end < text.len() => {
                let separator = edit_text.break_after(lines.end);
                let next = line_at(text, lines.end + separator.len()).range;
                (
                    lines.end..next.end,
                    lines.start,
                    [&source[next.clone()], separator],
                    lines.start + next.len() + separator.len(),
                    edit_text.break_after(next.end).len(),
                )
            }
            _ => continue,
        };
        // Move the neighbor instead of replacing the selected text. Positions
        // in the selected block can then map through ordinary change mapping.
        changes.push(Change::new(remove, ""));
        changes.push(Change::new(insert_at..insert_at, insert.concat()));
        let old_break_len = edit_text.break_after(lines.end).len();
        let relocate = |pos: usize| {
            if pos <= lines.end {
                moved_start + (pos - lines.start)
            } else {
                let offset = pos - lines.end;
                let offset = if offset == old_break_len {
                    next_break_len
                } else {
                    offset.min(next_break_len)
                };
                moved_start + lines.len() + offset
            }
        };
        for range in &mut ranges[block.selections] {
            *range = SelectionRange::new(relocate(range.anchor()), relocate(range.head()));
        }
    }
    let changes = ChangeSet::new(changes)?;
    if changes.is_empty() || !edit_text.keeps_breaks(&changes) {
        return Ok(None);
    }
    let selection = EditorSelection::new(ranges, state.selection().main_index())?;
    Ok(Some(
        Transaction::new(changes)
            .with_selection(selection)
            .with_scroll_into_view()
            .with_user_event("move.line"),
    ))
}

/// Move selected line blocks up. Leave blocks at the start of the document in place.
pub fn move_line_up(state: &EditorState) -> CommandResult {
    move_line(state, Backward, command_line_projection(state).as_ref())
}

/// Move selected line blocks down. Leave blocks at the end of the document in place.
pub fn move_line_down(state: &EditorState) -> CommandResult {
    move_line(state, Forward, command_line_projection(state).as_ref())
}

/// Move selected line blocks using a same-width projection of the raw text.
/// The projection must replace only lone CR bytes with LF, and replace all of them.
/// Move down when `down` is true. Keep the raw separator crossed by the move.
/// Leave the text unchanged if two breaks would join into a CRLF pair.
pub fn move_line_with_projection(state: &EditorState, lines: &Text, down: bool) -> CommandResult {
    move_line(state, if down { Forward } else { Backward }, Some(lines))
}

fn copy_line(
    state: &EditorState,
    direction: Direction,
    ending: Option<&str>,
    projection: Option<&Text>,
) -> CommandResult {
    if state.read_only() {
        return Ok(None);
    }
    let edit_text = LineEditText::new(state.text(), projection)?;
    let text = edit_text.lines;
    let source = edit_text.raw;
    let mut changes = Vec::new();
    for block in selected_blocks(state, text) {
        let content = &source[block.lines.clone()];
        let ending = ending.unwrap_or_else(|| {
            let last = line_at(text, block.lines.end).range;
            if !edit_text.projected {
                return preferred_break(source, &last);
            }
            edit_text.preferred_break(&last, "\n")
        });
        let (at, left, right) = match direction {
            Forward => (block.lines.start, content, ending),
            Backward => (block.lines.end, ending, content),
        };
        if edit_text.projected && left.ends_with('\r') && right.starts_with('\n') {
            return Ok(None);
        }
        changes.push(Change::new(at..at, [left, right].concat()));
    }
    let changes = ChangeSet::new(changes)?;
    if !edit_text.keeps_breaks(&changes) {
        return Ok(None);
    }
    let selection = selection_by(state, |range| {
        let extent = range.range();
        let empty_assoc = match direction {
            Backward => Assoc::Before,
            Forward => Assoc::After,
        };
        let map = |pos| {
            changes.map_pos(
                pos,
                if extent.is_empty() {
                    empty_assoc
                } else if pos == extent.start {
                    Assoc::After
                } else {
                    Assoc::Before
                },
            )
        };
        SelectionRange::new(map(range.anchor()), map(range.head()))
    })?;
    Ok(Some(
        Transaction::new(changes)
            .with_selection(selection)
            .with_user_event("input.copyline")
            .with_scroll_into_view(),
    ))
}

/// Duplicate selected lines, keeping the selection in the top copy.
pub fn copy_line_up(state: &EditorState) -> CommandResult {
    copy_line(
        state,
        Backward,
        None,
        command_line_projection(state).as_ref(),
    )
}

/// Duplicate selected lines, keeping the selection in the bottom copy.
pub fn copy_line_down(state: &EditorState) -> CommandResult {
    copy_line(
        state,
        Forward,
        None,
        command_line_projection(state).as_ref(),
    )
}

/// Copy selected line blocks using the same projection as [`move_line_with_projection`].
/// Keep the selection in the bottom copy when `down` is true. Use the raw break
/// after the last selected line, its preceding break if at EOF, or LF if neither
/// exists. Leave the text unchanged if two breaks would join into a CRLF pair.
pub fn copy_line_with_projection(state: &EditorState, lines: &Text, down: bool) -> CommandResult {
    copy_line(
        state,
        if down { Forward } else { Backward },
        None,
        Some(lines),
    )
}

/// Duplicate selected lines with an explicit LF or CRLF separator.
/// Keep the selection in the bottom copy when `down` is true.
pub fn copy_line_with_ending(state: &EditorState, down: bool, ending: &str) -> CommandResult {
    if !matches!(ending, "\n" | "\r\n") {
        return Err(EditError::InvalidRange);
    }
    copy_line(
        state,
        if down { Forward } else { Backward },
        Some(ending),
        command_line_projection(state).as_ref(),
    )
}
