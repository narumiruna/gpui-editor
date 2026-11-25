//! Resizable workspace sidebar and its navigation views.

use std::{
    collections::{BTreeMap, HashSet},
    io,
    path::{Path, PathBuf},
};

use gpui::{
    Context, CursorStyle, Div, Pixels, ScrollHandle, Stateful, Window, div, prelude::*, px,
};

use super::{DocumentTab, Intent, Shell, filename, window_chrome};
use crate::{
    app::developer::Picker,
    editor::toolbar,
    theme::Theme,
    workspace::{DirectoryEntry, EntryKind, Workspace},
};

mod navigation;
mod search;
mod strip;

use search::Search;
use strip::View;

pub(super) type Snapshot = BTreeMap<PathBuf, Vec<DirectoryEntry>>;

pub(super) fn snapshot(workspace: &Workspace, loaded: Vec<PathBuf>) -> io::Result<Snapshot> {
    let root = workspace.root();
    let mut snapshot = BTreeMap::from([(root.to_owned(), workspace.directory_entries(root)?)]);
    // `loaded` comes from the ordered map, so parents are refreshed before descendants.
    for path in loaded.into_iter().filter(|path| path != root) {
        if path
            .parent()
            .and_then(|parent| snapshot.get(parent))
            .is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry.kind == EntryKind::Folder && entry.path == path)
            })
            && let Ok(entries) = workspace.directory_entries(&path)
        {
            snapshot.insert(path, entries);
        }
    }
    workspace.check_root()?;
    Ok(snapshot)
}

pub(super) struct Sidebar {
    pub(super) visible: bool,
    width: Pixels,
    entries: Snapshot,
    expanded: HashSet<PathBuf>,
    scroll: ScrollHandle,
    recent_scroll: ScrollHandle,
    pins_scroll: ScrollHandle,
    outline_scroll: ScrollHandle,
    view: View,
    order: [View; 5],
    recent: Vec<PathBuf>,
    pins: Vec<PathBuf>,
    pub(super) search: Search,
}

impl Sidebar {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Shell>) -> Self {
        Self {
            visible: true,
            width: px(198.),
            entries: Snapshot::new(),
            expanded: HashSet::new(),
            scroll: ScrollHandle::new(),
            recent_scroll: ScrollHandle::new(),
            pins_scroll: ScrollHandle::new(),
            outline_scroll: ScrollHandle::new(),
            view: View::Tree,
            order: View::DEFAULT_ORDER,
            recent: Vec::new(),
            pins: Vec::new(),
            search: Search::new(window, cx),
        }
    }

    pub(super) fn width(&self, viewport: Pixels) -> Pixels {
        self.width.clamp(px(184.), (viewport * 0.45).max(px(184.)))
    }

    pub(super) fn reset(
        &mut self,
        root: &Path,
        entries: Snapshot,
        window: &mut Window,
        cx: &mut Context<Shell>,
    ) {
        self.entries = entries;
        self.expanded = HashSet::from([root.to_owned()]);
        self.scroll = ScrollHandle::new();
        self.recent_scroll = ScrollHandle::new();
        self.pins_scroll = ScrollHandle::new();
        self.outline_scroll = ScrollHandle::new();
        self.recent.clear();
        self.pins.clear();
        self.search = Search::new(window, cx);
    }

    pub(super) fn record_open(&mut self, path: &Path) {
        self.recent.retain(|entry| entry != path);
        self.recent.insert(0, path.to_owned());
        self.recent.truncate(50);
    }

    pub(super) fn refresh(&mut self, entries: Snapshot) {
        self.expanded.retain(|path| entries.contains_key(path));
        self.entries = entries;
    }

    pub(super) fn expand(&mut self, path: PathBuf, entries: Vec<DirectoryEntry>) {
        self.expanded.insert(path.clone());
        self.entries.insert(path, entries);
    }

    pub(super) fn loaded(&self) -> Vec<PathBuf> {
        self.entries.keys().cloned().collect()
    }

    #[cfg(test)]
    pub(super) fn paths(&self) -> (&[PathBuf], &[PathBuf]) {
        (&self.recent, &self.pins)
    }

    #[cfg(test)]
    pub(super) fn contains(&self, path: &Path) -> bool {
        path.parent()
            .and_then(|parent| self.entries.get(parent))
            .is_some_and(|entries| entries.iter().any(|entry| entry.path == path))
    }

    pub(super) fn visible_rows<'a>(&'a self, root: &'a Path) -> Vec<(&'a Path, EntryKind, usize)> {
        let mut rows = Vec::new();
        let mut pending = vec![(root, EntryKind::Folder, 0)];
        while let Some((path, kind, depth)) = pending.pop() {
            rows.push((path, kind, depth));
            if self.expanded.contains(path)
                && let Some(entries) = self.entries.get(path)
            {
                pending.extend(
                    entries
                        .iter()
                        .rev()
                        .map(|entry| (entry.path.as_path(), entry.kind, depth + 1)),
                );
            }
        }
        rows
    }
}

