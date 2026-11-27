use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{Arc, LazyLock},
    time::Duration,
};

use crate::{
    ConfigurationError, EditError, EditorState, Extension, Facet, MapMode, RangeSet,
    RangeSetBuilder, RangeValue, StateField, StateUpdate,
    language::line_projection::configured_editor_lines,
    view::gutters::{GutterConfig, GutterMarker, GutterWidget, gutter_with_widget_marker},
};

use super::{
    Diagnostic, DiagnosticFilter, LINT_STATE, PublicationScope, Severity, published_diagnostics,
};

#[derive(Clone, Default)]
pub struct LintGutterConfig {
    /// Filter published diagnostics before grouping them by line. This is
    /// independent of filters on document markers.
    pub marker_filter: Option<DiagnosticFilter>,
    /// Filter the hover list without changing the line marker or its severity.
    pub tooltip_filter: Option<DiagnosticFilter>,
    /// Delay before opening a gutter tooltip. The default is 300 ms.
    pub hover_time: Option<Duration>,
}

/// A marker at the start of a line, with the diagnostics in publication order.
#[derive(Clone, Debug)]
pub struct LintGutterMarker(Arc<[Diagnostic]>);

impl LintGutterMarker {
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.0
    }

    pub fn severity(&self) -> Severity {
        self.0
            .iter()
            .map(|d| d.severity)
            .max()
            .expect("nonempty line")
    }

    fn display_marker(&self) -> GutterMarker {
        let class = match self.severity() {
            Severity::Error => "cm-lint-marker cm-lint-marker-error",
            Severity::Warning => "cm-lint-marker cm-lint-marker-warning",
            Severity::Info => "cm-lint-marker cm-lint-marker-info",
            Severity::Hint => "cm-lint-marker cm-lint-marker-hint",
        };
        GutterMarker::class(class)
    }
}

impl RangeValue for LintGutterMarker {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    fn start_side(&self) -> i32 {
        -1
    }

    fn end_side(&self) -> i32 {
        -1
    }

    fn is_point(&self) -> bool {
        true
    }

    fn map_mode(&self) -> MapMode {
        MapMode::TrackBefore
    }
}

type GutterOptions = Result<LintGutterConfig, ConfigurationError>;
static CONFIG: LazyLock<Facet<LintGutterConfig, GutterOptions>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&LintGutterConfig]| {
        fn one_value<T>(
            mut values: impl Iterator<Item = T>,
            equal: impl Fn(&T, &T) -> bool,
        ) -> Result<Option<T>, ConfigurationError> {
            let first = values.next();
            if first
                .as_ref()
                .is_some_and(|first| values.any(|value| !equal(first, &value)))
            {
                return Err(ConfigurationError::InvalidFieldData);
            }
            Ok(first)
        }
        Ok(LintGutterConfig {
            marker_filter: one_value(
                inputs
                    .iter()
                    .filter_map(|input| input.marker_filter.clone()),
                Arc::ptr_eq,
            )?,
            tooltip_filter: one_value(
                inputs
                    .iter()
                    .filter_map(|input| input.tooltip_filter.clone()),
                Arc::ptr_eq,
            )?,
            hover_time: one_value(
                inputs.iter().filter_map(|input| input.hover_time),
                PartialEq::eq,
            )?,
        })
    })
});

type GutterMarkers = Arc<RangeSet<LintGutterMarker>>;
static GUTTER: LazyLock<StateField<GutterMarkers>> = LazyLock::new(|| {
    StateField::new(
        |state| {
            state
                .facet(&CONFIG)?
                .as_ref()
                .as_ref()
                .map_err(|err| *err)?;
            Ok(Arc::new(RangeSet::empty()))
        },
        update_markers,
    )
});

fn update_markers(
    previous: &GutterMarkers,
    update: &StateUpdate<'_>,
) -> Result<GutterMarkers, ConfigurationError> {
    let state = update.state();
    let config = state.facet(&CONFIG)?;
    let filter = &config.as_ref().as_ref().map_err(|err| *err)?.marker_filter;
    let lint = update.start().field(&LINT_STATE)?;
    let mut diagnostics = None;
    for published in update
        .transaction()
        .effects()
        .iter()
        .filter_map(|effect| published_diagnostics(effect, update, lint.as_deref()))
    {
        let diagnostics_for_gutter = match (published.scope, filter) {
            (PublicationScope::Discard, _) => Vec::new(),
            (_, Some(filter)) => filter(published.diagnostics.to_vec(), state),
            (_, None) => published.diagnostics.to_vec(),
        };
        diagnostics = Some((diagnostics_for_gutter, published.scope));
    }
    let Some((diagnostics, scope)) = diagnostics else {
        return Ok(if update.changes().is_empty() {
            previous.clone()
        } else {
            Arc::new(previous.map(update.transaction().changes()))
        });
    };
    let projected = configured_editor_lines(state)?;
    let text = projected.as_ref().as_ref().unwrap_or(state.text());
    let mut lines: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for diagnostic in diagnostics {
        if scope.accepts(&diagnostic.range) && text.is_valid_range(diagnostic.range.clone()) {
            let line = text.line_at(diagnostic.range.start).expect("valid start");
            lines.entry(line.range.start).or_default().push(diagnostic);
        }
    }
    let mut markers = RangeSetBuilder::new();
    for (line, diagnostics) in lines {
        markers
            .add(line, line, LintGutterMarker(diagnostics.into()))
            .expect("ordered lines");
    }
    Ok(Arc::new(markers.finish()))
}

