//! Span traversal and diff callbacks based on CodeMirror state 6.7.6.
//! See `docs/licenses/codemirror-state.txt` for the upstream MIT notice.

use std::cmp::Ordering;

use super::chunks::Edge;
use super::cursor::{Cursor, RankedRange};
use super::{RangeComparator, RangeValue, StoredRange};

pub(super) struct SpanCursor<'a, 's, T> {
    ranges: Cursor<'a, 's, T>,
    pub(super) active: Vec<RankedRange<'a, T>>,
    pub(super) point: Option<RankedRange<'a, T>>,
    pub(super) end: Option<Edge>,
}

impl<'a, 's, T: 'a> SpanCursor<'a, 's, T> {
    pub(super) fn new(ranges: Cursor<'a, 's, T>) -> Self {
        Self {
            ranges,
            active: Vec::new(),
            point: None,
            end: None,
        }
    }

    pub(super) fn goto(&mut self, pos: usize) {
        self.ranges.goto(pos);
        self.active.clear();
        // A previous diff gap can leave a point open. `next` uses it to
        // suppress nonempty points that end exactly at this gap's start.
        self.next(pos);
    }

    pub(super) fn next(&mut self, pos: usize) {
        let was_point = self.point.take().is_some();
        loop {
            let next = self.ranges.current();
            if let Some((i, &(_, active))) = self
                .active
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, entry))| entry.end())
                && next.is_none_or(|(_, entry)| active.end() < entry.start())
            {
                if active.range.to > pos {
                    self.end = Some(active.end());
                    return;
                }
                self.active.remove(i);
                continue;
            }
            let Some((rank, entry)) = next else {
                self.end = None;
                return;
            };
            if entry.range.from > pos {
                self.end = Some(entry.start());
                return;
            }
            self.ranges.next();
            if entry.point_size.is_none() {
                let index = self.active.partition_point(|&(other_rank, other)| {
                    (other_rank, other.range.to) < (rank, entry.range.to)
                });
                self.active.insert(index, (rank, entry));
            } else if !(was_point && entry.range.to == pos && entry.range.from < entry.range.to) {
                let end = entry.end();
                self.active.retain(|(_, active)| active.end() >= end);
                self.point = Some((rank, entry));
                self.end = Some(end);
                self.ranges.forward(end);
                return;
            }
        }
    }

    pub(super) fn point_entry(&self) -> Option<&'a StoredRange<T>> {
        self.point.map(|(_, entry)| entry)
    }

    pub(super) fn active_for_point(&self) -> impl Iterator<Item = RankedRange<'a, T>> + '_ {
        // The sweep already removed active ranges that end before the point.
        let rank = self.point.map_or(0, |(rank, _)| rank);
        self.active[self
            .active
            .partition_point(|(active_rank, _)| *active_rank < rank)..]
            .iter()
            .copied()
    }

    pub(super) fn open_start(&self, pos: usize) -> usize {
        self.active
            .iter()
            .rev()
            .take_while(|(_, entry)| entry.range.from < pos)
            .count()
    }

    pub(super) fn open_end(&self, pos: usize) -> usize {
        self.active
            .iter()
            .rev()
            .take_while(|(_, entry)| entry.range.to > pos)
            .count()
    }
}

pub(super) fn compare_gap<T: RangeValue>(
    a: &mut SpanCursor<'_, '_, T>,
    b: &mut SpanCursor<'_, '_, T>,
    start_a: usize,
    start_b: usize,
    length: usize,
    comparator: &mut impl RangeComparator<T>,
) {
    a.goto(start_a);
    b.goto(start_b);
    let mut pos = start_b;
    let mut bound_change = false;
    let (mut values_a, mut values_b) = (Vec::new(), Vec::new());
    loop {
        // Align the ends relative to their gap. No signed shift or sentinel is needed.
        let end_a = a.end.map(|(end, side)| (end - start_a, side));
        let end_b = b.end.map(|(end, side)| (end - start_b, side));
        let order = match (end_a, end_b) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        let end = if order.is_lt() { end_a } else { end_b }.map(|(end, _)| end);
        let clipped = start_b + end.unwrap_or(length).min(length);
        let points = (a.point_entry(), b.point_entry());
        if points.0.is_some() || points.1.is_some() {
            if !matches!(points, (Some(a), Some(b)) if same_value(a, b))
                || !same_values(a.active_for_point(), b.active_for_point())
            {
                comparator.compare_point(
                    pos,
                    clipped,
                    points.0.map(|entry| &entry.range.value),
                    points.1.map(|entry| &entry.range.value),
                );
            }
            bound_change = false;
        } else {
            if bound_change {
                comparator.bound_change(pos);
            }
            if clipped > pos && !same_values(a.active.iter().copied(), b.active.iter().copied()) {
                values_a.clear();
                values_b.clear();
                values_a.extend(a.active.iter().map(|(_, entry)| &entry.range.value));
                values_b.extend(b.active.iter().map(|(_, entry)| &entry.range.value));
                comparator.compare_range(pos, clipped, &values_a, &values_b);
            }
            // Upstream uses the new-document position for both open counts.
            bound_change = clipped < start_b + length
                && (end_a.map(|(end, _)| end) != end_b.map(|(end, _)| end)
                    || a.open_end(clipped) != b.open_end(clipped));
        }
        let Some(end) = end.filter(|&end| end <= length) else {
            return;
        };
        pos = start_b + end;
        if !order.is_gt() {
            a.next(start_a + end);
        }
        if !order.is_lt() {
            b.next(pos);
        }
    }
}

pub(super) fn same_value<T: RangeValue>(a: &StoredRange<T>, b: &StoredRange<T>) -> bool {
    a.id == b.id || a.range.value.eq(&b.range.value)
}

fn same_values<'a, T: RangeValue + 'a>(
    mut a: impl Iterator<Item = RankedRange<'a, T>>,
    mut b: impl Iterator<Item = RankedRange<'a, T>>,
) -> bool {
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some((_, a)), Some((_, b))) if same_value(a, b) => {}
            _ => return false,
        }
    }
}