fn sidebar_hint(text: &'static str, theme: Theme) -> Div {
    div().px_1().py_2().text_color(theme.muted).child(text)
}

struct Resize;

impl Shell {
    pub(super) fn sidebar(
        &self,
        width: Pixels,
        native_controls: Pixels,
        theme: Theme,
        picker: Option<&Picker>,
        cx: &Context<Self>,
    ) -> Div {
        let tools = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .h(window_chrome::TOP_ROW_HEIGHT)
            .pl(native_controls.max(px(8.)))
            .pr_2()
            .children(self.folder_tools(theme, picker, cx))
            .child(window_chrome::drag_region().flex_1().h_full())
            .child(self.sidebar_toggle(theme, picker, cx));
        let content = match self.sidebar.view {
            View::Tree => self.file_tree(theme, picker, cx).into_any_element(),
            View::Recent | View::Pins => self
                .sidebar_paths(self.sidebar.view, theme, picker, cx)
                .into_any_element(),
            View::Outline => self.sidebar_outline(theme, picker, cx).into_any_element(),
            View::Search => self.sidebar_search(theme, picker, cx).into_any_element(),
        };
        Picker::mark(
            picker,
            div()
                .debug_selector(|| "sidebar".into())
                .relative()
                .w(width)
                .h_full()
                .flex_none()
                .flex()
                .flex_col()
                .tab_group()
                .on_key_down(
                    cx.listener(|shell, event: &gpui::KeyDownEvent, window, cx| {
                        if !shell.can_change_sidebar(cx) {
                            return;
                        }
                        let key = &event.keystroke;
                        let modifiers = key.modifiers;
                        match key.key.as_str() {
                            "tab"
                                if !modifiers.platform
                                    && !modifiers.control
                                    && !modifiers.alt
                                    && !modifiers.function =>
                            {
                                if modifiers.shift {
                                    window.focus_prev();
                                } else {
                                    window.focus_next();
                                }
                            }
                            "escape" if !modifiers.modified() => shell.focus_document(window, cx),
                            _ => return,
                        }
                        window.prevent_default();
                        cx.stop_propagation();
                    }),
                )
                .bg(theme.panel)
                .border_r_1()
                .border_color(theme.border)
                .child(tools)
                .child(self.sidebar_strip(width, theme, picker, cx))
                .child(content)
                .child(
                    div()
                        .id("resize-sidebar")
                        .debug_selector(|| "resize-sidebar".into())
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .w(px(5.))
                        .cursor(CursorStyle::ResizeLeftRight)
                        .hover(move |style| style.bg(theme.accent))
                        .on_drag(Resize, |_, _, _, cx| cx.new(|_| Resize))
                        .on_drag_move(cx.listener(
                            |shell, event: &gpui::DragMoveEvent<Resize>, window, cx| {
                                shell.sidebar.width = event.event.position.x + px(2.);
                                shell.sidebar.width =
                                    shell.sidebar.width(window.viewport_size().width);
                                cx.notify();
                            },
                        )),
                ),
            "sidebar",
            "Workspace sidebar",
        )
    }

