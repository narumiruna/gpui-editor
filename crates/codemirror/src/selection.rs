use std::ops::Range;

use crate::{Assoc, ChangeSet, EditError};

/// A selection with a fixed anchor and a movable head.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectionRange {
    anchor: usize,
    head: usize,
}

impl SelectionRange {
    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
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

    fn with_direction(self, start: usize, end: usize) -> Self {
        if self.head < self.anchor {
            Self::new(end, start)
        } else {
            Self::new(start, end)
        }
    }

    fn map(self, changes: &ChangeSet) -> Self {
        let range = self.range();
        if range.is_empty() {
            let position = changes.map_pos(range.start, Assoc::Before);
            return Self::new(position, position);
        }
        let start = changes.map_pos(range.start, Assoc::After);
        let end = changes.map_pos(range.end, Assoc::Before);
        // A replacement that covers both endpoints maps them to opposite edges.
        self.with_direction(start.min(end), end)
    }
}

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

    pub fn new(ranges: Vec<SelectionRange>, main: usize) -> Result<Self, EditError> {
        if main >= ranges.len() {
            return Err(EditError::InvalidSelection);
        }
        Ok(Self::normalize(ranges, main))
    }

    pub fn main(&self) -> SelectionRange {
        self.ranges[self.main]
    }

    pub fn ranges(&self) -> &[SelectionRange] {
        &self.ranges
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
                    *last = next.with_direction(previous.start, previous.end.max(range.end));
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

    pub(crate) fn map(&self, changes: &ChangeSet) -> Self {
        Self::normalize(
            self.ranges.iter().map(|range| range.map(changes)).collect(),
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
