use gpui::{Context, Div, Pixels, Window, div, prelude::*, px};

use super::{Picker, Shell, Theme, toolbar};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum View {
    Tree,
    Recent,
    Pins,
    Outline,
    Search,
}

impl View {
    pub(super) const DEFAULT_ORDER: [Self; 5] = [
        Self::Tree,
        Self::Recent,
        Self::Pins,
        Self::Outline,
        Self::Search,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Tree => "Tree",
            Self::Recent => "Recent",
            Self::Pins => "Pins",
            Self::Outline => "Outline",
            Self::Search => "Search",
        }
    }
}

struct SidebarDrag(View);

impl Render for SidebarDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Shell {
    fn select_sidebar_view(&mut self, view: View, window: &mut Window, cx: &mut Context<Self>) {
        if self.can_change_sidebar(cx) {
            self.sidebar.view = view;
            if view == View::Search && self.workspace.is_some() {
                self.sidebar.search.focus(window, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn sidebar_strip(
        &self,
        width: Pixels,
        theme: Theme,
        picker: Option<&Picker>,
        cx: &Context<Self>,
    ) -> Div {
        let show_labels = width >= px(360.);
        div()
            .debug_selector(|| "sidebar-tabs".into())
            .flex_none()
            .flex()
            .gap(px(2.))
            .pl(px(6.))
            .pr(px(8.))
            .pb_1()
            .children(self.sidebar.order.map(|view| {
                let label = view.label();
                let selected = view == self.sidebar.view;
                let tab = div()
                    .id(label)
                    .debug_selector(move || format!("sidebar-tab-{label}"))
                    .relative()
                    .tab_index(0)
                    .border_1()
                    .border_color(theme.panel)
                    .focus(move |tab| tab.border_color(theme.accent))
                    .flex_1()
                    .min_w_0()
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .rounded_md()
                    .text_xs()
                    .text_color(if selected { theme.accent } else { theme.text })
                    .when(selected, |tab| tab.bg(theme.selected))
                    .hover(move |tab| tab.bg(theme.selected))
                    .cursor_pointer()
                    .tooltip(move |_, cx| cx.new(|_| toolbar::Tooltip(label)).into())
                    .child(toolbar::icon(label, 15.))
                    .when(show_labels, |tab| tab.child(label))
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.select_sidebar_view(view, window, cx);
                    }))
                    .on_drag(SidebarDrag(view), |drag, _, _, cx| {
                        cx.new(|_| SidebarDrag(drag.0))
                    })
                    .children([false, true].map(|after| {
                        let edge = if after { "after" } else { "before" };
                        div()
                            .id(edge)
                            .debug_selector(move || format!("sidebar-drop-{label}-{edge}"))
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .w(gpui::relative(0.5))
                            .when(after, |zone| zone.right_0())
                            .when(!after, |zone| zone.left_0())
                            .drag_over::<SidebarDrag>(move |zone, drag, _, _| {
                                if drag.0 == view {
                                    zone
                                } else if after {
                                    zone.border_r_2().border_color(theme.accent)
                                } else {
                                    zone.border_l_2().border_color(theme.accent)
                                }
                            })
                            .on_drop(cx.listener(move |shell, drag: &SidebarDrag, _, cx| {
                                if shell.can_change_sidebar(cx) && drag.0 != view {
                                    let order = &mut shell.sidebar.order;
                                    let from = order.iter().position(|&v| v == drag.0).unwrap();
                                    let target = order.iter().position(|&v| v == view).unwrap();
                                    let to =
                                        target + usize::from(after) - usize::from(from < target);
                                    if from < to {
                                        order[from..=to].rotate_left(1);
                                    } else if from > to {
                                        order[to..=from].rotate_right(1);
                                    }
                                    cx.notify();
                                }
                            }))
                    }));
                Picker::mark(picker, tab, "sidebar.tab", label)
            }))
    }
}
