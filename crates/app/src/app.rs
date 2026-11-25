//! The single-window editor shell. Filesystem operations run off the UI thread.

use std::path::{Path, PathBuf};

use gpui::{
    App, Application, Bounds, Context, Entity, FocusHandle, KeyBinding, Menu, MenuItem, OsAction,
    Pixels, ScrollHandle, SharedString, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowOptions, actions, div, prelude::*, px, rgb, rgba, size,
};

use crate::{
    editor::{self, Editor, OpenLocalDocument, toolbar},
    workspace::{DocumentFile, Workspace},
};

pub(crate) mod developer;
mod operation;
mod quit;
mod session;
mod tabs;
#[cfg(test)]
mod tests;

use developer::{Developer, Picker};

actions!(
    shell,
    [
        ChooseFolder,
        NewDocument,
        SaveDocument,
        Close,
        CloseWindow,
        Quit,
        Preferences
    ]
);

pub fn run() {
    let app = Application::new().with_assets(editor::ToolbarAssets);
    app.run(|cx| {
        editor::bind_keys(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-o", ChooseFolder, None),
            KeyBinding::new("cmd-n", NewDocument, None),
            KeyBinding::new("cmd-s", SaveDocument, None),
            KeyBinding::new("cmd-w", Close, None),
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-,", Preferences, None),
        ]);
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1040.), px(720.)),
                    cx,
                ))),
                window_min_size: Some(size(px(560.), px(360.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("GPUI Editor".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Shell::new(window, cx)),
        );
        let Ok(window) = window else {
            eprintln!("Could not open the editor window: {}", window.unwrap_err());
            cx.quit();
            return;
        };
        if let Err(error) = quit::install(cx) {
            eprintln!("Could not protect unsaved changes on macOS quit: {error}");
            cx.quit();
            return;
        }
        bind_quit(window, cx);
        if let Some(path) = session::production_path() {
            let _ = window.update(cx, |shell, window, cx| {
                shell.restore_session(path, window, cx)
            });
        }
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
                ],
            },
        ]);
        cx.activate(true);
    });
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
    files: Vec<PathBuf>,
    tabs: Vec<DocumentTab>,
    selected: Option<TabId>,
    next_tab: TabId,
    tab_scroll: ScrollHandle,
    tab_window_width: Option<Pixels>,
    session_path: Option<PathBuf>,
    restoring: bool,
    active: Option<ActiveOperation>,
    error: Option<String>,
    show_more_tools: bool,
    developer: Developer,
}

enum ActiveOperation {
    Save { exit_queued: bool },
    Other,
}

enum Intent {
    ChooseFolder,
    OpenFolder(PathBuf),
    New,
    Open(PathBuf),
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
        Self {
            focus,
            workspace: None,
            files: Vec::new(),
            tabs: Vec::new(),
            selected: None,
            next_tab: 0,
            tab_scroll: ScrollHandle::new(),
            tab_window_width: None,
            session_path: None,
            restoring: false,
            active: None,
            error: None,
            show_more_tools: false,
            developer: Developer::default(),
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
        if self.selected.is_none() && self.developer.blocks_document_actions() {
            return;
        }
        self.start(
            self.selected.map_or(Intent::Exit, Intent::CloseTab),
            window,
            cx,
        );
    }

