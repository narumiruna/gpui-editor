//! Headless gutter projections from `@codemirror/view` 6.44.0.
//! See `docs/licenses/codemirror-view.txt` for the upstream MIT notice.
//!
//! The host supplies the logical source lines to draw. Snapshots do not use
//! visual line blocks, wrapping, folds, or DOM events. The host supplies
//! visible block widgets in visual order and owns their layout and rendering.
//! A spacer supplies a sample marker, not a width.

use std::{
    any::Any,
    collections::BTreeMap,
    ops::{Range as ByteRange, RangeInclusive},
    sync::{Arc, LazyLock},
};

use super::{
    CoveredMargins, ScrollRect,
    bidi::Direction,
    decorations::{BlockPlacement, Decoration},
};
use crate::{
    ConfigurationError, EditError, EditorState, Extension, Facet, Line, MapMode, Range, RangeSet,
    RangeSetBuilder, RangeValue, language::line_projection::configured_editor_lines,
};

/// A class and optional plain text for the host to draw. `Some("")` still
/// supplies content, so it can replace a line number with an empty label.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GutterMarker {
    pub element_class: String,
    pub content: Option<String>,
}

impl GutterMarker {
    pub fn class(class: impl Into<String>) -> Self {
        Self {
            element_class: class.into(),
            content: None,
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self {
            element_class: String::new(),
            content: Some(text.into()),
        }
    }

    /// Attach a marker at a UTF-8 byte offset, usually the start of a line.
    pub fn at(self, from: usize) -> Range<Self> {
        Range::new(from, from, self)
    }
}

impl RangeValue for GutterMarker {
    fn eq(&self, other: &Self) -> bool {
        self == other
    }

    fn start_side(&self) -> i32 {
        -1
    }

    fn end_side(&self) -> i32 {
        -1
    }

    fn map_mode(&self) -> MapMode {
        MapMode::TrackBefore
    }

    fn is_point(&self) -> bool {
        true
    }
}

/// Store markers from the current document. Use [`RangeSet::map`] with the
/// accepted changes to keep them across edits. An insertion at a point leaves
/// the marker before the insertion. Deleting the text just before it drops it.
/// Mapping does not move markers to line starts.
pub type GutterMarkerSet = RangeSet<GutterMarker>;

type MarkerSets = Vec<Arc<GutterMarkerSet>>;

/// Classes for all gutters on a logical line, in extension precedence order.
/// Use class-only markers here unless content is wanted in every gutter.
pub static GUTTER_LINE_CLASS: LazyLock<Facet<Arc<GutterMarkerSet>, MarkerSets>> =
    LazyLock::new(Facet::collect);

/// Markers specific to the line number gutter, in extension precedence order.
/// A marker with content replaces the default line number on that line.
pub static LINE_NUMBER_MARKERS: LazyLock<Facet<Arc<GutterMarkerSet>, MarkerSets>> =
    LazyLock::new(Facet::collect);

/// A visible block widget. Inline widgets and replacements without a widget
/// have no widget gutter row.
pub struct GutterWidget<'a> {
    source: ByteRange<usize>,
    placement: BlockPlacement,
    widget: &'a dyn Any,
}

impl<'a> GutterWidget<'a> {
    pub fn from_decoration<W: 'static>(range: &'a Range<Decoration<W>>) -> Option<Self> {
        Some(Self {
            source: range.from..range.to,
            placement: range.value.block_placement()?,
            widget: range.value.widget_value()?,
        })
    }

    pub fn source(&self) -> ByteRange<usize> {
        self.source.clone()
    }

    pub fn placement(&self) -> BlockPlacement {
        self.placement
    }

    pub fn value<W: 'static>(&self) -> Option<&W> {
        self.widget.downcast_ref()
    }
}

