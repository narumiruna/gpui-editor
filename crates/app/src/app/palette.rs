//! Modal command search. Commands reuse the shell's normal operation guards.

use gpui::{App, Context, Entity, FocusHandle, Subscription, Window, div, prelude::*, px};

use super::{
    Intent, Shell,
    developer::{PaletteInput, TextField},
    window_chrome::TOP_ROW_HEIGHT,
};
use crate::theme::Theme;

pub(super) struct Palette {
    pub(super) text: Entity<TextField>,
    query: String,
    selected: Option<Command>,
    return_focus: Option<FocusHandle>,
    _subscriptions: [Subscription; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    OpenFolder,
    NewNote,
    Save,
    ToggleSidebar,
}

impl Command {
    const ALL: [Self; 4] = [
        Self::OpenFolder,
        Self::NewNote,
        Self::Save,
        Self::ToggleSidebar,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::OpenFolder => "Open Folder…",
            Self::NewNote => "New note",
            Self::Save => "Save",
            Self::ToggleSidebar => "Toggle Sidebar",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::OpenFolder => "palette-open-folder",
            Self::NewNote => "palette-new-note",
            Self::Save => "palette-save",
            Self::ToggleSidebar => "palette-toggle-sidebar",
        }
    }

    fn enabled(self, shell: &Shell) -> bool {
        self != Self::Save || shell.current().is_some()
    }
}

impl Palette {
    fn matches(&self) -> impl Iterator<Item = Command> + '_ {
        Command::ALL
            .into_iter()
            .filter(|command| command.label().to_ascii_lowercase().contains(&self.query))
    }

    fn selected(&self, shell: &Shell) -> Option<Command> {
        self.selected
            .filter(|selected| {
                selected.enabled(shell) && self.matches().any(|command| command == *selected)
            })
            .or_else(|| self.matches().find(|command| command.enabled(shell)))
    }
}

impl Shell {
    fn palette_available(&self, cx: &App) -> bool {
        self.active.is_none()
            && !self.restoring
            && !self.developer.blocks_document_actions()
            && !self.is_composing(cx)
    }

