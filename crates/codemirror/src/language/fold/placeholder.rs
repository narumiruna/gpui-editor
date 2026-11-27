//! Plain-text rendering data for native fold widgets.

use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    ConfigurationError, EditError, EditorState, Extension, Facet, StateContext,
    view::decorations::{Decoration, DecorationSet, ReplaceSpec},
};

use super::{code_folding, field};

/// Prepare text from the resulting state for a new fold. Called before an
/// explicit selection can remove the fold. Do not mutate external state.
pub type PrepareFoldPlaceholder =
    Arc<dyn Fn(&StateContext<'_>, &Range<usize>) -> Arc<str> + Send + Sync>;

/// Return a label using a fold's stored value. `None` uses the stock label.
pub type RenderFoldPlaceholder =
    Arc<dyn Fn(&EditorState, &Range<usize>, Option<&str>) -> Option<Arc<str>> + Send + Sync>;

#[derive(Clone, Default)]
pub struct FoldPlaceholderConfig {
    /// The stock widget's text. Defaults to an ellipsis.
    pub text: Option<Arc<str>>,
    pub prepare: Option<PrepareFoldPlaceholder>,
    pub render: Option<RenderFoldPlaceholder>,
}

type Options = Facet<FoldPlaceholderConfig, Vec<FoldPlaceholderConfig>>;
static OPTIONS: LazyLock<Options> = LazyLock::new(Facet::collect);
static CONFIG: LazyLock<Facet<FoldPlaceholderConfig, FoldPlaceholderConfig>> =
    LazyLock::new(|| {
        Facet::new(|inputs: &[&FoldPlaceholderConfig]| {
            inputs
                .first()
                .map_or_else(FoldPlaceholderConfig::default, |&first| first.clone())
        })
    });
static DEFAULT_TEXT: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from("…"));

static MERGE_OPTIONS: LazyLock<Extension> = LazyLock::new(|| {
    CONFIG.compute_with_dependencies(
        |deps| deps.facet(&OPTIONS),
        |state| {
            let options = state.facet(&OPTIONS)?;
            let mut merged = FoldPlaceholderConfig::default();
            for option in options.iter() {
                merge(&mut merged.text, &option.text, PartialEq::eq)?;
                merge(&mut merged.prepare, &option.prepare, Arc::ptr_eq)?;
                merge(&mut merged.render, &option.render, Arc::ptr_eq)?;
            }
            Ok(merged)
        },
    )
});

fn merge<T: ?Sized>(
    merged: &mut Option<Arc<T>>,
    input: &Option<Arc<T>>,
    eq: impl FnOnce(&Arc<T>, &Arc<T>) -> bool,
) -> Result<(), ConfigurationError> {
    if let Some(input) = input {
        if merged.as_ref().is_some_and(|prior| !eq(prior, input)) {
            return Err(ConfigurationError::InvalidFieldData);
        }
        *merged = Some(input.clone());
    }
    Ok(())
}

/// Install folding with configurable placeholder text and callbacks.
/// Distinct values for the same option conflict. Equal strings or shared
/// callback instances can be reused.
pub fn code_folding_with_placeholders(config: FoldPlaceholderConfig) -> Extension {
    Extension::group([code_folding(), OPTIONS.of(config), MERGE_OPTIONS.clone()])
}

pub(super) fn prepare(
    state: &StateContext<'_>,
) -> Result<Option<PrepareFoldPlaceholder>, ConfigurationError> {
    Ok(state.facet(&CONFIG)?.prepare.clone())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoldPlaceholder {
    pub text: Arc<str>,
    /// The stock widget's localized tooltip and accessibility label. A custom
    /// renderer supplies its own platform metadata instead.
    pub title: Option<String>,
    pub aria_label: Option<String>,
}

impl FoldPlaceholder {
    fn render(
        state: &EditorState,
        range: &Range<usize>,
        prepared: Option<&str>,
        config: &FoldPlaceholderConfig,
    ) -> Self {
        if let Some(text) = config
            .render
            .as_ref()
            .and_then(|render| render(state, range, prepared))
        {
            return Self {
                text,
                title: None,
                aria_label: None,
            };
        }
        Self {
            text: config.text.clone().unwrap_or_else(|| DEFAULT_TEXT.clone()),
            title: Some(state.phrase("unfold", &[])),
            aria_label: Some(state.phrase("folded code", &[])),
        }
    }
}

/// Resolve the current label for a stored fold. Prepared values follow their
/// fold through edits; a JSON-restored fold has no prepared value.
pub fn fold_placeholder(state: &EditorState, range: &Range<usize>) -> Option<FoldPlaceholder> {
    let folds = field(state)?;
    let index = folds
        .entries
        .partition_point(|entry| (entry.range.start, entry.range.end) < (range.start, range.end));
    let entry = folds
        .entries
        .get(index)
        .filter(|entry| &entry.range == range)?;
    let config = state
        .context()
        .facet(&CONFIG)
        .expect("valid fold placeholder config");
    Some(FoldPlaceholder::render(
        state,
        range,
        entry.prepared.as_deref(),
        &config,
    ))
}

/// Project all stored folds as replacement widgets. Use the same ranges as
/// atomic spans and handle clicks with the matching unfold effect.
pub fn fold_placeholder_decorations(
    state: &EditorState,
) -> Result<DecorationSet<FoldPlaceholder>, EditError> {
    let Some(folds) = field(state) else {
        return Ok(DecorationSet::empty());
    };
    let config = state.context().facet(&CONFIG)?;
    let ranges = folds
        .entries
        .iter()
        .map(|entry| {
            let widget =
                FoldPlaceholder::render(state, &entry.range, entry.prepared.as_deref(), &config);
            Decoration::replace(Some(widget), ReplaceSpec::default()).try_range(entry.range.clone())
        })
        .collect::<Result<Vec<_>, _>>()?;
    DecorationSet::new(state.text(), ranges, true).map_err(|_| EditError::InvalidRange)
}
