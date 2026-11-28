//! Borrow logical lines and line endings without exposing the tree's leaf boundaries.

use std::{borrow::Cow, iter::FusedIterator, ops::Range};

use super::{Text, tree::Chunk};
use crate::EditError;

struct Span {
    range: Range<usize>,
    ending: Option<Range<usize>>,
}

/// Cache the last leaf so successive queries within it do not traverse the tree.
struct LeafCursor<'a> {
    text: &'a Text,
    chunk: Option<Chunk<'a>>,
}

impl<'a> LeafCursor<'a> {
    fn new(text: &'a Text) -> Self {
        Self { text, chunk: None }
    }

    fn chunk_at(&mut self, position: usize) -> Option<Chunk<'a>> {
        if let Some(chunk) = self.chunk
            && (chunk.start..chunk.end()).contains(&position)
        {
            return Some(chunk);
        }
        self.chunk = self.text.0.tree.chunk_at(position);
        self.chunk
    }

    fn byte(&mut self, position: usize) -> Option<u8> {
        Some(self.chunk_at(position)?.byte(position))
    }

    fn next_break(&mut self, position: usize) -> Option<usize> {
        let chunk = self.chunk_at(position)?;
        let index = chunk.breaks_before(position);
        chunk
            .break_at(index)
            .or_else(|| self.text.0.tree.break_at(index))
    }

    fn previous_break(&mut self, position: usize) -> Option<usize> {
        let chunk = self.chunk_at(position.checked_sub(1)?)?;
        let index = chunk.breaks_before(position).checked_sub(1)?;
        chunk
            .break_at(index)
            .or_else(|| self.text.0.tree.break_at(index))
    }

    fn ending_start(&mut self, lf: usize) -> usize {
        lf - usize::from(lf.checked_sub(1).and_then(|cr| self.byte(cr)) == Some(b'\r'))
    }

    fn next_span(&mut self, range: Range<usize>) -> Option<Span> {
        if range.is_empty() {
            return None;
        }
        let (end, ending) = match self.next_break(range.start) {
            Some(lf) => {
                let cr_or_lf = self.ending_start(lf);
                if range.start < cr_or_lf {
                    (cr_or_lf, None)
                } else {
                    (lf + 1, Some(cr_or_lf..lf + 1))
                }
            }
            None => (range.end, None),
        };
        Some(Span {
            range: range.start..end.min(range.end),
            ending,
        })
    }

    fn previous_span(&mut self, range: Range<usize>) -> Option<Span> {
        if range.is_empty() {
            return None;
        }
        let end = range.end;
        let (start, ending) = if self.byte(end) == Some(b'\n') && self.ending_start(end) < end {
            // The range stops between the CR and LF.
            (end - 1, Some(end - 1..end + 1))
        } else {
            match self.previous_break(end) {
                Some(lf) if lf + 1 == end => {
                    let start = self.ending_start(lf);
                    (start, Some(start..end))
                }
                Some(lf) => (lf + 1, None),
                None => (0, None),
            }
        };
        Some(Span {
            range: start.max(range.start)..end,
            ending,
        })
    }

    fn chunk_slice(&mut self, range: Range<usize>) -> Option<&'a str> {
        if range.is_empty() {
            Some("")
        } else {
            self.chunk_at(range.start)?.slice(range)
        }
    }

    fn borrow(&mut self, range: Range<usize>) -> &'a str {
        self.chunk_slice(range.clone())
            .unwrap_or_else(|| &self.text.as_str()[range])
    }
}

pub(super) fn slice_with_line_separator(
    text: &Text,
    mut range: Range<usize>,
    separator: &str,
) -> String {
    let mut cursor = LeafCursor::new(text);
    let mut result = String::with_capacity(range.len());
    while let Some(span) = cursor.next_span(range.clone()) {
        range.start = span.range.end;
        if span.ending.as_ref() == Some(&span.range) {
            result.push_str(separator);
        } else if let Some(slice) = cursor.chunk_slice(span.range.clone()) {
            result.push_str(slice);
        } else {
            for chunk in text.0.tree.chunks(span.range) {
                result.push_str(chunk);
            }
        }
    }
    result
}

