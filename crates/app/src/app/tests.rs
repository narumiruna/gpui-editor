use std::{fs, ops::Range};

use gpui::{
    EntityInputHandler as _, Focusable as _, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent,
    TestAppContext, TouchPhase, VisualTestContext, point,
};

use super::*;

fn click_tool(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing element: {selector}"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

// GPUI's test-only debug lookup requires a static string, even for an element with a dynamic ID.
fn tab_selector(prefix: &str, id: TabId) -> &'static str {
    Box::leak(format!("{prefix}-{id}").into_boxed_str())
}

struct Fixture {
    directory: tempfile::TempDir,
    workspace: Workspace,
    session_path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let workspace = Workspace::open(&root).unwrap();
        let session_path = directory.path().join("state/session.json");
        Self {
            directory,
            workspace,
            session_path,
        }
    }

    fn document(&self, name: &str, source: &str) -> PathBuf {
        let path = self.workspace.root().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, source).unwrap();
        path
    }

    fn open(&self, cx: &mut VisualTestContext, shell: &Entity<Shell>) {
        start(cx, shell, Intent::OpenFolder(self.workspace.root().into()));
        cx.update(|_, cx| assert_eq!(shell.read(cx).workspace.as_ref(), Some(&self.workspace)));
    }

    fn record(&self, open: &[&Path], selected: Option<&Path>) {
        session::Session::capture(Some(&self.workspace), open.iter().copied(), selected)
            .write(&self.session_path)
            .unwrap();
    }
}

fn start(cx: &mut VisualTestContext, shell: &Entity<Shell>, intent: Intent) {
    cx.update(|window, cx| shell.update(cx, |shell, cx| shell.start(intent, window, cx)));
    cx.run_until_parked();
}

fn restore(cx: &mut VisualTestContext, shell: &Entity<Shell>, path: &Path) {
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.restore_session(path.into(), window, cx)
        });
    });
    cx.run_until_parked();
}

fn use_session(cx: &mut VisualTestContext, shell: &Entity<Shell>, path: &Path) {
    cx.update(|_, cx| shell.update(cx, |shell, _| shell.session_path = Some(path.into())));
}

fn tab_paths(shell: &Shell) -> Vec<&Path> {
    shell.tabs.iter().map(|tab| tab.path().unwrap()).collect()
}

fn selection(cx: &mut VisualTestContext, editor: &Entity<Editor>) -> Range<usize> {
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.selected_text_range(false, window, cx).unwrap().range
        })
    })
}

fn replace_current(
    cx: &mut VisualTestContext,
    shell: &Entity<Shell>,
    source: &str,
) -> (TabId, Entity<Editor>) {
    cx.update(|window, cx| {
        let tab = shell.read(cx).current().unwrap();
        let (id, editor) = (tab.id, tab.editor.clone());
        editor.update(cx, |editor, cx| {
            let len = editor.source().encode_utf16().count();
            editor.replace_text_in_range(Some(0..len), source, window, cx);
        });
        (id, editor)
    })
}

fn answer(cx: &mut VisualTestContext, button: &str) {
    assert!(cx.has_pending_prompt(), "missing prompt for {button}");
    cx.simulate_prompt_answer(button);
    cx.run_until_parked();
}

#[gpui::test]
fn unified_toolbar_formats_and_saves_the_current_document(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(directory.path()).unwrap();
    let path = workspace.root().join("note.md");
    fs::write(&path, "note").unwrap();
    let file = workspace.read_document(&path).unwrap();
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.update(|window, cx| {
        editor::bind_keys(cx);
        shell.update(cx, |shell, cx| {
            shell.open_editor(Some(file), window, cx);
        });
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("Display formula").is_none());
    cx.simulate_keystrokes("cmd-a");
    click_tool(cx, "Bold");
    cx.update(|_, cx| {
        let current = shell.read(cx).current().unwrap();
        assert_eq!(current.editor.read(cx).source(), "**note**");
        assert!(current.dirty(cx));
    });
    click_tool(cx, "More tools");
    click_tool(cx, "Display formula");
    click_tool(cx, "Fewer tools");
    cx.update(|_, cx| assert!(!shell.read(cx).show_more_tools));
    click_tool(cx, "Save");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "**note**\n\n$$\nx^2\n$$\n\n"
    );
    cx.update(|_, cx| assert!(!shell.read(cx).current().unwrap().dirty(cx)));
}

#[gpui::test]
fn compact_toolbar_fits_on_one_row_at_minimum_window_width(cx: &mut TestAppContext) {
    let (shell, cx) = cx.add_window_view(Shell::new);
    click_tool(cx, "New note");
    cx.update(|_, cx| assert!(shell.read(cx).current().is_some()));
    cx.simulate_resize(size(px(1040.), px(720.)));
    cx.run_until_parked();
    let tab_row = cx.debug_bounds("document-tabs").unwrap();
    let toolbar = cx.debug_bounds("editor-toolbar").unwrap();
    let body = cx.debug_bounds("document-body").unwrap();
    let more = cx.debug_bounds("More tools").unwrap();
    assert_eq!(tab_row.top(), px(0.));
    assert_eq!(tab_row.bottom() + px(1.), toolbar.top());
    assert_eq!(toolbar.bottom(), body.top());
    assert_eq!(body.bottom(), px(720.));
    assert!(more.right() <= px(1040.));
    cx.simulate_resize(size(px(560.), px(360.)));
    cx.run_until_parked();
    cx.update(|window, _| assert_eq!(window.viewport_size().width, px(560.)));
    let save = cx.debug_bounds("Save").unwrap();
    let more = cx.debug_bounds("More tools").unwrap();
    assert_eq!(more.top(), save.top());
    assert!(more.right() <= px(560.));
    assert_eq!(cx.debug_bounds("document-body").unwrap().bottom(), px(360.));
    click_tool(cx, "More tools");
    assert_eq!(cx.debug_bounds("Fewer tools").unwrap(), more);
    let remove_item = cx.debug_bounds("Remove item").unwrap();
    assert!(remove_item.right() <= px(560.));
    click_tool(cx, "Fewer tools");
    cx.update(|_, cx| assert!(!shell.read(cx).show_more_tools));
}

