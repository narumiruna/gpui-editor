//! Range storage, span traversal, and comparison based on CodeMirror state 6.7.6 (`651084c`).

use std::{
    fmt,
    ops::RangeInclusive,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::{ChangeDesc, ChangeSet, EditError, MapMode};

mod chunks;
mod cursor;
mod spans;
use chunks::{Layer, distinct_sets, shared_chunks};
use cursor::Cursor;
use spans::{SpanCursor, compare_gap, same_value};

/// The behavior of a value stored in a [`RangeSet`].
///
/// The start side orders ranges that start at the same offset. A negative
/// side stays before an insertion at that edge. A zero or positive side
/// moves after it. Sides and the point flag must stay fixed while a value
/// is in a set.
pub trait RangeValue {
    /// Compare distinct values. The default uses reference identity. Values
    /// keep their stored identity when a range set maps, updates, or joins
    /// them. Use [`Arc`] to share a value across independently created ranges.
    /// Override this method to compare distinct values by their contents.
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }

    fn start_side(&self) -> i32 {
        0
    }

    fn end_side(&self) -> i32 {
        0
    }

    /// The deletion policy for a range whose start and end are equal.
    fn map_mode(&self) -> MapMode {
        MapMode::TrackDel
    }

    /// Points can be empty. A nonempty point is atomic during span traversal.
    fn is_point(&self) -> bool {
        false
    }

    fn range(self, from: usize, to: usize) -> Range<Self>
    where
        Self: Sized,
    {
        Range::new(from, to, self)
    }
}

impl<T: RangeValue + ?Sized> RangeValue for Arc<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(self, other) || self.as_ref().eq(other.as_ref())
    }

    fn start_side(&self) -> i32 {
        self.as_ref().start_side()
    }

    fn end_side(&self) -> i32 {
        self.as_ref().end_side()
    }

    fn map_mode(&self) -> MapMode {
        self.as_ref().map_mode()
    }

    fn is_point(&self) -> bool {
        self.as_ref().is_point()
    }
}

/// A value and its range in UTF-8 byte offsets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Range<T> {
    pub from: usize,
    pub to: usize,
    pub value: T,
}

impl<T> Range<T> {
    pub fn new(from: usize, to: usize, value: T) -> Self {
        Self { from, to, value }
    }
}

impl<T: RangeValue> Range<T> {
    fn start(&self) -> (usize, i32) {
        (self.from, self.value.start_side())
    }
}

/// An invalid range or input order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeSetError {
    ReversedRange,
    UnsortedRanges,
}

impl fmt::Display for RangeSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReversedRange => f.write_str("range end is before its start"),
            Self::UnsortedRanges => f.write_str("ranges must be sorted by start and start side"),
        }
    }
}

impl std::error::Error for RangeSetError {}

/// Adds ranges in order of start offset and start side.
#[derive(Debug)]
pub struct RangeSetBuilder<T> {
    ranges: Vec<Range<T>>,
}

impl<T: RangeValue> RangeSetBuilder<T> {
    pub const fn new() -> Self {
        Self { ranges: Vec::new() }
    }

    pub fn add(&mut self, from: usize, to: usize, value: T) -> Result<(), RangeSetError> {
        self.push(Range::new(from, to, value))
    }

    pub fn push(&mut self, range: Range<T>) -> Result<(), RangeSetError> {
        if range.from > range.to {
            return Err(RangeSetError::ReversedRange);
        }
        if self
            .ranges
            .last()
            .is_some_and(|last| last.start() > range.start())
        {
            return Err(RangeSetError::UnsortedRanges);
        }
        self.ranges.push(range);
        Ok(())
    }

    pub fn finish(self) -> RangeSet<T> {
        RangeSet::ordered(self.ranges)
    }
}

impl<T: RangeValue> Default for RangeSetBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

type RangeFilter<'a, T> = &'a mut dyn FnMut(&Range<T>) -> bool;

