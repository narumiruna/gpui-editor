//! Native extensions and style values from `@codemirror/theme-one-dark` 6.1.3.
//!
//! Install [`one_dark_theme`] for view roles, [`one_dark_highlighting`] for
//! syntax styles, or [`one_dark`] for both. Use separate [compartments](crate::Compartment)
//! to reconfigure the two parts independently. Read the current configuration
//! with [`view_style`] and [`highlighting_for`]. [`COLOR`], [`ONE_DARK_THEME`],
//! [`ONE_DARK_HIGHLIGHT_STYLE`], and [`ONE_DARK`] are also available as data.
//!
//! These extensions do not install a parser, widgets, or a renderer. The host
//! supplies the span tags and region scope, and owns focus, selection drawing,
//! and widget layout. For GPUI, pass [`Color::rgba32`] to `gpui::rgba`. The
//! view roles do not model DOM classes, selector specificity, or base themes.
//! The syntax resolver uses native properties, not CSS classes or ancestor
//! decoration. WAST spans can use an exact root row or its language-data facet.
//!
//! See `docs/licenses/codemirror-theme-one-dark.txt` for the upstream MIT notice.

mod highlight;
mod install;

pub use highlight::{
    HighlightLayer, HighlightStyle, ONE_DARK_HIGHLIGHT_STYLE, TextDecoration, highlight_style,
    layered_highlight_style,
};
pub use install::{
    EditorSurfaceColors, editor_surface_colors, highlight_configuration, highlight_layer,
    highlighting_for, highlighting_for_native, one_dark, one_dark_highlighting, one_dark_theme,
    override_editor_surface, view_style,
};

/// An sRGB color with an 8-bit alpha channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Color(u32);

impl Color {
    /// Creates a color from `0xRRGGBBAA`. An alpha of `0xff` is opaque.
    pub const fn from_rgba32(rgba: u32) -> Self {
        Self(rgba)
    }

    /// Returns `0xRRGGBBAA`. An alpha of `0xff` is opaque.
    pub const fn rgba32(self) -> u32 {
        self.0
    }
}

const fn rgb(rgb: u32) -> Color {
    Color((rgb << 8) | 0xff)
}

/// The named colors exported as `color` by One Dark.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    pub chalky: Color,
    pub coral: Color,
    pub cyan: Color,
    pub invalid: Color,
    pub ivory: Color,
    pub stone: Color,
    pub malibu: Color,
    pub sage: Color,
    pub whiskey: Color,
    pub violet: Color,
    pub dark_background: Color,
    pub highlight_background: Color,
    pub background: Color,
    pub tooltip_background: Color,
    pub selection: Color,
    pub cursor: Color,
}

pub const COLOR: Palette = Palette {
    chalky: rgb(0xe5c07b),
    coral: rgb(0xe06c75),
    cyan: rgb(0x56b6c2),
    invalid: rgb(0xffffff),
    ivory: rgb(0xabb2bf),
    stone: rgb(0x7d8799),
    malibu: rgb(0x61afef),
    sage: rgb(0x98c379),
    whiskey: rgb(0xd19a66),
    violet: rgb(0xc678dd),
    dark_background: rgb(0x21252b),
    highlight_background: rgb(0x2c313a),
    background: rgb(0x282c34),
    tooltip_background: rgb(0x353a42),
    selection: rgb(0x3e4451),
    cursor: rgb(0x528bff),
};

/// Foreground text and background fill colors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceStyle {
    pub foreground: Color,
    pub background: Color,
}

