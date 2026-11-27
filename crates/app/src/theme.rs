//! Colors for the window and its document editor.

use gpui::{Rgba, Window, WindowAppearance, rgb, rgba};
use gpui_editor_codemirror::{
    EditorState,
    theme_one_dark::{ONE_DARK_THEME, ViewStyle, editor_surface_colors, view_style},
};

#[derive(Clone, Copy)]
pub(crate) struct Theme {
    pub surface: Rgba,
    pub panel: Rgba,
    pub toolbar: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
    pub syntax_keyword: Rgba,
    pub syntax_string: Rgba,
    pub syntax_constant: Rgba,
    pub selected: Rgba,
    pub border: Rgba,
    pub tooltip: Rgba,
    pub on_tooltip: Rgba,
    pub error_bg: Rgba,
    pub error: Rgba,
    pub selection: Rgba,
    pub preview: Rgba,
    pub preview_header: Rgba,
    pub on_accent: Rgba,
    pub view: Option<&'static ViewStyle>,
}

impl Theme {
    pub fn is_system_dark(window: &Window) -> bool {
        Self::is_dark(window.appearance())
    }

    fn is_dark(appearance: WindowAppearance) -> bool {
        matches!(
            appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        )
    }

    /// Keep the window chrome unchanged. Apply One Dark roles to the editor.
    pub fn for_editor(window: &Window, state: &EditorState) -> Self {
        let system_dark = Self::is_system_dark(window);
        let installed = view_style(state);
        let view = installed.or(system_dark.then_some(&ONE_DARK_THEME));
        let mut theme = Self::palette(view.map_or(system_dark, |style| style.dark));
        theme.view = view;
        if installed.is_some_and(|style| style.dark) {
            // CodeMirror's dark tooltip base sets white. One Dark overrides its fill only.
            theme.on_tooltip = rgb(0xffffff);
        }
        if let Some(style) = view {
            theme.surface = rgba(style.editor.background.rgba32());
            theme.text = rgba(style.editor.foreground.rgba32());
            theme.accent = rgba(style.caret.rgba32());
            theme.selection = rgba(style.selection.rgba32());
            theme.panel = rgba(style.panels.background.rgba32());
            theme.border = rgba(style.panels.top_border.color.rgba32());
        }
        let colors = editor_surface_colors(state);
        if let Some(background) = colors.background {
            theme.surface = rgba(background.rgba32());
        }
        if let Some(foreground) = colors.foreground {
            theme.text = rgba(foreground.rgba32());
        }
        theme
    }

    pub fn panel_text(self) -> Rgba {
        self.view
            .map_or(self.text, |style| rgba(style.panels.foreground.rgba32()))
    }

    pub fn for_window(window: &Window) -> Self {
        Self::palette(Self::is_system_dark(window))
    }

    fn palette(dark: bool) -> Self {
        let choose = |light, night| rgb(if dark { night } else { light });
        Self {
            surface: choose(0xffffff, 0x181b21),
            panel: choose(0xf0f3f9, 0x20252e),
            toolbar: choose(0xf8faff, 0x1c2028),
            text: choose(0x172336, 0xe9edf6),
            muted: choose(0x475569, 0xa5b1c4),
            accent: choose(0x174fc3, 0x92b8ff),
            syntax_keyword: choose(0x782080, 0xf0a7df),
            syntax_string: choose(0x116530, 0x8ee3a0),
            syntax_constant: choose(0x9a3412, 0xffbd82),
            selected: choose(0xe3ecfc, 0x293f62),
            border: choose(0xd3dce9, 0x384152),
            tooltip: choose(0x1e293b, 0xe9edf6),
            on_tooltip: choose(0xffffff, 0x172336),
            error_bg: choose(0xfef2f2, 0x4b232b),
            error: choose(0x991b1b, 0xffcacd),
            selection: rgba(if dark { 0x80aaff68 } else { 0x4a86ed50 }),
            preview: choose(0xf6f8fc, 0x232832),
            preview_header: choose(0xeaf0f8, 0x2c3545),
            on_accent: choose(0xffffff, 0x162439),
            view: None,
        }
    }
}