pub type GutterWidgetMarker =
    Arc<dyn Fn(&EditorState, &GutterWidget<'_>) -> Option<GutterMarker> + Send + Sync>;
type WidgetMarkers = Facet<GutterWidgetMarker, Vec<GutterWidgetMarker>>;

/// Markers for each gutter beside a widget. All providers run in extension
/// precedence order, after the gutter's local widget marker.
pub static GUTTER_WIDGET_CLASS: LazyLock<WidgetMarkers> = LazyLock::new(Facet::collect);

/// The first provider that returns a marker supplies the line number gutter's
/// local widget marker. The formatter does not run for a widget row.
pub static LINE_NUMBER_WIDGET_MARKER: LazyLock<WidgetMarkers> = LazyLock::new(Facet::collect);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GutterSide {
    #[default]
    Before,
    After,
}

pub type GutterMarkerProvider = Arc<dyn Fn(&EditorState) -> MarkerSets + Send + Sync>;
pub type GutterLineMarker =
    Arc<dyn Fn(&EditorState, &Line<'_>, &[GutterMarker]) -> Option<GutterMarker> + Send + Sync>;
pub type GutterSpacer = Arc<dyn Fn(&EditorState) -> GutterMarker + Send + Sync>;

#[derive(Clone, Default)]
pub struct GutterConfig {
    pub class: String,
    pub side: GutterSide,
    pub render_empty_elements: bool,
    pub markers: Option<GutterMarkerProvider>,
    /// Inspect the local markers followed by the shared line markers. A
    /// returned marker goes before all of them.
    pub line_marker: Option<GutterLineMarker>,
    /// An optional marker for the host to size the gutter. Compute it from
    /// the current state; it is not a row and has no lifecycle callbacks.
    pub spacer: Option<GutterSpacer>,
}

pub type LineNumberFormatter = Arc<dyn Fn(usize, &EditorState) -> String + Send + Sync>;

#[derive(Clone, Default)]
pub struct LineNumberConfig {
    /// Format a one-based line number or the spacer's sample number.
    /// If omitted, use decimal text.
    pub format_number: Option<LineNumberFormatter>,
}

#[derive(Clone)]
enum Gutter {
    Custom(Arc<GutterConfig>, Option<GutterWidgetMarker>),
    Numbers(Option<LineNumberFormatter>),
}

impl Gutter {
    fn side(&self) -> GutterSide {
        match self {
            Self::Custom(config, _) => config.side,
            Self::Numbers(_) => GutterSide::Before,
        }
    }

    fn matches_config(&self, config: &Arc<GutterConfig>) -> bool {
        matches!(self, Self::Custom(installed, _) if Arc::ptr_eq(installed, config))
    }
}

type Gutters = Facet<Gutter, Vec<Gutter>>;
static ACTIVE_GUTTERS: LazyLock<Gutters> = LazyLock::new(Facet::collect);
static UNFIX_GUTTERS: LazyLock<Facet<bool, bool>> =
    LazyLock::new(|| Facet::new(|values| values.iter().any(|&&unfix| unfix)));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuttersConfig {
    /// Fixed gutters remain visible during horizontal scrolling.
    pub fixed: bool,
}

impl Default for GuttersConfig {
    fn default() -> Self {
        Self { fixed: true }
    }
}

/// Configure horizontal gutter scrolling. Any `fixed: false` input disables
/// fixed gutters, regardless of precedence. This does not add a column.
pub fn gutters(config: GuttersConfig) -> Extension {
    UNFIX_GUTTERS.of(!config.fixed)
}

pub fn gutters_fixed(state: &EditorState) -> Result<bool, ConfigurationError> {
    state.context().facet(&UNFIX_GUTTERS).map(|unfix| !*unfix)
}

/// Return the covered scroll edges from the host's measured fixed gutter
/// containers, in the same coordinate space as the scroller's client bounds.
/// `before` can be zero width when only after-side columns are configured.
/// Use this from a [`super::covered_margins`] provider after measuring.
pub fn gutter_covered_margins(
    state: &EditorState,
    direction: Direction,
    before: ScrollRect,
    after: Option<ScrollRect>,
) -> Result<Option<CoveredMargins>, ConfigurationError> {
    if !gutters_fixed(state)?
        || state.context().facet(&ACTIVE_GUTTERS)?.is_empty()
        || !before.is_valid()
        || after.is_some_and(|rect| !rect.is_valid())
    {
        return Ok(None);
    }
    Ok(Some(match direction {
        Direction::Ltr => CoveredMargins {
            left: Some(before.right),
            right: after.map(|rect| rect.left),
            ..CoveredMargins::default()
        },
        Direction::Rtl => CoveredMargins {
            left: after.map(|rect| rect.right),
            right: Some(before.left),
            ..CoveredMargins::default()
        },
    }))
}
type LineNumberInputs = Facet<LineNumberConfig, Vec<LineNumberConfig>>;
static LINE_NUMBER_CONFIG: LazyLock<LineNumberInputs> = LazyLock::new(Facet::collect);
static LINE_NUMBER_GUTTER: LazyLock<Extension> = LazyLock::new(|| {
    ACTIVE_GUTTERS.compute(|state| {
        let configs = state.facet(&LINE_NUMBER_CONFIG)?;
        let mut formats = configs
            .iter()
            .filter_map(|config| config.format_number.as_ref());
        let first = formats.next();
        if let Some(first) = first
            && formats.any(|other| !Arc::ptr_eq(first, other))
        {
            return Err(ConfigurationError::InvalidFieldData);
        }
        Ok(Gutter::Numbers(first.cloned()))
    })
});

/// Add a gutter in extension precedence order. Reusing one extension adds
/// one slot; separate calls add separate slots even for the same class.
pub fn gutter(config: GutterConfig) -> Extension {
    gutter_with_identity(Arc::new(config))
}

// Keep the config identity so the preset can supply a host syntax fallback
// without changing any other slot that uses the same CSS class.
pub(crate) fn gutter_with_identity(config: Arc<GutterConfig>) -> Extension {
    ACTIVE_GUTTERS.of(Gutter::Custom(config, None))
}

pub(crate) fn gutter_installed(
    state: &EditorState,
    config: &Arc<GutterConfig>,
) -> Result<bool, ConfigurationError> {
    let gutters = state.context().facet(&ACTIVE_GUTTERS)?;
    Ok(gutters.iter().any(|gutter| gutter.matches_config(config)))
}

/// Add a custom gutter with an optional local marker for each visible widget.
pub fn gutter_with_widget_marker(config: GutterConfig, marker: GutterWidgetMarker) -> Extension {
    ACTIVE_GUTTERS.of(Gutter::Custom(Arc::new(config), Some(marker)))
}

/// Add the before-side line number gutter. Multiple calls share one slot.
/// Their defined formatters must be the same `Arc`, or configuration fails.
pub fn line_numbers(config: LineNumberConfig) -> Extension {
    Extension::group([LINE_NUMBER_CONFIG.of(config), LINE_NUMBER_GUTTER.clone()])
}

static ACTIVE_LINE_ENABLED: LazyLock<Facet<(), bool>> =
    LazyLock::new(|| Facet::new(|inputs| !inputs.is_empty()));
static ACTIVE_LINE: LazyLock<Extension> = LazyLock::new(|| {
    let markers = GUTTER_LINE_CLASS.compute(|state| {
        let projected = configured_editor_lines(state)?;
        let text = projected.as_ref().as_ref().unwrap_or(state.text());
        let mut markers = RangeSetBuilder::new();
        let mut last = None;
        for range in state.selection().ranges() {
            let from = text.line_at(range.head()).unwrap().range.start;
            if last.is_none_or(|previous| from > previous) {
                markers
                    .push(GutterMarker::class("cm-activeLineGutter").at(from))
                    .expect("ordered line starts");
                last = Some(from);
            }
        }
        Ok(Arc::new(markers.finish()))
    });
    Extension::group([ACTIVE_LINE_ENABLED.of(()), markers])
});

/// Add `cm-activeLineGutter` once per line with a selection head. The
/// selection can be nonempty. This extension does not add a gutter by itself.
pub fn highlight_active_line_gutter() -> Extension {
    ACTIVE_LINE.clone()
}

/// Check whether the stock active-line gutter extension is installed.
pub fn highlight_active_line_gutter_enabled(state: &EditorState) -> bool {
    *state
        .context()
        .facet(&ACTIVE_LINE_ENABLED)
        .expect("typed active-line gutter facet")
}

/// A gutter column. `index` is zero-based within its side, not an identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GutterSlot {
    pub side: GutterSide,
    pub index: usize,
    pub class: String,
    pub rows: Vec<GutterRow>,
    pub spacer: Option<GutterMarker>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GutterRow {
    pub line_number: usize,
    pub from: usize,
    pub markers: Vec<GutterMarker>,
}

/// Project gutters for a one-based inclusive range of logical lines. Before
/// slots precede after slots. Each side keeps extension precedence order.
/// Callbacks receive the original state, even when line bounds are projected.
/// A range marker contributes only where its start equals a line start.
/// Marker ties follow the range set cursor, including its layer heap order
/// across sets. Marker classes are not deduplicated.
///
/// An empty interval returns slots and spacer inputs but no rows. Return
/// [`EditError::InvalidRange`] if a nonempty interval is outside the document.
pub fn gutter_slots(
    state: &EditorState,
    lines: RangeInclusive<usize>,
) -> Result<Vec<GutterSlot>, EditError> {
    project_slots(state, lines, None)
}

type ProjectLineMarker<'a> =
    dyn Fn(&EditorState, &Line<'_>, &[GutterMarker]) -> Option<GutterMarker> + 'a;

/// Replace the line marker callback for this slot for one projection.
pub(crate) fn gutter_slots_with_line_marker(
    state: &EditorState,
    lines: RangeInclusive<usize>,
    config: &Arc<GutterConfig>,
    marker: &ProjectLineMarker<'_>,
) -> Result<Vec<GutterSlot>, EditError> {
    project_slots(state, lines, Some((config, marker)))
}

fn project_slots(
    state: &EditorState,
    lines: RangeInclusive<usize>,
    line_marker: Option<(&Arc<GutterConfig>, &ProjectLineMarker<'_>)>,
) -> Result<Vec<GutterSlot>, EditError> {
    let context = state.context();
    let projected = configured_editor_lines(&context)?;
    let text = projected.as_ref().as_ref().unwrap_or(context.text());
    let line_count = text.line_count();
    if !lines.is_empty() && (*lines.start() == 0 || *lines.end() > line_count) {
        return Err(EditError::InvalidRange);
    }
    let gutters = context.facet(&ACTIVE_GUTTERS)?;
    if gutters.is_empty() {
        return Ok(Vec::new());
    }
    let lines: Vec<_> = lines.map(|number| text.line(number).unwrap()).collect();
    let shared_sets = context.facet(&GUTTER_LINE_CLASS)?;
    let shared = markers_by_start(&shared_sets, &lines);
    let mut before = Vec::new();
    let mut after = Vec::new();
    for gutter in gutters.iter() {
        let marker = line_marker
            .filter(|(config, _)| gutter.matches_config(config))
            .map(|(_, marker)| marker);
        let mut slot = project(state, &lines, line_count, &shared, gutter, marker);
        let slots = match slot.side {
            GutterSide::Before => &mut before,
            GutterSide::After => &mut after,
        };
        slot.index = slots.len();
        slots.push(slot);
    }
    before.append(&mut after);
    Ok(before)
}

/// A marked block widget and the gutter slot where its markers go.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GutterWidgetRow {
    /// The zero-based index in the host's visible widget list.
    pub widget_index: usize,
    pub side: GutterSide,
    /// The zero-based gutter index within `side`.
    pub gutter_index: usize,
    pub markers: Vec<GutterMarker>,
}

/// Return marker rows for visible block widgets, in the order supplied by the
/// host. For each widget, call gutters in extension precedence order, as the
/// pinned view does even when before/after gutters are interleaved. Use the
/// side and index to join rows to [`gutter_slots`]. No row is emitted for an
/// empty marker list, even in a gutter that renders empty text rows.
pub fn gutter_widget_rows(
    state: &EditorState,
    widgets: &[GutterWidget<'_>],
) -> Result<Vec<GutterWidgetRow>, EditError> {
    if widgets
        .iter()
        .any(|widget| !state.text().is_valid_range(widget.source()))
    {
        return Err(EditError::InvalidRange);
    }
    let context = state.context();
    let gutters = context.facet(&ACTIVE_GUTTERS)?;
    let shared = context.facet(&GUTTER_WIDGET_CLASS)?;
    let numbers = context.facet(&LINE_NUMBER_WIDGET_MARKER)?;
    let (mut before, mut after) = (0, 0);
    let slots: Vec<_> = gutters
        .iter()
        .map(|gutter| {
            let side = gutter.side();
            let count = match side {
                GutterSide::Before => &mut before,
                GutterSide::After => &mut after,
            };
            let index = *count;
            *count += 1;
            (side, index, gutter)
        })
        .collect();
    let mut rows = Vec::new();
    for (widget_index, widget) in widgets.iter().enumerate() {
        for &(side, gutter_index, gutter) in &slots {
            let marker = match gutter {
                Gutter::Custom(_, marker) => {
                    marker.as_ref().and_then(|marker| marker(state, widget))
                }
                Gutter::Numbers(_) => numbers.iter().find_map(|marker| marker(state, widget)),
            };
            let markers: Vec<_> = marker
                .into_iter()
                .chain(shared.iter().filter_map(|marker| marker(state, widget)))
                .collect();
            if !markers.is_empty() {
                rows.push(GutterWidgetRow {
                    widget_index,
                    side,
                    gutter_index,
                    markers,
                });
            }
        }
    }
    Ok(rows)
}

fn project(
    state: &EditorState,
    lines: &[Line<'_>],
    line_count: usize,
    shared: &BTreeMap<usize, Vec<GutterMarker>>,
    gutter: &Gutter,
    line_marker: Option<&ProjectLineMarker<'_>>,
) -> GutterSlot {
    let (class, side, render_empty, sets, spacer) = match gutter {
        Gutter::Custom(config, _) => (
            config.class.clone(),
            config.side,
            config.render_empty_elements,
            config
                .markers
                .as_ref()
                .map_or_else(Vec::new, |markers| markers(state)),
            config.spacer.as_ref().map(|spacer| spacer(state)),
        ),
        Gutter::Numbers(format) => (
            "cm-lineNumbers".into(),
            GutterSide::Before,
            false,
            state
                .context()
                .facet(&LINE_NUMBER_MARKERS)
                .expect("valid markers")
                .as_ref()
                .clone(),
            Some(number_marker(
                state,
                format.as_ref(),
                spacer_number(line_count),
            )),
        ),
    };
    let mut local = markers_by_start(&sets, lines);
    let rows = lines
        .iter()
        .filter_map(|line| {
            let from = line.range.start;
            let mut markers = local.remove(&from).unwrap_or_default();
            if let Some(extra) = shared.get(&from) {
                markers.extend_from_slice(extra);
            }
            let marker = match gutter {
                Gutter::Custom(config, _) => match line_marker {
                    Some(marker) => marker(state, line, &markers),
                    None => config
                        .line_marker
                        .as_ref()
                        .and_then(|marker| marker(state, line, &markers)),
                },
                Gutter::Numbers(format) => (!markers.iter().any(|marker| marker.content.is_some()))
                    .then(|| number_marker(state, format.as_ref(), line.number)),
            };
            if let Some(marker) = marker {
                markers.insert(0, marker);
            }
            (render_empty || !markers.is_empty()).then_some(GutterRow {
                line_number: line.number,
                from,
                markers,
            })
        })
        .collect();
    GutterSlot {
        side,
        index: 0,
        class,
        rows,
        spacer,
    }
}

fn markers_by_start(
    sets: &[Arc<GutterMarkerSet>],
    lines: &[Line<'_>],
) -> BTreeMap<usize, Vec<GutterMarker>> {
    let mut markers: BTreeMap<_, Vec<_>> = BTreeMap::new();
    if let (Some(first), Some(last)) = (lines.first(), lines.last()) {
        let sets: Vec<_> = sets.iter().map(Arc::as_ref).collect();
        for (_, range) in RangeSet::iter_sets(&sets, first.range.start)
            .skip_while(|(_, range)| range.from < first.range.start)
            .take_while(|(_, range)| range.from <= last.range.start)
        {
            markers
                .entry(range.from)
                .or_default()
                .push(range.value.clone());
        }
    }
    markers
}

fn number_marker(
    state: &EditorState,
    format: Option<&LineNumberFormatter>,
    number: usize,
) -> GutterMarker {
    GutterMarker::text(format.map_or_else(|| number.to_string(), |format| format(number, state)))
}

fn spacer_number(lines: usize) -> usize {
    let mut max = 9usize;
    while max < lines {
        max = max.saturating_mul(10).saturating_add(9);
    }
    max
}
