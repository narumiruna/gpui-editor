//! Native diagnostic state adapted from `@codemirror/lint` 6.9.8.
//!
//! Positions are UTF-8 byte offsets into [`crate::Text`]. Invalid diagnostic
//! ranges are skipped. Native range layers and line endings can differ from
//! CodeMirror's. The host drives the clock and executes provider futures.
//!
//! This module stores panel and gutter state but does not render it. The host
//! handles focus, tooltips, diagnostic HTML, and UI events. See
//! `docs/licenses/codemirror-lint.txt` for the upstream MIT notice.

use std::{
    collections::HashSet,
    fmt,
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, EditError, EditorSelection, EditorState, Extension, Facet, MapMode, RangeSet,
    StateContext, StateEffect, StateEffectType, StateField, StateUpdate, Transaction,
};

mod gutter;
mod markers;
mod panel;
mod providers;
mod scheduler;
mod tooltip;

pub use gutter::{
    LintGutterConfig, LintGutterMarker, LintGutterTooltip, lint_gutter, lint_gutter_markers,
    lint_gutter_tooltip, lint_gutter_tooltip_at, lint_gutter_widget_marker,
};
pub use markers::DiagnosticMarker;
pub use panel::{LintCommand, close_lint_panel, lint_keymap, open_lint_panel};
pub use providers::{
    DiagnosticFilter, LintBatch, LintConfig, LintConfiguration, LintFuture, LintResult,
    LintSession, LintSource, NeedsRefresh, force_linting, linter,
};
use scheduler::PublicationScope;
pub use scheduler::{DEFAULT_LINT_DELAY, LintRun, LintScheduler};
pub use tooltip::{
    LintTooltip, LintTooltipHide, filter_tooltip_diagnostics, lint_hover_tooltip, lint_tooltip_at,
};

/// Diagnostic priority, from lowest to highest.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    Hint,
    Info,
    Warning,
    Error,
}

/// A diagnostic and its metadata. `range` is the original source range.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub range: Range<usize>,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
    pub mark_class: Option<String>,
    pub actions: Vec<Action>,
}

impl Diagnostic {
    pub fn new(range: Range<usize>, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            range,
            severity,
            message: message.into(),
            source: None,
            mark_class: None,
            actions: Vec::new(),
        }
    }
}

type ActionFn = dyn Fn(&EditorState, Range<usize>) -> Result<Transaction, EditError> + Send + Sync;

/// A named action that builds a transaction from the diagnostic's current range.
#[derive(Clone)]
pub struct Action {
    pub name: String,
    pub mark_class: Option<String>,
    apply: Arc<ActionFn>,
}

impl Action {
    pub fn new(
        name: impl Into<String>,
        apply: impl Fn(&EditorState, Range<usize>) -> Result<Transaction, EditError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            mark_class: None,
            apply: Arc::new(apply),
        }
    }
}

impl fmt::Debug for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Action")
            .field("name", &self.name)
            .field("mark_class", &self.mark_class)
            .finish_non_exhaustive()
    }
}

/// A diagnostic and its current range. Keep the `Arc` to identify the same
/// diagnostic after edits. A replacement list creates new diagnostic identities.
#[derive(Clone, Debug)]
pub struct LocatedDiagnostic {
    pub range: Range<usize>,
    pub diagnostic: Arc<Diagnostic>,
}

#[derive(Clone, Debug)]
struct DocumentVersion(Arc<()>);

impl DocumentVersion {
    fn new() -> Self {
        Self(Arc::new(()))
    }

    fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Diagnostic state for one editor snapshot.
#[derive(Clone, Debug)]
pub struct LintState {
    markers: Arc<RangeSet<DiagnosticMarker>>,
    selected: Option<LocatedDiagnostic>,
    panel_open: bool,
    version: DocumentVersion,
}

impl LintState {
    /// Read lint state, or return `None` if [`extension`] is not enabled.
    pub fn get(state: &EditorState) -> Option<Arc<Self>> {
        state
            .context()
            .field(&LINT_STATE)
            .expect("valid lint field")
    }

    /// Get the native marker segments, ordered by position and side.
    pub fn markers(&self) -> &RangeSet<DiagnosticMarker> {
        &self.markers
    }

    /// Get the selected lint-list entry, independently of the editor selection.
    pub fn selected(&self) -> Option<&LocatedDiagnostic> {
        self.selected.as_ref()
    }

    pub fn panel_open(&self) -> bool {
        self.panel_open
    }

    /// Get the panel entries in display order, without a placeholder when empty.
    /// Use [`select_diagnostic`] to move to one of these entries.
    pub fn panel_diagnostics(&self) -> Vec<Arc<Diagnostic>> {
        let mut seen = HashSet::new();
        markers::in_layer_order(&self.markers, 0)
            .flat_map(|marker| marker.value.diagnostics())
            .filter(|diagnostic| seen.insert(Arc::as_ptr(diagnostic)))
            .cloned()
            .collect()
    }

