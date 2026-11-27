//! GPUI presentation for the host's single-draft references panel.

use gpui::{
    AnyElement, Context, Div, FocusHandle, FontWeight, HighlightStyle, MouseButton, ScrollHandle,
    StyledText, Window, div, prelude::*, px, relative, rgba,
};
use gpui_editor_codemirror::view::panels::PanelId;

use super::{Editor, ReferenceRow};
use crate::theme::Theme;

pub(in crate::editor) struct ReferencePanelUi {
    focus: FocusHandle,
    rendered: Option<(PanelId, usize)>,
    scroll: ScrollHandle,
}

impl ReferencePanelUi {
    pub(in crate::editor) fn new(cx: &Context<Editor>) -> Self {
        Self {
            focus: cx.focus_handle().tab_index(0).tab_stop(true),
            rendered: None,
            scroll: ScrollHandle::new(),
        }
    }
}

impl Editor {
    pub(in crate::editor) fn render_source_lsp_references(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let results = self.source_lsp_references();
        let current = results.as_ref().map(|panel| (panel.id, panel.selected));
        let previous = std::mem::replace(&mut self.reference_panel_ui.rendered, current);
        // GPUI focus changes must run outside the draw. Recheck ownership and
        // the live panel when the deferred change runs.
        if let Some((id, _)) = current.filter(|current| previous.map(|p| p.0) != Some(current.0)) {
            self.reference_panel_ui.scroll = ScrollHandle::new();
            if window.is_window_active() && self.focus.is_focused(window) {
                cx.defer_in(window, move |editor, window, _| {
                    if window.is_window_active()
                        && editor.focus.is_focused(window)
                        && !editor.is_reviewing_changes()
                        && editor
                            .source_lsp_references()
                            .is_some_and(|panel| panel.id == id)
                    {
                        window.focus(&editor.reference_panel_ui.focus);
                    }
                });
            }
        } else if current.is_none()
            && previous.is_some()
            && self.reference_panel_ui.focus.is_focused(window)
        {
            cx.defer_in(window, |editor, window, _| {
                if editor.reference_panel_ui.focus.is_focused(window)
                    && editor.source_lsp_references().is_none()
                {
                    window.focus(&editor.focus);
                }
            });
        }
        let panel = results?;
        if current != previous {
            self.reference_panel_ui
                .scroll
                .scroll_to_item(panel.selected);
        }
        let id = panel.id;
        let theme = Theme::for_editor(window, self.view.state());
        let (border_width, border_color) = theme.view.map_or((px(1.), theme.border), |style| {
            let border = style.panels.bottom_border;
            (px(f32::from(border.width_px)), rgba(border.color.rgba32()))
        });
        Some({
            let panel = div()
                .id("source-lsp-references")
                .occlude()
                .track_focus(&self.reference_panel_ui.focus)
                .on_key_down(
                    cx.listener(move |editor, event: &gpui::KeyDownEvent, window, cx| {
                        let modifiers = event.keystroke.modifiers;
                        if modifiers.platform
                            || modifiers.control
                            || modifiers.alt
                            || modifiers.shift
                        {
                            return;
                        }
                        let key = match event.keystroke.key.as_str() {
                            "up" => "ArrowUp",
                            "down" => "ArrowDown",
                            "pageup" => "PageUp",
                            "pagedown" => "PageDown",
                            "home" => "Home",
                            "end" => "End",
                            "enter" => "Enter",
                            "escape" => "Escape",
                            _ => return,
                        };
                        if editor.reference_panel_ui.focus.is_focused(window)
                            && editor.run_reference_panel_key(id, key, window, cx)
                        {
                            window.prevent_default();
                            cx.stop_propagation();
                        }
                    }),
                )
                .flex()
                .flex_col()
                .flex_none()
                .w_full()
                .max_h(relative(0.4))
                .overflow_hidden()
                .border_t(border_width)
                .border_color(border_color)
                .bg(theme.panel)
                .text_sm()
                .text_color(theme.panel_text())
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .justify_between()
                        .items_center()
                        .px_3()
                        .py_1()
                        .child("Find References")
                        .child(
                            div()
                                .debug_selector(|| "source-lsp-references-close".into())
                                .cursor_pointer()
                                .rounded_sm()
                                .px_2()
                                .py_1()
                                .hover(|button| button.bg(theme.selected))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |editor, _, window, cx| {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                        editor.run_reference_panel_key(id, "Escape", window, cx);
                                    }),
                                )
                                .child("Close"),
                        ),
                )
                .child(
                    div()
                        .id("source-lsp-reference-rows")
                        .min_h_0()
                        .max_h(px(160.))
                        .overflow_y_scroll()
                        .track_scroll(&self.reference_panel_ui.scroll)
                        .children(panel.rows.into_iter().enumerate().map(|(index, row)| {
                            reference_row(row, index == panel.selected, theme)
                                .id(("source-lsp-reference-row", index))
                                .debug_selector(move || format!("source-lsp-reference-row-{index}"))
                                .cursor_pointer()
                                .on_click(cx.listener(move |editor, _, window, cx| {
                                    if !editor.is_reviewing_changes()
                                        && editor.activate_source_lsp_reference_row(id, index, cx)
                                    {
                                        window.focus(&editor.focus);
                                        window.prevent_default();
                                        cx.stop_propagation();
                                    }
                                }))
                        })),
                );
            #[cfg(test)]
            let panel = crate::editor::one_dark_tests::panel_menu::probe(
                panel.debug_selector(|| "source-lsp-references-panel".into()),
                crate::editor::one_dark_tests::panel_menu::Role::References,
            );
            panel.into_any_element()
        })
    }

    fn run_reference_panel_key(
        &mut self,
        id: PanelId,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_reviewing_changes() || !self.source_lsp_reference_key(id, key, cx) {
            return false;
        }
        if matches!(key, "Escape" | "Enter") && self.reference_panel_ui.focus.is_focused(window) {
            window.focus(&self.focus);
        }
        true
    }
}

fn reference_row(row: ReferenceRow, selected: bool, theme: Theme) -> Div {
    let matched = row.before.len()..row.before.len() + row.matched.len();
    let line = row.before + &row.matched + &row.after;
    let preview = StyledText::new(line).with_highlights((!matched.is_empty()).then_some((
        matched,
        HighlightStyle {
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        },
    )));
    div()
        .px_3()
        .py_1()
        .flex()
        .gap_2()
        .when(selected, |row| row.bg(theme.selected))
        .child(
            div()
                .flex_none()
                .text_color(theme.muted)
                .child(format!("{}:", row.line_number)),
        )
        .child(div().min_w_0().truncate().child(preview))
}
