use super::indent::Indentation;
use super::motion::{GroupStyle, group_boundary};
use super::*;
use crate::language::line_projection::{command_line_projection, keeps_separate_breaks};

fn delete_by(state: &EditorState, to: impl Fn(usize) -> usize) -> CommandResult {
    if state.read_only() {
        return Ok(None);
    }
    let source = state.text().as_str();
    let mut event = "delete.selection";
    let transaction = state.change_by_range(|selection| {
        let mut range = selection.range();
        if range.is_empty() {
            let at = range.start;
            let next = to(at);
            event = match next.cmp(&at) {
                std::cmp::Ordering::Less => "delete.backward",
                std::cmp::Ordering::Greater => "delete.forward",
                std::cmp::Ordering::Equal => return Ok(unchanged(selection)),
            };
            range = at.min(next)..at.max(next);
        }
        let range = include_crlf(source, range);
        let start = range.start;
        replace_range(range, "", start)
    })?;
    Ok(finish_deletion(state, transaction, event))
}

pub(super) fn finish_deletion(
    state: &EditorState,
    transaction: Transaction,
    event: &'static str,
) -> Option<Transaction> {
    if transaction.changes().is_empty()
        || (command_line_projection(state).is_some()
            && !keeps_separate_breaks(state.text().as_str(), transaction.changes()))
    {
        return None;
    }
    let mut transaction = transaction.with_scroll_into_view().with_user_event(event);
    if event == "delete.selection" {
        transaction = transaction.with_effects([announce_selection_deleted(state)]);
    }
    Some(transaction)
}

/// Announce a completed selection deletion to the host.
pub fn announce_selection_deleted(state: &EditorState) -> crate::StateEffect {
    crate::view::ANNOUNCE.of(state.phrase("Selection deleted", &[]))
}

/// Delete each selection, or the following grapheme at a caret.
pub fn delete_char_forward(state: &EditorState) -> CommandResult {
    delete_by(state, |pos| Forward.grapheme(state.text().as_str(), pos))
}

// CodeMirror backspace can remove extending characters one at a time. Keep
// flags and ZWJ sequences together, and do not leave a base variation behind.
fn backspace_cluster(source: &str, pos: usize) -> usize {
    let prefix = &source[..pos];
    if prefix.ends_with('\n') || inside_crlf(source, pos) {
        return Backward.grapheme(source, pos);
    }
    let step = |at: usize| {
        let mut chars = source[..at].char_indices().rev();
        let Some((mut start, mut current)) = chars.next() else {
            return at;
        };
        if ('\u{1f1e6}'..='\u{1f1ff}').contains(&current) {
            return Backward.grapheme(source, at);
        }
        for (before, previous) in chars {
            if previous == '\n' || current != '\u{200d}' && previous != '\u{200d}' {
                break;
            }
            start = before;
            current = previous;
        }
        start
    };
    let start = step(pos);
    if source[start..pos]
        .chars()
        .any(|c| ('\u{fe00}'..='\u{fe0f}').contains(&c))
    {
        step(start)
    } else {
        start
    }
}

/// Delete each selection, or backspace without removing an entire indent unit.
pub fn delete_char_backward_strict(state: &EditorState) -> CommandResult {
    delete_by(state, |pos| backspace_cluster(state.text().as_str(), pos))
}

/// Backspace with the default two-space indentation and four-column tab stops.
pub fn delete_char_backward(state: &EditorState) -> CommandResult {
    delete_char_backward_with(state, Indentation::for_state(state))
}

/// Delete each selection, or backspace to the previous indent stop in leading spaces.
pub fn delete_char_backward_with(state: &EditorState, indent: Indentation) -> CommandResult {
    let projection = command_line_projection(state);
    let lines = projection.as_ref().unwrap_or(state.text());
    delete_by(state, |pos| {
        let line = line_at(lines, pos);
        if let Some(prefix) = line.text.get(..pos - line.range.start)
            && !prefix.is_empty()
            && prefix.len() < 200
            && prefix.bytes().all(|byte| matches!(byte, b' ' | b'\t'))
        {
            if prefix.ends_with('\t') {
                return pos - 1;
            }
            let width = indent.width();
            let remainder = indent.columns(prefix) % width;
            let drop = if remainder == 0 { width } else { remainder };
            return pos
                - prefix
                    .bytes()
                    .rev()
                    .take(drop)
                    .take_while(|&byte| byte == b' ')
                    .count();
        }
        backspace_cluster(state.text().as_str(), pos)
    })
}

/// Delete each selection, or delete forward by a group on the same hard line.
/// Skip one leading ASCII space. At the line end, delete only the line break.
pub fn delete_group_forward(state: &EditorState) -> CommandResult {
    delete_by(state, |pos| {
        group_boundary(state, pos, Forward, GroupStyle::Delete)
    })
}

/// Delete each selection, or delete backward by a group on the same hard line.
/// Skip one trailing ASCII space. At the line start, delete only the line break.
pub fn delete_group_backward(state: &EditorState) -> CommandResult {
    delete_by(state, |pos| {
        group_boundary(state, pos, Backward, GroupStyle::Delete)
    })
}

/// Delete each selection, or delete forward by a group and its following whitespace.
pub fn delete_group_forward_win(state: &EditorState) -> CommandResult {
    delete_by(state, |pos| {
        group_boundary(state, pos, Forward, GroupStyle::Windows)
    })
}

/// Delete each selection, or delete to the start of its hard line.
/// At line start, delete the preceding line break instead.
pub fn delete_to_line_start(state: &EditorState) -> CommandResult {
    let projection = command_line_projection(state);
    let lines = projection.as_ref().unwrap_or(state.text());
    delete_by(state, |pos| {
        let start = line_at(lines, pos).range.start;
        if pos > start {
            start
        } else {
            start - break_before(lines.as_str(), start).len()
        }
    })
}

/// Delete each selection, or delete to the end of its hard line.
/// At line end, delete the following line break instead.
pub fn delete_to_line_end(state: &EditorState) -> CommandResult {
    let projection = command_line_projection(state);
    let lines = projection.as_ref().unwrap_or(state.text());
    delete_by(state, |pos| {
        let end = line_at(lines, pos).range.end;
        if pos < end {
            end
        } else {
            end + break_after(lines.as_str(), end).len()
        }
    })
}

/// Remove trailing whitespace on all lines, including the last line.
pub fn delete_trailing_whitespace(state: &EditorState) -> CommandResult {
    if state.read_only() {
        return Ok(None);
    }
    let projection = command_line_projection(state);
    let lines = projection.as_ref().unwrap_or(state.text());
    let mut pos = 0;
    let mut changes = Vec::new();
    for chunk in lines.iter() {
        if !matches!(chunk, "\n" | "\r\n") {
            let from = pos + chunk.trim_end_matches(space).len();
            let to = pos + chunk.len();
            if from < to {
                changes.push(Change::new(from..to, ""));
            }
        }
        pos += chunk.len();
    }
    let changes = ChangeSet::new(changes)?;
    if changes.is_empty()
        || (projection.is_some() && !keeps_separate_breaks(state.text().as_str(), &changes))
    {
        return Ok(None);
    }
    Ok(Some(Transaction::new(changes).with_user_event("delete")))
}
