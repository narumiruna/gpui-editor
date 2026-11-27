use gpui::{
    App, Context, Div, Entity, KeyBinding, Subscription, Window, actions, div, prelude::*, px,
};
use gpui_editor_codemirror::{EditorState, search::goto_line_transaction, view::ScrollStrategy};

use super::Editor;
use crate::{
    app::developer::{FindInput, TextField},
    theme::Theme,
};

actions!(editor, [OpenGotoLine, CloseGotoLine]);

pub(super) fn bind_keys(cx: &mut App) {
    for context in ["Editor", "DocumentFind", "GotoLine"] {
        cx.bind_keys([KeyBinding::new("cmd-alt-g", OpenGotoLine, Some(context))]);
    }
    cx.bind_keys([KeyBinding::new("escape", CloseGotoLine, Some("GotoLine"))]);
}

pub(super) struct GotoLinePanel {
    input: Entity<TextField>,
    opening: EditorState,
    revision: u64,
    _submit: Subscription,
}

impl GotoLinePanel {
    pub(super) fn set_editable(&self, enabled: bool, cx: &mut Context<Editor>) {
        self.input
            .update(cx, |field, cx| field.set_editable(enabled, cx));
    }
}

impl Editor {
    pub(super) fn open_goto_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.input_enabled || self.is_composing() || self.find_is_composing(cx) {
            return;
        }
        if let Some(panel) = &self.goto_line {
            panel.input.read(cx).focus(window);
            return;
        }
        let opening = self.view.state().clone();
        let number = opening
            .text()
            .line_at(opening.selection().main().head())
            .expect("valid caret")
            .number;
        let colors = Self::panel_input_colors(cx);
        let input = cx
            .new(|cx| TextField::document_find("goto-line-value", number.to_string(), colors, cx));
        let submit = cx.subscribe_in(&input, window, |editor, _, _: &FindInput, window, cx| {
            editor.finish_goto_line(true, window, cx);
        });
        input.read(cx).focus(window);
        self.goto_line = Some(GotoLinePanel {
            input,
            opening,
            revision: self.view.revision(),
            _submit: submit,
        });
        cx.notify();
    }

    fn finish_goto_line(&mut self, submit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = &self.goto_line else { return };
        let input = panel.input.read(cx);
        if input.is_composing() || self.is_composing() {
            return;
        }
        let jump = if submit && self.input_enabled && panel.revision == self.view.revision() {
            goto_line_transaction(&panel.opening, input.text())
        } else {
            None
        };
        self.goto_line = None;
        if let Some((target, transaction)) =
            jump.and_then(|transaction| Some((transaction.selection()?.main().head(), transaction)))
        {
            self.leave_source_if_outside(target);
            self.reveal_folded_range(&(target..target), cx);
            self.completion = None;
            let active = self.active.clone();
            if self
                .view
                .update(transaction, active.clone(), |_| (None, active))
                .is_ok()
            {
                self.preferred_x = None;
                self.upstream = false;
                if let Some(scroll) = self.view.take_scroll_target() {
                    self.request_source_reveal(
                        scroll.range.head(),
                        scroll.y == ScrollStrategy::Center,
                        cx,
                    );
                }
            }
        }
        window.focus(&self.focus);
        cx.notify();
    }

    pub(super) fn render_goto_line(&self, window: &Window, cx: &Context<Self>) -> Option<Div> {
        let panel = self.goto_line.as_ref()?;
        let theme = Theme::for_editor(window, self.view.state());
        let panels = gpui_editor_codemirror::theme_one_dark::view_style(self.view.state())
            .map(|style| style.panels);
        Some(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .bg(if panels.is_some() {
                    theme.panel
                } else {
                    theme.toolbar
                })
                .text_color(theme.panel_text())
                .border_b(px(
                    panels.map_or(1., |style| f32::from(style.top_border.width_px))
                ))
                .border_color(theme.border)
                .key_context("GotoLine")
                .on_action(cx.listener(|editor, _: &OpenGotoLine, window, cx| {
                    editor.open_goto_line(window, cx)
                }))
                .on_action(cx.listener(|editor, _: &CloseGotoLine, window, cx| {
                    editor.finish_goto_line(false, window, cx)
                }))
                .child("Go to line")
                .child(div().min_w_0().flex_1().child(panel.input.clone()))
                .child(
                    div()
                        .id("goto-line-close")
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .hover(|style| style.bg(theme.selected))
                        .on_click(cx.listener(|editor, _, window, cx| {
                            editor.finish_goto_line(false, window, cx)
                        }))
                        .child("Close"),
                ),
        )
    }
}

#[cfg(test)]
mod tests;
