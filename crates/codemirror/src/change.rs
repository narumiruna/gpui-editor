use std::{ops::Range, sync::Arc};

use crate::{EditError, Text};

mod filter;
mod transform;
pub use transform::{ChangeDesc, RangeTouch};

/// The side to which a position sticks when text is inserted or replaced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Assoc {
    Before,
    After,
}

/// Select which deleted context prevents a position from mapping.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MapMode {
    #[default]
    Simple,
    TrackDel,
    TrackBefore,
    TrackAfter,
}

impl MapMode {
    fn removes(self, at: usize, start: usize, end: usize) -> bool {
        match self {
            Self::Simple => false,
            Self::TrackDel => start < at && at < end,
            Self::TrackBefore => start < at && at <= end,
            Self::TrackAfter => start <= at && at < end,
        }
    }
}

/// A replacement measured in the old document.
#[derive(Clone, Debug, Eq, PartialEq)]
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

/// An input to [`EditorState::changes_from_specs`](crate::EditorState::changes_from_specs).
/// All ranges use UTF-8 byte offsets in the start document.
pub enum ChangeSpec<'a> {
    /// Parse the insertion using the state's line separator.
    String(Range<usize>, &'a str),
    /// Insert the text without parsing its line endings again.
    Text(Range<usize>, &'a Text),
    /// Apply a prebuilt, validated group at this point in the sequence.
    Set(&'a ChangeSet),
    /// Process specifications in their given order.
    Group(&'a [Self]),
}

pub(crate) enum ResolvedSpec<'a> {
    Edit(Change),
    Set(&'a ChangeSet),
}

/// Sorted, non-overlapping changes measured in the same document.
/// A set decoded from JSON retains its original length and section boundaries.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChangeSet {
    changes: Vec<Change>,
    scope: Option<Arc<ChangeScope>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ChangeScope {
    length: usize,
    // Boundaries between consecutive unchanged sections, not implied by edits.
    split_gaps: Vec<usize>,
}

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
        let mut merged: Vec<Change> = Vec::with_capacity(changes.len());
        for change in changes {
            if change.range.is_empty() && change.insert.is_empty() {
                continue;
            }
            if let Some(previous) = merged.last_mut()
                && previous.range.end == change.range.start
                && previous.insert.is_empty()
                && change.insert.is_empty()
            {
                previous.range.end = change.range.end;
            } else {
                merged.push(change);
            }
        }
        Ok(Self::from_ordered_changes(merged))
    }

    pub fn single(change: Change) -> Result<Self, EditError> {
        Self::new(vec![change])
    }

    /// Map a position from the old document into the new document.
    pub fn map_pos(&self, position: usize, assoc: Assoc) -> usize {
        self.map_pos_mode(position, assoc, MapMode::Simple)
            .expect("invalid byte position or mapped position exceeds usize")
    }

    /// Map a position, returning `None` if the selected context was replaced or deleted.
    pub fn map_pos_mode(&self, position: usize, assoc: Assoc, mode: MapMode) -> Option<usize> {
        if self
            .scope
            .as_ref()
            .is_some_and(|scope| position > scope.length)
        {
            return None;
        }
        let mut old = 0;
        let mut new = 0usize;
        for change in &self.changes {
            let start = change.range.start;
            let end = change.range.end;
            if position < start {
                break;
            }
            if mode.removes(position, start, end) {
                return None;
            }
            new = new.checked_add(start - old)?;
            if position == start && (!change.range.is_empty() || assoc == Assoc::Before) {
                return Some(new);
            }
            if position < end {
                return match assoc {
                    Assoc::Before => Some(new),
                    Assoc::After => new.checked_add(change.insert.len()),
                };
            }
            old = end;
            new = new.checked_add(change.insert.len())?;
        }
        new.checked_add(position - old)
    }

    /// A decoded set with split unchanged sections is not empty.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.split_gaps().is_empty()
    }

    /// Iterate over replacements in old-document order without applying them.
    pub fn changes(&self) -> impl Iterator<Item = (&Range<usize>, &str)> {
        self.changes
            .iter()
            .map(|change| (&change.range, change.insert.as_str()))
    }

    pub(crate) fn from_ordered_changes(changes: Vec<Change>) -> Self {
        debug_assert!(
            changes
                .iter()
                .all(|change| change.range.start <= change.range.end)
        );
        debug_assert!(
            changes
                .windows(2)
                .all(|pair| pair[0].range.end <= pair[1].range.start)
        );
        Self {
            changes,
            scope: None,
        }
    }

    fn check_length(&self, length: usize) -> Result<(), EditError> {
        if self
            .scope
            .as_ref()
            .is_some_and(|scope| scope.length != length)
        {
            return Err(EditError::MismatchedLength);
        }
        Ok(())
    }

    fn split_gaps(&self) -> &[usize] {
        self.scope.as_ref().map_or(&[], |scope| &scope.split_gaps)
    }

    fn bind_length(mut self, length: usize) -> Self {
        debug_assert!(self.check_length(length).is_ok());
        self.scope.get_or_insert_with(|| {
            Arc::new(ChangeScope {
                length,
                split_gaps: Vec::new(),
            })
        });
        self
    }

    pub(crate) fn check(&self, source: &Text) -> Result<(), EditError> {
        self.check_length(source.len())?;
        let mut length = source.len();
        for change in self.changes.iter().rev() {
            if !source.is_valid_range(change.range.clone()) {
                return Err(EditError::InvalidRange);
            }
            length = Text::checked_edit_len(length, change.range.len(), change.insert.len())?;
        }
        if self
            .split_gaps()
            .iter()
            .any(|&end| !source.is_char_boundary(end))
        {
            return Err(EditError::InvalidRange);
        }
        Ok(())
    }

    pub(crate) fn remove_noops(&mut self, source: &Text) {
        self.changes
            .retain(|change| !source.range_eq(change.range.clone(), &change.insert));
    }

    pub(crate) fn is_boundary_after(&self, source: &Text, mut position: usize) -> bool {
        let mut start = 0;
        for change in &self.changes {
            let unchanged = change.range.start - start;
            if position <= unchanged {
                return source.is_char_boundary(start + position);
            }
            position -= unchanged;
            if position <= change.insert.len() {
                return change.insert.is_char_boundary(position);
            }
            position -= change.insert.len();
            start = change.range.end;
        }
        position <= source.len() - start && source.is_char_boundary(start + position)
    }

    /// Calculate the inverse against the exact original document.
    pub fn inverted(&self, text: &Text) -> Result<Self, EditError> {
        self.check(text)?;
        Ok(self.invert(text))
    }

    pub(crate) fn invert(&self, source: &Text) -> Self {
        let mut old = 0;
        let mut new = 0;
        let mut inverse = Vec::with_capacity(self.changes.len());
        let mut split_gaps = self.split_gaps().iter().copied().peekable();
        let mut inverse_splits = Vec::new();
        for change in &self.changes {
            while let Some(pos) = split_gaps.next_if(|&pos| pos < change.range.start) {
                inverse_splits.push(new + (pos - old));
            }
            new += change.range.start - old;
            let end = new + change.insert.len();
            let insert = source
                .slice_string(change.range.clone())
                .expect("change range is valid");
            inverse.push(Change::new(new..end, insert));
            old = change.range.end;
            new = end;
        }
        inverse_splits.extend(split_gaps.map(|pos| new + (pos - old)));
        Self {
            changes: inverse,
            scope: self.scope.as_ref().map(|_| {
                Arc::new(ChangeScope {
                    length: new + (source.len() - old),
                    split_gaps: inverse_splits,
                })
            }),
        }
    }

    /// Apply the changes after checking the original length, if known, and all
    /// section boundaries in the original text.
    pub fn apply_to(&self, text: &mut Text) -> Result<(), EditError> {
        self.check(text)?;
        self.apply(text);
        Ok(())
    }

    pub(crate) fn apply(&self, text: &mut Text) {
        for change in self.changes.iter().rev() {
            text.replace_range(change.range.clone(), &change.insert);
        }
    }
}

#[cfg(test)]
impl ChangeSet {
    pub(crate) fn payload_len(&self) -> usize {
        self.changes.iter().map(|change| change.insert.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::{Change, ChangeSet, Text};

    #[test]
    fn removes_noops_and_inverts_changes_across_text_chunks() {
        let block = "a😀\r\n".repeat(1024);
        let source = Text::new(format!("{block}{block}"));
        let at = block.len();
        let mut changes = ChangeSet::new(vec![
            Change::new(0..at, block.as_str()),
            Change::new(at..source.len(), "é"),
        ])
        .unwrap();
        changes.check(&source).unwrap();
        changes.remove_noops(&source);
        assert_eq!(changes.changes().count(), 1);

        let edited = format!("{block}é");
        for position in [0, 2, at, at + 1, edited.len(), usize::MAX] {
            assert_eq!(
                changes.is_boundary_after(&source, position),
                edited.is_char_boundary(position)
            );
        }
        let inverse = changes.inverted(&source).unwrap();
        let mut result = source.clone();
        changes.apply_to(&mut result).unwrap();
        assert_eq!(result.as_str(), edited);
        inverse.apply_to(&mut result).unwrap();
        assert_eq!(result, source);
    }
}