/// A directional cursor over nonempty line content and line endings.
///
/// Use [`Text::cursor`] or [`Text::cursor_range`] to start a cursor. Call
/// [`Iterator::next`] to advance without skipping.
pub struct TextCursor<'a> {
    leaf: LeafCursor<'a>,
    bounds: Range<usize>,
    position: usize,
    reverse: bool,
    value: &'a str,
    done: bool,
    line_break: bool,
}

impl<'a> TextCursor<'a> {
    pub(super) fn new(text: &'a Text, from: usize, to: usize) -> Self {
        Self {
            leaf: LeafCursor::new(text),
            bounds: from.min(to)..from.max(to),
            position: from,
            reverse: from > to,
            value: "",
            done: false,
            line_break: false,
        }
    }

    /// The last chunk, or `""` before the first step or after completion.
    pub fn value(&self) -> &'a str {
        self.value
    }

    /// Whether the last step returned no chunk.
    pub fn done(&self) -> bool {
        self.done
    }

    /// Whether the last chunk is all or part of a line ending.
    pub fn line_break(&self) -> bool {
        self.line_break
    }

    /// Skip bytes, then return the next chunk in the cursor's direction.
    /// Count from the cursor's start or from the far edge of its last chunk.
    /// A negative skip moves against the direction and can resume a completed
    /// cursor, including a range cursor. Skips stop at the cursor's bounds.
    /// Unlike CodeMirror, a skip cannot split a Unicode scalar value. It can
    /// split CRLF.
    ///
    /// # Errors
    ///
    /// Return [`EditError::InvalidRange`] without changing the cursor if the
    /// resulting position is not a UTF-8 boundary.
    pub fn next_with_skip(&mut self, skip: isize) -> Result<Option<&'a str>, EditError> {
        let distance = skip.unsigned_abs();
        let position = if (skip < 0) == self.reverse {
            self.position.saturating_add(distance).min(self.bounds.end)
        } else {
            self.position
                .saturating_sub(distance)
                .max(self.bounds.start)
        };
        if skip != 0 && !self.leaf.text.is_char_boundary(position) {
            return Err(EditError::InvalidRange);
        }
        Ok(self.advance(position))
    }

    fn advance(&mut self, position: usize) -> Option<&'a str> {
        let span = if self.reverse {
            self.leaf.previous_span(self.bounds.start..position)
        } else {
            self.leaf.next_span(position..self.bounds.end)
        };
        let Some(span) = span else {
            self.position = position;
            self.value = "";
            self.done = true;
            self.line_break = false;
            return None;
        };
        self.position = if self.reverse {
            span.range.start
        } else {
            span.range.end
        };
        self.value = self.leaf.borrow(span.range);
        self.done = false;
        self.line_break = span.ending.is_some();
        Some(self.value)
    }
}

impl<'a> Iterator for TextCursor<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        self.advance(self.position)
    }
}

/// A forward cursor over logical line content, without line endings.
///
/// Use [`Text::line_cursor`] or [`Text::line_cursor_range`]. Empty lines
/// yield `""`. A line within one tree leaf is borrowed. Only a line that
/// spans leaves is copied.
pub struct TextLineCursor<'a> {
    leaf: LeafCursor<'a>,
    bounds: Option<Range<usize>>,
    position: usize,
    state: LineCursorState,
}

#[derive(PartialEq, Eq)]
enum LineCursorState {
    AfterBreak,
    AfterLine,
    Done,
}

impl<'a> TextLineCursor<'a> {
    pub(super) fn new(text: &'a Text, bounds: Option<Range<usize>>) -> Self {
        Self {
            leaf: LeafCursor::new(text),
            position: bounds.as_ref().map_or(0, |range| range.start),
            bounds,
            state: LineCursorState::AfterBreak,
        }
    }

    /// Whether the cursor has returned `None`.
    pub fn done(&self) -> bool {
        self.state == LineCursorState::Done
    }

