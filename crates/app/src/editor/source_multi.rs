use std::ops::Range;

use gpui::{ClipboardItem, Context, Global};
use gpui_editor_codemirror::{
    Change, ChangeSet, RangeEdit, SelectionRange, Transaction,
    autocomplete::{active_snippet, snippet_range_in_active_field},
    commands::{delete_char_backward, delete_char_forward},
    view::{
        ClipboardLinewiseMemory, filter_clipboard_output, plan_linewise_copy,
        plan_linewise_copy_projected, plan_linewise_paste, plan_linewise_paste_projected,
    },
};

use super::{EditReceipt, Editor};

#[cfg(test)]
mod tests;

#[derive(Default)]
struct ClipboardMemory(ClipboardLinewiseMemory);

impl Global for ClipboardMemory {}

impl Editor {
    pub(super) fn has_multiple_selections(&self) -> bool {
        self.view.state().selection().ranges().len() > 1
    }

    pub(super) fn collapse_multiple_selections(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.has_multiple_selections() {
            return false;
        }
        self.set_selection(self.anchor(), self.cursor());
        self.preferred_x = None;
        self.reveal_caret = true;
        cx.notify();
        true
    }

    pub(super) fn copy_or_cut(&mut self, cut: bool, cx: &mut Context<Self>) {
        if cut && (!self.input_enabled || self.view.state().read_only()) {
            return;
        }
        if let Some(scope) = self.linewise_clipboard_scope() {
            let lines = self.source_body_line_projection();
            let state = self.view.state();
            let separator = self.clipboard_separator();
            let plan = if let Some(lines) = &lines {
                plan_linewise_copy_projected(state, scope.clone(), separator, lines)
            } else {
                plan_linewise_copy(state, scope.clone(), separator)
            };
            let Ok(Some(copy)) = plan else {
                return;
            };
            if cut {
                let Ok(Some(transaction)) = copy.cut_transaction(state) else {
                    return;
                };
                if !self.apply_source_transaction(transaction, scope, cx) {
                    return;
                }
            }
            cx.default_global::<ClipboardMemory>().0.record_copy(&copy);
            cx.write_to_clipboard(ClipboardItem::new_string(copy.text().to_owned()));
        } else if let Some(text) = self.selected_text_for_clipboard()
            && (!cut || self.replace(self.selection(), "", cx))
        {
            cx.default_global::<ClipboardMemory>().0.clear();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub(super) fn paste_source_lines(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let Some(scope) = self.linewise_clipboard_scope() else {
            return false;
        };
        let Some(ClipboardMemory(memory)) = cx.try_global::<ClipboardMemory>() else {
            return false;
        };
        let lines = self.source_body_line_projection();
        let state = self.view.state();
        let separator = self.clipboard_separator();
        let plan = if let Some(lines) = &lines {
            plan_linewise_paste_projected(state, text, memory, separator, scope.clone(), lines)
        } else {
            plan_linewise_paste(state, text, memory, separator)
        };
        match plan {
            Ok(None) => false,
            Ok(Some(transaction)) => {
                self.apply_source_transaction(transaction, scope, cx);
                true
            }
            Err(_) => true,
        }
    }

    fn linewise_clipboard_scope(&self) -> Option<Range<usize>> {
        if self.is_composing() || active_snippet(self.view.state()).is_some() {
            return None;
        }
        let body = self.source_fence()?.body;
        self.view
            .state()
            .selection()
            .ranges()
            .iter()
            .all(|range| range.is_empty() && (body.start..=body.end).contains(&range.head()))
            .then_some(body)
    }

    fn selected_text_for_clipboard(&self) -> Option<String> {
        let state = self.view.state();
        let parts: Vec<_> = state
            .selection()
            .ranges()
            .iter()
            .filter(|range| !range.is_empty())
            .map(|range| &self.source()[range.range()])
            .collect();
        if parts.is_empty() {
            return None;
        }
        filter_clipboard_output(state, parts.join(self.clipboard_separator()))
            .ok()
            .filter(|text| !text.is_empty())
    }

    fn clipboard_separator(&self) -> &str {
        match self.source().split_once('\n') {
            Some((prefix, _)) if prefix.ends_with('\r') => "\r\n",
            _ => "\n",
        }
    }

    pub(super) fn replace_multiple_selections(
        &mut self,
        range: &Range<usize>,
        inserted: &str,
        paste: bool,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        if self.is_composing() || !self.has_multiple_selections() || *range != self.selection() {
            return None;
        }
        if !self.input_enabled {
            return Some(false);
        }
        let state = self.view.state();
        let clip = paste.then(|| inserted.replace("\r\n", "\n").replace('\r', "\n"));
        let by_line = clip
            .as_deref()
            .filter(|clip| clip.split('\n').count() == state.selection().ranges().len());
        let transaction = if let Some(clip) = by_line {
            let mut lines = clip.split('\n');
            state.change_by_range(|selection| {
                let from = selection.range().start;
                let line = lines.next().expect("one line per range");
                let end = from + line.len();
                Ok(RangeEdit {
                    changes: ChangeSet::single(Change::new(selection.range(), line))?,
                    selection: SelectionRange::new(end, end),
                    effects: Vec::new(),
                })
            })
        } else {
            state.replace_selection(inserted)
        };
        let event = if paste { "input.paste" } else { "input" };
        Some(transaction.is_ok_and(|transaction| {
            self.apply_multi_edit(
                transaction.with_user_event(event).with_scroll_into_view(),
                cx,
            )
        }))
    }

    pub(super) fn delete_multiple_selections(
        &mut self,
        right: bool,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        if self.is_composing() || !self.has_multiple_selections() {
            return None;
        }
        if !self.input_enabled {
            return Some(false);
        }
        let delete = if right {
            delete_char_forward
        } else {
            delete_char_backward
        };
        Some(delete(self.view.state()).is_ok_and(|transaction| {
            transaction.is_some_and(|transaction| self.apply_multi_edit(transaction, cx))
        }))
    }

    pub(super) fn apply_multi_edit(
        &mut self,
        transaction: Transaction,
        cx: &mut Context<Self>,
    ) -> bool {
        self.apply_multi_edit_with_receipt(transaction, None, cx)
    }

    pub(super) fn apply_multi_edit_with_receipt(
        &mut self,
        transaction: Transaction,
        receipt: Option<&mut Option<EditReceipt>>,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(snippet) = active_snippet(self.view.state())
            && !transaction
                .changes()
                .changes()
                .all(|(range, _)| snippet_range_in_active_field(&snippet, range.clone()))
        {
            return false;
        }
        let result = if let Some(active) = &self.active {
            let scope = self
                .source_fence()
                .map_or_else(|| active.content.clone(), |fence| fence.body);
            self.apply_source_transaction_with_receipt(transaction, scope, receipt, cx)
        } else {
            self.apply_prose_transaction_with_receipt(transaction, receipt, cx)
        };
        if !result {
            self.message = Some(
                if self.is_markdown() {
                    "Cannot edit all selected Markdown ranges"
                } else {
                    "Cannot edit all selected text ranges"
                }
                .into(),
            );
            cx.notify();
        }
        result
    }
}
