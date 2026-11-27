//! Headless selection and cursor metadata from `@codemirror/view` 6.44.0.
//! See `docs/licenses/codemirror-view.txt` for the upstream MIT notice.
//!
//! The host supplies hit tests and layout. It owns input routing, painting,
//! scrolling, and cursor animation. Document positions are UTF-8 byte offsets.

use std::{ops::Range, sync::LazyLock};

use crate::{
    Assoc, ConfigurationError, EditError, EditorSelection, EditorState, Extension, Facet, MapMode,
    SelectionRange, StateEffect, StateEffectType, StateField,
};

mod rectangle;

pub use rectangle::{
    CrosshairCursor, CrosshairModifier, RectangleHit, RectangleStartEvent, RectangularSelection,
    RectangularSelectionFilter, RectangularSelectionOptions, crosshair_cursor,
    rectangular_selection,
};

/// Optional inputs to [`draw_selection`]. Unset options do not take part in merging.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrawSelectionOptions {
    pub cursor_blink_rate: Option<u32>,
    pub draw_range_cursor: Option<bool>,
    pub ios_selection_handles: Option<bool>,
}

/// Resolved options, including defaults when drawing is not installed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrawSelectionConfig {
    /// Blink interval in milliseconds. Zero disables blinking.
    pub cursor_blink_rate: u32,
    pub draw_range_cursor: bool,
    pub ios_selection_handles: bool,
}

impl Default for DrawSelectionConfig {
    fn default() -> Self {
        Self {
            cursor_blink_rate: 1200,
            draw_range_cursor: true,
            ios_selection_handles: true,
        }
    }
}

type DrawConfig = Result<DrawSelectionConfig, ConfigurationError>;
static DRAW_OPTIONS: LazyLock<Facet<DrawSelectionOptions, DrawConfig>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&DrawSelectionOptions]| {
        let mut handles = inputs
            .iter()
            .filter_map(|option| option.ios_selection_handles);
        let first = handles.next();
        if first.is_some_and(|first| handles.any(|input| input != first)) {
            return Err(ConfigurationError::InvalidFieldData);
        }
        Ok(DrawSelectionConfig {
            cursor_blink_rate: inputs
                .iter()
                .filter_map(|option| option.cursor_blink_rate)
                .min()
                .unwrap_or(1200),
            draw_range_cursor: inputs
                .iter()
                .filter_map(|option| option.draw_range_cursor)
                .reduce(|a, b| a || b)
                .unwrap_or(true),
            ios_selection_handles: first.unwrap_or(true),
        })
    })
});

static DRAW_CONFIG: LazyLock<StateField<DrawSelectionConfig>> = LazyLock::new(|| {
    StateField::new(
        |state| *state.facet(&DRAW_OPTIONS)?,
        |_, update| *update.state().facet(&DRAW_OPTIONS)?,
    )
});

/// Register the cursor and selection layers. Explicit blink rates take their
/// minimum, and explicit range-cursor flags are ORed. Conflicting explicit
/// `ios_selection_handles` values reject the configuration.
///
/// This does not hide native painting or restart the host's blink animation.
pub fn draw_selection(options: DrawSelectionOptions) -> Extension {
    static DRAW_LAYERS: LazyLock<Extension> = LazyLock::new(|| {
        Extension::group([
            register_layer(LayerKind::Cursor, true, "cm-cursorLayer"),
            register_layer(LayerKind::Selection, false, "cm-selectionLayer"),
        ])
    });
    Extension::group([
        DRAW_OPTIONS.of(options),
        DRAW_CONFIG.extension(),
        DRAW_LAYERS.clone(),
    ])
}

/// Return the resolved options only when [`draw_selection`] is installed.
pub fn installed_draw_config(state: &EditorState) -> Option<DrawSelectionConfig> {
    state
        .context()
        .field(&DRAW_CONFIG)
        .expect("valid selection drawing field")
        .map(|config| *config)
}

pub fn get_draw_selection_config(state: &EditorState) -> DrawSelectionConfig {
    installed_draw_config(state).unwrap_or_default()
}

