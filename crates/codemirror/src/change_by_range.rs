//! Coordinate rules follow CodeMirror state 6.7.6, `EditorState.changeByRange`.

use crate::{
    Assoc, Change, ChangeSet, EditError, EditorSelection, EditorState, SelectionRange, StateEffect,
    Transaction,
};

/// The result of editing one range in a multi-range selection.
#[derive(Debug)]
pub struct RangeEdit {
    /// Changes in the original document.
    pub changes: ChangeSet,
    /// The selection in the document after these changes alone.
    pub selection: SelectionRange,
    /// Positional effects use the same coordinates as the new selection.
    pub effects: Vec<StateEffect>,
}

impl EditorState {
    /// Replace every selected range with the given bytes. Keep line endings
    /// and put each caret after its replacement.
    pub fn replace_selection(&self, text: &str) -> Result<Transaction, EditError> {
        self.replace_selection_with_assoc(text, None)
    }

    /// Parse the replacement using the state's line separator before replacing
    /// every selected range. Each caret associates with the inserted text.
    pub fn replace_selection_parsed(&self, text: &str) -> Result<Transaction, EditError> {
        self.replace_selection_with_assoc(self.to_text(text).as_str(), Some(Assoc::Before))
    }

    fn replace_selection_with_assoc(
        &self,
        text: &str,
        assoc: Option<Assoc>,
    ) -> Result<Transaction, EditError> {
        self.change_by_range(|selection| {
            let range = selection.range();
            let end = range
                .start
                .checked_add(text.len())
                .ok_or(EditError::InvalidRange)?;
            Ok(RangeEdit {
                changes: ChangeSet::single(Change::new(range, text))?,
                selection: EditorSelection::cursor(end, assoc, None, None),
                effects: Vec::new(),
            })
        })
    }

    /// Combine edits computed independently for each range in the current selection.
    /// Each callback sees the original document. The result is not applied yet.
    pub fn change_by_range(
        &self,
        mut edit: impl FnMut(SelectionRange) -> Result<RangeEdit, EditError>,
    ) -> Result<Transaction, EditError> {
        let source = self.text();
        let validate = |edit: &RangeEdit| {
            edit.changes.check(source)?;
            if !edit
                .changes
                .is_boundary_after(source, edit.selection.anchor())
                || !edit
                    .changes
                    .is_boundary_after(source, edit.selection.head())
            {
                return Err(EditError::InvalidSelection);
            }
            Ok(())
        };

        let mut original = self.selection().ranges().iter().copied();
        let first = edit(original.next().expect("an editor selection is nonempty"))?;
        validate(&first)?;
        let mut changes = first.changes;
        let mut effects = first.effects;
        let mut ranges = vec![first.selection];
        for selection in original {
            let next = edit(selection)?;
            validate(&next)?;
            let accumulated = changes.desc(source.len())?;
            let next_desc = next.changes.desc(source.len())?;

            let mapped = next.changes.map(&accumulated, false)?;
            let after_accumulated = mapped.desc(accumulated.new_length())?;
            let after_next = accumulated.map(&next_desc, true)?;
            for range in &mut ranges {
                *range = range.map(&after_accumulated)?;
            }
            ranges.push(next.selection.map(&after_next)?);

            effects = StateEffect::map_all(&effects, &after_accumulated);
            effects.extend(StateEffect::map_all(&next.effects, &after_next));
            changes = changes.compose(&mapped, source.len())?;
        }

        let selection = EditorSelection::new(ranges, self.selection().main_index())?;
        Ok(Transaction::new(changes)
            .with_selection(selection)
            .with_effects(effects))
    }
}
