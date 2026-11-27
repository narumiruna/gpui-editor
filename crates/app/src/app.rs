//! The single-window editor shell. Filesystem operations run off the UI thread.

use std::path::{Path, PathBuf};

use gpui::{
    App, Application, Bounds, Context, Entity, FocusHandle, Pixels, ScrollHandle, Subscription,
    WeakEntity, Window, WindowBounds, WindowHandle, WindowOptions, actions, div, prelude::*, px,
    size,
};

use crate::{
    editor::{self, Editor, OpenLocalDocument, toolbar},
    theme::Theme,
    workspace::{DocumentFile, SearchLocation, Workspace},
};

pub(crate) mod developer;
mod editor_pane;
mod hotkeys;
mod operation;
mod palette;
#[cfg(target_os = "macos")]
mod quit;
#[cfg(target_os = "windows")]
mod quit {
    pub(super) fn after_confirmation(cx: &gpui::App) {
        cx.quit();
    }
}
mod session;
mod sidebar;
mod state_file;
mod tabs;
#[cfg(test)]
mod tests;
mod window_chrome;

use developer::{Developer, TextField};
use hotkeys::Hotkeys;
use palette::Palette;
use sidebar::Sidebar;

actions!(
    shell,
    [
        ChooseFolder,
        NewDocument,
        SaveDocument,
        Close,
        CloseWindow,
        ToggleSidebar,
        Quit,
        Preferences,
        OpenPalette
    ]
);

pub fn run() {
    let hotkeys = Hotkeys::load(state_file::path("hotkeys.json"));
    let app = Application::new().with_assets(editor::ToolbarAssets);
    app.run(move |cx| {
        #[cfg(target_os = "macos")]
        editor::announcements::install_native_delivery(cx);
        editor::bind_keys(cx);
        hotkeys.bindings.bind(cx);
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1040.), px(720.)),
                    cx,
                ))),
                window_min_size: Some(size(px(560.), px(360.))),
                titlebar: Some(window_chrome::titlebar()),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let mut shell = Shell::new(window, cx);
                    shell.hotkeys = hotkeys;
                    shell
                })
            },
        );
        let Ok(window) = window else {
            eprintln!("Could not open the editor window: {}", window.unwrap_err());
            cx.quit();
            return;
        };
        #[cfg(target_os = "macos")]
        if let Err(error) = quit::install(cx) {
            eprintln!("Could not protect unsaved changes on macOS quit: {error}");
            cx.quit();
            return;
        }
        bind_quit(window, cx);
        if let Some(path) = state_file::path("session.json") {
            let _ = window.update(cx, |shell, window, cx| {
                shell.restore_session(path, window, cx)
            });
        }
        set_menus(cx);
        cx.activate(true);
    });
}