    fn replace(
        &mut self,
        publication: PublishedDiagnostics<'_>,
        state: &StateContext<'_>,
        configs: &[Arc<providers::LintInput>],
    ) -> Result<(), crate::ConfigurationError> {
        if matches!(publication.scope, PublicationScope::Discard) {
            self.markers = Arc::new(RangeSet::empty());
            self.selected = None;
            return Ok(());
        }
        let mut filtered = std::borrow::Cow::Borrowed(publication.diagnostics);
        for filter in providers::combine_callbacks(
            configs
                .iter()
                .filter_map(|input| input.config.marker_filter.as_ref()),
        ) {
            filtered = std::borrow::Cow::Owned(filter(filtered.into_owned(), state));
        }
        let filters = state.facet(&MARKER_FILTERS)?;
        let text = state.text();
        self.markers = Arc::new(markers::build(
            text,
            filtered.iter().filter(|d| {
                publication.scope.accepts(&d.range)
                    && text.is_valid_range(d.range.clone())
                    && filters.iter().all(|keep| keep(d, state))
            }),
        ));
        self.selected = markers::find(&self.markers, None, 0);
        Ok(())
    }

    fn update(&self, update: &StateUpdate<'_>) -> Result<Self, crate::ConfigurationError> {
        let configs = update.state().facet(&providers::LINTERS)?;
        let auto_panel = configs.iter().any(|input| input.config.auto_panel);
        let mut next = self.clone();
        if !update.changes().is_empty() {
            next.version = DocumentVersion::new();
            if !self.markers.is_empty() {
                next.markers = Arc::new(self.markers.map(update.transaction().changes()));
            }
            if let Some(selected) = &self.selected {
                let after =
                    update
                        .changes()
                        .map_pos(selected.range.start, Assoc::After, MapMode::Simple);
                next.selected = after.and_then(|after| {
                    markers::find(&next.markers, Some(&selected.diagnostic), after)
                        .or_else(|| markers::find(&next.markers, None, after))
                });
            }
            if auto_panel && !self.markers.is_empty() && next.markers.is_empty() {
                next.panel_open = false;
            }
        }
        for effect in update.transaction().effects() {
            if let Some(publication) = published_diagnostics(effect, update, Some(self)) {
                if auto_panel {
                    next.panel_open = !publication.diagnostics.is_empty();
                }
                next.replace(publication, update.state(), &configs)?;
            } else if let Some(open) = effect.get(&panel::TOGGLE_PANEL) {
                next.panel_open = *open;
            } else if let Some(diagnostic) = effect.get(&SELECT_DIAGNOSTIC)
                && let Some(found) = markers::find(&next.markers, Some(diagnostic), 0)
            {
                next.selected = Some(found);
            }
        }
        Ok(next)
    }
}

static LINT_STATE: LazyLock<StateField<LintState>> = LazyLock::new(|| {
    StateField::new(
        |_| {
            Ok(LintState {
                markers: Arc::new(RangeSet::empty()),
                selected: None,
                panel_open: false,
                version: DocumentVersion::new(),
            })
        },
        LintState::update,
    )
});
static SET_DIAGNOSTICS: LazyLock<StateEffectType<Vec<Diagnostic>>> =
    LazyLock::new(StateEffectType::new);
static SELECT_DIAGNOSTIC: LazyLock<StateEffectType<Arc<Diagnostic>>> =
    LazyLock::new(StateEffectType::new);
type MarkerFilter = Arc<dyn Fn(&Diagnostic, &StateContext<'_>) -> bool + Send + Sync>;
static MARKER_FILTERS: LazyLock<Facet<MarkerFilter, Vec<MarkerFilter>>> =
    LazyLock::new(Facet::collect);

/// Install lint state and its hover source without configuring or running a
/// provider. A host resolves and draws hover content.
pub fn extension() -> Extension {
    Extension::group([LINT_STATE.extension(), lint_hover_tooltip().extension()])
}

/// Add a predicate that determines which valid diagnostics produce markers.
/// All installed predicates must accept a diagnostic. They run only on a new
/// publication. Do not read lint state from within a predicate.
pub fn marker_filter(
    keep: impl Fn(&Diagnostic, &StateContext<'_>) -> bool + Send + Sync + 'static,
) -> Extension {
    MARKER_FILTERS.of(Arc::new(keep))
}

/// Replace the current diagnostics. Ranges are measured in the transaction's
/// resulting text. Invalid ranges are skipped rather than clipped. Like the
/// upstream effect, this effect does not rebase its payload when mapped.
pub fn set_diagnostics_effect() -> &'static StateEffectType<Vec<Diagnostic>> {
    &SET_DIAGNOSTICS
}

/// Build a replacement transaction for an editor with [`extension`] enabled.
pub fn set_diagnostics(diagnostics: Vec<Diagnostic>) -> Transaction {
    Transaction::default().with_effects([set_diagnostics_effect().of(diagnostics)])
}

/// Build a replacement transaction that also installs lint state if needed.
/// This is the state-aware form of [`set_diagnostics`].
pub fn set_diagnostics_for(state: &EditorState, diagnostics: Vec<Diagnostic>) -> Transaction {
    enable_lint(state, set_diagnostics(diagnostics))
}

/// Clear all current diagnostics, without cancelling pending provider work.
pub fn clear_diagnostics(state: &EditorState) -> Transaction {
    set_diagnostics_for(state, Vec::new())
}

fn enable_lint(state: &EditorState, transaction: Transaction) -> Transaction {
    if LintState::get(state).is_some() {
        transaction
    } else {
        transaction.with_effects([extension().append()])
    }
}

struct PublishedDiagnostics<'a> {
    diagnostics: &'a [Diagnostic],
    scope: &'a PublicationScope,
}

fn published_diagnostics<'a>(
    effect: &'a StateEffect,
    update: &StateUpdate<'_>,
    lint: Option<&LintState>,
) -> Option<PublishedDiagnostics<'a>> {
    if let Some(diagnostics) = effect.get(set_diagnostics_effect()) {
        return Some(PublishedDiagnostics {
            diagnostics,
            scope: &PublicationScope::Document,
        });
    }
    let result = effect.get(&scheduler::PUBLISH_DIAGNOSTICS)?;
    (update.changes().is_empty() && lint.is_some_and(|lint| lint.version.matches(&result.version)))
        .then_some(PublishedDiagnostics {
            diagnostics: &result.diagnostics,
            scope: &result.scope,
        })
}

