use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    EditorState, Extension, Facet, Transaction,
    view::gutters::{GutterConfig, GutterMarker, gutter},
};

use super::{code_folding, fold_effect, foldable, folded_ranges, syntax_fallback, unfold_effect};

pub const FOLD_GUTTER_CLASS: &str = "cm-foldGutter";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldGutterState {
    Open,
    Closed,
}

pub type FoldMarkerRenderer =
    Arc<dyn Fn(&EditorState, FoldGutterState) -> GutterMarker + Send + Sync>;
pub type FoldGutterClickHandler = Arc<dyn Fn(&EditorState, &Range<usize>) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct FoldGutterConfig {
    pub open_text: String,
    pub closed_text: String,
    /// Replace the stock plain-text marker. The host supplies any custom title.
    pub render_marker: Option<FoldMarkerRenderer>,
    /// Give the host the first chance to handle a click without a fold effect.
    pub handle_click: Option<FoldGutterClickHandler>,
}

impl Default for FoldGutterConfig {
    fn default() -> Self {
        Self {
            open_text: "⌄".into(),
            closed_text: "›".into(),
            render_marker: None,
            handle_click: None,
        }
    }
}

type FoldGutters = Facet<Arc<FoldGutterConfig>, Vec<Arc<FoldGutterConfig>>>;
static FOLD_GUTTERS: LazyLock<FoldGutters> = LazyLock::new(Facet::collect);

#[derive(Debug, Eq, PartialEq)]
pub struct FoldGutterMarker {
    pub state: FoldGutterState,
    pub marker: GutterMarker,
    pub title: Option<String>,
}

#[derive(Debug)]
pub enum FoldGutterClick {
    Ignored,
    Handled,
    Dispatch(Transaction),
}

impl FoldGutterConfig {
    /// Get the first installed fold gutter in extension precedence order.
    /// A host with one fold column uses this configuration.
    pub fn configured(state: &EditorState) -> Option<Arc<Self>> {
        state.context().facet(&FOLD_GUTTERS).ok()?.first().cloned()
    }

    /// Resolve a marker for a line block. The native host can supply the
    /// current parser as a fallback after all registered fold services.
    pub fn marker(
        &self,
        state: &EditorState,
        line: Range<usize>,
        syntax: impl FnOnce(&EditorState, usize, usize) -> Option<Range<usize>>,
    ) -> Option<FoldGutterMarker> {
        self.marker_if(state, line, syntax, |_, _| true)
    }

    /// Resolve a marker only if the host accepts its fold range.
    pub fn marker_if(
        &self,
        state: &EditorState,
        line: Range<usize>,
        syntax: impl FnOnce(&EditorState, usize, usize) -> Option<Range<usize>>,
        accept: impl FnOnce(FoldGutterState, &Range<usize>) -> bool,
    ) -> Option<FoldGutterMarker> {
        let (kind, range) = candidate(state, line, syntax)?;
        accept(kind, &range).then(|| self.render(state, kind))
    }

    /// Use the closed marker when measuring a sample gutter width.
    pub fn spacer(&self, state: &EditorState) -> FoldGutterMarker {
        self.render(state, FoldGutterState::Closed)
    }

    /// Plan a gutter click from the current line block. Clicks do not produce
    /// command announcements and do not change the selection. The host
    /// dispatches the transaction against the same snapshot.
    pub fn click(
        &self,
        state: &EditorState,
        line: Range<usize>,
        syntax: impl FnOnce(&EditorState, usize, usize) -> Option<Range<usize>>,
    ) -> FoldGutterClick {
        if !state.text().is_valid_range(line.clone()) {
            return FoldGutterClick::Ignored;
        }
        if self
            .handle_click
            .as_ref()
            .is_some_and(|click| click(state, &line))
        {
            return FoldGutterClick::Handled;
        }
        let Some((kind, range)) = candidate(state, line, syntax) else {
            return FoldGutterClick::Ignored;
        };
        let effect = match kind {
            FoldGutterState::Open => fold_effect().of(range),
            FoldGutterState::Closed => unfold_effect().of(range),
        };
        FoldGutterClick::Dispatch(
            super::commands::transaction(state, vec![effect]).expect("fold effect"),
        )
    }

    /// Render the given marker state, without resolving a fold candidate.
    pub fn render(&self, state: &EditorState, kind: FoldGutterState) -> FoldGutterMarker {
        let (marker, title) = if let Some(renderer) = &self.render_marker {
            (renderer(state, kind), None)
        } else {
            let (text, title) = match kind {
                FoldGutterState::Open => (&self.open_text, "Fold line"),
                FoldGutterState::Closed => (&self.closed_text, "Unfold line"),
            };
            (
                GutterMarker::text(text.clone()),
                Some(state.phrase(title, &[])),
            )
        };
        FoldGutterMarker {
            state: kind,
            marker,
            title,
        }
    }
}

/// Install a fold gutter and fold state. The logical-line projection in
/// `view::gutters` uses registered fold services, then attached syntax. For
/// measured blocks or a caller-owned syntax rule, use [`FoldGutterConfig::marker`] and
/// [`FoldGutterConfig::click`] with the same configuration.
pub fn fold_gutter(config: FoldGutterConfig) -> Extension {
    let config = Arc::new(config);
    Extension::group([
        FOLD_GUTTERS.of(config.clone()),
        gutter(GutterConfig {
            class: FOLD_GUTTER_CLASS.into(),
            line_marker: Some(Arc::new({
                let config = config.clone();
                move |state, line, _| {
                    config
                        .marker(state, line.range.clone(), syntax_fallback(None))
                        .map(|item| item.marker)
                }
            })),
            spacer: Some(Arc::new(move |state| config.spacer(state).marker)),
            ..GutterConfig::default()
        }),
        code_folding(),
    ])
}

fn candidate(
    state: &EditorState,
    line: Range<usize>,
    syntax: impl FnOnce(&EditorState, usize, usize) -> Option<Range<usize>>,
) -> Option<(FoldGutterState, Range<usize>)> {
    if !state.text().is_valid_range(line.clone()) {
        return None;
    }
    if let Some(existing) = folded_ranges(state)
        .iter(line.start)
        .find(|fold| (line.start..=line.end).contains(&fold.from))
    {
        return Some((FoldGutterState::Closed, existing.from..existing.to));
    }
    foldable(state, line.start, line.end, syntax).map(|range| (FoldGutterState::Open, range))
}