fn set_menus(cx: &App) {
    use gpui::{Menu, MenuItem, OsAction};

    cx.set_menus(vec![
        Menu {
            name: "GPUI Editor".into(),
            items: vec![
                MenuItem::action("Preferences…", Preferences),
                MenuItem::separator(),
                MenuItem::action("Quit GPUI Editor", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Document", NewDocument),
                MenuItem::action("Open Folder…", ChooseFolder),
                MenuItem::action("Save", SaveDocument),
                MenuItem::separator(),
                MenuItem::action("Close Window", CloseWindow),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", editor::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", editor::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", editor::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", editor::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", editor::Paste, OsAction::Paste),
                MenuItem::os_action("Select All", editor::SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find in Document…", editor::OpenFind),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Command Palette…", OpenPalette),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
            ],
        },
    ]);
}

fn bind_quit(window: WindowHandle<Shell>, cx: &mut App) {
    // Quit also works when a native panel is active. Defer the action so the window can
    // be borrowed even when it was dispatching the shortcut.
    cx.on_action(move |_: &Quit, cx| {
        cx.defer(move |cx| {
            window
                .update(cx, |shell, window, cx| {
                    shell.start(Intent::Exit, window, cx)
                })
                .ok();
        });
    });
}

type TabId = usize;

struct DocumentTab {
    id: TabId,
    editor: Entity<Editor>,
    file: Option<DocumentFile>,
    _subscriptions: [Subscription; 3],
}

impl DocumentTab {
    fn dirty(&self, cx: &App) -> bool {
        self.editor.read(cx).source() != self.file.as_ref().map_or("", DocumentFile::source)
    }

    fn path(&self) -> Option<&Path> {
        self.file.as_ref().map(DocumentFile::path)
    }
}

struct Shell {
    focus: FocusHandle,
    workspace: Option<Workspace>,
    sidebar: Sidebar,
    tabs: Vec<DocumentTab>,
    selected: Option<TabId>,
    next_tab: TabId,
    tab_scroll: ScrollHandle,
    tab_strip_width: Option<Pixels>,
    document_drag: Option<WeakEntity<tabs::DragPreview>>,
    _appearance: Subscription,
    _activation: Subscription,
    _hotkey_capture: Subscription,
    hotkeys: Hotkeys,
    session_path: Option<PathBuf>,
    restoring: bool,
    active: Option<ActiveOperation>,
    error: Option<String>,
    show_more_tools: bool,
    developer: Developer,
    palette: Option<Palette>,
}

enum ActiveOperation {
    Save { exit_queued: bool },
    Other,
}

enum Intent {
    ChooseFolder,
    OpenFolder(PathBuf),
    ExpandFolder(PathBuf),
    New,
    Open(PathBuf),
    OpenAt(PathBuf, SearchLocation),
    Save,
    CloseTab(TabId),
    Exit,
}

impl Shell {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let shell = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            shell
                .update(cx, |shell, cx| shell.start(Intent::Exit, window, cx))
                .is_err()
        });
        let appearance = cx.observe_window_appearance(window, |_, _, cx| cx.notify());
        let activation = cx.observe_window_activation(window, |shell, window, cx| {
            if !window.is_window_active() {
                shell.cancel_document_drag(window, cx);
            }
        });
        let sidebar = Sidebar::new(window, cx);
        let hotkey_capture = Self::intercept_hotkeys(window, cx);
        Self {
            focus,
            workspace: None,
            sidebar,
            tabs: Vec::new(),
            selected: None,
            next_tab: 0,
            tab_scroll: ScrollHandle::new(),
            tab_strip_width: None,
            document_drag: None,
            _appearance: appearance,
            _activation: activation,
            _hotkey_capture: hotkey_capture,
            hotkeys: Hotkeys::default(),
            session_path: None,
            restoring: false,
            active: None,
            error: None,
            show_more_tools: false,
            developer: Developer::default(),
            palette: None,
        }
    }

    fn current(&self) -> Option<&DocumentTab> {
        self.tab(self.selected?)
    }

    fn tab(&self, id: TabId) -> Option<&DocumentTab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    fn open_editor(
        &mut self,
        file: Option<DocumentFile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TabId {
        let id = self.add_tab(file, window, cx);
        self.select_tab(id, window, cx);
        id
    }

    fn add_tab(
        &mut self,
        file: Option<DocumentFile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TabId {
        if let Some(file) = &file
            && let Some(tab) = self.tabs.iter().find(|tab| tab.path() == Some(file.path()))
        {
            return tab.id;
        }

        let id = self.next_tab;
        self.next_tab += 1;
        let workspace = self.workspace.clone();
        let editor = cx.new(|cx| {
            let mut editor = Editor::new(
                file.as_ref()
                    .map_or_else(String::new, |file| file.source().to_owned()),
                window,
                cx,
            );
            editor.set_path(file.as_ref().map(|file| file.path().to_owned()), workspace);
            editor
        });
        let events = cx.subscribe_in(
            &editor,
            window,
            move |shell, _, event: &OpenLocalDocument, window, cx| {
                if let Some(current) = shell.current().filter(|tab| tab.id == id) {
                    let path = if event.0.is_relative() {
                        current
                            .path()
                            .and_then(Path::parent)
                            .map_or_else(|| event.0.clone(), |directory| directory.join(&event.0))
                    } else {
                        event.0.clone()
                    };
                    shell.start(Intent::Open(path), window, cx);
                }
            },
        );
        let updates = cx.observe(&editor, |_, _, cx| cx.notify());
        let diagnostics =
            cx.subscribe(&editor, |shell, _, event: &editor::PreviewDiagnostic, _| {
                shell.developer.record(event.0);
            });
        self.tabs.push(DocumentTab {
            id,
            editor,
            file,
            _subscriptions: [events, updates, diagnostics],
        });
        id
    }

    fn select_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
            if self.selected != Some(id) {
                self.show_more_tools = false;
            }
            self.selected = Some(id);
            self.tab_scroll.scroll_to_item(index);
            self.focus_document(window, cx);
            cx.notify();
        }
    }

    fn focus_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            return;
        }
        if self.developer.blocks_document_actions() {
            self.resume_developer_focus(window, cx);
        } else if let Some(current) = self.current() {
            cx.focus_view(&current.editor, window);
        } else {
            window.focus(&self.focus);
        }
    }

    fn remove_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.selected == Some(id) {
            self.selected = None;
            if let Some(tab) = self.tabs.get(index).or_else(|| self.tabs.last()) {
                self.select_tab(tab.id, window, cx);
            } else {
                self.show_more_tools = false;
                self.tab_scroll = ScrollHandle::new();
            }
        } else if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| Some(tab.id) == self.selected)
        {
            self.tab_scroll.scroll_to_item(index);
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some()
            || (self.selected.is_none() && self.developer.blocks_document_actions())
        {
            return;
        }
        self.start(
            self.selected.map_or(Intent::Exit, Intent::CloseTab),
            window,
            cx,
        );
    }

    fn is_composing(&self, cx: &App) -> bool {
        self.palette_is_composing(cx)
            || self.sidebar.search.is_composing(cx)
            || self.tabs.iter().any(|tab| {
                let editor = tab.editor.read(cx);
                editor.is_composing() || editor.find_is_composing(cx)
            })
    }

    fn allow_document_action(&mut self, window_close: bool, cx: &mut Context<Self>) -> bool {
        if self.is_composing(cx) {
            self.error =
                Some("Finish or cancel the current input composition, then try again.".into());
            cx.notify();
            return false;
        }
        window_close || (self.palette.is_none() && !self.developer.blocks_document_actions())
    }

    fn restore_session(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.session_path = Some(path.clone());
        self.restoring = true;
        cx.spawn_in(window, async move |shell, cx| {
            let restored = cx.background_executor().spawn(async move { session::restore(&path) }).await;
            let _ = shell.update_in(cx, |shell, window, cx| {
                if !std::mem::take(&mut shell.restoring) {
                    return;
                }
                match restored {
                    Ok(Some(restored)) => {
                        shell.sidebar.reset(restored.workspace.root(), restored.tree, window, cx);
                        shell.workspace = Some(restored.workspace);
                        let mut selected = None;
                        for (index, file) in restored.documents.into_iter().enumerate() {
                            let id = shell.add_tab(Some(file), window, cx);
                            if restored.selected == Some(index) {
                                selected = Some(id);
                            }
                        }
                        if let Some(id) = selected.or_else(|| shell.tabs.first().map(|tab| tab.id)) {
                            shell.select_tab(id, window, cx);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        shell.error = Some(format!(
                            "Could not restore the last workspace: {error}. Choose a folder to continue."
                        ));
                    }
                }
                cx.notify();
            });
        }).detach();
    }
}