#[gpui::test]
fn main_toolbar_buttons_show_labels_on_hover(cx: &mut TestAppContext) {
    let (_, cx) = cx.add_window_view(Shell::new);
    cx.simulate_resize(size(px(1040.), px(720.)));
    click_tool(cx, "New note");
    for name in [
        "Open Folder…",
        "New note",
        "Save",
        "Heading",
        "Bold",
        "Italic",
        "List",
        "Diagram",
        "Formula",
        "Link",
        "More tools",
    ] {
        assert_eq!(cx.debug_bounds(name).unwrap().size, size(px(30.), px(30.)));
    }
    for (name, tooltip) in [
        ("Heading", "toolbar-tooltip: Heading"),
        ("Open Folder…", "toolbar-tooltip: Open Folder…"),
        ("More tools", "toolbar-tooltip: More tools"),
    ] {
        assert!(cx.debug_bounds(tooltip).is_none());
        let center = cx.debug_bounds(name).unwrap().center();
        cx.simulate_mouse_move(center, None, Modifiers::none());
        cx.run_until_parked();
        cx.cx
            .executor()
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        assert!(cx.debug_bounds(tooltip).is_some());
    }
}

fn start_save_with_queued_quit(
    cx: &mut VisualTestContext,
    shell: &Entity<Shell>,
    file: DocumentFile,
) -> Entity<Editor> {
    let editor = cx.update(|window, cx| {
        bind_quit(window.window_handle().downcast().unwrap(), cx);
        shell.update(cx, |shell, cx| {
            shell.open_editor(Some(file), window, cx);
            let editor = shell.current().unwrap().editor.clone();
            editor.update(cx, |editor, cx| {
                editor.replace_text_in_range(Some(0..8), "my draft", window, cx);
            });
            shell.start(Intent::Save, window, cx);
            // Simulate a platform text callback before the input handler is redrawn.
            editor.update(cx, |editor, cx| {
                editor.replace_text_in_range(Some(0..8), "late input", window, cx);
                assert_eq!(editor.source(), "my draft");
            });
            editor
        })
    });
    // Do not run the executor until both Quit requests reach the shell.
    cx.cx.update(|cx| cx.dispatch_action(&Quit));
    cx.cx.update(|cx| cx.dispatch_action(&Quit));
    cx.update(|_, cx| {
        assert!(matches!(
            shell.read(cx).active,
            Some(ActiveOperation::Save { exit_queued: true })
        ));
    });
    editor
}

#[gpui::test]
fn quit_during_save_closes_after_writing_the_snapshot(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(directory.path()).unwrap();
    let path = workspace.root().join("note.md");
    fs::write(&path, "original").unwrap();
    let file = workspace.read_document(&path).unwrap();
    let (shell, cx) = cx.add_window_view(Shell::new);

    start_save_with_queued_quit(cx, &shell, file);
    cx.run_until_parked();

    assert_eq!(fs::read_to_string(&path).unwrap(), "my draft");
    cx.cx.read(|cx| assert!(cx.windows().is_empty()));
}

#[gpui::test]
fn failed_save_cancels_queued_quit_and_restores_editor_input(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(directory.path()).unwrap();
    let path = workspace.root().join("note.md");
    fs::write(&path, "original").unwrap();
    let file = workspace.read_document(&path).unwrap();
    fs::write(&path, "external update").unwrap();
    let (shell, cx) = cx.add_window_view(Shell::new);

    let editor = start_save_with_queued_quit(cx, &shell, file);
    cx.run_until_parked();

    assert!(!cx.has_pending_prompt());
    assert_eq!(fs::read_to_string(&path).unwrap(), "external update");
    cx.update(|window, cx| {
        let shell = shell.read(cx);
        assert!(shell.active.is_none());
        assert!(shell.error.is_some());
        assert!(shell.current().unwrap().dirty(cx));
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), "my draft");
            editor.replace_text_in_range(Some(0..8), "keep editing", window, cx);
            assert_eq!(editor.source(), "keep editing");
        });
    });
}

#[gpui::test]
fn following_a_link_opens_a_tab_without_losing_the_draft(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let original = fixture.document("notes/original.md", "original");
    let destination = fixture.document("notes/linked.md", "linked");
    let outside = fixture.directory.path().join("outside.md");
    fs::write(outside, "outside the workspace").unwrap();
    let (shell, cx) = cx.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    start(cx, &shell, Intent::Open(original.clone()));
    let (id, editor) = replace_current(cx, &shell, "my draft");
    cx.update(|_, cx| {
        editor.update(cx, |_, cx| cx.emit(OpenLocalDocument("linked.md".into())));
    });
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    let (other, linked) = cx.update(|_, cx| {
        let shell = shell.read(cx);
        let current = shell.current().unwrap();
        assert_eq!(current.path(), Some(destination.as_path()));
        assert_eq!(current.editor.read(cx).source(), "linked");
        assert_eq!(shell.tab(id).unwrap().editor, editor);
        assert_eq!(editor.read(cx).source(), "my draft");
        assert!(shell.tab(id).unwrap().dirty(cx));
        assert_eq!(tab_paths(shell), [&original, &destination]);
        (current.id, current.editor.clone())
    });
    cx.update(|_, cx| {
        linked.update(cx, |_, cx| {
            cx.emit(OpenLocalDocument("../../outside.md".into()))
        });
    });
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.selected, Some(other));
        assert_eq!(tab_paths(shell), [&original, &destination]);
        assert!(shell.error.is_some());
    });
    assert_eq!(fs::read_to_string(original).unwrap(), "original");
}

#[gpui::test]
fn switching_saving_and_closing_wait_for_committed_ime_text(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(directory.path()).unwrap();
    let path = workspace.root().join("note.md");
    fs::write(&path, "original").unwrap();
    let file = workspace.read_document(&path).unwrap();
    let (shell, cx) = cx.add_window_view(Shell::new);
    let (other_id, id, editor) = cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            let other_id = shell.open_editor(None, window, cx);
            let id = shell.open_editor(Some(file), window, cx);
            let editor = shell.current().unwrap().editor.clone();
            editor.update(cx, |editor, cx| {
                editor.replace_and_mark_text_in_range(Some(0..8), "ㄓ", None, window, cx);
            });
            (other_id, id, editor)
        })
    });
    cx.run_until_parked();
    click_tool(cx, tab_selector("tab", other_id));
    cx.update(|window, cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.selected, Some(id));
        assert!(editor.focus_handle(cx).is_focused(window));
        assert!(editor.read(cx).is_composing());
    });
    click_tool(cx, "Hide Sidebar");
    cx.update(|window, cx| {
        assert!(!shell.read(cx).sidebar.visible);
        assert!(editor.focus_handle(cx).is_focused(window));
        assert!(editor.read(cx).is_composing());
    });
    click_tool(cx, "Show Sidebar");
    start(cx, &shell, Intent::Save);
    cx.update(|_, cx| assert!(shell.read(cx).active.is_none()));
    assert!(!cx.simulate_close());
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    cx.update(|window, cx| {
        let shell = shell.read(cx);
        assert!(shell.active.is_none());
        assert!(shell.error.is_some());
        editor.update(cx, |editor, cx| {
            assert!(editor.is_composing());
            editor.replace_text_in_range(None, "中文", window, cx);
            assert_eq!(editor.source(), "中文");
        });
    });
    assert!(!cx.simulate_close());
    cx.run_until_parked();
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    assert_eq!(fs::read_to_string(&path).unwrap(), "中文");
    cx.cx.read(|cx| assert!(cx.windows().is_empty()));
}

