use std::ops::Range;

use crate::{Assoc, ChangeDesc, ChangeSet, EditError, MapMode};

/// A selection with an anchor and a movable head, in UTF-8 bytes.
///
/// Equality compares the anchor, head, and goal column. Use
/// [`Self::eq_with_assoc`] to also compare cursor association.
#[derive(Clone, Copy, Debug)]
pub struct SelectionRange {
    anchor: usize,
    head: usize,
    assoc: Option<Assoc>,
    bidi_level: Option<u8>,
    goal_column: Option<f64>,
    undirectional: bool,
}

impl SelectionRange {
    pub fn new(anchor: usize, head: usize) -> Self {
        EditorSelection::range(anchor, head, None, None, None)
    }

    pub fn anchor(self) -> usize {
        self.anchor
    }

    pub fn head(self) -> usize {
        self.head
    }

    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }

    /// The side of the head that the range is associated with, if set.
    pub fn assoc(self) -> Option<Assoc> {
        self.assoc
    }

    /// Whether selection-extending commands can exchange the anchor and head.
    pub fn undirectional(self) -> bool {
        self.undirectional
    }

    /// The bidirectional text level at the head, if set.
    pub fn bidi_level(self) -> Option<u8> {
        self.bidi_level
    }

    /// The retained horizontal coordinate in view units, if set.
    pub fn goal_column(self) -> Option<f64> {
        self.goal_column
    }

    /// Like `==`, but compare the association of cursors too.
    pub fn eq_with_assoc(&self, other: &Self) -> bool {
        self == other && (!self.is_empty() || self.assoc == other.assoc)
    }

    /// Extend from the anchor to the farther endpoint, or use both endpoints
    /// if they surround the anchor. Do not retain the old range's metadata.
    pub fn extend(self, from: usize, to: usize, assoc: Option<Assoc>) -> Self {
        let (anchor, head) = if from <= self.anchor && to >= self.anchor {
            (from, to)
        } else if from.abs_diff(self.anchor) > to.abs_diff(self.anchor) {
            (self.anchor, from)
        } else {
            (self.anchor, to)
        };
        EditorSelection::range(anchor, head, None, None, assoc)
    }

    /// Map the range through a length-bound description with `Assoc::Before`
    /// for cursors, regardless of their stored association.
    pub fn map(self, changes: &ChangeDesc) -> Result<Self, EditError> {
        self.map_with_assoc(changes, Assoc::Before)
    }

    /// Map the range and retain its metadata. `assoc` only affects cursors.
    /// Nonempty ranges map their endpoints inward. If the mapped endpoints
    /// cross, the anchor and head change order and [`Self::range`] stays ordered.
    pub fn map_with_assoc(self, changes: &ChangeDesc, assoc: Assoc) -> Result<Self, EditError> {
        self.map_positions(assoc, |pos, side| {
            changes.map_pos(pos, side, MapMode::Simple)
        })
        .ok_or(EditError::InvalidSelection)
    }

    fn map_positions(
        mut self,
        assoc: Assoc,
        f: impl Fn(usize, Assoc) -> Option<usize>,
    ) -> Option<Self> {
        let range = self.range();
        if self.is_empty() {
            let pos = f(range.start, assoc)?;
            self.anchor = pos;
            self.head = pos;
        } else {
            let start = f(range.start, Assoc::After)?;
            let end = f(range.end, Assoc::Before)?;
            (self.anchor, self.head) = if self.head < self.anchor {
                (end, start)
            } else {
                (start, end)
            };
        }
        Some(self)
    }
}

impl PartialEq for SelectionRange {
    /// Compare the anchor, head, and goal column. Ignore the other metadata.
    fn eq(&self, other: &Self) -> bool {
        self.anchor == other.anchor
            && self.head == other.head
            && self.goal_column == other.goal_column
    }
}

// A range only stores finite goal columns.
impl Eq for SelectionRange {}

/// A nonempty set of ordered, normalized selection ranges and one primary range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorSelection {
    ranges: Vec<SelectionRange>,
    main: usize,
}

impl EditorSelection {
    pub fn single(anchor: usize, head: usize) -> Self {
        Self {
            ranges: vec![SelectionRange::new(anchor, head)],
            main: 0,
        }
    }

    /// Make a cursor with optional association, bidi level, and goal column.
    /// Bidi levels above six are set to six. Non-finite columns are not stored.
    pub fn cursor(
        pos: usize,
        assoc: Option<Assoc>,
        bidi_level: Option<u8>,
        goal_column: Option<f64>,
    ) -> SelectionRange {
        Self::range(pos, pos, goal_column, bidi_level, assoc)
    }