/// A solid stroke. Convert the width from CSS pixels to native logical pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stroke {
    pub width_px: u8,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PanelStyle {
    pub foreground: Color,
    pub background: Color,
    /// The bottom border of a panel at the top of the editor.
    pub top_border: Stroke,
    /// The top border of a panel at the bottom of the editor.
    pub bottom_border: Stroke,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchMatchStyle {
    pub background: Color,
    /// Stroke around the match that does not change text layout.
    pub outline: Stroke,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramedSurfaceStyle {
    pub foreground: Color,
    pub background: Color,
    /// `None` sets `border: none`; it does not mean to keep a default border.
    pub border: Option<Stroke>,
}

/// Colors of the top and bottom edges of one tooltip arrow layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArrowBorderColors {
    pub top: Color,
    pub bottom: Color,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TooltipStyle {
    pub background: Color,
    /// `None` sets `border: none`; it does not mean to keep a default border.
    pub border: Option<Stroke>,
    /// Border colors for the arrow's `:before` layer.
    pub arrow_before: ArrowBorderColors,
    /// Border colors for the arrow's `:after` layer.
    pub arrow_after: ArrowBorderColors,
}

/// Native roles for the properties set by `oneDarkTheme`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewStyle {
    pub dark: bool,
    /// Editor root (`&`).
    pub editor: SurfaceStyle,
    /// Native text caret (`.cm-content`'s `caret-color`), unless the host hides it.
    pub caret: Color,
    /// Drawn cursor (`.cm-cursor`'s left border color).
    pub cursor: Color,
    /// Drag-and-drop insertion cursor (`.cm-dropCursor`'s left border color).
    pub drop_cursor: Color,
    /// Background of drawn selections and visible native text selections.
    /// One Dark sets this color both with and without focus.
    pub selection: Color,
    /// Editor panels (`.cm-panels`) and their connecting borders.
    pub panels: PanelStyle,
    /// Search result (`.cm-searchMatch`).
    pub search_match: SearchMatchStyle,
    /// Current search result (`.cm-searchMatch.cm-searchMatch-selected`).
    pub selected_search_match: SearchMatchStyle,
    /// Background of the active line (`.cm-activeLine`).
    pub active_line: Color,
    /// Background of text equal to the selection (`.cm-selectionMatch`).
    pub selection_match: Color,
    /// Matching bracket background, only while the editor has focus.
    pub matching_bracket: Color,
    /// Nonmatching bracket background, only while the editor has focus.
    pub nonmatching_bracket: Color,
    /// Editor gutters (`.cm-gutters`).
    pub gutters: FramedSurfaceStyle,
    /// Background of the active line in the gutter (`.cm-activeLineGutter`).
    pub active_line_gutter: Color,
    /// Placeholder for folded code (`.cm-foldPlaceholder`).
    pub fold_placeholder: FramedSurfaceStyle,
    /// Editor tooltip (`.cm-tooltip`) and the layers of its arrow.
    pub tooltip: TooltipStyle,
    /// Item with an `aria-selected` attribute in the completion tooltip.
    pub selected_completion: SurfaceStyle,
}

/// Native roles and highlighter for the combined `oneDark` export.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OneDarkPreset {
    pub view: ViewStyle,
    pub highlight: HighlightLayer,
}

/// The combined One Dark preset. The highlighter also works without the view theme.
pub const ONE_DARK: OneDarkPreset = OneDarkPreset {
    view: ONE_DARK_THEME,
    highlight: HighlightLayer::OneDark,
};

const PANEL_STROKE: Stroke = Stroke {
    width_px: 2,
    color: rgb(0x000000),
};
const SEARCH_OUTLINE: Stroke = Stroke {
    width_px: 1,
    color: rgb(0x457dff),
};
const TRANSPARENT: Color = Color(0x0000_0000);

/// The One Dark view styles, including alpha values and border overrides.
pub const ONE_DARK_THEME: ViewStyle = ViewStyle {
    dark: true,
    editor: SurfaceStyle {
        foreground: COLOR.ivory,
        background: COLOR.background,
    },
    caret: COLOR.cursor,
    cursor: COLOR.cursor,
    drop_cursor: COLOR.cursor,
    selection: COLOR.selection,
    panels: PanelStyle {
        foreground: COLOR.ivory,
        background: COLOR.dark_background,
        top_border: PANEL_STROKE,
        bottom_border: PANEL_STROKE,
    },
    search_match: SearchMatchStyle {
        background: Color(0x72a1_ff59),
        outline: SEARCH_OUTLINE,
    },
    selected_search_match: SearchMatchStyle {
        background: Color(0x6199_ff2f),
        outline: SEARCH_OUTLINE,
    },
    active_line: Color(0x6699_ff0b),
    selection_match: Color(0xaafe_661a),
    matching_bracket: Color(0xbad0_f847),
    nonmatching_bracket: Color(0xbad0_f847),
    gutters: FramedSurfaceStyle {
        foreground: COLOR.stone,
        background: COLOR.background,
        border: None,
    },
    active_line_gutter: COLOR.highlight_background,
    fold_placeholder: FramedSurfaceStyle {
        foreground: rgb(0xdddddd),
        background: TRANSPARENT,
        border: None,
    },
    tooltip: TooltipStyle {
        background: COLOR.tooltip_background,
        border: None,
        arrow_before: ArrowBorderColors {
            top: TRANSPARENT,
            bottom: TRANSPARENT,
        },
        arrow_after: ArrowBorderColors {
            top: COLOR.tooltip_background,
            bottom: COLOR.tooltip_background,
        },
    },
    selected_completion: SurfaceStyle {
        foreground: COLOR.ivory,
        background: COLOR.highlight_background,
    },
};
