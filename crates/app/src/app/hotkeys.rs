use std::{
    collections::BTreeMap,
    io::{self, Read as _},
    path::{Path, PathBuf},
};

use gpui::{
    App, Context, Div, KeyBinding, Keystroke, SharedString, Subscription, Window, div, prelude::*,
    px,
};

use super::{
    ChooseFolder, Close, NewDocument, OpenPalette, Preferences, Quit, SaveDocument, Shell,
    TextField, ToggleSidebar, state_file,
};
use crate::{editor, theme::Theme};

#[cfg(test)]
mod tests;

struct Command {
    id: &'static str,
    label: &'static str,
    default: &'static str,
    bind: fn(&str) -> KeyBinding,
}

const COMMANDS: [Command; 6] = [
    Command {
        id: "open-folder",
        label: "Open Folder",
        default: "secondary-o",
        bind: |key| KeyBinding::new(key, ChooseFolder, None),
    },
    Command {
        id: "new-document",
        label: "New Document",
        default: "secondary-n",
        bind: |key| KeyBinding::new(key, NewDocument, None),
    },
    Command {
        id: "save",
        label: "Save",
        default: "secondary-s",
        bind: |key| KeyBinding::new(key, SaveDocument, None),
    },
    Command {
        id: "close-tab",
        label: "Close Tab",
        default: "secondary-w",
        bind: |key| KeyBinding::new(key, Close, None),
    },
    Command {
        id: "toggle-sidebar",
        label: "Toggle Sidebar",
        default: "secondary-/",
        bind: |key| KeyBinding::new(key, ToggleSidebar, None),
    },
    Command {
        id: "command-palette",
        label: "Command Palette",
        default: "secondary-p",
        bind: |key| KeyBinding::new(key, OpenPalette, None),
    },
];
const FIXED: &[&str] = &[
    "secondary-q",
    "secondary-shift-q",
    "secondary-,",
    "secondary-a",
    "secondary-c",
    "secondary-x",
    "secondary-v",
    "secondary-z",
    "secondary-shift-z",
    "secondary-h",
    "secondary-m",
    "secondary-`",
    "secondary-~",
];
const MAX_BYTES: u64 = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Bindings([String; COMMANDS.len()]);

impl Default for Bindings {
    fn default() -> Self {
        Self(COMMANDS.map(|command| normalize(command.default)))
    }
}

impl Bindings {
    pub(super) fn bind(&self, cx: &mut App) {
        cx.bind_keys(
            COMMANDS
                .iter()
                .zip(&self.0)
                .map(|(command, key)| (command.bind)(key)),
        );
        cx.bind_keys([
            KeyBinding::new("secondary-q", Quit, None),
            KeyBinding::new("secondary-,", Preferences, None),
        ]);
    }

    fn load(path: &Path) -> io::Result<Self> {
        let Some(file) = state_file::open(path)? else {
            return Ok(Self::default());
        };
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid("shortcut settings are too large"));
        }
        let saved: BTreeMap<String, String> = serde_json::from_slice(&bytes).map_err(invalid)?;
        let mut bindings = Self::default();
        for (index, command) in COMMANDS.iter().enumerate() {
            if let Some(key) = saved.get(command.id) {
                bindings.0[index] = parse(key).map_err(invalid)?;
            }
        }
        for (index, key) in bindings.0.iter().enumerate() {
            if bindings.conflict(index, key).is_some() {
                return Err(invalid("duplicate shortcuts"));
            }
        }
        Ok(bindings)
    }

    fn save(&self, path: &Path) -> io::Result<()> {
        let overrides: BTreeMap<_, _> = COMMANDS
            .iter()
            .zip(&self.0)
            .filter(|(command, key)| normalize(command.default) != key.as_str())
            .map(|(command, key)| (command.id, key))
            .collect();
        state_file::write_atomically(path, |file| {
            serde_json::to_writer(file, &overrides).map_err(io::Error::other)
        })
    }

    fn conflict(&self, command: usize, key: &str) -> Option<&'static str> {
        self.0.iter().enumerate().find_map(|(index, current)| {
            (index != command && current == key).then_some(COMMANDS[index].label)
        })
    }
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

fn normalize(key: &str) -> String {
    Keystroke::parse(key)
        .expect("valid built-in shortcut")
        .unparse()
}

fn parse(key: &str) -> Result<String, &'static str> {
    let required = if cfg!(target_os = "macos") {
        "Use Cmd with a letter, number, or punctuation key."
    } else {
        "Use Ctrl with a letter, number, or punctuation key."
    };
    let chord = Keystroke::parse(key).map_err(|_| required)?;
    if !chord.modifiers.secondary()
        || chord.modifiers.function
        || chord.key_char.is_some()
        || chord.key.len() != 1
        || !chord.key.as_bytes()[0].is_ascii_graphic()
    {
        return Err(required);
    }
    let key = chord.unparse();
    if FIXED.iter().any(|fixed| normalize(fixed) == key) {
        return Err("This shortcut is reserved.");
    }
    Ok(key)
}

fn label(key: &str) -> String {
    let chord = Keystroke::parse(key).expect("validated shortcut");
    let mut label = String::new();
    if chord.modifiers.platform {
        label.push_str(if cfg!(target_os = "macos") {
            "Cmd+"
        } else {
            "Win+"
        });
    }
    if chord.modifiers.control {
        label.push_str("Ctrl+");
    }
    if chord.modifiers.alt {
        label.push_str(if cfg!(target_os = "macos") {
            "Option+"
        } else {
            "Alt+"
        });
    }
    if chord.modifiers.shift {
        label.push_str("Shift+");
    }
    label.push_str(&chord.key.to_ascii_uppercase());
    label
}

