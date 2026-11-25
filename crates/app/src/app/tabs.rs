//! The scrollable tab strip and labels shared with unsaved-change prompts.

use gpui::{Context, Div, FontWeight, SharedString, Stateful, Window, div, prelude::*, px, rgb};

use super::{DocumentTab, Intent, Shell, filename};

impl Shell {
    pub(super) fn tab_title(&self, tab: &DocumentTab, index: usize) -> String {
        let Some(path) = tab.path() else {
            return if self.tabs.iter().filter(|tab| tab.file.is_none()).count() > 1 {
                format!("Untitled.md (tab {})", index + 1)
            } else {
                "Untitled.md".into()
            };
        };
        if self.tabs.iter().any(|other| {
            other.id != tab.id
                && other
                    .path()
                    .is_some_and(|other_path| other_path.file_name() == path.file_name())
        }) || (path.file_name().is_some_and(|name| name == "Untitled.md")
            && self.tabs.iter().any(|other| other.file.is_none()))
        {
            if let Some(relative) = self
                .workspace
                .as_ref()
                .and_then(|workspace| path.strip_prefix(workspace.root()).ok())
            {
                return if relative.components().count() == 1 {
                    format!("./{}", relative.display())
                } else {
                    relative.display().to_string()
                };
            }
            return path.display().to_string();
        }
        filename(path)
    }

    pub(super) fn tab_strip(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if self.tabs.is_empty() {
            return None;
        }
        let mut row = div()
            .id("document-tabs")
            .debug_selector(|| "document-tabs".into())
            .flex_none()
            .flex()
            .gap_1()
            .px_1()
            .py_1()
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&self.tab_scroll)
            .bg(rgb(0xf2f5fa))
            .border_b_1()
            .border_color(rgb(0xd3dce9));
        for (index, tab) in self.tabs.iter().enumerate() {
            let id = tab.id;
            let selected = self.selected == Some(id);
            let dirty = tab.dirty(cx);
            let title = self.tab_title(tab, index);
            let tip = SharedString::from(if dirty {
                format!("{title} — Unsaved changes")
            } else {
                title.clone()
            });
            let close_tip = SharedString::from(format!("Close {title}"));
            row = row.child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .rounded_md()
                    .bg(rgb(if selected { 0xe4eeff } else { 0xf9fbff }))
                    .text_color(rgb(if selected { 0x234e9a } else { 0x475569 }))
                    .child(
                        div()
                            .id(("tab", id))
                            .debug_selector(move || format!("tab-{id}"))
                            .h(px(30.))
                            .max_w(px(220.))
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0xdce8fa)))
                            .when(selected, |tab| tab.font_weight(FontWeight::SEMIBOLD))
                            .when(dirty, |tab| {
                                tab.child(div().flex_none().text_color(rgb(0x9a3412)).child("●"))
                            })
                            .child(div().min_w_0().truncate().child(title))
                            .tooltip(move |_, cx| cx.new(|_| TabTooltip(tip.clone())).into())
                            .on_click(cx.listener(move |shell, _, window, cx| {
                                if shell.active.is_none() && shell.allow_document_action(false, cx)
                                {
                                    shell.error = None;
                                    shell.select_tab(id, window, cx);
                                }
                            })),
                    )
                    .child(
                        div()
                            .id(("close-tab", id))
                            .debug_selector(move || format!("close-tab-{id}"))
                            .flex_none()
                            .size(px(30.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0xdce8fa)))
                            .child("×")
                            .tooltip(move |_, cx| cx.new(|_| TabTooltip(close_tip.clone())).into())
                            .on_click(cx.listener(move |shell, _, window, cx| {
                                shell.start(Intent::CloseTab(id), window, cx);
                            })),
                    ),
            );
        }
        Some(row)
    }
}

struct TabTooltip(SharedString);

impl Render for TabTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .bg(rgb(0x0f172a))
            .text_color(rgb(0xffffff))
            .text_xs()
            .rounded_md()
            .px_2()
            .py_1()
            .child(self.0.clone())
    }
}