#[gpui::test]
fn closing_an_untitled_tab_can_cancel_and_first_save_on_exit_is_restored(app: &mut TestAppContext) {
    let fixture = Fixture::new();
    let path = fixture.workspace.root().join("saved.md");
    let (shell, cx) = app.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    use_session(cx, &shell, &fixture.session_path);
    start(cx, &shell, Intent::New);
    let (id, editor) = replace_current(cx, &shell, "my draft");

    start(cx, &shell, Intent::CloseTab(id));
    answer(cx, "Save");
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    cx.update(|_, cx| {
        let tab = shell.read(cx).current().unwrap();
        assert_eq!(tab.id, id);
        assert!(tab.dirty(cx));
    });
    assert!(!fixture.session_path.exists());

    assert!(!cx.simulate_close());
    cx.run_until_parked();
    answer(cx, "Save");
    assert!(cx.did_prompt_for_new_path());
    assert!(!cx.simulate_close()); // Do not queue another exit during Save on exit.
    cx.simulate_new_path_selection(|root| {
        assert_eq!(root, path.parent().unwrap());
        Some(path.clone())
    });
    cx.run_until_parked();
    assert_eq!(fs::read_to_string(&path).unwrap(), "my draft");
    cx.cx.read(|cx| assert!(cx.windows().is_empty()));

    let (reopened, cx) = app.add_window_view(Shell::new);
    restore(cx, &reopened, &fixture.session_path);
    cx.update(|window, cx| {
        let shell = reopened.read(cx);
        assert_eq!(tab_paths(shell), [&path]);
        let tab = shell.current().unwrap();
        assert_ne!(tab.editor, editor);
        assert_eq!(tab.editor.read(cx).source(), "my draft");
        assert!(tab.editor.focus_handle(cx).is_focused(window));
        assert!(!tab.dirty(cx));
    });
}

#[gpui::test]
fn switching_tabs_preserves_drafts_selection_and_undo_and_save_uses_current_tab(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new();
    let first = fixture.document("a.md", "first");
    let second = fixture.document("b.md", "second");
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.update(|_, cx| editor::bind_keys(cx));
    fixture.open(cx, &shell);

    click_tool(cx, "workspace-file-0");
    let (first_id, first_editor) = replace_current(cx, &shell, "first draft");
    cx.simulate_keystrokes("left shift-left");
    assert_eq!(selection(cx, &first_editor), 9..10);
    click_tool(cx, "workspace-file-1");
    let (_, second_editor) = replace_current(cx, &shell, "second draft");
    assert_eq!(selection(cx, &second_editor), 12..12);

    // Reopening an alias must select the existing editor, not reload the disk contents.
    std::os::unix::fs::symlink(&first, fixture.workspace.root().join("alias.md")).unwrap();
    start(cx, &shell, Intent::Open("alias.md".into()));
    assert!(!cx.has_pending_prompt());
    cx.update(|window, cx| {
        let shell = shell.read(cx);
        assert_eq!(tab_paths(shell), [&first, &second]);
        let tab = shell.current().unwrap();
        assert_eq!(tab.id, first_id);
        assert_eq!(tab.editor, first_editor);
        assert!(tab.editor.focus_handle(cx).is_focused(window));
        assert_eq!(tab.editor.read(cx).source(), "first draft");
        assert_eq!(second_editor.read(cx).source(), "second draft");
    });
    assert_eq!(selection(cx, &first_editor), 9..10);
    click_tool(cx, "Save");
    assert_eq!(fs::read_to_string(first).unwrap(), "first draft");
    assert_eq!(fs::read_to_string(second).unwrap(), "second");

    click_tool(cx, "workspace-file-1");
    assert_eq!(selection(cx, &second_editor), 12..12);
    cx.simulate_keystrokes("cmd-z");
    cx.update(|_, cx| {
        assert_eq!(second_editor.read(cx).source(), "second");
        assert_eq!(first_editor.read(cx).source(), "first draft");
    });
}

#[gpui::test]
fn nested_tree_preserves_selection_and_expansion_when_sidebar_is_hidden(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let note = fixture.document("guides/install/quickstart.md", "Original");
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.simulate_resize(size(px(780.), px(540.)));
    cx.update(|_, cx| cx.bind_keys([KeyBinding::new("cmd-alt-s", ToggleSidebar, None)]));
    fixture.open(cx, &shell);
    assert!(cx.debug_bounds("workspace-file-0").is_none());
    click_tool(cx, "workspace-folder-1");
    click_tool(cx, "workspace-folder-2");
    click_tool(cx, "workspace-file-0");
    let (id, editor) = replace_current(cx, &shell, "Changed");
    let sidebar = cx.debug_bounds("sidebar").unwrap();
    assert_eq!(sidebar.bottom(), px(540.));
    assert_eq!(
        cx.debug_bounds("document-body").unwrap().left(),
        sidebar.right()
    );
    click_tool(cx, "Hide Sidebar");
    cx.update(|_, cx| assert!(!shell.read(cx).sidebar.visible));
    assert_eq!(cx.debug_bounds("document-body").unwrap().left(), px(0.));
    assert!(cx.debug_bounds("Show Sidebar").is_some());
    cx.simulate_keystrokes("cmd-alt-s");
    cx.run_until_parked();
    cx.update(|_, cx| assert!(shell.read(cx).sidebar.visible));
    assert_eq!(
        cx.debug_bounds("document-body").unwrap().left(),
        sidebar.right()
    );

    // Folder refresh preserves the expanded path and the selected draft.
    let fresh = fixture.document("guides/install/reference.md", "Also on disk");
    fixture.open(cx, &shell);
    assert!(cx.debug_bounds("workspace-file-1").is_some());
    click_tool(cx, "workspace-folder-1");
    cx.update(|_, cx| {
        let sidebar = &shell.read(cx).sidebar;
        assert!(
            !sidebar
                .visible_rows(fixture.workspace.root())
                .iter()
                .any(|(path, _, _)| *path == note)
        );
    });
    click_tool(cx, "workspace-folder-1");
    click_tool(cx, "workspace-file-0");
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert!(shell.sidebar.contains(&fresh));
        assert_eq!(shell.current().unwrap().path(), Some(note.as_path()));
        assert_eq!(shell.selected, Some(id));
        assert_eq!(editor.read(cx).source(), "Changed");
    });
}

