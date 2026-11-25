//! Prepare destinations and protect drafts before applying a shell operation.

use std::path::{Path, PathBuf};

use gpui::{
    AsyncWindowContext, Context, PathPromptOptions, PromptButton, PromptLevel, ScrollHandle,
    WeakEntity, Window,
};

use super::{
    ActiveOperation, DocumentFile, DocumentTab, Intent, Shell, TabId, Workspace, quit, session,
    sidebar::{Snapshot, snapshot},
};
use crate::workspace::{DirectoryEntry, SearchLocation};

enum Change {
    Folder(Workspace, Snapshot),
    RefreshTree(Snapshot),
    ExpandFolder(PathBuf, Vec<DirectoryEntry>),
    Open(DocumentFile, Option<SearchLocation>),
    Select(TabId, Option<SearchLocation>),
    Remove(TabId),
    Saved,
    Exit,
}

struct Operation {
    workspace: Option<Workspace>,
    selected: Option<TabId>,
    loaded_folders: Vec<PathBuf>,
    open: Vec<(PathBuf, TabId)>,
    session_path: Option<PathBuf>,
}

struct SaveResult {
    file: DocumentFile,
    sidebar: Option<Result<Snapshot, String>>,
}

impl Shell {
    pub(super) fn start(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(active) = &mut self.active {
            if let ActiveOperation::Save { exit_queued } = active
                && matches!(intent, Intent::Exit)
            {
                *exit_queued = true;
            }
            return;
        }
        if !self.allow_document_action(matches!(intent, Intent::Exit), cx) {
            return;
        }
        if matches!(intent, Intent::Save) && self.selected.is_none()
            || matches!(intent, Intent::CloseTab(id) if self.tab(id).is_none())
        {
            return;
        }
        if self.palette.is_some() && matches!(intent, Intent::Exit) {
            self.close_palette(window, cx);
        }
        self.cancel_document_drag(window, cx);
        let interrupted_restore = std::mem::take(&mut self.restoring);
        self.error = None;
        if matches!(intent, Intent::New) {
            self.open_editor(None, window, cx);
            self.developer.record("New document");
            return;
        }
        let (operation_name, failure) = match &intent {
            Intent::ChooseFolder | Intent::OpenFolder(_) => {
                ("Choose workspace", "Choose workspace failed")
            }
            Intent::New => unreachable!(),
            Intent::ExpandFolder(_) => {
                ("Expand workspace folder", "Expand workspace folder failed")
            }
            Intent::Open(_) | Intent::OpenAt(..) => ("Open document", "Open document failed"),
            Intent::Save => ("Save document", "Save document failed"),
            Intent::CloseTab(_) => ("Close tab", "Close tab failed"),
            Intent::Exit => ("Close editor", "Close editor failed"),
        };
        let operation = Operation {
            workspace: self.workspace.clone(),
            selected: self.selected,
            loaded_folders: self.sidebar.loaded(),
            open: self
                .tabs
                .iter()
                .filter_map(|tab| Some((tab.path()?.to_owned(), tab.id)))
                .collect(),
            // An immediate exit must not replace a session we have not loaded yet.
            session_path: if interrupted_restore {
                None
            } else {
                self.session_path.clone()
            },
        };
        self.active = Some(if matches!(intent, Intent::Save) {
            ActiveOperation::Save { exit_queued: false }
        } else {
            ActiveOperation::Other
        });
        self.enable_editors(false, cx);
        window.focus(&self.focus);
        cx.notify();
        cx.spawn_in(window, async move |shell, cx| {
            let result = operation.execute(intent, &shell, cx).await;
            let _ = shell.update_in(cx, |shell, window, cx| {
                shell.finish(result, operation_name, failure, window, cx);
            });
        })
        .detach();
    }