/// Add new ranges and filter existing ranges.
pub struct RangeSetUpdate<'a, T> {
    pub add: Vec<Range<T>>,
    /// Sort the added ranges instead of returning an error for unsorted input.
    pub sort: bool,
    /// Keep an existing range when this function returns `true`.
    pub filter: Option<RangeFilter<'a, T>>,
    /// Apply the filter only to ranges that touch these offsets.
    /// When absent, apply the filter to all existing ranges.
    pub filter_range: Option<RangeInclusive<usize>>,
}

impl<T> Default for RangeSetUpdate<'_, T> {
    fn default() -> Self {
        Self {
            add: Vec::new(),
            sort: false,
            filter: None,
            filter_range: None,
        }
    }
}

/// A collection of possibly overlapping ranges.
///
/// Range offsets are UTF-8 bytes. The caller supplies offsets for the document
/// of interest; the set does not check document length or character boundaries.
#[derive(Clone, Debug)]
pub struct RangeSet<T> {
    ranges: Vec<StoredRange<T>>,
    layers: Vec<Layer>,
}

// Cache the cursor keys so queries do not need a `T: RangeValue` bound.
#[derive(Clone, Debug)]
struct StoredRange<T> {
    range: Range<T>,
    id: usize,
    start_side: i32,
    end_side: i32,
    point_size: Option<usize>,
}

impl<T> StoredRange<T> {
    fn start(&self) -> (usize, i32) {
        (self.range.from, self.start_side)
    }

    fn end(&self) -> (usize, i32) {
        (self.range.to, self.end_side)
    }

    fn at(&self, from: usize, to: usize) -> Self
    where
        T: Clone,
    {
        Self {
            range: Range::new(from, to, self.range.value.clone()),
            point_size: self.point_size.map(|_| to - from),
            ..*self
        }
    }
}

fn store_ranges<T: RangeValue>(ranges: Vec<Range<T>>) -> Vec<StoredRange<T>> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let count = ranges.len();
    if count == 0 {
        return Vec::new();
    }
    let first = NEXT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
            id.checked_add(count)
        })
        .expect("range value identities exhausted");
    ranges
        .into_iter()
        .zip(first..first + count)
        .map(|(range, id)| StoredRange {
            id,
            start_side: range.value.start_side(),
            end_side: range.value.end_side(),
            point_size: range.value.is_point().then(|| range.to - range.from),
            range,
        })
        .collect()
}