#[gpui::test]
fn sidebar_views_fit_and_preserve_the_tree_at_minimum_width(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.document("notes/start.md", "# Start");
    for index in 0..30 {
        fixture.document(&format!("file-{index:02}.md"), "contents");
    }
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.simulate_resize(size(px(560.), px(360.)));
    cx.run_until_parked();
    let grip = cx.debug_bounds("resize-sidebar").unwrap().center();
    let button = MouseButton::Left;
    cx.simulate_mouse_down(grip, button, Modifiers::none());
    for distance in [10., 70.] {
        cx.simulate_mouse_move(
            grip - point(px(distance), px(0.)),
            button,
            Modifiers::none(),
        );
    }
    cx.simulate_mouse_up(grip - point(px(70.), px(0.)), button, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(cx.debug_bounds("sidebar").unwrap().size.width, px(184.));
    for (tab, panel) in [
        ("sidebar-tab-Recent", "workspace-recent"),
        ("sidebar-tab-Pins", "workspace-pins"),
        ("sidebar-tab-Outline", "document-outline"),
        ("sidebar-tab-Search", "workspace-search-empty"),
        ("sidebar-tab-Tree", "workspace-tree"),
    ] {
        let bounds = cx.debug_bounds(tab).unwrap();
        assert!(bounds.size.width >= px(28.));
        assert!(bounds.top() >= cx.debug_bounds("Hide Sidebar").unwrap().bottom());
        assert!(bounds.right() <= cx.debug_bounds("resize-sidebar").unwrap().left());
        click_tool(cx, tab);
        assert!(cx.debug_bounds(panel).is_some(), "{panel}");
    }
    fixture.open(cx, &shell);
    click_tool(cx, "workspace-folder-1");
    click_tool(cx, "workspace-file-0");
    let (id, editor) = replace_current(cx, &shell, "# Unsaved heading");
    let list = cx.debug_bounds("workspace-tree").unwrap();
    cx.simulate_event(ScrollWheelEvent {
        position: list.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-260.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    let scrolled = cx.debug_bounds("workspace-file-0").unwrap().top();
    assert!(scrolled < list.top());
    click_tool(cx, "sidebar-tab-Outline");
    assert!(cx.debug_bounds("outline-heading-0").is_some());
    click_tool(cx, "Hide Sidebar");
    click_tool(cx, "Show Sidebar");
    assert!(cx.debug_bounds("outline-heading-0").is_some());
    click_tool(cx, "sidebar-tab-Tree");
    assert_eq!(cx.debug_bounds("workspace-file-0").unwrap().top(), scrolled);
    assert!(cx.debug_bounds("workspace-file-30").is_some());
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.selected, Some(id));
        assert_eq!(editor.read(cx).source(), "# Unsaved heading");
        assert!(shell.current().unwrap().dirty(cx));
    });
}

#[gpui::test]
fn sidebar_drag_reorders_only_the_sidebar_and_only_inside_its_strip(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let first = fixture.document("first.md", "initial");
    let second = fixture.document("second.md", "other");
    let (shell, cx) = cx.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    start(cx, &shell, Intent::Open(first.clone()));
    start(cx, &shell, Intent::Open(second.clone()));
    let (selected, editor) = replace_current(cx, &shell, "unsaved");
    fn drag(cx: &mut VisualTestContext, from: &'static str, to: &'static str) {
        let source = cx.debug_bounds(from).unwrap().center();
        let dest = cx.debug_bounds(to).unwrap().center();
        let button = MouseButton::Left;
        cx.simulate_mouse_down(source, button, Modifiers::none());
        cx.simulate_mouse_move(source + point(px(1.), px(7.)), button, Modifiers::none());
        cx.simulate_mouse_move(dest, button, Modifiers::none());
        cx.simulate_mouse_up(dest, button, Modifiers::none());
        cx.run_until_parked();
    }
    drag(cx, "sidebar-tab-Search", "sidebar-drop-Tree-before");
    let first_position = cx.debug_bounds("sidebar-tab-Search").unwrap().left();
    assert!(first_position < cx.debug_bounds("sidebar-tab-Tree").unwrap().left());
    assert!(cx.debug_bounds("workspace-tree").is_some());
    click_tool(cx, "Hide Sidebar");
    click_tool(cx, "Show Sidebar");
    assert_eq!(
        cx.debug_bounds("sidebar-tab-Search").unwrap().left(),
        first_position
    );
    drag(cx, "sidebar-tab-Search", "document-tabs");
    assert_eq!(
        cx.debug_bounds("sidebar-tab-Search").unwrap().left(),
        first_position
    );
    drag(cx, "sidebar-tab-Search", "sidebar-drop-Outline-after");
    assert!(
        cx.debug_bounds("sidebar-tab-Search").unwrap().left()
            > cx.debug_bounds("sidebar-tab-Outline").unwrap().left()
    );
    assert!(cx.debug_bounds("workspace-tree").is_some());
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.selected, Some(selected));
        assert_eq!(tab_paths(shell), [&first, &second]);
        assert_eq!(editor.read(cx).source(), "unsaved");
    });
}

#[gpui::test]
fn sidebar_pin_can_be_focused_and_toggled_once_per_key_press(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let file = fixture.document("note.md", "");
    let (shell, cx) = cx.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    click_tool(cx, "sidebar-tab-Tree");
    cx.simulate_keystrokes("shift-tab");
    for (key, pinned) in [("enter", true), ("space", false)] {
        cx.simulate_keystrokes(key);
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse(key).unwrap(),
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let shell = shell.read(cx);
            let expected = if pinned {
                std::slice::from_ref(&file)
            } else {
                &[]
            };
            assert_eq!(shell.sidebar.paths().1, expected);
            assert!(shell.tabs.is_empty());
        });
    }
}

