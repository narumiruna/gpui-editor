use std::fs;

use gpui::{Modifiers, TestAppContext, VisualTestContext};

use super::*;

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn open(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("cmd-, tab tab");
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: Keystroke::parse("enter").unwrap(),
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("hotkey-toggle-sidebar").is_some());
}

#[gpui::test]
fn records_and_resets_shortcuts_without_running_them(cx: &mut TestAppContext) {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("state/hotkeys.json");
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.update(|_, cx| {
        Bindings::default().bind(cx);
        shell.update(cx, |shell, _| {
            shell.hotkeys = Hotkeys::load(Some(path.clone()))
        });
    });
    open(cx);
    click(cx, "hotkey-toggle-sidebar");
    cx.simulate_keystrokes("cmd-s");
    cx.update(|_, cx| {
        let state = &shell.read(cx).hotkeys;
        assert_eq!(
            state.notice.as_deref(),
            Some("This shortcut is assigned to Save.")
        );
        assert_eq!(state.bindings, Bindings::default());
    });
    cx.simulate_keystrokes("escape");
    assert!(cx.debug_bounds("Preferences").is_some());
    assert!(!path.exists());
    click(cx, "hotkey-toggle-sidebar");
    cx.simulate_keystrokes("alt-cmd-l");
    cx.run_until_parked();
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert!(shell.sidebar.visible);
        assert_eq!(shell.hotkeys.bindings, Bindings::load(&path).unwrap());
        assert_ne!(shell.hotkeys.bindings, Bindings::default());
    });
    click(cx, "Done");
    cx.simulate_keystrokes("cmd-/");
    cx.update(|_, cx| assert!(shell.read(cx).sidebar.visible));
    cx.simulate_keystrokes("alt-cmd-l");
    cx.update(|_, cx| assert!(!shell.read(cx).sidebar.visible));
    cx.simulate_keystrokes("cmd-p");
    cx.simulate_input("toggle sidebar");
    cx.simulate_keystrokes("enter");
    cx.update(|_, cx| assert!(shell.read(cx).sidebar.visible));
    open(cx);
    click(cx, "hotkeys-reset");
    assert_eq!(Bindings::load(&path).unwrap(), Bindings::default());
    click(cx, "Done");
    cx.simulate_keystrokes("alt-cmd-l");
    cx.update(|_, cx| assert!(shell.read(cx).sidebar.visible));
    cx.simulate_keystrokes("cmd-/");
    cx.update(|_, cx| assert!(!shell.read(cx).sidebar.visible));
}

#[gpui::test]
fn a_write_failure_keeps_the_current_bindings(cx: &mut TestAppContext) {
    let temporary = tempfile::tempdir().unwrap();
    let not_directory = temporary.path().join("not-directory");
    fs::write(&not_directory, "keep this").unwrap();
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.update(|_, cx| {
        Bindings::default().bind(cx);
        shell.update(cx, |shell, _| {
            shell.hotkeys.path = Some(not_directory.join("hotkeys.json"))
        });
    });
    open(cx);
    click(cx, "hotkey-toggle-sidebar");
    cx.simulate_keystrokes("alt-cmd-l");
    cx.run_until_parked();
    cx.update(|_, cx| {
        let state = &shell.read(cx).hotkeys;
        assert_eq!(state.bindings, Bindings::default());
        assert_eq!(
            state.notice.as_deref(),
            Some("Could not save hotkeys. No changes were applied.")
        );
    });
    assert_eq!(fs::read_to_string(not_directory).unwrap(), "keep this");
}

#[test]
fn invalid_settings_use_defaults_without_overwriting_the_file() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("hotkeys.json");
    for data in [
        r#"{"toggle-sidebar":"cmd-n"}"#,
        r#"{"toggle-sidebar":"cmd-q"}"#,
        "[",
    ] {
        fs::write(&path, data).unwrap();
        let settings = Hotkeys::load(Some(path.clone()));
        assert_eq!(settings.bindings, Bindings::default());
        assert!(settings.notice.is_some());
        assert_eq!(fs::read_to_string(&path).unwrap(), data);
    }
}
