use std::time::Duration;

use gpui::{Context, KeyDownEvent, Keystroke, KeystrokeEvent, Window};
use gpui_editor_codemirror::{
    commands::bindings::toggle_tab_focus_mode,
    view::keymap::{KeyEvent, KeyModifiers},
};

use super::Editor;

impl Editor {
    fn key_time(&self, cx: &Context<Self>) -> Duration {
        cx.background_executor()
            .now()
            .saturating_duration_since(self.selection_epoch)
    }

    pub(super) fn handle_editor_tab(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.focus.is_focused(window) || self.is_composing() {
            return false;
        }
        let key = &event.keystroke;
        let now = self.key_time(cx);
        let bypass = self.tab_focus.bypass_key_bindings(editor_key(key), now);
        let mods = key.modifiers;
        if key.key != "tab" || mods.platform || mods.control || mods.alt || mods.function {
            return false;
        }
        if bypass || !self.move_source_snippet(mods.shift, cx) {
            if mods.shift {
                window.focus_prev();
            } else {
                window.focus_next();
            }
        }
        window.prevent_default();
        cx.stop_propagation();
        true
    }

    pub(super) fn observe_editor_key(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus.is_focused(window) || self.is_composing() {
            return;
        }
        let key = editor_key(&event.keystroke);
        let now = self.key_time(cx);
        // GPUI dispatches bound actions before the raw key listener.
        self.tab_focus.bypass_key_bindings(key, now);
        if event.action.is_none() && !window.default_prevented() {
            self.tab_focus.handle_unhandled_key(key, now);
        }
    }

    pub(super) fn toggle_editor_tab_focus(&mut self, cx: &Context<Self>) -> bool {
        if self.is_composing() {
            return false;
        }
        // The toggle shortcut is a non-modifier key and cancels temporary mode first.
        let now = self.key_time(cx);
        self.tab_focus
            .bypass_key_bindings(KeyEvent::new("m", KeyModifiers::default()), now);
        toggle_tab_focus_mode(&mut self.tab_focus)
    }
}

fn editor_key(keystroke: &Keystroke) -> KeyEvent<'_> {
    let key = match keystroke.key.as_str() {
        "tab" => "Tab",
        "escape" => "Escape",
        "alt" => "Alt",
        "control" => "Control",
        "platform" => "Meta",
        "shift" => "Shift",
        "capslock" => "CapsLock",
        "altgraph" => "AltGraph",
        "function" => "Fn",
        other => other,
    };
    let modifiers = keystroke.modifiers;
    let mut event = KeyEvent::new(
        key,
        KeyModifiers {
            alt: modifiers.alt,
            control: modifiers.control,
            meta: modifiers.platform,
            shift: modifiers.shift,
        },
    );
    event.modifier_only |= key == "Fn";
    event
}

#[cfg(test)]
mod tests {
    use gpui::{Entity, FocusHandle, Render, TestAppContext, div, prelude::*};
    use gpui_editor_codemirror::{
        autocomplete::{Snippet, start_snippet},
        commands::bindings::TabFocusMode,
    };

    use super::*;
    use crate::editor::bind_keys;

    struct DocumentTab {
        editor: Entity<Editor>,
        other: FocusHandle,
    }

    impl DocumentTab {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
            Self {
                editor: cx.new(|cx| Editor::new(String::new(), window, cx)),
                other: cx.focus_handle().tab_index(1).tab_stop(true),
            }
        }
    }

    impl Render for DocumentTab {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .child(self.editor.clone())
                .child(div().track_focus(&self.other).child("Next control"))
        }
    }

    fn install_snippet(editor: &mut Editor, cx: &mut Context<Editor>) {
        let range = 0..editor.source().len();
        let (insert, session) = Snippet::parse("${1:first} ${2:second}")
            .insert(editor.view.state(), range, "  ")
            .unwrap();
        assert!(
            editor.apply_prose_transaction(
                insert.with_effects([start_snippet(session.unwrap())]),
                cx
            )
        );
    }

    #[gpui::test]
    fn tab_advances_a_snippet_until_the_focus_mode_is_enabled(cx: &mut TestAppContext) {
        let (root, cx) = cx.add_window_view(DocumentTab::new);
        let (editor, other) = cx.update(|window, cx| {
            bind_keys(cx);
            let root = root.read(cx);
            let editor = root.editor.clone();
            let other = root.other.clone();
            editor.update(cx, |editor, cx| {
                window.focus(&editor.focus);
                install_snippet(editor, cx);
            });
            (editor, other)
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("tab");
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            assert!(editor.focus.is_focused(window));
            assert_eq!(&editor.source()[editor.selection()], "second");
        });
        cx.update(|_, cx| editor.update(cx, install_snippet));
        cx.run_until_parked();
        cx.simulate_keystrokes("alt-shift-m");
        cx.update(|_, cx| assert_eq!(editor.read(cx).tab_focus, TabFocusMode::Persistent));
        cx.simulate_keystrokes("shift-tab");
        cx.update(|window, cx| {
            assert!(other.is_focused(window));
            assert_eq!(editor.read(cx).source(), "first second");
        });
    }

    #[gpui::test]
    fn only_an_unhandled_escape_enables_temporary_tab_focus(cx: &mut TestAppContext) {
        let (root, cx) = cx.add_window_view(DocumentTab::new);
        let (editor, other) = cx.update(|window, cx| {
            bind_keys(cx);
            let root = root.read(cx);
            let editor = root.editor.clone();
            let other = root.other.clone();
            editor.update(cx, |editor, cx| {
                window.focus(&editor.focus);
                install_snippet(editor, cx);
            });
            (editor, other)
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("escape");
        cx.update(|_, cx| {
            assert_eq!(editor.read(cx).tab_focus, TabFocusMode::Disabled);
            assert!(!editor.read(cx).has_source_snippet());
        });
        cx.simulate_keystrokes("escape");
        cx.update(|_, cx| {
            assert!(matches!(
                editor.read(cx).tab_focus,
                TabFocusMode::TemporaryUntil(_)
            ))
        });
        cx.simulate_keystrokes("cmd-a");
        cx.update(|_, cx| assert_eq!(editor.read(cx).tab_focus, TabFocusMode::Disabled));
        cx.simulate_keystrokes("escape");
        let clock = cx.update(|_, cx| {
            editor.update(cx, install_snippet);
            cx.background_executor().clone()
        });
        clock.advance_clock(Duration::from_millis(2001));
        cx.simulate_keystrokes("tab");
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                assert!(editor.focus.is_focused(window));
                assert_eq!(&editor.source()[editor.selection()], "second");
                editor.cancel_source_snippet(cx);
            })
        });
        cx.simulate_keystrokes("escape");
        cx.update(|_, cx| editor.update(cx, install_snippet));
        cx.run_until_parked();
        cx.simulate_keystrokes("tab");
        cx.update(|window, _| assert!(other.is_focused(window)));
    }
}