#[gpui::test]
fn sidebar_recent_and_pins_follow_successful_opens_and_workspace_changes(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let first = fixture.document("same.md", "initial");
    let second = fixture.document("nested/same.md", "other");
    let destination = Fixture::new();
    let (shell, cx) = cx.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    click_tool(cx, "Pin file");
    cx.update(|_, cx| assert!(shell.read(cx).tabs.is_empty()));
    click_tool(cx, "sidebar-tab-Pins");
    click_tool(cx, "workspace-pins-0");
    let id = cx.update(|_, cx| shell.read(cx).selected.unwrap());
    start(cx, &shell, Intent::CloseTab(id));
    assert!(cx.debug_bounds("workspace-pins-0").is_some());
    click_tool(cx, "workspace-pins-0");
    click_tool(cx, "Unpin file");
    cx.update(|_, cx| assert!(shell.read(cx).sidebar.paths().1.is_empty()));
    assert_eq!(fs::read_to_string(&first).unwrap(), "initial");
    let (id, editor) = replace_current(cx, &shell, "draft");
    start(cx, &shell, Intent::Open(second.clone()));
    start(cx, &shell, Intent::Open(first.clone()));
    start(
        cx,
        &shell,
        Intent::Open(fixture.workspace.root().join("missing.md")),
    );
    click_tool(cx, "sidebar-tab-Recent");
    assert!(cx.debug_bounds("workspace-recent-1").is_some());
    cx.update(|_, cx| {
        assert_eq!(
            shell.read(cx).sidebar.paths().0,
            [first.clone(), second.clone()]
        )
    });
    click_tool(cx, "workspace-recent-0");
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert_eq!(tab_paths(shell), [&first, &second]);
        assert_eq!(shell.selected, Some(id));
        assert_eq!(editor.read(cx).source(), "draft");
    });
    click_tool(cx, "sidebar-tab-Tree");
    click_tool(cx, "Pin file");
    fixture.open(cx, &shell);
    click_tool(cx, "sidebar-tab-Pins");
    let assert_preserved = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let (recent, pins) = shell.read(cx).sidebar.paths();
            assert_eq!(recent, [first.clone(), second.clone()]);
            assert_eq!(pins, std::slice::from_ref(&first));
        })
    };
    assert_preserved(cx);
    let switch = || Intent::OpenFolder(destination.workspace.root().into());
    start(cx, &shell, switch());
    answer(cx, "Cancel");
    assert_preserved(cx);
    start(cx, &shell, switch());
    answer(cx, "Don’t Save");
    cx.update(|_, cx| {
        assert_eq!(
            shell.read(cx).sidebar.paths(),
            ([].as_slice(), [].as_slice())
        )
    });
}

#[gpui::test]
fn sidebar_outline_tracks_drafts_and_tab_changes(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let first = fixture.document("first.md", "# First");
    let second = fixture.document("second.md", "No headings");
    let (shell, cx) = cx.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    start(cx, &shell, Intent::Open(first));
    let first_id = cx.update(|_, cx| shell.read(cx).selected.unwrap());
    start(cx, &shell, Intent::Open(second));
    let second_id = cx.update(|_, cx| shell.read(cx).selected.unwrap());
    click_tool(cx, "sidebar-tab-Outline");
    assert!(cx.debug_bounds("outline-no-headings").is_some());
    click_tool(cx, tab_selector("tab", first_id));
    assert!(cx.debug_bounds("outline-heading-0").is_some());
    let (id, editor) = replace_current(cx, &shell, "# First\n\n## 第二個標題");
    cx.run_until_parked();
    let selected = selection(cx, &editor);
    click_tool(cx, "outline-heading-1");
    assert_eq!(selection(cx, &editor), selected);
    cx.update(|_, cx| assert_eq!(shell.read(cx).selected, Some(id)));
    click_tool(cx, tab_selector("tab", second_id));
    cx.update(|_, cx| {
        let current = shell.read(cx).current().unwrap();
        assert_eq!(current.id, second_id);
        assert!(current.editor.read(cx).headings().is_empty());
    });
    click_tool(cx, tab_selector("tab", first_id));
    assert!(cx.debug_bounds("outline-heading-1").is_some());
}

#[gpui::test]
fn sidebar_search_opens_filename_and_content_results_without_overwriting_drafts(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new();
    let intro = "Line\n\n".repeat(50);
    let first = fixture.document("notes/中文.md", &format!("# Heading\n\n{intro}中文內容"));
    let second = fixture.document("other.md", &format!("{intro}中文例子"));
    let destination = Fixture::new();
    let (shell, cx) = cx.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    click_tool(cx, "sidebar-tab-Search");
    cx.simulate_keystrokes("enter");
    assert!(cx.debug_bounds("search-result-0").is_none());
    cx.simulate_input("中文");
    assert!(cx.debug_bounds("search-result-0").is_none());
    cx.simulate_keystrokes("enter");
    assert!(cx.debug_bounds("search-result-2").is_some());
    click_tool(cx, "search-result-2");
    cx.update(|_, cx| {
        let tab = shell.read(cx).current().unwrap();
        assert_eq!(tab.path(), Some(second.as_path()));
        assert!(tab.editor.read(cx).vertical_scroll_offset() < px(0.));
    });
    click_tool(cx, "search-result-0");
    cx.update(|_, cx| {
        let tab = shell.read(cx).current().unwrap();
        assert_eq!(tab.path(), Some(first.as_path()));
        assert_eq!(tab.editor.read(cx).vertical_scroll_offset(), px(0.));
    });
    click_tool(cx, "search-result-1");
    cx.update(|_, cx| {
        let tab = shell.read(cx).current().unwrap();
        assert!(tab.editor.read(cx).vertical_scroll_offset() < px(0.));
        assert_eq!(shell.read(cx).tabs.len(), 2);
    });
    let (id, editor) = replace_current(cx, &shell, "# Heading\n\nchanged in draft");
    click_tool(cx, "search-result-1");
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.selected, Some(id));
        assert_eq!(tab_paths(shell), [&second, &first]);
        assert!(shell.error.as_deref().unwrap().contains("out of date"));
        assert_eq!(editor.read(cx).source(), "# Heading\n\nchanged in draft");
    });
    fs::remove_file(second).unwrap();
    click_tool(cx, "search-result-2");
    cx.update(|_, cx| assert_eq!(shell.read(cx).selected, Some(id)));
    click_tool(cx, "sidebar-tab-Search");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("absent");
    cx.update(|_, cx| assert_eq!(shell.read(cx).sidebar.search.idle_query(), Some("absent")));
    cx.simulate_keystrokes("enter");
    assert!(cx.debug_bounds("search-no-matches").is_some());
    start(
        cx,
        &shell,
        Intent::OpenFolder(destination.workspace.root().into()),
    );
    answer(cx, "Don’t Save");
    cx.update(|_, cx| assert_eq!(shell.read(cx).sidebar.search.idle_query(), Some("")));
    assert!(cx.debug_bounds("workspace-search-input").is_some());
}

#[gpui::test]
fn first_save_refreshes_an_expanded_nested_folder(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let folder = fixture.workspace.root().join("notes");
    fs::create_dir(&folder).unwrap();
    let (shell, cx) = cx.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    click_tool(cx, "workspace-folder-1");
    click_tool(cx, "New note");
    let (id, _) = replace_current(cx, &shell, "# A nested note");
    let target = folder.join("saved.md");
    click_tool(cx, "Save");
    cx.simulate_new_path_selection(|_| Some(target.clone()));
    cx.run_until_parked();
    click_tool(cx, "workspace-file-0");
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.selected, Some(id));
        assert!(shell.sidebar.contains(&target));
        assert!(!shell.current().unwrap().dirty(cx));
    });
    assert_eq!(fs::read_to_string(&target).unwrap(), "# A nested note");
}