fn filename(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = window.viewport_size().width;
        let controls = window_chrome::controls_width(window);
        let sidebar_width = self.sidebar.width(width);
        let strip_width = tabs::strip_width(width, sidebar_width, self.sidebar.visible, controls);
        if self.tab_strip_width.replace(strip_width) != Some(strip_width) {
            self.reveal_tab_at_width(strip_width, window, cx);
        }
        let busy = self.active.is_some();
        let theme = Theme::for_window(window);
        let picker = self.developer.picker();
        div()
            .relative()
            .track_focus(&self.focus)
            .capture_any_mouse_down(cx.listener(|shell, _, window, cx| {
                // Keep focus in the palette, or in the field with marked text,
                // until the clicked control decides how to handle the input.
                if shell.palette.is_some() || shell.is_composing(cx) {
                    window.prevent_default();
                }
            }))
            .size_full()
            .flex()
            .bg(theme.surface)
            .text_color(theme.text)
            .text_sm()
            .on_action(cx.listener(|shell, _: &Preferences, window, cx| {
                shell.developer_preferences(window, cx);
            }))
            .capture_key_down(
                cx.listener(|shell, event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" && !window.default_prevented() {
                        if shell.cancel_document_drag(window, cx) {
                            cx.stop_propagation();
                            window.prevent_default();
                        } else if shell.palette.is_some() {
                            if !shell.palette_is_composing(cx) {
                                cx.stop_propagation();
                                window.prevent_default();
                                shell.close_palette(window, cx);
                            }
                        } else if shell.developer.blocks_document_actions() {
                            cx.stop_propagation();
                            shell.cancel_developer(window, cx);
                        } else if shell.show_more_tools && !shell.is_composing(cx) {
                            cx.stop_propagation();
                            shell.show_more_tools = false;
                            cx.notify();
                        }
                    }
                }),
            )
            .on_action(cx.listener(|shell, _: &OpenPalette, window, cx| {
                shell.open_palette(window, cx);
            }))
            .on_action(cx.listener(|shell, _: &ChooseFolder, window, cx| {
                shell.start(Intent::ChooseFolder, window, cx)
            }))
            .on_action(cx.listener(|shell, _: &NewDocument, window, cx| {
                shell.start(Intent::New, window, cx)
            }))
            .on_action(cx.listener(|shell, _: &SaveDocument, window, cx| {
                shell.start(Intent::Save, window, cx)
            }))
            .on_action(cx.listener(|shell, _: &Close, window, cx| shell.close(window, cx)))
            .on_action(cx.listener(|shell, _: &CloseWindow, window, cx| {
                shell.start(Intent::Exit, window, cx)
            }))
            .on_action(
                cx.listener(|shell, _: &ToggleSidebar, window, cx| {
                    shell.toggle_sidebar(window, cx)
                }),
            )
            .when(self.sidebar.visible, |el| {
                el.child(self.sidebar(sidebar_width, controls, theme, picker.as_ref(), cx))
            })
            .child(self.editor_pane(controls, theme, picker.as_ref(), cx))
            .children(self.developer_layer(picker, theme, cx))
            .children(self.palette_layer(theme, window, cx))
            .when(busy, |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .bg(gpui::Hsla::from(theme.surface).opacity(0.1)),
                )
            })
    }
}
