//! Colors for the editor window, selected from the native appearance.

use gpui::{Rgba, Window, WindowAppearance, rgb, rgba};

#[derive(Clone, Copy)]
pub(crate) struct Theme {
    pub surface: Rgba,
    pub panel: Rgba,
    pub toolbar: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub accent: Rgba,
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
}

impl Theme {
    pub fn for_window(window: &Window) -> Self {
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let choose = |light, night| rgb(if dark { night } else { light });
        Self {
            surface: choose(0xffffff, 0x181b21),
            panel: choose(0xf0f3f9, 0x20252e),
            toolbar: choose(0xf8faff, 0x1c2028),
            text: choose(0x172336, 0xe9edf6),
            muted: choose(0x475569, 0xa5b1c4),
            accent: choose(0x174fc3, 0x92b8ff),
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
        }
    }
}
