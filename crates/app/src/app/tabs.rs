//! The scrollable tab strip and labels shared with unsaved-change prompts.

use gpui::{Context, Div, FontWeight, Pixels, SharedString, Window, div, prelude::*, px};

use super::{DocumentTab, Intent, Shell, developer::Picker, filename, window_chrome};
use crate::{editor::toolbar::TextTooltip, theme::Theme};

const DRAG_AREA_WIDTH: Pixels = px(16.);

pub(super) fn strip_width(
    viewport: Pixels,
    sidebar: Pixels,
    sidebar_visible: bool,
    controls: Pixels,
) -> Pixels {
    viewport
        - DRAG_AREA_WIDTH
        - if sidebar_visible {
            sidebar
        } else {
            controls.max(px(8.)) + px(30.)
        }
}

impl Shell {
    pub(super) fn reveal_tab_at_width(
        &self,
        width: Pixels,
        window: &mut Window,
        cx: &Context<Self>,
    ) {
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| Some(tab.id) == self.selected)
        else {
            return;
        };
        // ScrollHandle::scroll_to_item uses the previous viewport width. When
        // resizing an existing strip, use the new width and old tab positions.
        if let Some(tab) = self.tab_scroll.bounds_for_item(index) {
            let old = self.tab_scroll.bounds();
            let left = tab.left() - old.left();
            let right = tab.right() - old.left();
            let mut offset = self.tab_scroll.offset();
            if left + offset.x < px(0.) {
                offset.x = -left;
            } else if right + offset.x > width {
                offset.x = width - right;
            }
            self.tab_scroll.set_offset(offset);
        } else {
            // Session restore can select a tab before its strip has a layout.
            cx.on_next_frame(window, |shell, _, cx| {
                if let Some(index) = shell
                    .tabs
                    .iter()
                    .position(|tab| Some(tab.id) == shell.selected)
                {
                    shell.tab_scroll.scroll_to_item(index);
                    cx.notify();
                }
            });
        }
    }

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

    pub(super) fn tab_strip(
        &self,
        native_controls: gpui::Pixels,
        theme: Theme,
        picker: Option<&Picker>,
        cx: &Context<Self>,
    ) -> Div {
        let mut row = div()
            .id("document-tabs")
            .debug_selector(|| "document-tabs".into())
            .size_full()
            .flex_1()
            .flex()
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&self.tab_scroll);
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
                    .px_1()
                    .border_b_2()
                    .border_color(if selected {
                        theme.accent
                    } else {
                        gpui::transparent_black().into()
                    })
                    .when(selected, |tab| tab.bg(theme.surface))
                    .text_color(if selected { theme.accent } else { theme.muted })
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
                            .hover(move |style| style.bg(theme.selected))
                            .when(selected, |tab| tab.font_weight(FontWeight::SEMIBOLD))
                            .when(dirty, |tab| {
                                tab.child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(theme.accent)
                                        .child("●"),
                                )
                            })
                            .child(div().min_w_0().truncate().child(title))
                            .tooltip(move |_, cx| cx.new(|_| TextTooltip(tip.clone())).into())
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
                            .size(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |style| style.bg(theme.selected))
                            .child("×")
                            .tooltip(move |_, cx| cx.new(|_| TextTooltip(close_tip.clone())).into())
                            .on_click(cx.listener(move |shell, _, window, cx| {
                                shell.start(Intent::CloseTab(id), window, cx);
                            })),
                    ),
            );
        }
        div()
            .flex_none()
            .flex()
            .items_center()
            .h(window_chrome::TOP_ROW_HEIGHT)
            .bg(theme.panel)
            .border_b_1()
            .border_color(theme.border)
            .when(!self.sidebar.visible, |bar| {
                bar.pl(native_controls.max(px(8.)))
                    .child(self.sidebar_toggle(theme, picker, cx))
            })
            .child(row.child(window_chrome::drag_region().flex_1().h_full()))
            .child(
                window_chrome::drag_region()
                    .w(DRAG_AREA_WIDTH)
                    .h_full()
                    .flex_none(),
            )
    }
}
