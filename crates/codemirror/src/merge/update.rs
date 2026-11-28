use std::ops::Range;

use super::{Chunk, Side, near, offset};
use crate::{ChangeDesc, Text};

const MARGIN: usize = 1_000;

// None requests a full rebuild. A build error must reach the caller instead.
pub(super) fn update<E>(
    chunks: &[Chunk],
    a: &Text,
    b: &Text,
    side: Side,
    changes: &ChangeDesc,
    build: impl Fn(&str, &str) -> Result<Vec<Chunk>, E>,
) -> Option<Result<Vec<Chunk>, E>> {
    let (edited, other) = sides(side, a.as_str(), b.as_str());
    let (old_len, new_len) = (changes.length(), changes.new_length());
    if new_len != edited.len() {
        return None;
    }
    if changes.is_empty() {
        return Some(Ok(chunks.to_vec()));
    }
    // Give each edit its own context, rounded out to paired line boundaries.
    // Join overlapping windows, but retain chunks between distant edits.
    let mut windows: Vec<Window> = Vec::new();
    for (old, new) in changes.changed_ranges() {
        edited.get(new.clone())?;
        let lo = old.start.saturating_sub(MARGIN);
        let hi = old.end.saturating_add(MARGIN).min(old_len);
        let (first, from, other_from) = if lo == 0 {
            (0, 0, 0)
        } else {
            start_anchor(chunks, side, other, lo)?
        };
        let (last, to, other_to) = if hi == old_len {
            (chunks.len(), old_len, other.len())
        } else {
            end_anchor(chunks, side, other, hi)?
        };
        let to_new = new.end.checked_add(to.checked_sub(old.end)?)?;
        if let Some(previous) = windows.last_mut()
            && (from <= previous.old_to || other_from <= previous.other.end)
        {
            previous.chunks.end = last;
            previous.old_to = to;
            previous.edited.end = to_new;
            previous.other.end = other_to;
        } else {
            let from_new = new.start.checked_sub(old.start.checked_sub(from)?)?;
            windows.push(Window {
                chunks: first..last,
                old_to: to,
                edited: from_new..to_new,
                other: other_from..other_to,
            });
        }
    }
    let mut result = Vec::new();
    let (mut index, mut old_base, mut new_base) = (0, 0, 0);
    for window in windows {
        reuse(
            &mut result,
            chunks.get(index..window.chunks.start)?,
            side,
            old_base,
            new_base,
        )?;
        let (off_a, off_b) = sides(side, window.edited.start, window.other.start);
        let (a, b) = sides(
            side,
            edited.get(window.edited.clone())?,
            other.get(window.other)?,
        );
        let rebuilt = match build(a, b) {
            Ok(chunks) => chunks,
            Err(error) => return Some(Err(error)),
        };
        for mut chunk in rebuilt {
            chunk.a = offset(&chunk.a, off_a);
            chunk.b = offset(&chunk.b, off_b);
            push(&mut result, chunk)?;
        }
        (index, old_base, new_base) = (window.chunks.end, window.old_to, window.edited.end);
    }
    reuse(&mut result, chunks.get(index..)?, side, old_base, new_base)?;
    Some(Ok(result))
}

struct Window {
    chunks: Range<usize>,
    old_to: usize,
    edited: Range<usize>,
    other: Range<usize>,
}

fn reuse(
    result: &mut Vec<Chunk>,
    chunks: &[Chunk],
    side: Side,
    old_base: usize,
    new_base: usize,
) -> Option<()> {
    let shift = |pos: usize| new_base.checked_add(pos.checked_sub(old_base)?);
    for mut chunk in chunks.iter().cloned() {
        let range = match side {
            Side::A => &mut chunk.a,
            Side::B => &mut chunk.b,
        };
        *range = shift(range.start)?..shift(range.end)?;
        push(result, chunk)?;
    }
    Some(())
}

fn sides<T>(side: Side, a: T, b: T) -> (T, T) {
    match side {
        Side::A => (a, b),
        Side::B => (b, a),
    }
}

fn ranges(chunk: &Chunk, side: Side) -> (&Range<usize>, &Range<usize>) {
    sides(side, &chunk.a, &chunk.b)
}

fn start_anchor(
    chunks: &[Chunk],
    side: Side,
    other: &str,
    pos: usize,
) -> Option<(usize, usize, usize)> {
    let index = chunks.partition_point(|chunk| ranges(chunk, side).0.end < pos);
    if let Some(chunk) = chunks.get(index) {
        let (edit, stable) = ranges(chunk, side);
        if edit.start <= pos {
            return Some((index, edit.start, stable.start));
        }
    }
    let other_pos = corresponding(chunks, side, index, pos)?;
    let anchor = other
        .as_bytes()
        .get(..other_pos)?
        .iter()
        .rposition(|&b| b == b'\n');
    let other_start = anchor.map_or(0, |pos| pos + 1);
    Some((
        index,
        pos.checked_sub(other_pos - other_start)?,
        other_start,
    ))
}

fn end_anchor(
    chunks: &[Chunk],
    side: Side,
    other: &str,
    pos: usize,
) -> Option<(usize, usize, usize)> {
    let index = chunks.partition_point(|chunk| ranges(chunk, side).0.start <= pos);
    if index > 0 {
        let (edit, stable) = ranges(&chunks[index - 1], side);
        if edit.end >= pos {
            return Some((index, edit.end, stable.end));
        }
    }
    let other_pos = corresponding(chunks, side, index, pos)?;
    let bytes = other.as_bytes();
    let after = bytes.get(other_pos..)?;
    let other_end = if other_pos == 0 || bytes[other_pos - 1] == b'\n' {
        other_pos
    } else {
        after
            .iter()
            .position(|&b| b == b'\n')
            .map_or(other.len(), |end| other_pos + end + 1)
    };
    Some((index, pos.checked_add(other_end - other_pos)?, other_end))
}

fn corresponding(chunks: &[Chunk], side: Side, index: usize, pos: usize) -> Option<usize> {
    if index == 0 {
        Some(pos)
    } else {
        let (edit, other) = ranges(&chunks[index - 1], side);
        other.end.checked_add(pos.checked_sub(edit.end)?)
    }
}

fn push(chunks: &mut Vec<Chunk>, chunk: Chunk) -> Option<()> {
    if let Some(previous) = chunks.last()
        && (previous.a.end > chunk.a.start || previous.b.end > chunk.b.start)
    {
        return None;
    }
    if let Some(previous) = chunks.last_mut()
        && near(previous, chunk.a.start, chunk.b.start)
    {
        let (off_a, off_b) = (
            chunk.a.start - previous.a.start,
            chunk.b.start - previous.b.start,
        );
        previous
            .changes
            .extend(chunk.changes.into_iter().map(|mut change| {
                change.a = offset(&change.a, off_a);
                change.b = offset(&change.b, off_b);
                change
            }));
        previous.a.end = chunk.a.end;
        previous.b.end = chunk.b.end;
        previous.precise &= chunk.precise;
    } else {
        chunks.push(chunk);
    }
    Some(())
}
