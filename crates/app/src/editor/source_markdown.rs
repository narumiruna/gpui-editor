use gpui::Context;
use gpui_editor_codemirror::{
    EditError, Transaction,
    language::markdown::{
        MarkdownDialect, MarkdownDocument, MarkdownMarkupCommand, markdown_snapshot,
        paste_url_as_link, plan_markdown_markup, plan_markdown_paste, plan_markdown_refresh,
    },
};

use super::Editor;
use crate::document::SectionKind;

impl Editor {
    /// Queue publication of deferred Markdown fence loads already polled by the host.
    /// The callback uses the current editor state and does not poll any loads.
    pub fn request_markdown_refresh(&mut self, cx: &mut Context<Self>) {
        let editor = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = editor.update(cx, |editor, cx| {
                if !editor.is_markdown() {
                    return;
                }
                let Ok(Some(transaction)) = plan_markdown_refresh(editor.view.state()) else {
                    return;
                };
                let active = editor.active.clone();
                let effect_count = transaction.effects().len();
                let result = editor.view.update_checked(
                    transaction,
                    active.clone(),
                    |resolved| {
                        if !resolved.changes().is_empty()
                            || resolved.selection().is_some()
                            || resolved.add_to_history()
                            || resolved.scroll_into_view()
                            || resolved.effects().len() != effect_count
                        {
                            return Err(EditError::InvalidRange);
                        }
                        Ok(())
                    },
                    |_| (None, active),
                );
                if result.is_ok() {
                    cx.notify();
                }
            });
        });
    }

    pub(super) fn paste_source_link(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if !self.can_edit_markup() {
            return false;
        }
        let selected = self.selection();
        if selected.is_empty()
            || self.markdown().sections().iter().any(|section| {
                !matches!(
                    section.kind,
                    SectionKind::Heading
                        | SectionKind::Bullet
                        | SectionKind::Strong
                        | SectionKind::Emphasis
                ) && section.source.start < selected.end
                    && selected.start < section.source.end
            })
        {
            return false;
        }
        let state = self.view.state();
        let transaction = if markdown_snapshot(state).is_some() {
            plan_markdown_paste(state, text).ok().flatten()
        } else {
            paste_url_as_link(&self.markdown_syntax(), state, text)
        };
        let Some(transaction) = transaction else {
            return false;
        };
        self.apply_prose_transaction(transaction, cx)
    }

    pub(super) fn edit_markup(&mut self, backspace: bool, cx: &mut Context<Self>) -> bool {
        let state = self.view.state();
        if !self.can_edit_markup()
            || state
                .selection()
                .ranges()
                .iter()
                .any(|range| !range.is_empty())
        {
            return false;
        }
        if backspace
            && state.selection().ranges().iter().any(|range| {
                let head = range.head();
                !state
                    .text()
                    .line_at(head)
                    .and_then(|line| line.text.get(..head - line.range.start))
                    .is_some_and(|before| {
                        before.trim().is_empty() || before.contains(['-', '+', '*', '.', ')', '>'])
                    })
            })
        {
            return false;
        }
        let command = if backspace {
            MarkdownMarkupCommand::DeleteBackward
        } else {
            MarkdownMarkupCommand::Continue
        };
        let result = if markdown_snapshot(state).is_some() {
            plan_markdown_markup(state, command)
        } else {
            command.run(&self.markdown_syntax(), state)
        };
        let Ok(Some(transaction)) = result else {
            return false;
        };
        self.apply_prose_transaction(transaction, cx)
    }

    fn can_edit_markup(&self) -> bool {
        self.is_markdown()
            && self.input_enabled
            && !self.view.state().read_only()
            && self.active.is_none()
            && !self.is_composing()
    }

    fn markdown_syntax(&self) -> MarkdownDocument {
        MarkdownDocument::parse_with_dialect(
            self.view.state().text().clone(),
            MarkdownDialect::Extended,
        )
    }

    pub(super) fn apply_prose_transaction(
        &mut self,
        transaction: Transaction,
        cx: &mut Context<Self>,
    ) -> bool {
        self.apply_prose_transaction_with_receipt(transaction, None, cx)
    }

    pub(super) fn apply_prose_transaction_with_receipt(
        &mut self,
        transaction: Transaction,
        receipt: Option<&mut Option<super::EditReceipt>>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_markdown() {
            let edits = transaction.changes().changes();
            let Ok(planned) = self
                .markdown()
                .plan_replacements(edits.map(|(range, text)| (range.clone(), text)))
            else {
                return false;
            };
            if planned.len() != transaction.changes().changes().count()
                || !planned.iter().zip(transaction.changes().changes()).all(
                    |((range, text), (planned_range, planned_text))| {
                        range == planned_range && text == planned_text
                    },
                )
            {
                return false;
            }
        }
        self.finish_typing();
        let check = super::preserve_markdown_edit(&transaction, receipt);
        if self
            .view
            .update_checked(transaction, None, check, |_| (None, None))
            .is_err()
        {
            return false;
        }
        self.invalidate_edit_layout();
        self.message = None;
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests;