    fn file_tree(
        &self,
        theme: Theme,
        picker: Option<&Picker>,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let mut explorer = div()
            .id("workspace-tree")
            .debug_selector(|| "workspace-tree".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.sidebar.scroll)
            .px_2()
            .pb_2();
        if let Some(workspace) = &self.workspace {
            let active_file = self.current().and_then(DocumentTab::path);
            let mut folder_index = 0;
            let mut file_index = 0;
            for (path, kind, depth) in self.sidebar.visible_rows(workspace.root()) {
                let active = active_file == Some(path);
                let expanded = self.sidebar.expanded.contains(path);
                let label = filename(path);
                let mut row = div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(26.))
                    .pl(px(4. + 12. * depth.min(7) as f32))
                    .pr_1()
                    .rounded_md()
                    .text_color(if active {
                        theme.accent
                    } else if kind == EntryKind::File {
                        theme.muted
                    } else {
                        theme.text
                    })
                    .when(active, |row| row.bg(theme.selected));
                row = match kind {
                    EntryKind::Folder => row.child(
                        toolbar::icon(
                            if expanded {
                                "Expanded tree"
                            } else {
                                "Collapsed tree"
                            },
                            12.,
                        )
                        .text_color(theme.muted),
                    ),
                    _ => row.child(div().w(px(12.)).flex_none()),
                };
                let glyph = match kind {
                    EntryKind::Folder if expanded => "Open Folder…",
                    EntryKind::Folder => "Workspace",
                    _ => "File",
                };
                let color = if active { theme.accent } else { theme.muted };
                let row = row
                    .group("sidebar-document")
                    .child(toolbar::icon(glyph, 15.).text_color(color))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .when(kind == EntryKind::Markdown, |row| {
                        row.child(self.pin_button(path, theme, picker, cx))
                    });
                if kind == EntryKind::File {
                    explorer = explorer.child(
                        row.id(gpui::ElementId::Path(path.to_path_buf().into()))
                            .tooltip(|_, cx| {
                                cx.new(|_| toolbar::Tooltip("Only Markdown files can be opened"))
                                    .into()
                            }),
                    );
                    continue;
                }
                let (selector, index, target, description) = match kind {
                    EntryKind::Folder => {
                        let index = folder_index;
                        folder_index += 1;
                        (
                            "workspace-folder",
                            index,
                            "sidebar.folder",
                            "Workspace folder row",
                        )
                    }
                    EntryKind::Markdown => {
                        let index = file_index;
                        file_index += 1;
                        (
                            "workspace-file",
                            index,
                            "sidebar.file",
                            "Workspace file row",
                        )
                    }
                    EntryKind::File => unreachable!(),
                };
                let path = path.to_owned();
                let tip = path
                    .strip_prefix(workspace.root())
                    .ok()
                    .filter(|relative| !relative.as_os_str().is_empty())
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                explorer = explorer.child(Picker::mark(
                    picker,
                    row.id(gpui::ElementId::Path(path.clone().into()))
                        .debug_selector(move || format!("{selector}-{index}"))
                        .cursor_pointer()
                        .hover(move |row| row.bg(theme.selected))
                        .tooltip(move |_, cx| {
                            cx.new(|_| toolbar::TextTooltip(tip.clone().into())).into()
                        })
                        .on_click(cx.listener(move |shell, _, window, cx| match kind {
                            EntryKind::Folder if shell.sidebar.expanded.remove(&path) => {
                                cx.notify()
                            }
                            EntryKind::Folder => {
                                shell.start(Intent::ExpandFolder(path.clone()), window, cx)
                            }
                            EntryKind::Markdown => {
                                shell.start(Intent::Open(path.clone()), window, cx)
                            }
                            EntryKind::File => {}
                        })),
                    target,
                    description,
                ));
            }
        } else {
            explorer = explorer.child(sidebar_hint(
                "Open a workspace folder to browse files.",
                theme,
            ));
        }
        explorer
    }

    pub(super) fn folder_tools(
        &self,
        theme: Theme,
        picker: Option<&Picker>,
        cx: &Context<Self>,
    ) -> [gpui::Stateful<Div>; 2] {
        let enabled = self.active.is_none();
        [
            toolbar::marked_button("Open Folder…", enabled, theme, picker).when(
                enabled,
                |button| {
                    button.on_click(cx.listener(|shell, _, window, cx| {
                        shell.start(Intent::ChooseFolder, window, cx)
                    }))
                },
            ),
            toolbar::marked_button("New note", enabled, theme, picker).when(enabled, |button| {
                button.on_click(
                    cx.listener(|shell, _, window, cx| shell.start(Intent::New, window, cx)),
                )
            }),
        ]
    }

    pub(super) fn sidebar_toggle(
        &self,
        theme: Theme,
        picker: Option<&Picker>,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let label = if self.sidebar.visible {
            "Hide Sidebar"
        } else {
            "Show Sidebar"
        };
        toolbar::marked_button(label, self.active.is_none(), theme, picker)
            .on_click(cx.listener(|shell, _, window, cx| shell.toggle_sidebar(window, cx)))
    }

    fn can_change_sidebar(&self, cx: &gpui::App) -> bool {
        self.active.is_none()
            && self.palette.is_none()
            && !self.developer.blocks_document_actions()
            && !self.is_composing(cx)
    }

    pub(super) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.is_none()
            && self.palette.is_none()
            && !self.developer.blocks_document_actions()
            && !self.sidebar.search.is_composing(cx)
        {
            self.sidebar.visible = !self.sidebar.visible;
            if !self.sidebar.visible && !self.is_composing(cx) {
                self.focus_document(window, cx);
            }
            cx.notify();
        }
    }
}

impl Render for Resize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