impl<T> RangeSet<T> {
    pub const fn empty() -> Self {
        Self {
            ranges: Vec::new(),
            layers: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// The greatest end offset, or zero if the set is empty.
    pub fn end(&self) -> usize {
        self.ranges
            .iter()
            .map(|entry| entry.range.to)
            .max()
            .unwrap_or(0)
    }

    /// Iterate over ranges that end at or after `from`.
    ///
    /// Order is by start offset, start side, end offset, and end side.
    /// Exact ties follow the layer heap order, not the input order.
    pub fn iter(&self, from: usize) -> impl Iterator<Item = &Range<T>> {
        Cursor::new(&[self], None, None)
            .iter(from)
            .map(|(_, entry)| &entry.range)
    }

    /// Call `visit` for ranges that touch `from..=to`, in layer order.
    /// Stop when it returns `false`.
    pub fn between(&self, from: usize, to: usize, mut visit: impl FnMut(&Range<T>) -> bool) {
        if from <= to {
            for layer in &self.layers {
                for chunk in &layer.chunks {
                    if chunk.start > to || self.ranges[chunk.last()].range.to < from {
                        continue;
                    }
                    let entries = chunk.indices.iter().map(|&i| &self.ranges[i].range);
                    for range in entries
                        .skip_while(|range| range.to < from)
                        .take_while(|range| range.from <= to)
                    {
                        if !visit(range) {
                            return;
                        }
                    }
                }
            }
        }
    }
}

impl<T> Default for RangeSet<T> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<T: RangeValue> RangeSet<T> {
    /// Build from ranges sorted by start and start side, or sort them if `sort` is true.
    pub fn of(mut ranges: Vec<Range<T>>, sort: bool) -> Result<Self, RangeSetError> {
        check_order(&mut ranges, sort)?;
        Ok(Self::ordered(ranges))
    }

    /// Iterate over multiple sets in order. Each item includes its set index.
    pub fn iter_sets<'a>(
        sets: &[&'a Self],
        from: usize,
    ) -> impl Iterator<Item = (usize, &'a Range<T>)> + use<'a, T> {
        Cursor::new(sets, None, None)
            .iter(from)
            .map(|(rank, entry)| (rank, &entry.range))
    }

    /// Combine the ranges of multiple sets into a single set.
    pub fn join(sets: &[&Self]) -> Self
    where
        T: Clone,
    {
        Self::join_layers(sets)
    }

    /// Map the ranges from the old document into the changed document.
    ///
    /// A change that strictly covers a whole chunk drops all its ranges.
    /// A change that touches a chunk boundary maps its ranges individually.
    pub fn map(&self, changes: &ChangeSet) -> Self
    where
        T: Clone,
    {
        if changes.is_empty() || self.is_empty() {
            return self.clone();
        }
        self.map_chunks(changes)
    }

    /// Map ranges through a change description without using replacement text.
    /// Use this with the accepted changes in a [`ViewUpdate`](crate::view::ViewUpdate).
    /// Return `EditError::InvalidRange` if any old endpoint is beyond the
    /// description's document length. UTF-8 boundaries remain the caller's responsibility.
    pub fn map_desc(&self, changes: &ChangeDesc) -> Result<Self, EditError>
    where
        T: Clone,
    {
        if self.end() > changes.length() {
            return Err(EditError::InvalidRange);
        }
        if changes.is_empty() || self.is_empty() {
            return Ok(self.clone());
        }
        Ok(self.map_chunks(changes))
    }

    pub fn update(&self, update: RangeSetUpdate<'_, T>) -> Result<Self, RangeSetError>
    where
        T: Clone,
    {
        let mut update = update;
        check_order(&mut update.add, update.sort)?;
        Ok(self.update_layers(update))
    }

    /// Visit all of `from..to`, including spans with no active ranges.
    ///
    /// A point can be visited at either boundary. When `min_point_size` is
    /// present, skip all regular ranges and points smaller than that many bytes.
    /// Return the count of ranges and points still open at `to`.
    pub fn spans(
        sets: &[&Self],
        from: usize,
        to: usize,
        visitor: &mut impl SpanIterator<T>,
        min_point_size: Option<usize>,
    ) -> usize {
        if from > to {
            return 0;
        }
        let mut cursor = SpanCursor::new(Cursor::new(sets, None, min_point_size));
        cursor.goto(from);
        let mut pos = from;
        let mut open = cursor.open_start(from);
        let mut values = Vec::new();
        loop {
            let end = cursor.end.map(|(pos, _)| pos);
            let clipped = end.unwrap_or(to).min(to);
            values.clear();
            if let Some((index, point)) = cursor.point {
                values.extend(
                    cursor
                        .active_for_point()
                        .map(|(_, entry)| &entry.range.value),
                );
                let open_start = if point.range.from < from {
                    values.len() + 1
                } else if point.start_side < 0 {
                    values.len()
                } else {
                    values.len().min(open)
                };
                visitor.point(pos, clipped, &point.range.value, &values, open_start, index);
                open = cursor.open_end(clipped).min(values.len());
            } else if clipped > pos {
                values.extend(cursor.active.iter().map(|(_, entry)| &entry.range.value));
                visitor.span(pos, clipped, &values, open);
                open = cursor.open_end(clipped);
            }
            let Some(end) = end.filter(|&end| end <= to) else {
                return open + usize::from(cursor.point.is_some());
            };
            pos = end;
            cursor.next(pos);
        }
    }

    /// Report differences over the unchanged parts of `text_diff`.
    ///
    /// Reports use new-document UTF-8 byte offsets. When `min_point_size` is
    /// present, compare only points of at least that many bytes. As in
    /// upstream, the top layer determines which sets qualify, and aligned
    /// shared chunks without nonempty points are skipped.
    pub fn compare(
        old_sets: &[&Self],
        new_sets: &[&Self],
        text_diff: &ChangeDesc,
        comparator: &mut impl RangeComparator<T>,
        min_point_size: Option<usize>,
    ) {
        let eligible = |set: &&Self| {
            set.layers.first().is_some_and(|layer| {
                layer.max_point.is_some_and(|size| size > 0)
                    || min_point_size
                        .is_none_or(|min| layer.max_point.is_some_and(|size| size >= min))
            })
        };
        let old: Vec<_> = old_sets.iter().copied().filter(eligible).collect();
        let new: Vec<_> = new_sets.iter().copied().filter(eligible).collect();
        let shared = shared_chunks(&old, &new, Some(text_diff));
        let mut a = SpanCursor::new(Cursor::new(&old, Some(&shared), min_point_size));
        let mut b = SpanCursor::new(Cursor::new(&new, Some(&shared), min_point_size));
        let gaps = text_diff
            .gaps()
            .chain((text_diff.is_empty() && text_diff.length() == 0).then_some((0..0, 0..0)));
        for (old, new) in gaps {
            compare_gap(&mut a, &mut b, old.start, new.start, new.len(), comparator);
        }
    }

    /// Compare points from `from` through `to`, or without an upper limit if absent.
    ///
    /// As in upstream 6.7.6, this removes empty and common sets, checks the
    /// remaining set counts, and ignores regular ranges. It checks the next
    /// boundary even if it is past `to`, and only requires a matching point
    /// value when the old side has a point. This check can be asymmetric.
    /// Aligned shared chunks without nonempty points are skipped.
    pub fn eq(old_sets: &[&Self], new_sets: &[&Self], from: usize, to: Option<usize>) -> bool {
        let old = distinct_sets(old_sets, new_sets);
        let new = distinct_sets(new_sets, old_sets);
        if old.len() != new.len() {
            return false;
        }
        let shared = shared_chunks(&old, &new, None);
        let mut a = SpanCursor::new(Cursor::new(&old, Some(&shared), Some(0)));
        let mut b = SpanCursor::new(Cursor::new(&new, Some(&shared), Some(0)));
        a.goto(from);
        b.goto(from);
        loop {
            let end_a = a.end.map(|(end, _)| end);
            let end_b = b.end.map(|(end, _)| end);
            if end_a != end_b
                || a.point_entry()
                    .is_some_and(|a| b.point_entry().is_none_or(|b| !same_value(a, b)))
            {
                return false;
            }
            let Some(end) = end_a.filter(|&end| to.is_none_or(|to| end <= to)) else {
                return true;
            };
            a.next(end);
            b.next(end);
        }
    }
}

fn check_order<T: RangeValue>(ranges: &mut [Range<T>], sort: bool) -> Result<(), RangeSetError> {
    if ranges.iter().any(|range| range.from > range.to) {
        return Err(RangeSetError::ReversedRange);
    }
    if sort {
        ranges.sort_by_key(Range::start);
    } else if ranges
        .windows(2)
        .any(|pair| pair[0].start() > pair[1].start())
    {
        return Err(RangeSetError::UnsortedRanges);
    }
    Ok(())
}

/// Receives regular spans and atomic points in document order.
pub trait SpanIterator<T> {
    /// `active` is ordered by set index, then by end offset within that set.
    /// `open_start` counts ranges that continue into the start of the span.
    fn span(&mut self, from: usize, to: usize, active: &[&T], open_start: usize);

    /// `active` contains regular ranges of equal or greater set index that
    /// cover the point. If the point started before the query range,
    /// `open_start` is `active.len() + 1`. `index` is the point's set index.
    fn point(
        &mut self,
        from: usize,
        to: usize,
        value: &T,
        active: &[&T],
        open_start: usize,
        index: usize,
    );
}

/// Receives differences between two groups of range sets.
pub trait RangeComparator<T> {
    /// The active regular ranges differ over this span.
    fn compare_range(&mut self, from: usize, to: usize, old: &[&T], new: &[&T]);

    /// A point or the regular ranges that cover it differ. The span can be empty.
    fn compare_point(&mut self, from: usize, to: usize, old: Option<&T>, new: Option<&T>);

    /// The boundary of a regular span changed, even if its active values did not.
    fn bound_change(&mut self, _pos: usize) {}
}
