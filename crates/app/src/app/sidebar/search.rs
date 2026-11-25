use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering::Relaxed},
    },
};

use gpui::{App, Context, Div, Entity, ScrollHandle, Subscription, Window, div, prelude::*};

use super::{Intent, Picker, Shell, Theme, View, navigation, sidebar_hint, toolbar};
use crate::{
    app::developer::{SearchSubmitted, TextField},
    workspace::{SearchLocation, SearchResults},
};

#[derive(Default)]
enum Results {
    #[default]
    Idle,
    Pending,
    Complete(io::Result<SearchResults>),
}

pub(in crate::app) struct Search {
    field: Entity<TextField>,
    _subscriptions: [Subscription; 2],
    query: String,
    request: Option<Arc<AtomicBool>>,
    results: Results,
    scroll: ScrollHandle,
}

impl Search {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let field = cx.new(TextField::search);
        let updates = cx.observe(&field, |shell, field, cx| {
            if shell.sidebar.search.set_query(field.read(cx).text()) {
                cx.notify();
            }
        });
        let submit = cx.subscribe_in(
            &field,
            window,
            |shell, _, _: &SearchSubmitted, window, cx| {
                shell.start_sidebar_search(window, cx);
            },
        );
        Self {
            field,
            _subscriptions: [updates, submit],
            query: String::new(),
            request: None,
            results: Results::Idle,
            scroll: ScrollHandle::new(),
        }
    }

    pub(in crate::app) fn is_composing(&self, cx: &App) -> bool {
        self.field.read(cx).is_composing()
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &App) {
        self.field.read(cx).focus(window);
    }

    fn cancel(&mut self) {
        if let Some(request) = self.request.take() {
            request.store(true, Relaxed);
        }
    }

    fn set_query(&mut self, query: &str) -> bool {
        if self.query == query {
            return false;
        }
        self.query = query.to_owned();
        self.cancel();
        self.results = Results::Idle;
        self.scroll = ScrollHandle::new();
        true
    }

    #[cfg(test)]
    pub(in crate::app) fn idle_query(&self) -> Option<&str> {
        matches!(self.results, Results::Idle).then_some(self.query.as_str())
    }

    fn begin(&mut self) -> Arc<AtomicBool> {
        self.cancel();
        let request = Arc::new(AtomicBool::new(false));
        self.request = Some(request.clone());
        self.results = Results::Pending;
        self.scroll = ScrollHandle::new();
        request
    }

    fn finish(&mut self, request: &Arc<AtomicBool>, results: io::Result<SearchResults>) -> bool {
        if !self
            .request
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, request))
        {
            return false;
        }
        self.request = None;
        self.results = Results::Complete(results);
        true
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Shell {
    fn start_sidebar_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_change_sidebar(cx)
            || !self.sidebar.visible
            || self.sidebar.view != View::Search
        {
            return;
        }
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let search = &mut self.sidebar.search;
        let query = search.field.read(cx).text().to_owned();
        search.set_query(&query);
        if query.trim().is_empty() {
            cx.notify();
            return;
        }
        let request = search.begin();
        cx.notify();
        cx.spawn_in(window, async move |shell, cx| {
            let worker = request.clone();
            let target = workspace.clone();
            let results = cx
                .background_executor()
                .spawn(async move { target.search_markdown(&query, || worker.load(Relaxed)) })
                .await;
            let _ = shell.update_in(cx, |shell, _, cx| {
                if shell.workspace.as_ref() == Some(&workspace)
                    && shell.sidebar.search.finish(&request, results)
                {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(in crate::app) fn reveal_search_result(
        &mut self,
        location: Option<SearchLocation>,
        cx: &mut Context<Self>,
    ) {
        if let (Some(location), Some(tab)) = (location, self.current()) {
            let editor = tab.editor.clone();
            if let Some(offset) = location.offset_in(editor.read(cx).source()) {
                editor.update(cx, |editor, cx| editor.reveal_source(offset, cx));
            } else {
                self.error = Some(
                    "This search result is out of date. Search again to find the current location."
                        .into(),
                );
            }
        }
    }

    pub(super) fn sidebar_search(
        &self,
        theme: Theme,
        picker: Option<&Picker>,
        cx: &Context<Self>,
    ) -> Div {
        let search = &self.sidebar.search;
        let mut list = navigation::list("workspace-search-results", &search.scroll);
        let Some(workspace) = &self.workspace else {
            return div()
                .debug_selector(|| "workspace-search-empty".into())
                .flex_1()
                .min_h_0()
                .child(sidebar_hint(
                    "Open a workspace folder to search files.",
                    theme,
                ));
        };
        match &search.results {
            Results::Idle => {
                list = list.child(sidebar_hint(
                    "Search file names and saved Markdown with Enter.",
                    theme,
                ));
            }
            Results::Pending => {
                list = list.child(sidebar_hint("Searching…", theme));
            }
            Results::Complete(Err(_)) => {
                list = list.child(sidebar_hint(
                    "Could not search this workspace. Reopen the folder and try again.",
                    theme,
                ));
            }
            Results::Complete(Ok(results)) => {
                if results.truncated {
                    list = list.child(sidebar_hint(
                        "Search limit reached. Refine your query.",
                        theme,
                    ));
                }
                if results.skipped {
                    list = list.child(sidebar_hint(
                        "Some files or folders could not be searched.",
                        theme,
                    ));
                }
                if results.hits.is_empty() {
                    list = list.child(
                        sidebar_hint("No matches in the files searched.", theme)
                            .debug_selector(|| "search-no-matches".into()),
                    );
                }
                list = list.children(results.hits.iter().enumerate().map(|(index, hit)| {
                    let path = navigation::label(&hit.path, workspace.root());
                    let (title, detail) = match &hit.location {
                        Some(location) => (format!("{path}:{}", location.line), snippet(location)),
                        None => (path, "File name match".to_owned()),
                    };
                    let tip = title.clone();
                    let hit = hit.clone();
                    Picker::mark(
                        picker,
                        navigation::row(theme, false)
                            .id(index)
                            .debug_selector(move || format!("search-result-{index}"))
                            .py_1()
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_col()
                                    .flex()
                                    .child(div().truncate().child(title))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(theme.muted)
                                            .child(detail),
                                    ),
                            )
                            .tooltip(move |_, cx| {
                                cx.new(|_| toolbar::TextTooltip(tip.clone().into())).into()
                            })
                            .on_click(cx.listener(move |shell, _, window, cx| {
                                let path = hit.path.clone();
                                let intent = hit.location.clone().map_or_else(
                                    || Intent::Open(path.clone()),
                                    |location| Intent::OpenAt(path.clone(), location),
                                );
                                shell.start(intent, window, cx);
                            })),
                        "sidebar.search-result",
                        "Workspace search result",
                    )
                }));
            }
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .debug_selector(|| "workspace-search-input".into())
                    .flex_none()
                    .px_2()
                    .pb_1()
                    .child(search.field.clone()),
            )
            .child(list)
    }
}

fn snippet(location: &SearchLocation) -> String {
    let text = &location.text;
    let start = text[..location.column]
        .char_indices()
        .rev()
        .nth(12)
        .map_or(0, |(index, _)| index);
    let mut snippet = String::new();
    if start != 0 {
        snippet.push('…');
    }
    let mut chars = text[start..].chars();
    snippet.extend(chars.by_ref().take(100));
    if chars.next().is_some() {
        snippet.push('…');
    }
    snippet
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::EntityInputHandler as _;

    #[gpui::test]
    fn new_ime_text_invalidates_the_previous_search_before_commit(cx: &mut gpui::TestAppContext) {
        let (shell, cx) = cx.add_window_view(Shell::new);
        let (field, previous) = cx.update(|_, cx| {
            shell.update(cx, |shell, _| {
                let search = &mut shell.sidebar.search;
                (search.field.clone(), search.begin())
            })
        });
        cx.update(|window, cx| {
            field.update(cx, |field, cx| {
                field.replace_and_mark_text_in_range(None, "查詢", None, window, cx);
                assert!(field.is_composing());
            })
        });
        cx.update(|_, cx| {
            shell.update(cx, |shell, _| {
                let search = &mut shell.sidebar.search;
                assert!(previous.load(Relaxed));
                assert!(!search.finish(&previous, Ok(SearchResults::default())));
                assert_eq!(search.idle_query(), Some("查詢"));
            })
        });
    }

    #[gpui::test]
    fn sidebar_search_waits_for_confirmed_chinese_text_and_returns_to_the_editor(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::Focusable as _;

        let fixture = crate::app::tests::Fixture::new();
        let note = fixture.document("note.md", "已確認中文");
        let (shell, cx) = cx.add_window_view(Shell::new);
        fixture.open(cx, &shell);
        let search_tab = cx.debug_bounds("sidebar-tab-Search").unwrap().center();
        cx.simulate_click(search_tab, gpui::Modifiers::none());
        cx.run_until_parked();
        let field = cx.update(|window, cx| {
            let field = shell.read(cx).sidebar.search.field.clone();
            assert!(field.read(cx).focus_handle(cx).is_focused(window));
            field.update(cx, |field, cx| {
                field.replace_and_mark_text_in_range(None, "ㄓㄨㄥ", None, window, cx);
            });
            field
        });
        cx.simulate_keystrokes("enter");
        cx.update(|window, cx| {
            assert_eq!(shell.read(cx).sidebar.search.idle_query(), Some("ㄓㄨㄥ"));
            field.update(cx, |field, cx| {
                assert!(field.is_composing());
                field.replace_text_in_range(None, "中文", window, cx);
            });
        });
        cx.update(|_, cx| assert_eq!(shell.read(cx).sidebar.search.idle_query(), Some("中文")));
        cx.simulate_keystrokes("enter");
        let hit = cx.debug_bounds("search-result-0").unwrap().center();
        cx.simulate_click(hit, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let tab = shell.read(cx).current().unwrap();
            assert_eq!(tab.path(), Some(note.as_path()));
            assert!(tab.editor.read(cx).focus_handle(cx).is_focused(window));
            assert_eq!(tab.editor.read(cx).source(), "已確認中文");
        });
    }

    #[gpui::test]
    fn older_queries_cannot_replace_newer_results_or_survive_workspace_reset(
        cx: &mut gpui::TestAppContext,
    ) {
        let root = tempfile::tempdir().unwrap();
        let (shell, cx) = cx.add_window_view(Shell::new);
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                let search = &mut shell.sidebar.search;
                let older = search.begin();
                let newer = search.begin();
                assert!(older.load(Relaxed));
                assert!(search.finish(&newer, Ok(SearchResults::default())));
                assert!(!search.finish(&older, Err(io::ErrorKind::Interrupted.into())));
                assert!(matches!(search.results, Results::Complete(Ok(_))));

                let while_switching = search.begin();
                shell
                    .sidebar
                    .reset(root.path(), super::super::Snapshot::new(), window, cx);
                let search = &mut shell.sidebar.search;
                assert!(while_switching.load(Relaxed));
                assert!(!search.finish(&while_switching, Ok(SearchResults::default())));
                assert!(matches!(search.results, Results::Idle));
            })
        });
    }
}