#[gpui::test]
fn dragging_the_sidebar_divider_resizes_only_the_editor_width(cx: &mut TestAppContext) {
    let (_, cx) = cx.add_window_view(Shell::new);
    cx.simulate_resize(size(px(900.), px(540.)));
    let grip = cx.debug_bounds("resize-sidebar").unwrap().center();
    let original = cx.debug_bounds("document-body").unwrap();
    let button = MouseButton::Left;
    cx.simulate_mouse_down(grip, button, Modifiers::none());
    cx.simulate_mouse_move(
        grip + point(px(10.), px(0.)),
        Some(button),
        Modifiers::none(),
    );
    cx.simulate_mouse_move(
        grip + point(px(70.), px(0.)),
        Some(button),
        Modifiers::none(),
    );
    cx.simulate_mouse_up(grip + point(px(70.), px(0.)), button, Modifiers::none());
    cx.run_until_parked();
    let resized = cx.debug_bounds("document-body").unwrap();
    assert!(resized.left() > original.left() + px(60.));
    assert_eq!(resized.right(), original.right());
    assert_eq!(resized.bottom(), original.bottom());
    click_tool(cx, "Hide Sidebar");
    click_tool(cx, "Show Sidebar");
    assert_eq!(cx.debug_bounds("document-body").unwrap(), resized);
}

#[gpui::test]
fn switching_tabs_keeps_the_editor_in_inline_source_mode(cx: &mut TestAppContext) {
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.update(|_, cx| editor::bind_keys(cx));
    start(cx, &shell, Intent::New);
    let (id, editor) = replace_current(cx, &shell, "prefix ");
    click_tool(cx, "Link");
    cx.simulate_keystrokes("left");
    assert_eq!(selection(cx, &editor), 7..7);
    start(cx, &shell, Intent::New);
    click_tool(cx, tab_selector("tab", id));
    assert_eq!(selection(cx, &editor), 7..7);
    cx.simulate_keystrokes("cmd-a");
    let end = cx.update(|_, cx| editor.read(cx).source().len());
    assert!(end > 7);
    // In the link source editor, Select All selects only the link, not the text before it.
    assert_eq!(selection(cx, &editor), 7..end);
}

#[gpui::test]
fn two_untitled_tabs_are_independent_and_first_save_reuses_the_sidebar_document(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new();
    let saved = fixture.workspace.root().join("saved.md");
    let (shell, cx) = cx.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    click_tool(cx, "New note");
    let (first, first_editor) = replace_current(cx, &shell, "first untitled draft");
    click_tool(cx, "New note");
    let (second, second_editor) = replace_current(cx, &shell, "second untitled draft");
    assert_ne!(first, second);
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.tabs.len(), 2);
        assert!(shell.tabs.iter().all(|tab| tab.path().is_none()));
    });

    click_tool(cx, "Save");
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| Some(saved.clone()));
    cx.run_until_parked();
    click_tool(cx, tab_selector("tab", first));
    click_tool(cx, "workspace-file-0");
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.tabs.len(), 2);
        assert!(shell.tab(first).unwrap().dirty(cx));
        assert_eq!(first_editor.read(cx).source(), "first untitled draft");
        let tab = shell.current().unwrap();
        assert_eq!(tab.id, second);
        assert_eq!(tab.editor, second_editor);
        assert_eq!(tab.path(), Some(saved.as_path()));
        assert!(!tab.dirty(cx));
        assert!(shell.sidebar.contains(&saved));
    });
    assert_eq!(fs::read_to_string(saved).unwrap(), "second untitled draft");
}

#[gpui::test]
fn clicking_an_inactive_close_button_does_not_select_it_and_cmd_w_uses_neighbors(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new();
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.simulate_resize(size(px(1040.), px(720.)));
    cx.update(|_, cx| cx.bind_keys([KeyBinding::new("cmd-w", Close, None)]));
    fixture.open(cx, &shell);
    let ids: Vec<_> = ["a.md", "b.md", "c.md", "d.md"]
        .into_iter()
        .map(|name| {
            start(cx, &shell, Intent::Open(fixture.document(name, name)));
            cx.update(|_, cx| shell.read(cx).current().unwrap().id)
        })
        .collect();

    click_tool(cx, tab_selector("tab", ids[1]));
    click_tool(cx, tab_selector("close-tab", ids[3]));
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.selected, Some(ids[1]));
        assert!(shell.tab(ids[3]).is_none());
    });
    for (closed, selected) in [
        (ids[1], Some(ids[2])),
        (ids[2], Some(ids[0])),
        (ids[0], None),
    ] {
        cx.simulate_keystrokes("cmd-w");
        cx.update(|window, cx| {
            let shell = shell.read(cx);
            assert!(shell.tab(closed).is_none());
            assert_eq!(shell.selected, selected);
            if let Some(tab) = shell.current() {
                assert!(tab.editor.focus_handle(cx).is_focused(window));
            }
        });
    }
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert!(shell.tabs.is_empty());
        assert_eq!(shell.workspace.as_ref(), Some(&fixture.workspace));
        assert_eq!(cx.windows().len(), 1);
    });
    cx.simulate_keystrokes("cmd-w");
    cx.cx.read(|cx| assert!(cx.windows().is_empty()));
}