    pub(super) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() || cx.has_active_drag() || !self.palette_available(cx) {
            return;
        }
        self.cancel_document_drag(window, cx);
        let return_focus = window.focused(cx);
        let text = cx.new(TextField::palette);
        let observe = cx.observe(&text, |shell, text, cx| {
            let Some(palette) = &mut shell.palette else {
                return;
            };
            let query = text.read(cx).text().trim();
            if !palette.query.eq_ignore_ascii_case(query) {
                palette.query = query.to_ascii_lowercase();
                palette.selected = None;
                cx.notify();
            }
        });
        let keys = cx.subscribe_in(&text, window, |shell, _, input, window, cx| {
            shell.palette_input(input, window, cx);
        });
        text.read(cx).focus(window);
        self.palette = Some(Palette {
            text,
            query: String::new(),
            selected: None,
            return_focus,
            _subscriptions: [observe, keys],
        });
        cx.notify();
    }

    pub(super) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette_is_composing(cx) {
            return;
        }
        if let Some(palette) = self.palette.take() {
            if let Some(focus) = palette.return_focus {
                window.focus(&focus);
            } else {
                self.focus_document(window, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn palette_is_composing(&self, cx: &App) -> bool {
        self.palette
            .as_ref()
            .is_some_and(|palette| palette.text.read(cx).is_composing())
    }

    fn palette_input(&mut self, input: &PaletteInput, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette_is_composing(cx) {
            return;
        }
        let Some(palette) = &self.palette else { return };
        let Some(selected) = palette.selected(self) else {
            return;
        };
        if matches!(input, PaletteInput::Run) {
            self.run_palette_command(selected, window, cx);
            return;
        }
        let enabled: Vec<_> = palette
            .matches()
            .filter(|command| command.enabled(self))
            .collect();
        let index = enabled
            .iter()
            .position(|command| *command == selected)
            .unwrap();
        let next = if matches!(input, PaletteInput::Next) {
            (index + 1) % enabled.len()
        } else {
            index.checked_sub(1).unwrap_or(enabled.len() - 1)
        };
        if let Some(palette) = &mut self.palette {
            palette.selected = Some(enabled[next]);
        }
        cx.notify();
    }

    fn run_palette_command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.palette_available(cx)
            || !command.enabled(self)
            || !self
                .palette
                .as_ref()
                .is_some_and(|palette| palette.matches().any(|item| item == command))
        {
            return;
        }
        self.close_palette(window, cx);
        match command {
            Command::OpenFolder => self.start(Intent::ChooseFolder, window, cx),
            Command::NewNote => self.start(Intent::New, window, cx),
            Command::Save => self.start(Intent::Save, window, cx),
            Command::ToggleSidebar => self.toggle_sidebar(window, cx),
        }
    }

    pub(super) fn palette_layer(
        &self,
        theme: Theme,
        _window: &mut Window,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let palette = self.palette.as_ref()?;
        let selected = palette.selected(self);
        let mut commands = palette.matches().peekable();
        let empty = commands.peek().is_none();
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .capture_any_mouse_down(|_, window, _| window.prevent_default())
                // Do not start an underlying drag from a mouse-down before the palette opened.
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .flex()
                .items_start()
                .justify_center()
                .p_3()
                .pt(TOP_ROW_HEIGHT + px(12.))
                .bg(gpui::Hsla::from(theme.surface).opacity(0.38))
                .child(
                    div()
                        .id("command-palette")
                        .debug_selector(|| "command-palette".into())
                        .w_full()
                        .max_w(px(440.))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_3()
                        .rounded_md()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.panel)
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("Command Palette"),
                        )
                        .child(palette.text.clone())
                        .when(empty, |panel| {
                            panel.child(
                                div()
                                    .debug_selector(|| "palette-empty".into())
                                    .p_2()
                                    .text_color(theme.muted)
                                    .child("No matching commands"),
                            )
                        })
                        .children(commands.map(|command| {
                            let id = command.id();
                            let enabled = command.enabled(self);
                            div()
                                .id(id)
                                .debug_selector(move || id.into())
                                .px_2()
                                .py_1()
                                .min_h(px(30.))
                                .flex()
                                .items_center()
                                .rounded_md()
                                .when(selected == Some(command), |row| {
                                    row.bg(theme.selected).text_color(theme.accent)
                                })
                                .when(!enabled, |row| row.text_color(theme.muted))
                                .when(enabled, |row| {
                                    row.cursor_pointer()
                                        .hover(move |style| style.bg(theme.selected))
                                        .on_click(cx.listener(move |shell, _, window, cx| {
                                            shell.run_palette_command(command, window, cx);
                                        }))
                                })
                                .child(command.label())
                        })),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Focusable as _, Modifiers, TestAppContext};

    use super::*;

    #[gpui::test]
    fn clicking_disabled_save_keeps_palette_focus_then_new_note_runs(cx: &mut TestAppContext) {
        let (shell, cx) = cx.add_window_view(Shell::new);
        cx.update(|window, cx| shell.update(cx, |shell, cx| shell.open_palette(window, cx)));
        cx.run_until_parked();

        let save = cx.debug_bounds("palette-save").unwrap().center();
        cx.simulate_click(save, Modifiers::none());
        cx.simulate_keystrokes("tab shift-tab");
        cx.update(|window, cx| {
            let shell = shell.read(cx);
            assert!(shell.tabs.is_empty());
            let field = shell.palette.as_ref().unwrap().text.read(cx);
            assert!(field.focus_handle(cx).is_focused(window));
        });

        let new_note = cx.debug_bounds("palette-new-note").unwrap().center();
        cx.simulate_click(new_note, Modifiers::none());
        cx.update(|window, cx| {
            let shell = shell.read(cx);
            assert!(shell.palette.is_none());
            let editor = shell.current().unwrap().editor.read(cx);
            assert!(editor.focus_handle(cx).is_focused(window));
        });
    }
}