    fn enable_editors(&self, enabled: bool, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            tab.editor
                .update(cx, |editor, cx| editor.set_input_enabled(enabled, cx));
        }
    }

    fn finish(
        &mut self,
        result: Result<Option<Change>, String>,
        operation_name: &'static str,
        failure: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let exit_queued = matches!(
            self.active.take(),
            Some(ActiveOperation::Save { exit_queued: true })
        );
        self.enable_editors(true, cx);
        match result {
            Ok(change) => {
                self.developer.record(if change.is_some() {
                    operation_name
                } else {
                    "Document operation canceled"
                });
                match change {
                    Some(Change::Folder(workspace, entries)) => {
                        self.tabs.clear();
                        self.selected = None;
                        self.show_more_tools = false;
                        self.tab_scroll = ScrollHandle::new();
                        self.sidebar.reset(workspace.root(), entries, window, cx);
                        self.workspace = Some(workspace);
                    }
                    Some(Change::RefreshTree(entries)) => self.sidebar.refresh(entries),
                    Some(Change::ExpandFolder(path, entries)) => self.sidebar.expand(path, entries),
                    Some(Change::Open(file, location)) => {
                        self.sidebar.record_open(file.path());
                        self.open_editor(Some(file), window, cx);
                        self.reveal_search_result(location, cx);
                    }
                    Some(Change::Select(id, location)) => {
                        self.select_tab(id, window, cx);
                        if let Some(path) = self.current().and_then(DocumentTab::path) {
                            let path = path.to_owned();
                            self.sidebar.record_open(&path);
                        }
                        self.reveal_search_result(location, cx);
                    }
                    Some(Change::Remove(id)) => self.remove_tab(id, window, cx),
                    Some(Change::Saved) if exit_queued => {
                        self.start(Intent::Exit, window, cx);
                        return;
                    }
                    Some(Change::Exit) => {
                        window.remove_window();
                        quit::after_confirmation(cx);
                        return;
                    }
                    Some(Change::Saved) | None => {}
                }
            }
            Err(error) => {
                self.developer.record(failure);
                self.error = Some(error);
            }
        }
        self.focus_document(window, cx);
        cx.notify();
    }

    fn apply_save(&mut self, id: TabId, saved: SaveResult, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            if tab.file.is_none() {
                tab.editor.update(cx, |editor, cx| {
                    editor.set_path(Some(saved.file.path().to_owned()), self.workspace.clone());
                    cx.notify();
                });
            }
            tab.file = Some(saved.file);
        }
        if let Some(sidebar) = saved.sidebar {
            match sidebar {
                Ok(entries) => self.sidebar.refresh(entries),
                Err(error) => self.error = Some(error),
            }
        }
        cx.notify();
    }
}

impl Operation {
    async fn execute(
        mut self,
        intent: Intent,
        shell: &WeakEntity<Shell>,
        cx: &mut AsyncWindowContext,
    ) -> Result<Option<Change>, String> {
        let intent = if let Intent::ChooseFolder = intent {
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
                return Ok(None);
            };
            Intent::OpenFolder(path)
        } else {
            intent
        };
        let mut change = match intent {
            Intent::Save => {
                let id = self.selected.expect("Save requires a selected tab");
                return Ok(self.save(id, shell, cx).await?.then_some(Change::Saved));
            }
            Intent::Open(_) | Intent::OpenAt(..) => {
                let (path, location) = match intent {
                    Intent::Open(path) => (path, None),
                    Intent::OpenAt(path, location) => (path, Some(location)),
                    _ => unreachable!(),
                };
                let workspace = self
                    .workspace
                    .ok_or("Open a workspace folder before following document links.")?;
                let change = cx
                    .background_executor()
                    .spawn(async move {
                        let result = (|| {
                            let normalized = workspace.resolve_document(&path)?;
                            if let Some((_, id)) =
                                self.open.iter().find(|(open, _)| open == &normalized)
                            {
                                return Ok(Change::Select(*id, location));
                            }
                            workspace
                                .read_document(&normalized)
                                .map(|file| Change::Open(file, location))
                        })();
                        result.map_err(|error: std::io::Error| {
                            format!("Could not open {}: {error}", path.display())
                        })
                    })
                    .await?;
                return Ok(Some(change));
            }
            Intent::ExpandFolder(path) => {
                let workspace = self.workspace.ok_or("Open a workspace folder first.")?;
                return cx
                    .background_executor()
                    .spawn(async move {
                        let entries = workspace.directory_entries(&path).map_err(|error| {
                            format!("Could not expand {}: {error}", path.display())
                        })?;
                        Ok(Some(Change::ExpandFolder(path, entries)))
                    })
                    .await;
            }
            Intent::OpenFolder(path) => {
                let current = self.workspace.clone();
                let loaded = self.loaded_folders.clone();
                let (workspace, entries, same_workspace) = cx
                    .background_executor()
                    .spawn(async move {
                        let result = (|| {
                            let workspace = Workspace::open(&path)?;
                            let same_workspace = current.as_ref() == Some(&workspace);
                            let entries = snapshot(
                                &workspace,
                                if same_workspace { loaded } else { Vec::new() },
                            )?;
                            Ok((workspace, entries, same_workspace))
                        })();
                        result.map_err(|error: std::io::Error| {
                            format!("Could not open folder {}: {error}", path.display())
                        })
                    })
                    .await?;
                if same_workspace {
                    return Ok(Some(Change::RefreshTree(entries)));
                }
                Change::Folder(workspace, entries)
            }
            Intent::CloseTab(id) => Change::Remove(id),
            Intent::Exit => Change::Exit,
            Intent::New | Intent::ChooseFolder => unreachable!(),
        };

