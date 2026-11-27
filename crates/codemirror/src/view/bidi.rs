//! Bidirectional runs and visual cursor motion from CodeMirror view 6.44.0.
//! See `docs/licenses/codemirror-view.txt`. Positions are UTF-8 byte offsets.

mod cluster;
mod order;

use std::sync::{Arc, LazyLock};

use super::MarkDecoration;
use crate::{
    Assoc, EditError, EditorSelection, EditorState, Facet, Line, RangeSet, SelectionRange,
    SpanIterator, Text,
};

/// The base direction of a line, or the direction of a bidi span or isolate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Ltr,
    Rtl,
}

impl Direction {
    fn from_level(level: usize) -> Self {
        if level.is_multiple_of(2) {
            Self::Ltr
        } else {
            Self::Rtl
        }
    }
}

/// A contiguous directional run. Offsets are relative to the start of the line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BidiSpan {
    pub from: usize,
    pub to: usize,
    pub level: usize,
}

impl BidiSpan {
    /// Even levels are left to right. Odd levels are right to left.
    pub fn direction(self) -> Direction {
        Direction::from_level(self.level)
    }

    /// Return the visual start (`end = false`) or end in the line's base direction.
    pub fn side(self, end: bool, direction: Direction) -> usize {
        if self.forward(end, direction) {
            self.to
        } else {
            self.from
        }
    }

    /// Return whether this visual motion increases the logical position in the run.
    pub fn forward(self, forward: bool, direction: Direction) -> bool {
        forward == (self.direction() == direction)
    }

    /// Find the run at a line-relative position, including both run edges.
    /// Prefer an exact level, then the associated side or (with no association)
    /// the lowest level.
    pub fn find(
        order: &[Self],
        index: usize,
        level: Option<usize>,
        assoc: Option<Assoc>,
    ) -> Option<usize> {
        let mut candidate: Option<usize> = None;
        for (i, span) in order.iter().enumerate() {
            if span.from <= index && index <= span.to {
                if level == Some(span.level) {
                    return Some(i);
                }
                if candidate.is_none_or(|previous| match assoc {
                    Some(Assoc::Before) => span.from < index,
                    Some(Assoc::After) => span.to > index,
                    None => order[previous].level > span.level,
                }) {
                    candidate = Some(i);
                }
            }
        }
        candidate
    }
}

/// An explicit directional isolate with line-relative endpoints.
/// `None` uses the first strong character outside child isolates, or LTR if none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BidiIsolate {
    pub from: usize,
    pub to: usize,
    pub direction: Option<Direction>,
    pub inner: Vec<Self>,
}

type MarkSet = Arc<RangeSet<MarkDecoration>>;
const MAX_ISOLATE_DEPTH: usize = 64;

/// Decoration sources in precedence order. In this facet, a mark without an
/// explicit direction uses the first strong character in its isolated text.
pub static BIDI_ISOLATED_RANGES: LazyLock<Facet<MarkSet, Vec<MarkSet>>> =
    LazyLock::new(Facet::collect);

/// Read configured marks for one logical line. Clip cross-line marks to the
/// line and split crossing marks into nested isolates in layer order.
pub fn line_isolates(
    state: &EditorState,
    line_number: usize,
) -> Result<Vec<BidiIsolate>, EditError> {
    let line = state
        .text()
        .line(line_number)
        .ok_or(EditError::InvalidRange)?;
    let sources = state.context().facet(&BIDI_ISOLATED_RANGES)?;
    let sets: Vec<_> = sources.iter().map(Arc::as_ref).collect();
    let mut visitor = Isolates {
        start: line.range.start,
        result: Vec::new(),
        valid: true,
        text: line.text,
    };
    RangeSet::spans(&sets, line.range.start, line.range.end, &mut visitor, None);
    if !visitor.valid {
        return Err(EditError::InvalidRange);
    }
    Ok(visitor.result)
}

struct Isolates<'a> {
    start: usize,
    text: &'a str,
    result: Vec<BidiIsolate>,
    valid: bool,
}

impl SpanIterator<MarkDecoration> for Isolates<'_> {
    fn span(&mut self, from: usize, to: usize, active: &[&MarkDecoration], mut open: usize) {
        if active.is_empty() || !self.valid {
            return;
        }
        if active.len() > MAX_ISOLATE_DEPTH {
            self.valid = false;
            return;
        }
        let from = from - self.start;
        let to = to - self.start;
        if !self.text.is_char_boundary(from) || !self.text.is_char_boundary(to) {
            self.valid = false;
            return;
        }
        let mut level = &mut self.result;
        for mark in active.iter().rev() {
            let direction = mark.bidi_direction();
            let continues = open != 0
                && level
                    .last()
                    .is_some_and(|prev| prev.to == from && prev.direction == direction);
            if continues {
                level.last_mut().unwrap().to = to;
            } else {
                level.push(BidiIsolate {
                    from,
                    to,
                    direction,
                    inner: Vec::new(),
                });
            }
            level = &mut level.last_mut().unwrap().inner;
            open = open.saturating_sub(1);
        }
    }

    fn point(
        &mut self,
        _: usize,
        _: usize,
        _: &MarkDecoration,
        _: &[&MarkDecoration],
        _: usize,
        _: usize,
    ) {
    }
}