/// Configuration for a layer. It has no rendering or lifecycle callbacks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayerConfig {
    pub above: bool,
    pub class: Option<String>,
    pub update_on_doc_view_update: bool,
}

impl Default for LayerConfig {
    fn default() -> Self {
        Self {
            above: false,
            class: None,
            update_on_doc_view_update: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayerKind {
    Custom,
    Cursor,
    Selection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayerMetadata {
    pub kind: LayerKind,
    pub config: LayerConfig,
    pub z_index: i64,
}

type LayerInput = (LayerKind, LayerConfig);
static LAYERS: LazyLock<Facet<LayerInput, Vec<LayerInput>>> = LazyLock::new(Facet::collect);

/// Register metadata for a host-painted layer. Clone the returned extension
/// to refer to the same registration. Each call creates a new registration.
pub fn layer(config: LayerConfig) -> Extension {
    LAYERS.of((LayerKind::Custom, config))
}

fn register_layer(kind: LayerKind, above: bool, class: &str) -> Extension {
    LAYERS.of((
        kind,
        LayerConfig {
            above,
            class: Some(class.into()),
            ..LayerConfig::default()
        },
    ))
}

/// Layers in extension precedence order. Above and below layers use the same
/// order index, as in CodeMirror. The host paints custom layers separately.
pub fn layer_metadata(state: &EditorState) -> Vec<LayerMetadata> {
    state
        .context()
        .facet(&LAYERS)
        .expect("valid layer metadata")
        .iter()
        .enumerate()
        .map(|(index, (kind, config))| LayerMetadata {
            kind: *kind,
            config: config.clone(),
            z_index: (if config.above { 150 } else { -1 }) - index as i64,
        })
        .collect()
}

/// A measured rectangle in layer coordinates. Supply finite coordinates and
/// a bottom edge at or below the top edge.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayerRect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// Measurements already converted to native layer units and origin.
/// The host does clipping, shaping, bidi and wrap splitting, and selection
/// edge fill. Return rectangles in paint order. Missing geometry is valid.
pub trait SelectionGeometry {
    fn caret_rect(&mut self, position: usize, assoc: Assoc) -> Option<LayerRect>;
    fn selection_rects(&mut self, range: Range<usize>) -> Vec<LayerRect>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct RectangleMarker {
    pub class: String,
    pub left: f64,
    pub top: f64,
    /// `None` asks the host to use the default width, for example for carets.
    pub width: Option<f64>,
    pub height: f64,
}

impl RectangleMarker {
    /// Use the caret's association, defaulting to [`Assoc::After`]. For a
    /// nonempty range, use each of the supplied selection rectangles.
    pub fn for_range(
        geometry: &mut impl SelectionGeometry,
        class: &str,
        range: SelectionRange,
    ) -> Vec<Self> {
        if range.is_empty() {
            geometry
                .caret_rect(range.head(), range.assoc().unwrap_or(Assoc::After))
                .map(|rect| Self::measured(class, rect, false))
                .into_iter()
                .collect()
        } else {
            geometry
                .selection_rects(range.range())
                .into_iter()
                .map(|rect| Self::measured(class, rect, true))
                .collect()
        }
    }

    fn measured(class: &str, rect: LayerRect, with_width: bool) -> Self {
        Self {
            class: class.into(),
            left: rect.left,
            top: rect.top,
            width: with_width.then(|| (rect.right - rect.left).max(0.0)),
            height: rect.bottom - rect.top,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SelectionPlatform {
    #[default]
    Default,
    /// The host paints the main range background outside this API.
    Ios,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawSelectionMarkers {
    pub cursors: Vec<RectangleMarker>,
    pub selection: Vec<RectangleMarker>,
}

/// Measure the registered built-in layers. The host controls focus visibility,
/// blink and handle rendering. No geometry is requested if drawing is disabled.
pub fn draw_selection_markers(
    state: &EditorState,
    platform: SelectionPlatform,
    geometry: &mut impl SelectionGeometry,
) -> Option<DrawSelectionMarkers> {
    let config = installed_draw_config(state)?;
    let selection = state.selection();
    let ios = platform == SelectionPlatform::Ios;
    let handles = ios && config.ios_selection_handles;
    let mut markers = DrawSelectionMarkers::default();
    for (index, &range) in selection.ranges().iter().enumerate() {
        let main = index == selection.main_index();
        if range.is_empty() || config.draw_range_cursor && !(main && handles) {
            let class = if main {
                "cm-cursor cm-cursor-primary"
            } else {
                "cm-cursor cm-cursor-secondary"
            };
            let cursor = EditorSelection::cursor(range.head(), range.assoc(), None, None);
            markers
                .cursors
                .extend(RectangleMarker::for_range(geometry, class, cursor));
        }
    }
    for (index, &range) in selection.ranges().iter().enumerate() {
        if !(range.is_empty() || ios && index == selection.main_index()) {
            markers.selection.extend(RectangleMarker::for_range(
                geometry,
                "cm-selectionBackground",
                range,
            ));
        }
    }
    let main = selection.main();
    if handles && !main.is_empty() {
        let span = main.range();
        for (position, class) in [
            (span.start, "cm-selectionHandle cm-selectionHandle-start"),
            (span.end, "cm-selectionHandle cm-selectionHandle-end"),
        ] {
            let cursor = EditorSelection::cursor(position, Some(Assoc::After), None, None);
            markers
                .selection
                .extend(RectangleMarker::for_range(geometry, class, cursor));
        }
    }
    Some(markers)
}

/// Set or clear the drop cursor. Offsets use the transaction's new document.
/// The position maps before an insertion at the same offset.
pub static SET_DROP_CURSOR: LazyLock<StateEffectType<Option<usize>>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|position: &Option<usize>, changes| match *position {
        Some(position) => changes
            .map_pos(position, Assoc::Before, MapMode::Simple)
            .map(Some),
        None => Some(None),
    })
});

static DROP_CURSOR: LazyLock<StateField<Option<usize>>> = LazyLock::new(|| {
    StateField::new(
        |_| Ok(None),
        |position: &Option<usize>, update| {
            let mut next = position.and_then(|pos| {
                update
                    .changes()
                    .map_pos(pos, Assoc::Before, MapMode::Simple)
            });
            for effect in update.transaction().effects() {
                if let Some(value) = effect.get(&SET_DROP_CURSOR) {
                    next = *value;
                }
            }
            if next.is_some_and(|pos| !update.state().text().is_char_boundary(pos)) {
                return Err(ConfigurationError::InvalidFieldData);
            }
            Ok(next)
        },
    )
});

/// Track a native drag target, without installing event listeners or accepting drops.
pub fn drop_cursor() -> Extension {
    DROP_CURSOR.extension()
}

pub fn drop_cursor_position(state: &EditorState) -> Option<usize> {
    state
        .context()
        .field(&DROP_CURSOR)
        .expect("valid drop cursor field")
        .and_then(|position| *position)
}

/// Request a change only if tracking is installed and the target changed.
/// Pass a hit test result on drag-over, or `None` on drop, drag end, or exit.
pub fn set_drop_cursor(
    state: &EditorState,
    position: Option<usize>,
) -> Result<Option<StateEffect>, EditError> {
    let Some(previous) = state.context().field(&DROP_CURSOR)? else {
        return Ok(None);
    };
    if position.is_some_and(|pos| !state.text().is_char_boundary(pos)) {
        return Err(EditError::InvalidRange);
    }
    Ok((*previous != position).then(|| SET_DROP_CURSOR.of(position)))
}

/// Use a caret measurement in the drop overlay's native coordinates. A
/// missing measurement hides the marker but preserves the tracked position.
pub fn drop_cursor_marker(
    state: &EditorState,
    measure: impl FnOnce(usize) -> Option<LayerRect>,
) -> Option<RectangleMarker> {
    let rect = measure(drop_cursor_position(state)?)?;
    Some(RectangleMarker::measured("cm-dropCursor", rect, false))
}