#[gpui::test]
fn overflowing_tabs_scroll_and_keep_the_editor_usable_at_minimum_window_size(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new();
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.simulate_resize(size(px(1040.), px(720.)));
    fixture.open(cx, &shell);
    let mut selectors = Vec::new();
    for index in 0..12 {
        let name = format!("note-{index:02}-long-filename.md");
        start(cx, &shell, Intent::Open(fixture.document(&name, "")));
        let id = cx.update(|_, cx| shell.read(cx).current().unwrap().id);
        selectors.push(tab_selector("tab", id));
    }
    cx.simulate_resize(size(px(560.), px(360.)));
    cx.run_until_parked();
    let row = cx.debug_bounds("document-tabs").unwrap();
    assert!(row.right() <= px(560.));
    assert!(row.bottom() < px(360.));
    let last = *selectors.last().unwrap();
    assert!(row.contains(&cx.debug_bounds(last).unwrap().center()));
    assert!(cx.debug_bounds(selectors[0]).unwrap().right() < row.left());

    cx.simulate_event(ScrollWheelEvent {
        position: row.center(),
        delta: ScrollDelta::Pixels(point(px(5000.), px(0.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    assert!(row.contains(&cx.debug_bounds(selectors[0]).unwrap().center()));
    click_tool(cx, selectors[0]);
    cx.simulate_input("still editable");
    cx.update(|window, cx| {
        let shell = shell.read(cx);
        let current = shell.current().unwrap();
        assert_eq!(current.id, shell.tabs[0].id);
        assert!(current.editor.focus_handle(cx).is_focused(window));
        assert_eq!(current.editor.read(cx).source(), "still editable");
    });
}

#[gpui::test]
fn closing_an_inactive_dirty_tab_only_prompts_for_that_tab(cx: &mut TestAppContext) {
    let (shell, cx) = cx.add_window_view(Shell::new);
    start(cx, &shell, Intent::New);
    let (first, _) = replace_current(cx, &shell, "inactive draft");
    start(cx, &shell, Intent::New);
    let (second, editor) = replace_current(cx, &shell, "active draft");

    start(cx, &shell, Intent::CloseTab(first));
    answer(cx, "Don’t Save");
    assert!(!cx.has_pending_prompt());
    cx.update(|window, cx| {
        let shell = shell.read(cx);
        assert!(shell.tab(first).is_none());
        let current = shell.current().unwrap();
        assert_eq!(current.id, second);
        assert!(current.dirty(cx));
        assert!(editor.focus_handle(cx).is_focused(window));
        assert_eq!(editor.read(cx).source(), "active draft");
    });
}

#[gpui::test]
fn aborted_workspace_switch_keeps_drafts_but_not_already_saved_changes(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let first = fixture.document("a.md", "original a");
    let second = fixture.document("b.md", "original b");
    let third = fixture.document("c.md", "original c");
    let destination = Fixture::new();
    let (shell, cx) = cx.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    let drafts: Vec<_> = [&first, &second, &third]
        .into_iter()
        .zip(["draft a", "draft b", "draft c"])
        .map(|(path, draft)| {
            start(cx, &shell, Intent::Open(path.clone()));
            replace_current(cx, &shell, draft)
        })
        .collect();

    let new_file = fixture.document("added.md", "added on disk");
    fixture.open(cx, &shell);
    assert!(!cx.has_pending_prompt());
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert!(shell.sidebar.contains(&new_file));
        for ((id, editor), draft) in drafts.iter().zip(["draft a", "draft b", "draft c"]) {
            assert_eq!(&shell.tab(*id).unwrap().editor, editor);
            assert_eq!(editor.read(cx).source(), draft);
        }
    });

    let switch = || Intent::OpenFolder(destination.workspace.root().into());
    start(cx, &shell, switch());
    assert!(cx.has_pending_prompt());
    start(cx, &shell, Intent::New); // A pending operation must not accept a new selection.
    cx.update(|window, cx| {
        assert_eq!(shell.read(cx).tabs.len(), 3);
        for ((_, editor), draft) in drafts.iter().zip(["draft a", "draft b", "draft c"]) {
            editor.update(cx, |editor, cx| {
                editor.replace_text_in_range(Some(0..7), "late input", window, cx);
                assert_eq!(editor.source(), draft);
            });
        }
    });
    answer(cx, "Save");
    assert_eq!(fs::read_to_string(&first).unwrap(), "draft a");
    answer(cx, "Don’t Save");
    answer(cx, "Cancel");

    for failed in [false, true] {
        if failed {
            fs::write(&third, "external edit").unwrap();
            start(cx, &shell, switch());
            answer(cx, "Don’t Save");
            answer(cx, "Save");
        }
        assert!(!cx.has_pending_prompt());
        cx.update(|window, cx| {
            let shell = shell.read(cx);
            assert!(shell.active.is_none());
            assert_eq!(shell.error.is_some(), failed);
            assert_eq!(shell.workspace.as_ref(), Some(&fixture.workspace));
            assert_eq!(tab_paths(shell), [&first, &second, &third]);
            assert_eq!(shell.current().unwrap().id, drafts[2].0);
            assert!(drafts[2].1.focus_handle(cx).is_focused(window));
            for (index, ((id, editor), draft)) in drafts
                .iter()
                .zip(["draft a", "draft b", "draft c"])
                .enumerate()
            {
                assert_eq!(editor.read(cx).source(), draft);
                assert_eq!(shell.tab(*id).unwrap().dirty(cx), index != 0);
            }
        });
        assert_eq!(fs::read_to_string(&second).unwrap(), "original b");
        assert_eq!(
            fs::read_to_string(&third).unwrap(),
            if failed {
                "external edit"
            } else {
                "original c"
            }
        );
    }
    start(cx, &shell, switch());
    answer(cx, "Don’t Save");
    answer(cx, "Don’t Save");
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert!(shell.tabs.is_empty());
        assert!(shell.selected.is_none());
        assert_eq!(shell.workspace.as_ref(), Some(&destination.workspace));
    });
}

#[gpui::test]
fn a_destination_that_moves_during_confirmation_does_not_discard_drafts(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let destination = Fixture::new();
    let (shell, cx) = cx.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    start(cx, &shell, Intent::New);
    let (id, editor) = replace_current(cx, &shell, "keep the draft");

    start(
        cx,
        &shell,
        Intent::OpenFolder(destination.workspace.root().into()),
    );
    assert!(cx.has_pending_prompt());
    fs::rename(
        destination.workspace.root(),
        destination.directory.path().join("moved"),
    )
    .unwrap();
    fs::create_dir(destination.workspace.root()).unwrap();
    answer(cx, "Don’t Save");

    cx.update(|window, cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.workspace.as_ref(), Some(&fixture.workspace));
        assert_eq!(shell.selected, Some(id));
        assert!(shell.error.is_some());
        assert!(editor.focus_handle(cx).is_focused(window));
        assert_eq!(editor.read(cx).source(), "keep the draft");
    });
}

#[gpui::test]
fn exit_restores_saved_tab_order_and_selection_but_never_discarded_text(app: &mut TestAppContext) {
    let fixture = Fixture::new();
    let first = fixture.document("z.md", "original z");
    let second = fixture.document("a.md", "original a");
    let (shell, cx) = app.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    use_session(cx, &shell, &fixture.session_path);
    start(cx, &shell, Intent::Open(first.clone()));
    let (_, old_editor) = replace_current(cx, &shell, "not saved to disk");
    start(cx, &shell, Intent::Open(second.clone()));

    // An older snapshot must survive a canceled exit, even when an inactive tab prompted.
    fixture.record(&[&first], Some(&first));
    let previous = fs::read(&fixture.session_path).unwrap();
    start(cx, &shell, Intent::Exit);
    answer(cx, "Cancel");
    assert_eq!(fs::read(&fixture.session_path).unwrap(), previous);
    cx.update(|_, cx| assert_eq!(old_editor.read(cx).source(), "not saved to disk"));

    start(cx, &shell, Intent::Exit);
    answer(cx, "Don’t Save");
    cx.cx.read(|cx| assert!(cx.windows().is_empty()));
    assert_eq!(fs::read_to_string(&first).unwrap(), "original z");
    fs::write(&second, "updated on disk after exit").unwrap();

    let (reopened, cx) = app.add_window_view(Shell::new);
    restore(cx, &reopened, &fixture.session_path);
    cx.update(|window, cx| {
        let shell = reopened.read(cx);
        assert_eq!(shell.workspace.as_ref(), Some(&fixture.workspace));
        assert_eq!(tab_paths(shell), [&first, &second]);
        assert_eq!(shell.tabs[0].editor.read(cx).source(), "original z");
        assert_ne!(shell.tabs[0].editor, old_editor);
        let current = shell.current().unwrap();
        assert_eq!(current.path(), Some(second.as_path()));
        assert_eq!(
            current.editor.read(cx).source(),
            "updated on disk after exit"
        );
        assert!(current.editor.focus_handle(cx).is_focused(window));
        assert!(shell.tabs.iter().all(|tab| !tab.dirty(cx)));
    });
}

