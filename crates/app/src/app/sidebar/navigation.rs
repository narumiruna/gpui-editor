use std::path::Path;

use gpui::{Context, Div, ScrollHandle, Stateful, div, prelude::*, px};

use super::{DocumentTab, Intent, Picker, Shell, Theme, View, sidebar_hint, toolbar};

pub(super) fn list(id: &'static str, scroll: &ScrollHandle) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .px_2()
        .pb_2()
}

pub(super) fn label(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

pub(super) fn row(theme: Theme, selected: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .min_h(px(26.))
        .px_1()
        .cursor_pointer()
        .rounded_md()
        .text_color(if selected { theme.accent } else { theme.text })
        .when(selected, |row| row.bg(theme.selected))
        .hover(move |row| row.bg(theme.selected))
}

impl Shell {
    fn toggle_pin(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.can_change_sidebar(cx) && self.workspace.is_some() {
            if let Some(index) = self.sidebar.pins.iter().position(|p| p == path) {
                self.sidebar.pins.remove(index);
            } else {
                self.sidebar.pins.push(path.to_owned());
            }
            cx.notify();
        }
    }

    pub(super) fn pin_button(
        &self,
        path: &Path,
        theme: Theme,
        picker: Option<&Picker>,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let name = if self.sidebar.pins.iter().any(|p| p == path) {
            "Unpin file"
        } else {
            "Pin file"
        };
        let click_path = path.to_owned();
        Picker::mark(
            picker,
            div()
                .id("pin-file")
                .debug_selector(move || name.into())
                .tab_index(0)
                .flex_none()
                .flex()
                .justify_center()
                .items_center()
                .size(px(22.))
                .rounded_md()
                .opacity(0.)
                .group_hover("sidebar-document", |button| button.opacity(1.))
                .focus(move |button| button.opacity(1.).bg(theme.selected))
                .tooltip(move |_, cx| cx.new(|_| toolbar::Tooltip(name)).into())
                .child(toolbar::icon(name, 14.).text_color(theme.accent))
                .on_click(cx.listener(move |shell, _, _, cx| {
                    cx.stop_propagation();
                    shell.toggle_pin(&click_path, cx);
                })),
            "sidebar.pin",
            name,
        )
    }

    pub(super) fn sidebar_paths(
        &self,
        view: View,
        theme: Theme,
        picker: Option<&Picker>,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let pins = view == View::Pins;
        let (id, paths, scroll, empty) = if pins {
            (
                "workspace-pins",
                &self.sidebar.pins,
                &self.sidebar.pins_scroll,
                "Pin a file from Tree to keep it here.",
            )
        } else {
            (
                "workspace-recent",
                &self.sidebar.recent,
                &self.sidebar.recent_scroll,
                "No recently opened files.",
            )
        };
        let el = list(id, scroll);
        let Some(workspace) = &self.workspace else {
            return el.child(sidebar_hint(
                "Open a workspace folder to list files.",
                theme,
            ));
        };
        if paths.is_empty() {
            return el.child(sidebar_hint(empty, theme));
        }
        let active = self.current().and_then(DocumentTab::path);
        el.children(paths.iter().enumerate().map(|(index, path)| {
            let name = label(path, workspace.root());
            let tip = name.clone();
            let open = path.to_owned();
            Picker::mark(
                picker,
                row(theme, active == Some(path))
                    .group("sidebar-document")
                    .id(gpui::ElementId::Path(open.clone().into()))
                    .debug_selector(move || format!("{id}-{index}"))
                    .child(div().flex_1().min_w_0().truncate().child(name))
                    .when(pins, |row| {
                        row.child(self.pin_button(path, theme, picker, cx))
                    })
                    .tooltip(move |_, cx| {
                        cx.new(|_| toolbar::TextTooltip(tip.clone().into())).into()
                    })
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.start(Intent::Open(open.clone()), window, cx)
                    })),
                "sidebar.file",
                "Workspace file row",
            )
        }))
    }

    pub(super) fn sidebar_outline(
        &self,
        theme: Theme,
        picker: Option<&Picker>,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let el = list("document-outline", &self.sidebar.outline_scroll);
        let Some(tab) = self.current() else {
            return el.child(sidebar_hint("Open a document to see its outline.", theme));
        };
        let editor = tab.editor.read(cx);
        if !editor.is_markdown() {
            return el.child(sidebar_hint(
                "Outline is available for Markdown files.",
                theme,
            ));
        }
        let headings = editor.headings();
        if headings.is_empty() {
            return el.child(
                sidebar_hint("This document has no headings.", theme)
                    .debug_selector(|| "outline-no-headings".into()),
            );
        }
        let id = tab.id;
        el.children(headings.into_iter().enumerate().map(|(index, heading)| {
            let offset = heading.offset;
            let name = if heading.label.is_empty() {
                "Untitled heading".to_owned()
            } else {
                heading.label
            };
            let tip = name.clone();
            Picker::mark(
                picker,
                row(theme, false)
                    .id(index)
                    .debug_selector(move || format!("outline-heading-{index}"))
                    .pl(px(4. + 12. * f32::from(heading.level.saturating_sub(1))))
                    .child(div().min_w_0().truncate().child(name))
                    .tooltip(move |_, cx| {
                        cx.new(|_| toolbar::TextTooltip(tip.clone().into())).into()
                    })
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        if shell.can_change_sidebar(cx) && shell.selected == Some(id) {
                            shell.focus_document(window, cx);
                            if let Some(tab) = shell.current() {
                                tab.editor
                                    .update(cx, |editor, cx| editor.reveal_source(offset, cx));
                            }
                        }
                    })),
                "sidebar.heading",
                "Document heading row",
            )
        }))
    }
}