    fn is_composing(&self, cx: &App) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.editor.read(cx).is_composing())
    }

    fn allow_document_action(&mut self, window_close: bool, cx: &mut Context<Self>) -> bool {
        if self.is_composing(cx) {
            self.error =
                Some("Finish or cancel the current input composition, then try again.".into());
            cx.notify();
            return false;
        }
        window_close || !self.developer.blocks_document_actions()
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
                        shell.workspace = Some(restored.workspace);
                        shell.files = restored.files;
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
        if self.tab_window_width.replace(width) != Some(width)
            && let Some(index) = self
                .tabs
                .iter()
                .position(|tab| Some(tab.id) == self.selected)
        {
            self.tab_scroll.scroll_to_item(index);
        }
        let busy = self.active.is_some();
        let current = self.current();
        let picker = self.developer.picker();
        let can_save = current.is_some() && !busy;
        let workspace_name = self
            .workspace
            .as_ref()
            .map(|workspace| filename(workspace.root()));
        let sidebar = div()
            .w(px(172.))
            .flex_none()
            .flex()
            .flex_col()
            .min_h_0()
            .border_r_1()
            .border_color(rgb(0xd3dce9))
            .bg(rgb(0xf2f5fa))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_3()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(toolbar::icon("Workspace", 15.).text_color(rgb(0x2859ad)))
                    .child(
                        div().min_w_0().truncate().child(
                            workspace_name
                                .clone()
                                .unwrap_or_else(|| "No workspace".into()),
                        ),
                    ),
            )
            .when(self.workspace.is_some(), |sidebar| {
                sidebar.child(
                    div()
                        .px_3()
                        .pb_1()
                        .text_xs()
                        .text_color(rgb(0x475569))
                        .child("Workspace files"),
                )
            });
        let mut files = div()
            .id("workspace-files")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .pb_2();
        for (index, path) in self.files.iter().enumerate() {
            let active = current.and_then(DocumentTab::path) == Some(path);
            let path = path.clone();
            files = files.child(Picker::mark(
                picker.as_ref(),
                div()
                    .id(("file", index))
                    .debug_selector(move || format!("workspace-file-{index}"))
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0xe2e8f0)))
                    .when(active, |el| {
                        el.bg(rgb(0xe4eeff))
                            .text_color(rgb(0x234e9a))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                    })
                    .child(toolbar::icon("File", 14.).text_color(rgb(0x2859ad)))
                    .child(div().min_w_0().truncate().child(filename(&path)))
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.start(Intent::Open(path.clone()), window, cx)
                    })),
                "sidebar.file",
                "Workspace file row",
            ));
        }
        if self.files.is_empty() {
            files = files.child(div().px_2().pb_2().text_color(rgb(0x64748b)).child(
                if self.workspace.is_some() {
                    "No Markdown files in this folder."
                } else {
                    "Choose a folder to browse its Markdown files."
                },
            ));
        }
        let sidebar = Picker::mark(
            picker.as_ref(),
            sidebar.child(files),
            "sidebar",
            "Workspace sidebar",
        );
        let file_tools = toolbar::group()
            .child(
                toolbar::marked_button("Open Folder…", !busy, picker.as_ref()).when(
                    !busy,
                    |button| {
                        button.on_click(cx.listener(|shell, _, window, cx| {
                            shell.start(Intent::ChooseFolder, window, cx)
                        }))
                    },
                ),
            )
            .child(
                toolbar::marked_button("New note", !busy, picker.as_ref()).when(!busy, |button| {
                    button.on_click(
                        cx.listener(|shell, _, window, cx| shell.start(Intent::New, window, cx)),
                    )
                }),
            )
            .child(
                toolbar::marked_button("Save", can_save, picker.as_ref()).when(
                    can_save,
                    |button| {
                        button.on_click(
                            cx.listener(|shell, _, window, cx| {
                                shell.start(Intent::Save, window, cx)
                            }),
                        )
                    },
                ),
            );
        let mut tools = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .child(file_tools);
        let mut more_tools = None;
        if let Some(current) = current {
            let edit_enabled = !busy && !current.editor.read(cx).is_composing();
            let label = if self.show_more_tools {
                "Fewer tools"
            } else {
                "More tools"
            };
            tools = tools
                .children(toolbar::primary_groups(
                    &current.editor,
                    edit_enabled,
                    picker.as_ref(),
                ))
                .child(toolbar::marked_button(label, !busy, picker.as_ref()).when(
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
                    toolbar::more_group(&current.editor, edit_enabled, picker.as_ref())
                        .px_2()
                        .py_1()
                        .border_t_1()
                        .border_color(rgb(0xd3dce9)),
                );
            }
        }
        let toolbar = div()
            .flex_none()
            .bg(rgb(0xf9fbff))
            .border_b_1()
            .border_color(rgb(0xd3dce9))
            .child(
                div()
                    .flex()
                    .items_start()
                    .child(tools.flex_1().min_w_0())
                    .when_some(self.developer_status(), |el, text| {
                        el.child(
                            div()
                                .py_2()
                                .text_xs()
                                .text_color(rgb(0x475569))
                                .child(text.to_owned()),
                        )
                    })
                    .children(self.developer_icon(!busy, cx)),
            )
            .children(more_tools);
        let (status, saved) = if busy {
            ("Working…", false)
        } else if let Some(current) = current {
            if current.dirty(cx) {
                ("Unsaved changes", false)
            } else if current.file.is_some() {
                ("Saved locally", true)
            } else {
                ("Not saved", false)
            }
        } else {
            ("", false)
        };
        let document_bar = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .bg(rgb(0xf2f5fa))
            .border_b_1()
            .border_color(rgb(0xd3dce9))
            .when_some(workspace_name, |bar, name| {
                bar.child(
                    div()
                        .max_w(px(125.))
                        .truncate()
                        .text_color(rgb(0x475569))
                        .child(name),
                )
            })
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(rgb(if saved { 0x234e9a } else { 0x475569 }))
                    .when(saved, |el| {
                        el.child(toolbar::icon("Saved", 13.).text_color(rgb(0x234e9a)))
                    })
                    .child(status),
            );
        let document_bar = Picker::mark(
            picker.as_ref(),
            document_bar,
            "document.status",
            "Document status bar",
        );
        let content = div().flex_1().min_w_0().min_h_0().overflow_hidden();
        let content = match current {
            Some(current) => content.child(current.editor.clone()),
            None => content
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_color(rgb(0x64748b))
                .child(if self.workspace.is_some() {
                    "Select a Markdown file or create a new one."
                } else {
                    "Open a workspace folder or create a new document."
                }),
        };
        let content = Picker::mark(
            picker.as_ref(),
            content,
            "editor.surface",
            "Markdown editing surface",
        );
        div()
            .relative()
            .track_focus(&self.focus)
            .capture_any_mouse_down(cx.listener(|shell, _, window, cx| {
                // The shell's default mouse focus would blur the editor before the
                // clicked control can check for marked text.
                if shell.is_composing(cx) {
                    window.prevent_default();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .text_color(rgb(0x0f172a))
            .text_sm()
            .on_action(cx.listener(|shell, _: &Preferences, window, cx| {
                shell.developer_preferences(window, cx);
            }))
            .capture_key_down(
                cx.listener(|shell, event: &gpui::KeyDownEvent, window, cx| {
                    if shell.developer.blocks_document_actions() && event.keystroke.key == "escape"
                    {
                        cx.stop_propagation();
                        shell.cancel_developer(window, cx);
                    }
                }),
            )
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
            .when(
                busy || current.is_some() || self.workspace.is_some(),
                |el| el.child(document_bar),
            )
            .child(toolbar)
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_2()
                        .bg(rgb(0xfef2f2))
                        .text_color(rgb(0x991b1b))
                        .child(SharedString::from(error)),
                )
            })
            .child(
                div().flex().flex_1().min_h_0().child(sidebar).child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .children(self.tab_strip(cx))
                        .child(content),
                ),
            )
            .children(self.developer_layer(picker, cx))
            .when(busy, |el| {
                el.child(div().absolute().inset_0().occlude().bg(rgba(0xffffff18)))
            })
    }
}