/// Count the active marker segments, as upstream does. Overlap can split one
/// diagnostic into multiple segments or combine multiple diagnostics in one.
pub fn diagnostic_count(state: &EditorState) -> usize {
    LintState::get(state).map_or(0, |lint| lint.markers.len())
}

/// Visit the marked diagnostics with their current ranges.
pub fn for_each_diagnostic(state: &EditorState, visit: impl FnMut(LocatedDiagnostic)) {
    if let Some(lint) = LintState::get(state) {
        markers::for_each(&lint.markers, visit);
    }
}

/// Select the next diagnostic and request scrolling. Wrap at the end.
pub fn next_diagnostic(state: &EditorState) -> Option<Transaction> {
    let lint = LintState::get(state)?;
    let selection = state.selection().main().range();
    let found = markers::find(&lint.markers, None, selection.end.saturating_add(1))
        .or_else(|| markers::find(&lint.markers, None, 0).filter(|d| d.range != selection))?;
    Some(selection_transaction(found.range))
}

/// Select the previous marker segment and request scrolling. Wrap at the start.
pub fn previous_diagnostic(state: &EditorState) -> Option<Transaction> {
    let lint = LintState::get(state)?;
    let selection = state.selection().main().range();
    let mut previous = None;
    let mut last = None;
    for marker in markers::in_layer_order(&lint.markers, 0) {
        if marker.to < selection.end
            && previous.is_none_or(|from: (usize, usize)| from.0 < marker.from)
        {
            previous = Some((marker.from, marker.to));
        }
        if last.is_none_or(|from: (usize, usize)| from.0 < marker.from) {
            last = Some((marker.from, marker.to));
        }
    }
    let (from, to) = previous.or_else(|| last.filter(|(from, _)| *from != selection.start))?;
    Some(selection_transaction(from..to))
}

/// Select a lint-list entry and move the editor selection to its current range.
/// Return `None` if the diagnostic is no longer active.
pub fn select_diagnostic(state: &EditorState, diagnostic: &Arc<Diagnostic>) -> Option<Transaction> {
    let lint = LintState::get(state)?;
    let found = markers::find(&lint.markers, Some(diagnostic), 0)?;
    Some(
        selection_transaction(found.range).with_effects([SELECT_DIAGNOSTIC.of(diagnostic.clone())]),
    )
}

/// Build a transaction for an action using the diagnostic's current range.
/// Return `None` if the diagnostic is no longer active or the index is invalid.
pub fn run_action(
    state: &EditorState,
    diagnostic: &Arc<Diagnostic>,
    index: usize,
) -> Result<Option<Transaction>, EditError> {
    let Some((action, lint)) = diagnostic.actions.get(index).zip(LintState::get(state)) else {
        return Ok(None);
    };
    markers::find(&lint.markers, Some(diagnostic), 0)
        .map(|found| (action.apply)(state, found.range))
        .transpose()
}

fn selection_transaction(range: Range<usize>) -> Transaction {
    Transaction::default()
        .with_selection(EditorSelection::single(range.start, range.end))
        .with_scroll_into_view()
}