/// Get the visual order of a line. LTR lines list the leftmost span first;
/// RTL lines list the rightmost span first.
///
/// Each isolate must be nonempty, within its parent, and on UTF-8 character
/// boundaries. Siblings must be sorted and must not overlap. Invalid isolates
/// return [`EditError::InvalidRange`]. A line can have up to 64 isolate levels.
/// As in CodeMirror's `EditorView`, lines
/// longer than 4096 UTF-16 code units return one LTR span without reading isolates.
pub fn bidi_spans(
    line: &str,
    direction: Direction,
    isolates: &[BidiIsolate],
) -> Result<Vec<BidiSpan>, EditError> {
    if line.encode_utf16().take(4097).count() > 4096 {
        return Ok(vec![BidiSpan {
            from: 0,
            to: line.len(),
            level: 0,
        }]);
    }
    order::compute(line, direction, isolates)
}

/// A single visual move. The selection is document-relative; `moved_over`
/// contains the logical text crossed inside one directional run.
#[derive(Debug)]
pub struct VisualMove<'a> {
    pub selection: SelectionRange,
    pub moved_over: &'a str,
}

/// Move by one cluster in the visual order of a line, without crossing lines.
/// `forward` follows the base direction, not necessarily left to right.
/// The order must come from [`bidi_spans`] for this line and base direction.
/// Returns `None` at the end of the traversal or for an invalid start position.
pub fn move_visually<'a>(
    line: &Line<'a>,
    order: &[BidiSpan],
    direction: Direction,
    start: SelectionRange,
    forward: bool,
) -> Option<VisualMove<'a>> {
    if line.text.is_empty() {
        return None;
    }
    let mut index = start.head().checked_sub(line.range.start)?;
    if !line.text.is_char_boundary(index) {
        return None;
    }
    let mut span_index = match (index, start.assoc()) {
        (0, Some(Assoc::Before)) => {
            if !forward {
                return None;
            }
            index = order.first()?.side(false, direction);
            0
        }
        (i, Some(Assoc::After)) if i == line.text.len() => {
            if forward {
                return None;
            }
            index = order.last()?.side(true, direction);
            order.len() - 1
        }
        _ => BidiSpan::find(
            order,
            index,
            start.bidi_level().map(usize::from),
            start.assoc(),
        )?,
    };
    let adjacent = |i: usize| {
        if forward {
            (i + 1 < order.len()).then_some(i + 1)
        } else {
            i.checked_sub(1)
        }
    };
    let mut span = order[span_index];
    let mut span_end = span.side(forward, direction);
    if index == span_end {
        span_index = adjacent(span_index)?;
        span = order[span_index];
        index = span.side(!forward, direction);
        span_end = span.side(forward, direction);
    }
    let logical_forward = span.forward(forward, direction);
    let mut next = cluster::find_break(line.text, index, logical_forward)?;
    if next < span.from || next > span.to {
        next = span_end;
    }
    let moved_over = line.text.get(index.min(next)..index.max(next))?;
    let make_cursor = |index, assoc, level: usize| {
        EditorSelection::cursor(
            line.range.start + index,
            Some(assoc),
            Some(level.min(6) as u8),
            None,
        )
    };
    let selection = if next == span_end {
        if let Some(after) = adjacent(span_index).map(|i| order[i]) {
            if after.level + usize::from(!forward) < span.level {
                make_cursor(
                    after.side(!forward, direction),
                    if after.forward(forward, direction) {
                        Assoc::After
                    } else {
                        Assoc::Before
                    },
                    after.level,
                )
            } else {
                make_cursor(next, approached_side(logical_forward), span.level)
            }
        } else if forward {
            EditorSelection::cursor(line.range.end, Some(Assoc::After), None, None)
        } else {
            EditorSelection::cursor(line.range.start, Some(Assoc::Before), None, None)
        }
    } else {
        make_cursor(next, approached_side(logical_forward), span.level)
    };
    Some(VisualMove {
        selection,
        moved_over,
    })
}

/// Move by one visual cluster, or by a group of clusters when `accept` is set.
/// The first cluster always moves the cursor. After that, stop before the
/// first cluster that `accept` rejects. A logical line break is one `"\n"`
/// cluster, even when the source has CRLF. Logical line breaks do not depend
/// on wrapping. `line_order` supplies the base direction and bidi spans for
/// each visited line. A group retains the direction of the starting line.
/// Atomic ranges are the caller's responsibility.
pub fn move_by_char(
    text: &Text,
    start: SelectionRange,
    forward: bool,
    mut line_order: impl FnMut(&Line<'_>) -> Result<(Direction, Vec<BidiSpan>), EditError>,
    mut accept: Option<&mut dyn FnMut(&str) -> bool>,
) -> Result<SelectionRange, EditError> {
    let mut line = text
        .line_at(start.head())
        .ok_or(EditError::InvalidSelection)?;
    let (direction, mut spans) = line_order(&line)?;
    let mut cursor = if start.head() <= line.range.end {
        start
    } else {
        EditorSelection::cursor(line.range.end, start.assoc(), start.bidi_level(), None)
    };
    let mut first = true;
    loop {
        let (next, crossed) =
            if let Some(step) = move_visually(&line, &spans, direction, cursor, forward) {
                (step.selection, step.moved_over)
            } else {
                let adjacent = if forward {
                    line.number + 1
                } else {
                    line.number - 1
                };
                let Some(other) = text.line(adjacent) else {
                    return Ok(cursor);
                };
                line = other;
                (_, spans) = line_order(&line)?;
                let (head, assoc) = if forward {
                    (line.range.start, Assoc::Before)
                } else {
                    (line.range.end, Assoc::After)
                };
                (EditorSelection::cursor(head, Some(assoc), None, None), "\n")
            };
        let Some(accept) = &mut accept else {
            return Ok(next);
        };
        if !accept(crossed) && !first {
            return Ok(cursor);
        }
        cursor = next;
        first = false;
    }
}

fn approached_side(forward: bool) -> Assoc {
    if forward { Assoc::Before } else { Assoc::After }
}
