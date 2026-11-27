use crate::{Change, ChangeSet, EditError, EditorSelection, EditorState, Text, Transaction};

use super::{Chunk, Side, UnifiedMerge, original_doc_change_effect, unified_merge_state};
use crate::merge::get_chunks;

/// A control for a chunk in a unified merge snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MergeControl {
    pub kind: MergeControlKind,
    /// UTF-8 byte position at the start of the chunk in the editor document.
    pub position: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeControlKind {
    Accept,
    Reject,
}

/// Prepare an `accept` transaction for the chunk at `position` or the main cursor.
/// Only the original document changes. Return `None` if no chunk is there.
/// Apply the transaction to the state used to prepare it.
pub fn accept_chunk(
    state: &EditorState,
    position: Option<usize>,
) -> Result<Option<Transaction>, EditError> {
    chunk_action(state, position, Side::A)
}

/// Prepare a `revert` transaction for the chunk at `position` or the main cursor.
/// The editor document changes normally, including filtering and undo history.
/// Return `None` if no chunk is there. Apply the transaction to the state used to prepare it.
pub fn reject_chunk(
    state: &EditorState,
    position: Option<usize>,
) -> Result<Option<Transaction>, EditError> {
    chunk_action(state, position, Side::B)
}

fn chunk_action(
    state: &EditorState,
    position: Option<usize>,
    destination: Side,
) -> Result<Option<Transaction>, EditError> {
    let Some(merge) = unified_merge_state(state)? else {
        return Ok(None);
    };
    let at = position.unwrap_or_else(|| state.selection().main().head());
    let b = state.text();
    if !b.is_char_boundary(at) {
        return Ok(None);
    }
    let Some(chunk) = merge.chunks().iter().find(|chunk| {
        let range = chunk.range(Side::B);
        range.start <= at && at <= range.end && at <= content_end(chunk, &merge, b)
    }) else {
        return Ok(None);
    };

    // Native chunk ranges carry the complete line endings and stop at real EOF.
    let (source, source_side) = match destination {
        Side::A => (b, Side::B),
        Side::B => (merge.original(), Side::A),
    };
    let insert = source
        .slice_string(chunk.range(source_side))
        .ok_or(EditError::InvalidRange)?;
    let changes = ChangeSet::single(Change::new(chunk.range(destination), insert))?;
    Ok(Some(match destination {
        Side::A => Transaction::default()
            .with_effects([original_doc_change_effect(state, changes)?])
            .with_user_event("accept"),
        Side::B => Transaction::new(changes).with_user_event("revert"),
    }))
}

// Upstream accepts positions through the end of the last changed line but not
// the next line's start. If the terminal empty line is part of the chunk, its
// end is EOF even though the native range does not include a virtual newline.
fn content_end(chunk: &Chunk, merge: &UnifiedMerge, b: &Text) -> usize {
    let range = chunk.range(Side::B);
    if range.is_empty() {
        return range.end;
    }
    if range.end == b.len()
        && let Some(last) = chunk.changes().last()
        && let Some(b_line) = b.line_at(range.start + last.b.end)
        && b_line.range.start == b.len()
    {
        let a_end = chunk.range(Side::A).start + last.a.end;
        if merge
            .original()
            .line_at(a_end)
            .is_some_and(|line| line.range.start != a_end)
        {
            return range.end;
        }
    }
    b.line_at(range.end - 1)
        .map_or(range.end, |line| line.range.end)
}

/// Prepare a selection transaction for the next changed chunk, wrapping at the end.
pub fn go_to_next_chunk(state: &EditorState) -> Result<Option<Transaction>, EditError> {
    move_by_chunk(state, true)
}

/// Prepare a selection transaction for the preceding changed chunk, wrapping at the start.
pub fn go_to_previous_chunk(state: &EditorState) -> Result<Option<Transaction>, EditError> {
    move_by_chunk(state, false)
}

fn move_by_chunk(state: &EditorState, forward: bool) -> Result<Option<Transaction>, EditError> {
    let Some((chunks, side)) = get_chunks(state)? else {
        return Ok(None);
    };
    if chunks.is_empty() {
        return Ok(None);
    }
    let head = state.selection().main().head();
    let mut index = 0;
    for (i, chunk) in chunks.iter().enumerate().rev() {
        let range = chunk.range(side);
        if range.end < head {
            index = i + 1;
            break;
        }
        if range.start <= head {
            if chunks.len() == 1 {
                return Ok(None);
            }
            index = i + usize::from(forward);
            break;
        }
    }
    if !forward {
        index += chunks.len() - 1;
    }
    let start = chunks[index % chunks.len()].range(side).start;
    Ok(Some(
        Transaction::default()
            .with_selection(EditorSelection::single(start, start))
            .with_user_event("select.byChunk")
            .with_scroll_into_view(),
    ))
}