        if !self.confirm_changes(&change, shell, cx).await? {
            return Ok(None);
        }
        match &mut change {
            Change::Folder(workspace, entries) => {
                // The destination might have changed while the user answered prompts.
                let workspace = workspace.clone();
                *entries = cx
                    .background_executor()
                    .spawn(async move {
                        snapshot(&workspace, Vec::new()).map_err(|error| {
                            format!(
                                "Could not open folder {}: {error}",
                                workspace.root().display()
                            )
                        })
                    })
                    .await?;
            }
            Change::Exit if !self.persist_session(shell, cx).await? => return Ok(None),
            _ => {}
        }
        Ok(Some(change))
    }

    async fn confirm_changes(
        &mut self,
        change: &Change,
        shell: &WeakEntity<Shell>,
        cx: &mut AsyncWindowContext,
    ) -> Result<bool, String> {
        let dirty = shell
            .update_in(cx, |shell, _, cx| {
                shell
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, tab)| {
                        !matches!(change, Change::Remove(id) if tab.id != *id) && tab.dirty(cx)
                    })
                    .map(|(index, tab)| (tab.id, shell.tab_title(tab, index)))
                    .collect::<Vec<_>>()
            })
            .map_err(message)?;
        for (id, title) in dirty {
            let detail = format!("Save your changes to {title} before continuing?");
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
                Ok(0) if self.save(id, shell, cx).await? => {}
                Ok(1) => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    async fn save(
        &mut self,
        id: TabId,
        shell: &WeakEntity<Shell>,
        cx: &mut AsyncWindowContext,
    ) -> Result<bool, String> {
        let (file, source) = shell
            .update_in(cx, |shell, _, cx| {
                let tab = shell
                    .tab(id)
                    .expect("The operation keeps its target tab open");
                (tab.file.clone(), tab.editor.read(cx).source().to_owned())
            })
            .map_err(message)?;
        let saved = if let Some(mut file) = file {
            cx.background_executor()
                .spawn(async move {
                    if file.source() != source {
                        file.save(&source).map_err(|error| {
                            format!("Could not save {}: {error}", file.path().display())
                        })?;
                    }
                    Ok::<_, String>(SaveResult {
                        file,
                        sidebar: None,
                    })
                })
                .await?
        } else {
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
                return Ok(false);
            };
            let workspace = self.workspace.clone();
            let open = self.open.clone();
            let loaded = self.loaded_folders.clone();
            cx.background_executor()
                .spawn(async move {
                    // An open file may have been deleted externally. Do not claim its path for a second tab.
                    let parent = path
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .unwrap_or(Path::new("."));
                    if let Some(mut target) = parent
                        .canonicalize()
                        .ok()
                        .zip(path.file_name())
                        .map(|(parent, name)| parent.join(name))
                    {
                        if target.extension().is_none() {
                            target.set_extension("md");
                        }
                        if open.iter().any(|(path, _)| path == &target) {
                            return Err(format!(
                                "{} is already open in another tab. Choose a different name.",
                                target.display()
                            ));
                        }
                    }
                    let file = DocumentFile::create(workspace.as_ref(), &path, source)
                        .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
                    let sidebar = workspace.map(|workspace| {
                        snapshot(&workspace, loaded).map_err(|error| {
                            format!("File saved, but could not refresh the sidebar: {error}")
                        })
                    });
                    Ok(SaveResult { file, sidebar })
                })
                .await?
        };
        if !self.open.iter().any(|(_, tab)| *tab == id) {
            self.open.push((saved.file.path().to_owned(), id));
        }
        shell
            .update_in(cx, |shell, _, cx| shell.apply_save(id, saved, cx))
            .map_err(message)?;
        Ok(true)
    }

    async fn persist_session(
        &self,
        shell: &WeakEntity<Shell>,
        cx: &mut AsyncWindowContext,
    ) -> Result<bool, String> {
        let Some(path) = self.session_path.clone() else {
            return Ok(true);
        };
        let session = shell
            .update_in(cx, |shell, _, _| {
                session::Session::capture(
                    shell.workspace.as_ref(),
                    shell.tabs.iter().filter_map(DocumentTab::path),
                    shell.current().and_then(DocumentTab::path),
                )
            })
            .map_err(message)?;
        let Err(error) = cx
            .background_executor()
            .spawn(async move { session.write(&path) })
            .await
        else {
            return Ok(true);
        };
        let detail = format!("The previous workspace record was not updated: {error}.");
        Ok(matches!(
            cx.prompt(
                PromptLevel::Warning,
                "Could not save the workspace session",
                Some(&detail),
                &[
                    PromptButton::cancel("Keep Window Open"),
                    PromptButton::new("Quit Without Updating"),
                ]
            )
            .await,
            Ok(1)
        ))
    }
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}
