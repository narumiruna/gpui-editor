use std::ops::Range;

use crate::{EditError, Text};

/// The side to which a position sticks when text is inserted or replaced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Assoc {
    Before,
    After,
}

/// A replacement measured in the old document.
#[derive(Debug)]
pub struct Change {
    range: Range<usize>,
    insert: String,
}

impl Change {
    pub fn new(range: Range<usize>, insert: impl Into<String>) -> Self {
        Self {
            range,
            insert: insert.into(),
        }
    }
}

/// Sorted, non-overlapping changes measured in the same document.
#[derive(Debug, Default)]
pub struct ChangeSet(Vec<Change>);

impl ChangeSet {
    /// Preserve the input order of insertions at the same position.
    pub fn new(mut changes: Vec<Change>) -> Result<Self, EditError> {
        changes.sort_by_key(|change| (change.range.start, change.range.end));
        let mut end = 0;
        for change in &changes {
            if change.range.start > change.range.end {
                return Err(EditError::InvalidRange);
            }
            if change.range.start < end {
                return Err(EditError::OverlappingChanges);
            }
            end = change.range.end;
        }
        Ok(Self(changes))
    }

    pub fn single(change: Change) -> Result<Self, EditError> {
        Self::new(vec![change])
    }

    /// Map a position from the old document into the new document.
    pub fn map_pos(&self, position: usize, assoc: Assoc) -> usize {
        let mut old = 0;
        let mut new = 0;
        for change in &self.0 {
            let start = change.range.start;
            if position < start {
                break;
            }
            new += start - old;
            if position == start && (!change.range.is_empty() || assoc == Assoc::Before) {
                return new;
            }
            if position < change.range.end {
                return match assoc {
                    Assoc::Before => new,
                    Assoc::After => new + change.insert.len(),
                };
            }
            old = change.range.end;
            new += change.insert.len();
        }
        new + (position - old)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn check(&self, source: &str) -> Result<(), EditError> {
        if self
            .0
            .iter()
            .any(|change| source.get(change.range.clone()).is_none())
        {
            return Err(EditError::InvalidRange);
        }
        Ok(())
    }

    pub(crate) fn remove_noops(&mut self, source: &str) {
        self.0
            .retain(|change| source[change.range.clone()] != change.insert);
    }

    pub(crate) fn is_boundary_after(&self, source: &str, mut position: usize) -> bool {
        let mut start = 0;
        for change in &self.0 {
            let unchanged = &source[start..change.range.start];
            if position <= unchanged.len() {
                return unchanged.is_char_boundary(position);
            }
            position -= unchanged.len();
            if position <= change.insert.len() {
                return change.insert.is_char_boundary(position);
            }
            position -= change.insert.len();
            start = change.range.end;
        }
        source[start..].is_char_boundary(position)
    }

    pub(crate) fn invert(&self, source: &str) -> Self {
        let mut old = 0;
        let mut new = 0;
        let mut inverse = Vec::with_capacity(self.0.len());
        for change in &self.0 {
            new += change.range.start - old;
            let end = new + change.insert.len();
            inverse.push(Change::new(new..end, &source[change.range.clone()]));
            old = change.range.end;
            new = end;
        }
        Self(inverse)
    }

    pub(crate) fn apply(&self, text: &mut Text) {
        for change in self.0.iter().rev() {
            text.0.replace_range(change.range.clone(), &change.insert);
        }
    }
}

#[cfg(test)]
impl ChangeSet {
    pub(crate) fn payload_len(&self) -> usize {
        self.0.iter().map(|change| change.insert.len()).sum()
    }
}
