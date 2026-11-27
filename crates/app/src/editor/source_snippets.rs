use gpui::Context;
use gpui_editor_codemirror::{
    EditorSelection, Transaction,
    autocomplete::{active_snippet, clear_snippet, next_snippet_field, previous_snippet_field},
};

use super::Editor;

impl Editor {
    pub(super) fn has_source_snippet(&self) -> bool {
        active_snippet(self.view.state()).is_some()
    }

    pub(super) fn move_source_snippet(&mut self, previous: bool, cx: &mut Context<Self>) -> bool {
        if !self.input_enabled || self.is_composing() {
            return false;
        }
        let transaction = if previous {
            previous_snippet_field(self.view.state())
        } else {
            next_snippet_field(self.view.state())
        };
        transaction.is_some_and(|transaction| self.apply_snippet_navigation(transaction, cx))
    }

    pub(super) fn cancel_source_snippet(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(transaction) = clear_snippet(self.view.state()) else {
            return false;
        };
        let transaction =
            transaction.with_selection(EditorSelection::single(self.anchor(), self.cursor()));
        self.apply_snippet_navigation(transaction, cx)
    }

    fn apply_snippet_navigation(
        &mut self,
        transaction: Transaction,
        cx: &mut Context<Self>,
    ) -> bool {
        let active = self.active.clone();
        if self
            .view
            .update(transaction, active.clone(), |_| (None, active))
            .is_err()
        {
            return false;
        }
        self.invalidate_edit_layout();
        cx.notify();
        true
    }
}
