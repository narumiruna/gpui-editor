use crate::{EditError, StateEffect, Transaction};

/// Coordinates used by the second transaction in a merge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionCoordinates {
    /// Both transactions describe changes in the original document.
    Original,
    /// The second transaction describes changes in the result of the first one.
    Sequential,
}

impl Transaction {
    /// Combine two transaction specifications without applying them.
    ///
    /// Explicit selections and effects start in the result of their own edit.
    /// The later explicit selection takes precedence. The first annotation of
    /// a given type takes precedence.
    pub fn merge(
        mut self,
        other: Self,
        original_length: usize,
        coordinates: TransactionCoordinates,
    ) -> Result<Self, EditError> {
        let first = self.changes.desc(original_length)?;
        let (later, after_first, after_second) = match coordinates {
            TransactionCoordinates::Original => {
                let second = other.changes.desc(original_length)?;
                let later = other.changes.map(&first, false)?;
                let after_first = later.desc(first.new_length())?;
                (later, after_first, Some(first.map(&second, true)?))
            }
            TransactionCoordinates::Sequential => {
                let after_first = other.changes.desc(first.new_length())?;
                (other.changes, after_first, None)
            }
        };
        self.selection = match (other.selection, after_second.as_ref()) {
            (Some(selection), Some(after)) => Some(selection.map_desc(after)?),
            (Some(selection), None) => Some(selection),
            (None, _) => self
                .selection
                .map(|selection| selection.map_desc(&after_first))
                .transpose()?,
        };
        self.effects = StateEffect::map_all(&self.effects, &after_first);
        self.effects.extend(match after_second {
            Some(after) => StateEffect::map_all(&other.effects, &after),
            None => other.effects,
        });
        self.changes = self.changes.compose(&later, original_length)?;
        self.annotations.extend(other.annotations);
        self.scroll_into_view |= other.scroll_into_view;
        self.unfiltered |= other.unfiltered;
        Ok(self)
    }
}
