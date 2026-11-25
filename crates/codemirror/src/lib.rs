//! Text, selections, transactions, and undo history for a native editor.
//!
//! All positions are UTF-8 byte offsets. Text retains its original line endings.

mod change;
mod history;
mod selection;
mod text;

pub use change::{Assoc, Change, ChangeSet};
pub use history::History;
pub use selection::{EditorSelection, SelectionRange};
pub use text::{Line, Text};

/// An invalid edit or selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditError {
    InvalidRange,
    OverlappingChanges,
    InvalidSelection,
}

/// A group of changes in old-document coordinates with an optional new selection.
#[derive(Debug, Default)]
pub struct Transaction {
    changes: ChangeSet,
    selection: Option<EditorSelection>,
}

impl Transaction {
    pub fn new(changes: ChangeSet) -> Self {
        Self {
            changes,
            selection: None,
        }
    }

    pub fn with_selection(mut self, selection: EditorSelection) -> Self {
        self.selection = Some(selection);
        self
    }
}

#[derive(Debug)]
struct AppliedEdit {
    forward: ChangeSet,
    inverse: ChangeSet,
}

/// The authoritative text and selection, with no dependency on a view.
#[derive(Debug)]
pub struct EditorState {
    text: Text,
    selection: EditorSelection,
}

impl EditorState {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            text: Text::new(source),
            selection: EditorSelection::single(0, 0),
        }
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn selection(&self) -> &EditorSelection {
        &self.selection
    }

    fn set_selection(&mut self, selection: EditorSelection) -> Result<(), EditError> {
        selection.check(|pos| self.text.as_str().is_char_boundary(pos))?;
        self.selection = selection;
        Ok(())
    }

    /// Apply an edit without recording history. Use [`History::update`] for undoable edits.
    /// Errors leave the state unchanged.
    pub fn update(&mut self, transaction: Transaction) -> Result<(), EditError> {
        self.apply(transaction).map(|_| ())
    }

    fn apply(&mut self, transaction: Transaction) -> Result<Option<AppliedEdit>, EditError> {
        let source = self.text.as_str();
        let Transaction {
            mut changes,
            selection,
        } = transaction;
        changes.check(source)?;
        changes.remove_noops(source);
        let selection = selection.unwrap_or_else(|| self.selection.map(&changes));
        selection.check(|pos| changes.is_boundary_after(source, pos))?;
        self.selection = selection;
        if changes.is_empty() {
            return Ok(None);
        }
        let inverse = changes.invert(source);
        changes.apply(&mut self.text);
        Ok(Some(AppliedEdit {
            forward: changes,
            inverse,
        }))
    }
}
