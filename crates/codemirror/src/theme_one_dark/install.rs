use std::sync::{Arc, LazyLock};

use crate::{
    EditorState, Extension, Facet,
    language::{self, HighlightLayer, HighlightProperties, HighlightTag, HighlightTheme},
    language_data::NativeLanguage,
};

use super::{Color, ONE_DARK_HIGHLIGHT_STYLE, ONE_DARK_THEME, ViewStyle};

/// Optional foreground and background for the editor surface. An absent
/// color leaves that property to a lower-priority layer or the host.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EditorSurfaceColors {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
}

#[derive(Clone, Copy)]
enum ViewLayer {
    OneDark,
    EditorSurface(EditorSurfaceColors),
}

#[derive(Default)]
struct ViewConfiguration {
    one_dark: bool,
    surface: EditorSurfaceColors,
}

type ViewStyles = Facet<ViewLayer, ViewConfiguration>;
type HighlightLayers = Facet<HighlightLayer<'static>, Vec<HighlightLayer<'static>>>;

static VIEW: LazyLock<ViewStyles> = LazyLock::new(|| {
    Facet::new(|layers: &[&ViewLayer]| {
        let mut config = ViewConfiguration::default();
        for layer in layers {
            let colors = match **layer {
                ViewLayer::OneDark => {
                    config.one_dark = true;
                    EditorSurfaceColors {
                        foreground: Some(ONE_DARK_THEME.editor.foreground),
                        background: Some(ONE_DARK_THEME.editor.background),
                    }
                }
                ViewLayer::EditorSurface(colors) => colors,
            };
            config.surface.foreground = config.surface.foreground.or(colors.foreground);
            config.surface.background = config.surface.background.or(colors.background);
        }
        config
    })
});
static HIGHLIGHTS: LazyLock<HighlightLayers> = LazyLock::new(Facet::collect);
static THEME: LazyLock<Extension> = LazyLock::new(|| VIEW.of(ViewLayer::OneDark));
static HIGHLIGHTING: LazyLock<Extension> =
    LazyLock::new(|| highlight_layer(HighlightLayer::Active(ONE_DARK_HIGHLIGHT_STYLE)));

/// Install One Dark's view roles and set the appearance to dark. Does not install a highlighter.
pub fn one_dark_theme() -> Extension {
    THEME.clone()
}

/// Install One Dark as an active, unscoped highlighter. Does not change the view appearance.
pub fn one_dark_highlighting() -> Extension {
    HIGHLIGHTING.clone()
}

/// Install both the view theme and the highlighter, like upstream `oneDark`.
pub fn one_dark() -> Extension {
    Extension::group([one_dark_theme(), one_dark_highlighting()])
}

/// Register a [language highlight layer](language::HighlightLayer) in the same
/// precedence order as One Dark. To use the setup fallback in this stack,
/// pass the layer from [`SetupEditor::highlight_fallback`](crate::setup::SetupEditor::highlight_fallback).
///
/// A [`HighlightLayer::Resolved`] input applies the same properties to every span.
pub fn highlight_layer(layer: HighlightLayer<'static>) -> Extension {
    HIGHLIGHTS.of(layer)
}

/// Set either editor surface color in extension precedence order. The first
/// value for each color wins. This does not enable One Dark, change appearance,
/// or set panel, tooltip, selection, or syntax colors.
pub fn override_editor_surface(colors: EditorSurfaceColors) -> Extension {
    VIEW.of(ViewLayer::EditorSurface(colors))
}

/// Read the editor surface colors after applying the installed layers.
pub fn editor_surface_colors(state: &EditorState) -> EditorSurfaceColors {
    state
        .context()
        .facet(&VIEW)
        .expect("static view facet")
        .surface
}

/// Read the installed stock One Dark roles. For the editor surface, use
/// [`editor_surface_colors`] to include host overrides.
pub fn view_style(state: &EditorState) -> Option<&'static ViewStyle> {
    state
        .context()
        .facet(&VIEW)
        .expect("static view facet")
        .one_dark
        .then_some(&ONE_DARK_THEME)
}

/// Resolve one span with the installed native layers in extension precedence order.
/// The One Dark view theme selects dark layers. Without it, this stack uses light
/// appearance, like a bare CodeMirror editor. The host supplies the region's scope tag.
/// Only an appearance mismatch makes an active layer inactive: an unmatched tag or
/// scope still suppresses fallback. See [`language::highlighting_for`].
#[must_use]
pub fn highlighting_for(
    state: &EditorState,
    tags: &[HighlightTag],
    scope: Option<&HighlightTag>,
) -> HighlightProperties {
    let (layers, theme) = highlight_configuration(state);
    language::highlighting_for(&layers, tags, scope, theme)
}

/// Resolve the installed layers for a parsed native span. Pass `None` if
/// the parser configuration for the span is not known. See
/// [`language::highlighting_for_native`].
#[must_use]
pub fn highlighting_for_native(
    state: &EditorState,
    tags: &[HighlightTag],
    language: Option<NativeLanguage>,
) -> HighlightProperties {
    let (layers, theme) = highlight_configuration(state);
    language::highlighting_for_native(&layers, tags, language, theme)
}

/// Read the installed layers in precedence order and their native appearance.
#[must_use]
pub fn highlight_configuration(
    state: &EditorState,
) -> (Arc<Vec<HighlightLayer<'static>>>, HighlightTheme) {
    let theme = if view_style(state).is_some_and(|style| style.dark) {
        HighlightTheme::Dark
    } else {
        HighlightTheme::Light
    };
    let layers = state
        .context()
        .facet(&HIGHLIGHTS)
        .expect("static highlight facet");
    (layers, theme)
}
