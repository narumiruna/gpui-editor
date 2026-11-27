//! Explicit completion in the active code fence of the attached draft.

use std::{cell::Cell, ops::Range, rc::Rc};

use gpui::Context;
use gpui_editor_codemirror::{
    ChangeSet, EditError, EditorSelection, HistoryIsolation, Transaction,
    lsp::features::{CompletionList, CompletionTrigger, FeatureRequest, completion_request},
    view::{EditorView, PluginId, ViewPlugin, ViewUpdate},
};

use super::{Editor, Pending, RequestId, RpcResolution, StdioClient, Value, send_request};
use crate::document::Section;

mod checks;

/// Single-use choices for one editor, attachment, and request.
pub struct SourceLspCompletion {
    attachment: Rc<Cell<u64>>,
    generation: u64,
    stamp: Stamp,
    items: Vec<SourceLspCompletionItem>,
}

impl SourceLspCompletion {
    /// Eligible choices in server order. Use an index from this slice to apply
    /// an item. The Markdown structure check runs on that item at apply time.
    pub fn items(&self) -> &[SourceLspCompletionItem] {
        &self.items
    }
}

/// A literal label and simultaneous edits in draft UTF-8 byte coordinates.
pub struct SourceLspCompletionItem {
    label: String,
    changes: ChangeSet,
    caret: usize,
}

impl SourceLspCompletionItem {
    /// Display this label as literal text. Do not interpret markup or links.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Preview these changes as literal source. They describe the request draft.
    pub fn changes(&self) -> &ChangeSet {
        &self.changes
    }
}

struct SelectionTracker(Rc<Cell<u64>>);

impl ViewPlugin for SelectionTracker {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        if update.selection_set
            || update.focus_changed
            || update.start_state.read_only() != update.state.read_only()
        {
            self.0.set(self.0.get().wrapping_add(1));
        }
    }
}

#[derive(Eq, PartialEq)]
struct Stamp {
    revision: u64,
    selection_epoch: u64,
    fence: Range<usize>,
    body: Range<usize>,
    caret: usize,
}

pub(super) struct Completion {
    selection_epoch: Rc<Cell<u64>>,
    plugin: PluginId,
    generation: u64,
    wanted: Option<Stamp>,
    pending: Option<Pending<Stamp, FeatureRequest<CompletionList>>>,
    checked: Option<SourceLspCompletion>,
}

impl Editor {
    /// Queue explicit completion at the current single caret in an active code
    /// fence. `offset` is a draft UTF-8 byte offset. Poll to send the request
    /// and receive choices. A valid request supersedes earlier choices.
    pub fn request_source_lsp_completion(&mut self, offset: usize) -> bool {
        let Some(lsp) = &self.source_lsp else {
            return false;
        };
        if lsp.client.server_capabilities().is_some_and(|caps| {
            !matches!(
                caps.get("completionProvider"),
                Some(Value::Object(_) | Value::Bool(true))
            )
        }) {
            return false;
        }
        let Some(stamp) = lsp
            .completion
            .stamp(self)
            .filter(|stamp| stamp.caret == offset)
        else {
            return false;
        };
        let completion = &mut self.source_lsp.as_mut().expect("attached").completion;
        completion.generation = completion.generation.wrapping_add(1);
        completion.wanted = Some(stamp);
        if let Some(pending) = &mut completion.pending {
            pending.target = None;
        }
        completion.checked = None;
        true
    }

    /// Take bounded, coordinate-checked choices without changing the draft,
    /// selection, or native completion popup. This does not check the effect
    /// of each choice on the full Markdown structure.
    pub fn take_source_lsp_completion(&mut self) -> Option<SourceLspCompletion> {
        let result = self.source_lsp.as_mut()?.completion.checked.take()?;
        self.source_lsp_completion_is_current(&result)
            .then_some(result)
    }

    /// Apply the host's choice in one undo step. Reject a stale choice, an
    /// unsafe Markdown change, or an extension rewrite. The result is consumed
    /// even when the choice cannot be applied.
    pub fn apply_source_lsp_completion(
        &mut self,
        result: SourceLspCompletion,
        index: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.source_lsp_completion_is_current(&result) {
            return false;
        }
        let Some(item) = result.items.into_iter().nth(index) else {
            return false;
        };
        let Some(active_after) = checks::checked_fence(self, &result.stamp, &item) else {
            return false;
        };
        let selection = EditorSelection::single(item.caret, item.caret);
        let transaction = Transaction::new(item.changes.clone())
            .with_selection(selection.clone())
            .with_user_event("input.complete")
            .isolate_history(HistoryIsolation::Full);
        let applied = self.view.update_checked(
            transaction,
            self.active.clone(),
            |accepted| {
                if accepted.changes() != &item.changes
                    || accepted.selection() != Some(&selection)
                    || !accepted.effects().is_empty()
                    || accepted.scroll_into_view()
                    || accepted.user_event() != Some("input.complete")
                    || !accepted.add_to_history()
                    || accepted.history_isolation() != Some(HistoryIsolation::Full)
                {
                    return Err(EditError::InvalidRange);
                }
                Ok(())
            },
            |_| (None, Some(active_after)),
        );
        let Ok(active) = applied else {
            return false;
        };
        self.active = active;
        self.invalidate_edit_layout();
        self.message = None;
        cx.notify();
        true
    }