#[gpui::test]
fn clean_exit_restores_a_workspace_with_no_open_tabs(app: &mut TestAppContext) {
    let fixture = Fixture::new();
    let first = fixture.document("a.md", "open me from the sidebar");
    let (shell, cx) = app.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    use_session(cx, &shell, &fixture.session_path);
    start(cx, &shell, Intent::Exit);
    assert!(!cx.has_pending_prompt());
    cx.cx.read(|cx| assert!(cx.windows().is_empty()));

    let (reopened, cx) = app.add_window_view(Shell::new);
    restore(cx, &reopened, &fixture.session_path);
    cx.update(|_, cx| {
        let shell = reopened.read(cx);
        assert_eq!(shell.workspace.as_ref(), Some(&fixture.workspace));
        assert!(shell.tabs.is_empty());
        assert!(shell.selected.is_none());
    });
    click_tool(cx, "workspace-file-0");
    cx.update(|_, cx| {
        assert_eq!(
            reopened.read(cx).current().unwrap().path(),
            Some(first.as_path())
        )
    });
}

#[gpui::test]
fn restore_skips_a_missing_selected_file_and_focuses_the_first_survivor(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let first = fixture.document("z.md", "first survivor");
    let missing = fixture.document("b.md", "will be missing");
    let last = fixture.document("a.md", "last survivor");
    fixture.record(&[&first, &missing, &last], Some(&missing));
    fs::remove_file(&missing).unwrap();
    let (shell, cx) = cx.add_window_view(Shell::new);
    restore(cx, &shell, &fixture.session_path);
    cx.update(|window, cx| {
        let shell = shell.read(cx);
        assert_eq!(tab_paths(shell), [&first, &last]);
        let current = shell.current().unwrap();
        assert_eq!(current.path(), Some(first.as_path()));
        assert!(current.editor.focus_handle(cx).is_focused(window));
    });
}

#[gpui::test]
fn missing_and_replaced_workspace_roots_require_manual_selection(app: &mut TestAppContext) {
    let fixture = Fixture::new();
    let root = fixture.workspace.root();
    let first = fixture.document("a.md", "original");
    fixture.record(&[&first], Some(&first));
    fs::rename(root, root.with_extension("moved")).unwrap();
    for replaced in [false, true] {
        if replaced {
            fs::create_dir(root).unwrap();
            fs::write(&first, "replacement").unwrap();
        }
        let (shell, cx) = app.add_window_view(Shell::new);
        restore(cx, &shell, &fixture.session_path);
        cx.update(|_, cx| {
            let shell = shell.read(cx);
            assert!(shell.workspace.is_none());
            assert!(shell.tabs.is_empty());
            assert!(shell.error.is_some());
        });
        if replaced {
            start(cx, &shell, Intent::OpenFolder(root.into()));
            click_tool(cx, "workspace-file-0");
            cx.update(|_, cx| {
                let shell = shell.read(cx);
                assert!(shell.error.is_none());
                assert_eq!(
                    shell.current().unwrap().editor.read(cx).source(),
                    "replacement"
                );
            });
        }
    }
}

#[gpui::test]
fn pending_restore_cannot_replace_a_new_note_or_block_exit(app: &mut TestAppContext) {
    let fixture = Fixture::new();
    let first = fixture.document("a.md", "original");
    fixture.record(&[&first], Some(&first));
    let original = fs::read(&fixture.session_path).unwrap();
    for exiting in [false, true] {
        let (shell, cx) = app.add_window_view(Shell::new);
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.restore_session(fixture.session_path.clone(), window, cx);
                shell.start(if exiting { Intent::Exit } else { Intent::New }, window, cx);
            });
        });
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt());
        if exiting {
            cx.cx.read(|cx| assert!(cx.windows().is_empty()));
        } else {
            cx.update(|window, cx| {
                let shell = shell.read(cx);
                assert!(shell.workspace.is_none());
                assert_eq!(shell.tabs.len(), 1);
                let current = shell.current().unwrap();
                assert!(current.file.is_none());
                assert!(current.editor.focus_handle(cx).is_focused(window));
            });
            // Remove this window without replacing the fixture for the next iteration.
            cx.update(|window, _| window.remove_window());
        }
        assert_eq!(fs::read(&fixture.session_path).unwrap(), original);
    }
}

#[gpui::test]
fn session_write_failure_can_keep_the_window_open_or_exit_without_updating(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new();
    let first = fixture.document("a.md", "original");
    fixture.record(&[&first], Some(&first));
    let original = fs::read(&fixture.session_path).unwrap();
    // The old snapshot is still readable, but the writer refuses a symlinked state directory.
    let state = fixture.session_path.parent().unwrap();
    let moved = state.with_extension("moved");
    fs::rename(state, &moved).unwrap();
    std::os::unix::fs::symlink(moved, state).unwrap();
    let (shell, cx) = cx.add_window_view(Shell::new);
    restore(cx, &shell, &fixture.session_path);
    let (id, editor) = replace_current(cx, &shell, "discard only when exiting");
    start(cx, &shell, Intent::Exit);
    answer(cx, "Don’t Save");
    answer(cx, "Keep Window Open");
    cx.update(|window, cx| {
        let shell = shell.read(cx);
        assert_eq!(shell.selected, Some(id));
        assert!(shell.current().unwrap().dirty(cx));
        assert!(editor.focus_handle(cx).is_focused(window));
    });
    assert_eq!(fs::read(&fixture.session_path).unwrap(), original);

    start(cx, &shell, Intent::Exit);
    answer(cx, "Don’t Save");
    answer(cx, "Quit Without Updating");
    cx.cx.read(|cx| assert!(cx.windows().is_empty()));
    assert_eq!(fs::read(&fixture.session_path).unwrap(), original);
}