static SLOT: LazyLock<Extension> = LazyLock::new(|| {
    gutter_with_widget_marker(
        GutterConfig {
            class: "cm-gutter-lint".into(),
            markers: Some(Arc::new(|state| {
                let Some(markers) = lint_gutter_markers(state) else {
                    return Vec::new();
                };
                let rows = markers
                    .iter(0)
                    .map(|marker| marker.value.display_marker().at(marker.from))
                    .collect();
                vec![Arc::new(
                    RangeSet::of(rows, false).expect("ordered lint marker rows"),
                )]
            })),
            ..Default::default()
        },
        Arc::new(|state, widget| {
            lint_gutter_widget_marker(state, widget)
                .ok()
                .flatten()
                .map(|marker| marker.display_marker())
        }),
    )
});

/// Install a single lint gutter slot and its diagnostic markers. This does
/// not install lint state or a provider. A host draws the markers and tooltips.
pub fn lint_gutter(config: LintGutterConfig) -> Extension {
    Extension::group([GUTTER.extension(), CONFIG.of(config), SLOT.clone()])
}

/// Read gutter markers, or return `None` if the gutter is disabled.
pub fn lint_gutter_markers(state: &EditorState) -> Option<Arc<RangeSet<LintGutterMarker>>> {
    state
        .context()
        .field(&GUTTER)
        .expect("valid gutter field")
        .map(|markers| (*markers).clone())
}

/// Combine diagnostics from line markers strictly inside a block widget.
/// Marker order is source line order, then publication order within a line.
/// A widget with no interior creates no marker.
pub fn lint_gutter_widget_marker(
    state: &EditorState,
    widget: &GutterWidget<'_>,
) -> Result<Option<LintGutterMarker>, EditError> {
    let Range { start, end } = widget.source();
    if !state.text().is_valid_range(start..end) {
        return Err(EditError::InvalidRange);
    }
    let Some(markers) = lint_gutter_markers(state) else {
        return Ok(None);
    };
    let mut diagnostics = Vec::new();
    markers.between(start, end, |marker| {
        if start < marker.from && marker.from < end {
            diagnostics.extend_from_slice(marker.value.diagnostics());
        }
        true
    });
    Ok((!diagnostics.is_empty()).then(|| LintGutterMarker(diagnostics.into())))
}

/// A native gutter tooltip for one line or block marker. The host owns pointer dwell,
/// cancellation, rendering and placement.
#[derive(Clone, Debug)]
pub struct LintGutterTooltip {
    pub delay: Duration,
    pub diagnostics: Vec<Diagnostic>,
}

/// Apply the gutter tooltip filter to the marker at `line_start`. Return
/// `None` if there is no marker or the filter removes its entire list.
/// The stored marker and its severity remain unchanged.
pub fn lint_gutter_tooltip_at(
    state: &EditorState,
    line_start: usize,
) -> Result<Option<LintGutterTooltip>, EditError> {
    let context = state.context();
    if !context.text().is_char_boundary(line_start) {
        return Err(EditError::InvalidRange);
    }
    let Some(markers) = lint_gutter_markers(state) else {
        return Ok(None);
    };
    let Some(mark) = markers
        .iter(line_start)
        .next()
        .filter(|mark| mark.from == line_start)
    else {
        return Ok(None);
    };
    lint_gutter_tooltip(state, &mark.value)
}

/// Build the filtered plan for a line or widget marker from this snapshot.
pub fn lint_gutter_tooltip(
    state: &EditorState,
    marker: &LintGutterMarker,
) -> Result<Option<LintGutterTooltip>, EditError> {
    let context = state.context();
    if context.field(&GUTTER)?.is_none() {
        return Ok(None);
    }
    let config = context.facet(&CONFIG)?;
    let config = config.as_ref().as_ref().map_err(|err| *err)?;
    let mut diagnostics = marker.diagnostics().to_vec();
    if let Some(filter) = &config.tooltip_filter {
        diagnostics = filter(diagnostics, &context);
    }
    Ok((!diagnostics.is_empty()).then_some(LintGutterTooltip {
        delay: config.hover_time.unwrap_or(Duration::from_millis(300)),
        diagnostics,
    }))
}