    fn source_lsp_completion_is_current(&self, result: &SourceLspCompletion) -> bool {
        self.source_lsp.as_ref().is_some_and(|lsp| {
            Rc::ptr_eq(&lsp.completion.selection_epoch, &result.attachment)
                && lsp.completion.generation == result.generation
                && lsp.completion.stamp(self).as_ref() == Some(&result.stamp)
        })
    }
}

impl Completion {
    pub(super) fn attach(view: &mut EditorView<Option<Section>>) -> Self {
        let selection_epoch = Rc::new(Cell::new(0));
        let plugin = view.add_plugin(SelectionTracker(selection_epoch.clone()));
        Self {
            selection_epoch,
            plugin,
            generation: 0,
            wanted: None,
            pending: None,
            checked: None,
        }
    }

    pub(super) fn plugin(&self) -> PluginId {
        self.plugin
    }

    fn stamp(&self, editor: &Editor) -> Option<Stamp> {
        if !editor.reference_input_allowed()
            || editor.selecting
            || editor.has_multiple_selections()
            || !editor.selection().is_empty()
        {
            return None;
        }
        let body = editor.source_fence_with_unlabeled(true)?.body;
        let caret = editor.cursor();
        let source = editor.source();
        if !(body.start..=body.end).contains(&caret)
            || !source.is_char_boundary(caret)
            || (caret > 0 && source.as_bytes().get(caret - 1..caret + 1) == Some(b"\r\n"))
        {
            return None;
        }
        Some(Stamp {
            revision: editor.view.revision(),
            selection_epoch: self.selection_epoch.get(),
            fence: editor.active.as_ref()?.source.clone(),
            body,
            caret,
        })
    }

    pub(super) fn retain_current(&mut self, editor: &Editor) {
        if self.wanted.is_none() && self.pending.is_none() && self.checked.is_none() {
            return;
        }
        let current = self.stamp(editor);
        self.wanted
            .take_if(|stamp| Some(&*stamp) != current.as_ref());
        if let Some(pending) = &mut self.pending {
            pending
                .target
                .take_if(|stamp| Some(&*stamp) != current.as_ref());
        }
        self.checked
            .take_if(|result| Some(&result.stamp) != current.as_ref());
    }

    pub(super) fn discard_wanted(&mut self) {
        self.wanted = None;
    }

    pub(super) fn can_send(&self) -> bool {
        self.wanted.is_some() && self.pending.is_none()
    }

    pub(super) fn send(&mut self, client: &mut StdioClient, uri: &str) -> bool {
        send_request(
            client,
            &mut self.wanted,
            &mut self.pending,
            |workspace, stamp| {
                completion_request(workspace, uri, stamp.caret, CompletionTrigger::Invoked)
            },
        )
    }

    pub(super) fn is_pending(&self, id: RequestId) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.id == id)
    }

    pub(super) fn timed_out(&mut self, id: RequestId) {
        self.pending.take_if(|pending| pending.id == id);
    }

    pub(super) fn resolve(
        &mut self,
        client: &StdioClient,
        uri: &str,
        editor: &Editor,
        reply: RpcResolution,
    ) {
        let Some(pending) = self.pending.take_if(|pending| pending.id == reply.id) else {
            return;
        };
        let (Some(stamp), Ok(mut result)) = (pending.target, reply.outcome) else {
            return;
        };
        if reply.cancel_requested
            || self.stamp(editor).as_ref() != Some(&stamp)
            || !checks::retain_plain_payload(&mut result)
        {
            return;
        }
        let Some(items) = client.with_workspace(|workspace| {
            let file = workspace.file(uri)?;
            if file.version() != pending.request.version()
                || file.has_pending_changes()
                || file.synced() != editor.view.state().text()
            {
                return None;
            }
            let list = pending.request.resolve(workspace, &result).ok()??;
            Some(checks::checked_items(
                workspace,
                file.synced(),
                &stamp,
                &result,
                &list,
            ))
        }) else {
            return;
        };
        if !items.is_empty() {
            self.checked = Some(SourceLspCompletion {
                attachment: self.selection_epoch.clone(),
                generation: self.generation,
                stamp,
                items,
            });
        }
    }
}

#[cfg(test)]
mod tests;