#[derive(Default)]
pub(super) struct Hotkeys {
    pub(super) bindings: Bindings,
    path: Option<PathBuf>,
    recording: Option<usize>,
    saving: bool,
    notice: Option<String>,
}

impl Hotkeys {
    pub(super) fn load(path: Option<PathBuf>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        let (bindings, notice) = match Bindings::load(&path) {
            Ok(bindings) => (bindings, None),
            Err(_) => (
                Bindings::default(),
                Some("Could not read the saved hotkeys. Using defaults.".into()),
            ),
        };
        Self {
            bindings,
            path: Some(path),
            notice,
            ..Self::default()
        }
    }

    pub(super) fn cancel_recording(&mut self) {
        if self.recording.take().is_some() {
            self.notice = None;
        }
    }
}

impl Shell {
    pub(super) fn intercept_hotkeys(window: &Window, cx: &mut Context<Self>) -> Subscription {
        let shell = cx.weak_entity();
        let handle = window.window_handle();
        cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() == handle
                && shell
                    .update(cx, |shell, cx| {
                        shell.record_hotkey(&event.keystroke, window, cx)
                    })
                    .unwrap_or(false)
            {
                window.prevent_default();
                cx.stop_propagation();
            }
        })
    }

    fn record_hotkey(
        &mut self,
        key: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(index) = self
            .hotkeys
            .recording
            .filter(|_| self.developer.hotkeys_open())
        else {
            return false;
        };
        if !key.modifiers.modified() && key.key == "escape" {
            self.hotkeys.cancel_recording();
        } else if key.modifiers.secondary()
            || !matches!(
                key.key.as_str(),
                "shift" | "control" | "alt" | "platform" | "function"
            )
        {
            match parse(&key.unparse()) {
                Ok(key) => {
                    if let Some(label) = self.hotkeys.bindings.conflict(index, &key) {
                        self.hotkeys.notice =
                            Some(format!("This shortcut is assigned to {label}."));
                    } else {
                        let mut updated = self.hotkeys.bindings.clone();
                        updated.0[index] = key;
                        self.save_hotkeys(updated, window, cx);
                    }
                }
                Err(error) => self.hotkeys.notice = Some(error.into()),
            }
        }
        cx.notify();
        true
    }

    fn save_hotkeys(&mut self, bindings: Bindings, window: &mut Window, cx: &mut Context<Self>) {
        if self.hotkeys.saving {
            return;
        }
        self.hotkeys.cancel_recording();
        let Some(path) = self.hotkeys.path.clone() else {
            self.hotkeys.notice =
                Some("Could not find the settings folder. No changes were applied.".into());
            cx.notify();
            return;
        };
        self.hotkeys.saving = true;
        self.hotkeys.notice = None;
        cx.notify();
        cx.spawn_in(window, async move |shell, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { bindings.save(&path).map(|()| bindings) })
                .await;
            let _ = shell.update_in(cx, |shell, _, cx| {
                shell.hotkeys.saving = false;
                match result {
                    Ok(bindings) => {
                        shell.hotkeys.bindings = bindings;
                        cx.clear_key_bindings();
                        editor::bind_keys(cx);
                        TextField::bind_keys(cx);
                        shell.hotkeys.bindings.bind(cx);
                        #[cfg(target_os = "macos")]
                        super::set_menus(cx);
                    }
                    Err(_) => {
                        shell.hotkeys.notice =
                            Some("Could not save hotkeys. No changes were applied.".into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn hotkeys_panel(&self, theme: Theme, cx: &Context<Self>) -> Div {
        let state = &self.hotkeys;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_xs().text_color(theme.muted).child(
                "Select a shortcut, then press the new key combination. Press Esc to cancel.",
            ))
            .children(COMMANDS.iter().enumerate().map(|(index, command)| {
                let recording = state.recording == Some(index);
                let text = if recording {
                    "Press a shortcut…".into()
                } else {
                    label(&state.bindings.0[index])
                };
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(command.label)
                    .child(
                        control(format!("hotkey-{}", command.id), text, theme, state.saving)
                            .when(recording, |el| el.border_color(theme.accent))
                            .on_click(cx.listener(move |shell, _, _, cx| {
                                if !shell.hotkeys.saving {
                                    shell.hotkeys.recording = Some(index);
                                    shell.hotkeys.notice = None;
                                    cx.notify();
                                }
                            })),
                    )
            }))
            .when_some(state.notice.clone(), |el, notice| {
                el.child(div().text_color(theme.error).child(notice))
            })
            .when(state.saving, |el| {
                el.child(div().text_color(theme.muted).child("Saving hotkeys…"))
            })
            .child(
                div().flex().child(
                    control(
                        "hotkeys-reset".into(),
                        "Reset Defaults".into(),
                        theme,
                        state.saving,
                    )
                    .on_click(cx.listener(|shell, _, window, cx| {
                        shell.save_hotkeys(Bindings::default(), window, cx)
                    })),
                ),
            )
    }
}

fn control(id: String, text: String, theme: Theme, disabled: bool) -> gpui::Stateful<Div> {
    div()
        .id(SharedString::from(id.clone()))
        .debug_selector(move || id.clone())
        .tab_index(0)
        .min_w(px(138.))
        .px_2()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .bg(theme.panel)
        .text_color(if disabled { theme.muted } else { theme.text })
        .focus(move |el| el.border_color(theme.accent))
        .when(!disabled, |el| {
            el.cursor_pointer().hover(move |el| el.bg(theme.selected))
        })
        .child(text)
}
