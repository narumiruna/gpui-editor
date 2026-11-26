//! Editor-local toolbar, document surface, and contextual notices.

use gpui::{Context, Div, div, prelude::*, px};

use super::{Intent, Shell, developer::Picker, window_chrome};
use crate::{editor::toolbar, theme::Theme};

const TOOLBAR_HEIGHT: gpui::Pixels = px(40.);

impl Shell {
    pub(super) fn editor_pane(
        &self,
        native_controls: gpui::Pixels,
        theme: Theme,
        picker: Option<&Picker>,
        cx: &Context<Self>,
    ) -> Div {
        let busy = self.active.is_some();
        let current = self.current();
        let markdown = current.is_some_and(|tab| tab.editor.read(cx).is_markdown());
        let can_save = current.is_some() && !busy;
        let mut actions = toolbar::group();
        if !self.sidebar.visible {
            actions = actions.children(self.folder_tools(theme, picker, cx));
        }
        actions = actions.child(
            toolbar::marked_button("Save", can_save, theme, picker).when(can_save, |button| {
                button.on_click(
                    cx.listener(|shell, _, window, cx| shell.start(Intent::Save, window, cx)),
                )
            }),
        );
        let mut tools = div()
            .id("editor-actions")
            .debug_selector(|| "editor-actions".into())
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_2()
            .px_2()
            .overflow_x_scroll()
            .child(actions);
        let mut more_button = None;
        let mut more_tools = None;
        let mut remote_images = None;
        if let Some(current) = current.filter(|_| markdown) {
            let edit_enabled = !busy && !current.editor.read(cx).is_composing();
            tools = tools.children(toolbar::primary_groups(
                &current.editor,
                edit_enabled,
                theme,
                picker,
            ));
            let label = if self.show_more_tools {
                "Fewer tools"
            } else {
                "More tools"
            };
            more_button = Some(toolbar::marked_button(label, !busy, theme, picker).when(
                !busy,
                |button| {
                    button.on_click(cx.listener(|shell, _, _, cx| {
                        shell.show_more_tools = !shell.show_more_tools;
                        cx.notify();
                    }))
                },
            ));
            if self.show_more_tools {
                more_tools = Some(
                    toolbar::more_group(&current.editor, edit_enabled, theme, picker)
                        .absolute()
                        .top(window_chrome::TOP_ROW_HEIGHT + TOOLBAR_HEIGHT)
                        .right_2()
                        .occlude()
                        .p_1()
                        .rounded_md()
                        .bg(theme.toolbar)
                        .border_1()
                        .border_color(theme.border),
                );
            }
            if current.editor.read(cx).has_blocked_images() {
                let editor = current.editor.clone();
                remote_images = Some(
                    toolbar::marked_button(
                        "Load remote images for this document",
                        !busy,
                        theme,
                        picker,
                    )
                    .when(!busy, |button| {
                        button.on_click(move |_, _, cx| {
                            editor.update(cx, |editor, cx| editor.allow_remote_images(cx));
                        })
                    }),
                );
            }
        }
        let toolbar = div()
            .debug_selector(|| "editor-toolbar".into())
            .flex_none()
            .flex()
            .items_center()
            .h(TOOLBAR_HEIGHT)
            .pr_2()
            .bg(theme.toolbar)
            .border_b_1()
            .border_color(theme.border)
            .child(tools)
            .children(remote_images)
            .children(more_button)
            .children(self.developer_icon(!busy, theme, cx));
        let body = div()
            .debug_selector(|| "document-body".into())
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden();
        let body = match current {
            Some(current) => body.child(current.editor.clone()),
            None => body
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_color(theme.muted)
                .child(if self.workspace.is_some() {
                    "Select a file or create a new note."
                } else {
                    "Open a workspace folder or create a new document."
                }),
        };
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(self.tab_strip(native_controls, theme, picker, cx))
            .child(toolbar)
            .when_some(self.error.clone(), |pane, error| {
                pane.child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_2()
                        .bg(theme.error_bg)
                        .text_color(theme.error)
                        .child(error),
                )
            })
            .when_some(self.developer_status(), |pane, text| {
                pane.child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(theme.muted)
                        .child(text.to_owned()),
                )
            })
            .child(Picker::mark(
                picker,
                body,
                "editor.surface",
                if markdown {
                    "Markdown editing surface"
                } else {
                    "Text editing surface"
                },
            ))
            .children(more_tools)
    }
}
