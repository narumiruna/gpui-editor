//! The single-window editor shell. Filesystem operations run off the UI thread.

use std::path::{Path, PathBuf};

use gpui::{
    App, Application, AsyncWindowContext, Bounds, Context, Entity, FocusHandle, KeyBinding, Menu,
    MenuItem, OsAction, PathPromptOptions, PromptButton, PromptLevel, SharedString, Stateful,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, actions, div, prelude::*,
    px, rgb, rgba, size,
};

use crate::{
    editor::{self, Editor, OpenLocalDocument},
    workspace::{DocumentFile, Workspace},
};

mod quit;

actions!(
    shell,
    [ChooseFolder, NewDocument, SaveDocument, Close, Quit]
);

pub fn run() {
    Application::new().run(|cx| {
        editor::bind_keys(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-o", ChooseFolder, None),
            KeyBinding::new("cmd-n", NewDocument, None),
            KeyBinding::new("cmd-s", SaveDocument, None),
            KeyBinding::new("cmd-w", Close, None),
            KeyBinding::new("cmd-q", Quit, None),
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
        // A native panel can be active instead of the editor window. Defer the global action
        // so the editor window can be borrowed even when it was dispatching the shortcut.
        cx.on_action(move |_: &Quit, cx| {
            cx.defer(move |cx| {
                window
                    .update(cx, |shell, window, cx| {
                        shell.start(Intent::Exit, window, cx)
                    })
                    .ok();
            });
        });
        cx.set_menus(vec![
            Menu {
                name: "GPUI Editor".into(),
                items: vec![MenuItem::action("Quit GPUI Editor", Quit)],
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("New Document", NewDocument),
                    MenuItem::action("Open Folder…", ChooseFolder),
                    MenuItem::action("Save", SaveDocument),
                    MenuItem::separator(),
                    MenuItem::action("Close Window", Close),
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

struct CurrentDocument {
    editor: Entity<Editor>,
    file: Option<DocumentFile>,
    _subscriptions: [Subscription; 2],
}

impl CurrentDocument {
    fn dirty(&self, cx: &App) -> bool {
        self.editor.read(cx).source() != self.file.as_ref().map_or("", DocumentFile::source)
    }
}

struct Shell {
    focus: FocusHandle,
    workspace: Option<Workspace>,
    files: Vec<PathBuf>,
    current: Option<CurrentDocument>,
    busy: bool,
    error: Option<String>,
}

#[derive(Clone)]
enum Intent {
    ChooseFolder,
    New,
    Open(PathBuf),
    Save,
    Exit,
}

// Prepare a destination before confirming any loss of edits. On cancellation or failure the
// shell keeps displaying the same workspace and editor.
enum Change {
    Folder(Workspace, Vec<PathBuf>),
    RefreshFiles(Vec<PathBuf>),
    Document(Option<DocumentFile>),
    Exit,
}

struct SaveResult {
    file: DocumentFile,
    sidebar: Option<Result<Vec<PathBuf>, String>>,
}

struct Operation {
    workspace: Option<Workspace>,
    file: Option<DocumentFile>,
    source: Option<String>,
    dirty: bool,
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
            current: None,
            busy: false,
            error: None,
        }
    }

    fn open_editor(
        &mut self,
        file: Option<DocumentFile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.root().to_owned());
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
            |shell, emitter, event: &OpenLocalDocument, window, cx| {
                if let Some(current) = &shell.current
                    && &current.editor == emitter
                {
                    let path = if event.0.is_relative() {
                        current
                            .file
                            .as_ref()
                            .and_then(|file| file.path().parent())
                            .map_or_else(|| event.0.clone(), |directory| directory.join(&event.0))
                    } else {
                        event.0.clone()
                    };
                    shell.start(Intent::Open(path), window, cx);
                }
            },
        );
        let updates = cx.observe(&editor, |_, _, cx| cx.notify());
        self.current = Some(CurrentDocument {
            editor,
            file,
            _subscriptions: [events, updates],
        });
    }

    fn start(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || (matches!(intent, Intent::Save) && self.current.is_none()) {
            return;
        }
        let operation = Operation {
            workspace: self.workspace.clone(),
            file: self
                .current
                .as_ref()
                .and_then(|current| current.file.clone()),
            source: self
                .current
                .as_ref()
                .map(|current| current.editor.read(cx).source().to_owned()),
            dirty: self
                .current
                .as_ref()
                .is_some_and(|current| current.dirty(cx)),
        };
        self.busy = true;
        self.error = None;
        // Keep the document unchanged while the operation works from its source snapshot.
        window.focus(&self.focus);
        cx.notify();
        cx.spawn_in(window, async move |shell, cx| {
            let result = operation.execute(intent, cx).await;
            shell
                .update_in(cx, |shell, window, cx| {
                    shell.busy = false;
                    match result {
                        Ok((saved, change)) => {
                            if let Some(saved) = saved {
                                shell.apply_save(saved, cx);
                            }
                            match change {
                                Some(Change::RefreshFiles(files)) => shell.files = files,
                                Some(Change::Document(file)) => shell.open_editor(file, window, cx),
                                Some(Change::Folder(workspace, files)) => {
                                    shell.workspace = Some(workspace);
                                    shell.files = files;
                                    shell.current = None;
                                }
                                Some(Change::Exit) => {
                                    window.remove_window();
                                    quit::after_confirmation(cx);
                                    return;
                                }
                                None => {}
                            }
                        }
                        Err(error) => shell.error = Some(error),
                    }
                    if let Some(current) = &shell.current {
                        cx.focus_view(&current.editor, window);
                    }
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }

    fn apply_save(&mut self, saved: SaveResult, cx: &mut Context<Self>) {
        if let Some(current) = &mut self.current {
            if current.file.is_none() {
                current.editor.update(cx, |editor, cx| {
                    editor.set_path(
                        Some(saved.file.path().to_owned()),
                        self.workspace
                            .as_ref()
                            .map(|workspace| workspace.root().to_owned()),
                    );
                    cx.notify();
                });
            }
            current.file = Some(saved.file);
        }
        if let Some(sidebar) = saved.sidebar {
            match sidebar {
                Ok(files) => self.files = files,
                Err(error) => self.error = Some(error),
            }
        }
    }
}

impl Operation {
    async fn execute(
        self,
        intent: Intent,
        cx: &mut AsyncWindowContext,
    ) -> Result<(Option<SaveResult>, Option<Change>), String> {
        let change = match intent {
            Intent::Save => return Ok((self.save(cx).await?, None)),
            Intent::New => Change::Document(None),
            Intent::Exit => Change::Exit,
            Intent::ChooseFolder => {
                let picker = cx
                    .update(|_, cx| {
                        cx.prompt_for_paths(PathPromptOptions {
                            files: false,
                            directories: true,
                            multiple: false,
                            prompt: Some("Open Workspace".into()),
                        })
                    })
                    .map_err(message)?;
                let Some(path) = picker
                    .await
                    .map_err(message)?
                    .map_err(message)?
                    .and_then(|mut paths| paths.pop())
                else {
                    return Ok((None, None));
                };
                let (workspace, files) = cx
                    .background_executor()
                    .spawn(async move {
                        (|| {
                            let workspace = Workspace::open(&path)?;
                            let files = workspace.markdown_files()?;
                            Ok((workspace, files))
                        })()
                        .map_err(|error: std::io::Error| {
                            format!("Could not open folder {}: {error}", path.display())
                        })
                    })
                    .await?;
                if self
                    .workspace
                    .as_ref()
                    .is_some_and(|current| current.root() == workspace.root())
                {
                    // Opening the same folder refreshes the sidebar, not the document.
                    return Ok((None, Some(Change::RefreshFiles(files))));
                }
                Change::Folder(workspace, files)
            }
            Intent::Open(path) => {
                let workspace = self
                    .workspace
                    .clone()
                    .ok_or("Open a workspace folder before following document links.")?;
                let file = cx
                    .background_executor()
                    .spawn(async move {
                        workspace
                            .read_document(&path)
                            .map_err(|error| format!("Could not open {}: {error}", path.display()))
                    })
                    .await?;
                if self.file.as_ref() == Some(&file) {
                    return Ok((None, None));
                }
                Change::Document(Some(file))
            }
        };
        let mut saved = None;
        if self.dirty {
            let name = self
                .file
                .as_ref()
                .map_or_else(|| "Untitled.md".to_owned(), |file| filename(file.path()));
            let detail = format!("Save your changes to {name} before continuing?");
            match cx
                .prompt(
                    PromptLevel::Warning,
                    "Unsaved changes",
                    Some(&detail),
                    &[
                        PromptButton::ok("Save"),
                        PromptButton::new("Don’t Save"),
                        PromptButton::cancel("Cancel"),
                    ],
                )
                .await
            {
                Ok(0) => {
                    saved = self.save(cx).await?;
                    if saved.is_none() {
                        return Ok((None, None));
                    }
                }
                Ok(1) => {}
                _ => return Ok((None, None)),
            }
        }
        Ok((saved, Some(change)))
    }

    async fn save(self, cx: &mut AsyncWindowContext) -> Result<Option<SaveResult>, String> {
        let Some(source) = self.source else {
            return Ok(None);
        };
        if let Some(mut file) = self.file {
            return cx
                .background_executor()
                .spawn(async move {
                    if file.source() != source {
                        file.save(&source).map_err(|error| {
                            format!("Could not save {}: {error}", file.path().display())
                        })?;
                    }
                    Ok(Some(SaveResult {
                        file,
                        sidebar: None,
                    }))
                })
                .await;
        }
        let directory = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.root().to_owned())
            .or_else(std::env::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let picker = cx
            .update(|_, cx| cx.prompt_for_new_path(&directory, Some("Untitled.md")))
            .map_err(message)?;
        let Some(path) = picker.await.map_err(message)?.map_err(message)? else {
            return Ok(None);
        };
        cx.background_executor()
            .spawn(async move {
                let file = DocumentFile::create(self.workspace.as_ref(), &path, source)
                    .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
                let sidebar = self.workspace.map(|workspace| {
                    workspace.markdown_files().map_err(|error| {
                        format!("File saved, but could not refresh the sidebar: {error}")
                    })
                });
                Ok(Some(SaveResult { file, sidebar }))
            })
            .await
    }
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn filename(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn button(id: &'static str, label: &'static str, enabled: bool) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(rgb(0xcbd5e1))
        .child(label)
        .when(enabled, |el| {
            el.cursor_pointer().hover(|style| style.bg(rgb(0xe2e8f0)))
        })
        .when(!enabled, |el| el.text_color(rgb(0x64748b)))
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_save = self.current.is_some() && !self.busy;
        let mut sidebar = div()
            .w(px(220.))
            .flex_none()
            .flex()
            .flex_col()
            .min_h_0()
            .border_r_1()
            .border_color(rgb(0xe2e8f0))
            .bg(rgb(0xf8fafc))
            .child(
                div()
                    .px_3()
                    .py_2()
                    .truncate()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(self.workspace.as_ref().map_or_else(
                        || "No workspace".to_owned(),
                        |workspace| filename(workspace.root()),
                    )),
            );
        let mut files = div()
            .id("workspace-files")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .pb_2();
        for (index, path) in self.files.iter().enumerate() {
            let active = self
                .current
                .as_ref()
                .and_then(|current| current.file.as_ref())
                .is_some_and(|file| file.path() == path);
            let path = path.clone();
            files = files.child(
                div()
                    .id(("file", index))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .truncate()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0xe2e8f0)))
                    .when(active, |el| el.bg(rgb(0xdbeafe)))
                    .child(filename(&path))
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.start(Intent::Open(path.clone()), window, cx)
                    })),
            );
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
        sidebar = sidebar.child(files);
        let title = self.current.as_ref().map(|current| {
            let name = current
                .file
                .as_ref()
                .map_or_else(|| "Untitled.md".to_owned(), |file| filename(file.path()));
            if current.dirty(cx) {
                format!("{name} • Unsaved")
            } else {
                name
            }
        });
        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .p_2()
            .bg(rgb(0xf8fafc))
            .border_b_1()
            .border_color(rgb(0xe2e8f0))
            .child(button("open-folder", "Open Folder…", !self.busy).on_click(
                cx.listener(|shell, _, window, cx| shell.start(Intent::ChooseFolder, window, cx)),
            ))
            .child(
                button("new", "New", !self.busy).on_click(
                    cx.listener(|shell, _, window, cx| shell.start(Intent::New, window, cx)),
                ),
            )
            .child(button("save", "Save", can_save).on_click(
                cx.listener(|shell, _, window, cx| shell.start(Intent::Save, window, cx)),
            ))
            .when_some(title, |el, title| {
                el.child(div().ml_2().min_w_0().truncate().child(title))
            })
            .when(self.busy, |el| {
                el.child(
                    div()
                        .ml_auto()
                        .flex_none()
                        .text_color(rgb(0x64748b))
                        .child("Working…"),
                )
            });
        let content = div().flex_1().min_w_0().min_h_0().overflow_hidden();
        let content = match &self.current {
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
        div()
            .relative()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .text_color(rgb(0x0f172a))
            .text_sm()
            .on_action(cx.listener(|shell, _: &ChooseFolder, window, cx| {
                shell.start(Intent::ChooseFolder, window, cx)
            }))
            .on_action(cx.listener(|shell, _: &NewDocument, window, cx| {
                shell.start(Intent::New, window, cx)
            }))
            .on_action(cx.listener(|shell, _: &SaveDocument, window, cx| {
                shell.start(Intent::Save, window, cx)
            }))
            .on_action(
                cx.listener(|shell, _: &Close, window, cx| shell.start(Intent::Exit, window, cx)),
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
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar)
                    .child(content),
            )
            .when(self.busy, |el| {
                el.child(div().absolute().inset_0().occlude().bg(rgba(0xffffff18)))
            })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gpui::{EntityInputHandler as _, TestAppContext};

    use super::*;

    #[gpui::test]
    fn editing_survives_cancel_and_save_failure_before_following_a_link(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(directory.path()).unwrap();
        let original_path = workspace.root().join("original.md");
        let destination = workspace.root().join("notes/linked.md");
        fs::create_dir(destination.parent().unwrap()).unwrap();
        fs::write(&original_path, "original").unwrap();
        fs::write(&destination, "linked").unwrap();
        let original_file = workspace.read_document(&original_path).unwrap();
        let (shell, cx) = cx.add_window_view(Shell::new);
        let editor = cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.workspace = Some(workspace);
                shell.open_editor(Some(original_file), window, cx);
                shell.current.as_ref().unwrap().editor.clone()
            })
        });
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.replace_text_in_range(Some(0..8), "my edits", window, cx);
            });
        });
        fs::write(&original_path, "external update").unwrap();
        for answer in ["Cancel", "Save", "Don’t Save"] {
            cx.update(|_, cx| {
                editor.update(cx, |_, cx| {
                    cx.emit(OpenLocalDocument("notes/linked.md".into()))
                });
            });
            cx.run_until_parked();
            assert!(cx.has_pending_prompt());
            cx.simulate_prompt_answer(answer);
            cx.run_until_parked();
            cx.update(|_, cx| {
                let shell = shell.read(cx);
                let current = shell.current.as_ref().unwrap();
                if answer == "Don’t Save" {
                    assert_eq!(current.editor.read(cx).source(), "linked");
                    assert_eq!(current.file.as_ref().unwrap().path(), destination);
                    assert_eq!(
                        shell.workspace.as_ref().unwrap().root(),
                        directory.path().canonicalize().unwrap()
                    );
                } else {
                    assert_eq!(current.editor, editor);
                    assert_eq!(current.editor.read(cx).source(), "my edits");
                    assert!(current.dirty(cx));
                    assert_eq!(shell.error.is_some(), answer == "Save");
                }
            });
        }
        assert_eq!(
            fs::read_to_string(&original_path).unwrap(),
            "external update"
        );
    }

    #[gpui::test]
    fn closing_an_untitled_document_waits_for_a_successful_save(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(directory.path()).unwrap();
        let path = workspace.root().join("saved.md");
        let (shell, cx) = cx.add_window_view(Shell::new);
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.workspace = Some(workspace);
                shell.open_editor(None, window, cx);
                shell
                    .current
                    .as_ref()
                    .unwrap()
                    .editor
                    .update(cx, |editor, cx| {
                        editor.replace_text_in_range(None, "my draft", window, cx);
                    });
            });
        });
        assert!(!cx.simulate_close());
        cx.run_until_parked();
        cx.simulate_prompt_answer("Save");
        cx.run_until_parked();
        assert!(cx.did_prompt_for_new_path());
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        cx.update(|_, cx| assert!(shell.read(cx).current.as_ref().unwrap().dirty(cx)));

        assert!(!cx.simulate_close());
        cx.run_until_parked();
        cx.simulate_prompt_answer("Save");
        cx.run_until_parked();
        cx.simulate_new_path_selection(|root| {
            assert_eq!(root, path.parent().unwrap());
            Some(path.clone())
        });
        cx.run_until_parked();
        assert_eq!(fs::read_to_string(&path).unwrap(), "my draft");
        cx.cx.read(|cx| assert!(cx.windows().is_empty()));
    }
}