    /// Skip bytes, then return the next line fragment. Count from the start of
    /// the cursor's range or the end of the last content or ending it read.
    /// Negative skips rewind. Skips stop at the bounds and count both bytes of
    /// CRLF. Landing on either byte of CRLF selects that line ending. A ranged
    /// cursor cannot rewind from its upper bound. Once the cursor returns
    /// `None`, skips cannot restart it.
    ///
    /// # Errors
    ///
    /// Return [`EditError::InvalidRange`] without changing the cursor if the
    /// skip lands inside a UTF-8 scalar value.
    pub fn next_with_skip(&mut self, skip: isize) -> Result<Option<Cow<'a, str>>, EditError> {
        if self.done() {
            return Ok(None);
        }
        let bounds = self.bounds.clone().unwrap_or(0..self.leaf.text.len());
        if self.bounds.is_some() && self.position == bounds.end {
            return Ok(self.next());
        }
        let position = self
            .position
            .saturating_add_signed(skip)
            .clamp(bounds.start, bounds.end);
        if position != self.position && !self.leaf.text.is_char_boundary(position) {
            return Err(EditError::InvalidRange);
        }
        self.position = position;
        Ok(self.next())
    }
}

impl<'a> Iterator for TextLineCursor<'a> {
    type Item = Cow<'a, str>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done() {
            return None;
        }
        let end = self
            .bounds
            .as_ref()
            .map_or(self.leaf.text.len(), |range| range.end);
        while let Some(span) = self.leaf.next_span(self.position..end) {
            self.position = span.range.end;
            if span.ending.is_none() {
                self.state = LineCursorState::AfterLine;
                if let Some(line) = self.leaf.chunk_slice(span.range.clone()) {
                    return Some(Cow::Borrowed(line));
                }
                // Do not use `LeafCursor::borrow`, which can flatten the snapshot.
                let mut line = String::with_capacity(span.range.len());
                for chunk in self.leaf.text.0.tree.chunks(span.range) {
                    line.push_str(chunk);
                }
                return Some(Cow::Owned(line));
            }
            if self.state == LineCursorState::AfterBreak {
                return Some(Cow::Borrowed(""));
            }
            self.state = LineCursorState::AfterBreak;
        }
        if self.state == LineCursorState::AfterBreak {
            self.state = LineCursorState::AfterLine;
            Some(Cow::Borrowed(""))
        } else {
            self.state = LineCursorState::Done;
            None
        }
    }
}

impl FusedIterator for TextLineCursor<'_> {}

pub(super) struct Chunks<'a> {
    front: LeafCursor<'a>,
    back: LeafCursor<'a>,
    range: Range<usize>,
}

impl<'a> Chunks<'a> {
    pub(super) fn new(text: &'a Text, range: Range<usize>) -> Self {
        Self {
            front: LeafCursor::new(text),
            back: LeafCursor::new(text),
            range,
        }
    }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let span = self.front.next_span(self.range.clone())?;
        self.range.start = span.range.end;
        Some(self.front.borrow(span.range))
    }
}

impl DoubleEndedIterator for Chunks<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let span = self.back.previous_span(self.range.clone())?;
        self.range.end = span.range.start;
        Some(self.back.borrow(span.range))
    }
}

impl FusedIterator for Chunks<'_> {}

pub(super) struct Lines<'a> {
    front: LeafCursor<'a>,
    back: LeafCursor<'a>,
    start: usize,
    end: usize,
    remaining: usize,
}

impl<'a> Lines<'a> {
    pub(super) fn new(text: &'a Text, lines: Range<usize>) -> Self {
        let break_before = |number: usize| {
            number
                .checked_sub(2)
                .and_then(|index| text.0.tree.break_at(index))
        };
        Self {
            front: LeafCursor::new(text),
            back: LeafCursor::new(text),
            start: break_before(lines.start).map_or(0, |lf| lf + 1),
            end: break_before(lines.end).unwrap_or(text.len()),
            remaining: lines.len(),
        }
    }
}

impl<'a> Iterator for Lines<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        self.remaining = self.remaining.checked_sub(1)?;
        let start = self.start;
        let end = match self.front.next_break(start) {
            Some(lf) => {
                self.start = lf + 1;
                self.front.ending_start(lf)
            }
            None => self.end,
        };
        Some(self.front.borrow(start..end))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl DoubleEndedIterator for Lines<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.remaining = self.remaining.checked_sub(1)?;
        let end = if self.end == self.back.text.len() {
            self.end
        } else {
            self.back.ending_start(self.end)
        };
        let start = self.back.previous_break(self.end).map_or(0, |lf| {
            self.end = lf;
            lf + 1
        });
        Some(self.back.borrow(start..end))
    }
}

impl ExactSizeIterator for Lines<'_> {}
impl FusedIterator for Lines<'_> {}