    /// Make a directed range. A nonempty range defaults to the side of the
    /// head toward its anchor. Bidi levels above six are set to six, and
    /// non-finite goal columns are treated as absent.
    pub fn range(
        anchor: usize,
        head: usize,
        goal_column: Option<f64>,
        bidi_level: Option<u8>,
        assoc: Option<Assoc>,
    ) -> SelectionRange {
        SelectionRange {
            anchor,
            head,
            assoc: assoc.or(if head < anchor {
                Some(Assoc::After)
            } else if head > anchor {
                Some(Assoc::Before)
            } else {
                None
            }),
            bidi_level: bidi_level.map(|level| level.min(6)),
            goal_column: goal_column.filter(|column| column.is_finite()),
            undirectional: false,
        }
    }

    /// Make a range with no fixed direction and bidi level zero.
    pub fn undirectional_range(from: usize, to: usize) -> SelectionRange {
        SelectionRange {
            anchor: from.min(to),
            head: from.max(to),
            assoc: None,
            bidi_level: Some(0),
            goal_column: None,
            undirectional: true,
        }
    }

    pub fn new(ranges: Vec<SelectionRange>, main: usize) -> Result<Self, EditError> {
        if main >= ranges.len() {
            return Err(EditError::InvalidSelection);
        }
        Ok(Self::normalize(ranges, main))
    }

    pub fn main(&self) -> SelectionRange {
        self.ranges[self.main]
    }

    pub fn main_index(&self) -> usize {
        self.main
    }

    pub fn ranges(&self) -> &[SelectionRange] {
        &self.ranges
    }

    /// Compare the ranges and main index, including the association of cursors.
    pub fn eq_with_assoc(&self, other: &Self) -> bool {
        self.main == other.main
            && self.ranges.len() == other.ranges.len()
            && self
                .ranges
                .iter()
                .zip(&other.ranges)
                .all(|(a, b)| a.eq_with_assoc(b))
    }

    /// Keep only the main range, including its metadata.
    pub fn as_single(&self) -> Self {
        Self {
            ranges: vec![self.main()],
            main: 0,
        }
    }

    /// Add a range and optionally make it the main range. Normalize overlaps.
    pub fn add_range(&self, range: SelectionRange, main: bool) -> Self {
        let ranges = std::iter::once(range)
            .chain(self.ranges.iter().copied())
            .collect();
        Self::normalize(ranges, if main { 0 } else { self.main + 1 })
    }

    /// Replace the range at `index` and normalize overlaps.
    pub fn replace_range(&self, index: usize, range: SelectionRange) -> Result<Self, EditError> {
        let mut ranges = self.ranges.clone();
        *ranges.get_mut(index).ok_or(EditError::InvalidSelection)? = range;
        Ok(Self::normalize(ranges, self.main))
    }

    fn normalize(ranges: Vec<SelectionRange>, main: usize) -> Self {
        if ranges.len() == 1 {
            return Self { ranges, main };
        }
        let mut indexed: Vec<_> = ranges.into_iter().enumerate().collect();
        indexed.sort_by_key(|(_, range)| range.range().start);
        let mut ranges: Vec<SelectionRange> = Vec::with_capacity(indexed.len());
        let mut primary = 0;
        for (index, next) in indexed {
            let range = next.range();
            if let Some(last) = ranges.last_mut() {
                let previous = last.range();
                if range.start < previous.end || (range.is_empty() && range.start == previous.end) {
                    // A merged span takes the later range's direction but not its metadata.
                    let start = previous.start;
                    let end = previous.end.max(range.end);
                    *last = if next.head < next.anchor {
                        SelectionRange::new(end, start)
                    } else {
                        SelectionRange::new(start, end)
                    };
                    if index == main {
                        primary = ranges.len() - 1;
                    }
                    continue;
                }
            }
            if index == main {
                primary = ranges.len();
            }
            ranges.push(next);
        }
        Self {
            ranges,
            main: primary,
        }
    }

    /// Map with the default cursor association, `Assoc::Before`.
    pub fn map_desc(&self, changes: &ChangeDesc) -> Result<Self, EditError> {
        self.map_desc_with_assoc(changes, Assoc::Before)
    }

    /// Map through a change using `assoc` for cursors, then normalize the ranges.
    pub fn map_desc_with_assoc(
        &self,
        changes: &ChangeDesc,
        assoc: Assoc,
    ) -> Result<Self, EditError> {
        let ranges = self
            .ranges
            .iter()
            .map(|range| range.map_with_assoc(changes, assoc))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::normalize(ranges, self.main))
    }

    pub(crate) fn map(&self, changes: &ChangeSet) -> Self {
        Self::normalize(
            self.ranges
                .iter()
                .map(|range| {
                    range
                        .map_positions(Assoc::Before, |pos, assoc| {
                            Some(changes.map_pos(pos, assoc))
                        })
                        .expect("simple position mapping returns a position")
                })
                .collect(),
            self.main,
        )
    }

    pub(crate) fn check(&self, is_boundary: impl Fn(usize) -> bool) -> Result<(), EditError> {
        if self
            .ranges
            .iter()
            .any(|range| !is_boundary(range.anchor) || !is_boundary(range.head))
        {
            Err(EditError::InvalidSelection)
        } else {
            Ok(())
        }
    }
}
