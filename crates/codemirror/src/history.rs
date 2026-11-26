use std::collections::VecDeque;

use crate::{AppliedEdit, EditError, EditorSelection, EditorState, Transaction};

const LIMIT: usize = 100;

#[derive(Debug, Default)]
enum Group {
    #[default]
    Closed,
    Open,
    Recorded,
}

#[derive(Debug)]
struct Entry<M> {
    edits: Vec<AppliedEdit>,
    before_selection: EditorSelection,
    after_selection: EditorSelection,
    before: M,
    after: M,
}

/// An editor state with bounded, delta-based undo and redo. Metadata belongs to the view.
#[derive(Debug)]
pub struct History<M> {
    state: EditorState,
    undo: VecDeque<Entry<M>>,
    redo: Vec<Entry<M>>,
    group: Group,
}

impl<M: Clone> History<M> {
    pub fn new(state: EditorState) -> Self {
        Self {
            state,
            undo: VecDeque::new(),
            redo: Vec::new(),
            group: Group::Closed,
        }
    }

    pub fn state(&self) -> &EditorState {
        &self.state
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Begin a group of subsequent edits. The group may be empty.
    pub fn start_group(&mut self) {
        self.group = Group::Open;
    }

    pub fn end_group(&mut self) {
        self.group = Group::Closed;
    }

    /// Set the selection without adding a history entry.
    pub fn set_selection(&mut self, selection: EditorSelection) -> Result<(), EditError> {
        self.state.set_selection(selection)
    }

    /// Apply an edit and let the caller derive the final selection and metadata from
    /// the resulting state. An invalid transaction or final selection leaves no edit.
    pub fn update(
        &mut self,
        transaction: Transaction,
        before: M,
        finish: impl FnOnce(&EditorState) -> (Option<EditorSelection>, M),
    ) -> Result<M, EditError> {
        let before_selection = self.state.selection.clone();
        let edit = self.state.apply(transaction)?;
        let (selection, after) = finish(&self.state);
        if let Some(selection) = selection
            && let Err(error) = self.state.set_selection(selection)
        {
            if let Some(edit) = edit {
                edit.inverse.apply(&mut self.state.text);
            }
            self.state.selection = before_selection;
            return Err(error);
        }
        if matches!(self.group, Group::Recorded)
            && let Some(entry) = self.undo.back_mut()
        {
            entry.edits.extend(edit);
            entry.after = after.clone();
            entry.after_selection = self.state.selection.clone();
        } else if let Some(edit) = edit {
            self.redo.clear();
            if self.undo.len() == LIMIT {
                self.undo.pop_front();
            }
            self.undo.push_back(Entry {
                edits: vec![edit],
                before_selection,
                after_selection: self.state.selection.clone(),
                before,
                after: after.clone(),
            });
            if matches!(self.group, Group::Open) {
                self.group = Group::Recorded;
            }
        }
        Ok(after)
    }

    pub fn undo(&mut self) -> Option<M> {
        let entry = self.undo.pop_back()?;
        self.end_group();
        for edit in entry.edits.iter().rev() {
            edit.inverse.apply(&mut self.state.text);
        }
        self.state.selection = entry.before_selection.clone();
        let before = entry.before.clone();
        self.redo.push(entry);
        Some(before)
    }

    pub fn redo(&mut self) -> Option<M> {
        let entry = self.redo.pop()?;
        self.end_group();
        for edit in &entry.edits {
            edit.forward.apply(&mut self.state.text);
        }
        self.state.selection = entry.after_selection.clone();
        let after = entry.after.clone();
        self.undo.push_back(entry);
        Some(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Change, ChangeSet};

    #[test]
    fn stores_only_changed_bytes_for_a_small_edit() {
        let mut history = History::new(EditorState::new("a".repeat(1_000_000)));
        let changes = ChangeSet::single(Change::new(123..124, "b")).unwrap();
        history
            .update(Transaction::new(changes), (), |_| (None, ()))
            .unwrap();
        let edit = &history.undo[0].edits[0];
        assert_eq!(edit.forward.payload_len(), 1);
        assert_eq!(edit.inverse.payload_len(), 1);
    }
}
