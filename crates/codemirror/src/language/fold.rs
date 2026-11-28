//! Fold state and commands based on `@codemirror/language` 6.13.1.
//! See `docs/licenses/codemirror-language.txt` for the upstream MIT notice.
//!
//! Ranges use UTF-8 byte offsets. This module does not render folds.

use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use serde_json::{Value, json};

use crate::{
    Assoc, ChangeDesc, ConfigurationError, EditorState, Extension, Facet, JsonField, MapMode,
    RangeSet, RangeValue, StateEffectType, StateField, StateUpdate, Text,
    language::markdown::markdown_snapshot,
    language_data::{NativeSyntax, attached_language_snapshot, fold_catalog_markdown_html},
};

mod commands;
mod gutter;
mod placeholder;
mod syntax;
pub use commands::{
    FoldCommand, announce_fold, fold_all, fold_code, fold_keymap, toggle_fold, unfold_all,
    unfold_code,
};
pub use gutter::{
    FOLD_GUTTER_CLASS, FoldGutterClick, FoldGutterClickHandler, FoldGutterConfig, FoldGutterMarker,
    FoldGutterState, FoldMarkerRenderer, fold_gutter,
};
pub use placeholder::{
    FoldPlaceholder, FoldPlaceholderConfig, PrepareFoldPlaceholder, RenderFoldPlaceholder,
    code_folding_with_placeholders, fold_placeholder, fold_placeholder_decorations,
};
pub(in crate::language) use syntax::fold_on_line;

type FoldService = Arc<dyn Fn(&EditorState, usize, usize) -> Option<Range<usize>> + Send + Sync>;
static SERVICES: LazyLock<Facet<FoldService, Vec<FoldService>>> = LazyLock::new(Facet::collect);
static SYNTAX_FALLBACKS: LazyLock<Facet<FoldService, Vec<FoldService>>> =
    LazyLock::new(Facet::collect);

/// Register a fold service. Higher-precedence services run first, before the
/// host's syntax rule. Return `None` to defer to the next service or rule.
pub fn fold_service(
    provider: impl Fn(&EditorState, usize, usize) -> Option<Range<usize>> + Send + Sync + 'static,
) -> Extension {
    SERVICES.of(Arc::new(provider))
}

pub(super) fn syntax_fold_fallback(
    provider: impl Fn(&EditorState, usize, usize) -> Option<Range<usize>> + Send + Sync + 'static,
) -> Extension {
    SYNTAX_FALLBACKS.of(Arc::new(provider))
}

/// Ask configured services whether a line block is foldable. Use `syntax`
/// only when all services defer. Both block endpoints and the proposed fold
/// must be valid UTF-8 byte offsets. The first returned range takes
/// precedence. Reject an invalid range without asking other providers.
pub fn foldable(
    state: &EditorState,
    start: usize,
    end: usize,
    syntax: impl FnOnce(&EditorState, usize, usize) -> Option<Range<usize>>,
) -> Option<Range<usize>> {
    if !state.text().is_valid_range(start..end) {
        return None;
    }
    let services = state
        .context()
        .facet(&SERVICES)
        .expect("typed fold services");
    services
        .iter()
        .find_map(|service| service(state, start, end))
        .or_else(|| syntax(state, start, end))
        .filter(|range| valid_range(state.text(), range))
}

/// Ask configured services, then attached syntax, whether these line bounds
/// have a fold. An attached whole-document C or C++ parser reads typed fold
/// rules. Without an attached parser, a Markdown WAST, JSON, Lezer,
/// or configured Rust mount can supply a fold. The fold can span gaps if both endpoints belong to
/// the mount. Ignore invalid node callback ranges before selecting a candidate. Bounds
/// and the returned range use original UTF-8 byte offsets.
/// Use [`foldable`] to supply an explicit syntax rule instead.
pub fn foldable_in_state(state: &EditorState, start: usize, end: usize) -> Option<Range<usize>> {
    foldable(state, start, end, syntax_fallback(None))
}

// The caller has checked that explicit syntax is current. Its no-fold result
// takes precedence over the attachment, too.
pub(crate) fn syntax_fallback(
    explicit: Option<&NativeSyntax>,
) -> impl Fn(&EditorState, usize, usize) -> Option<Range<usize>> + '_ {
    move |state, start, end| {
        let attached;
        let syntax = match explicit {
            Some(syntax) => syntax,
            None => {
                if let Some(snapshot) = attached_language_snapshot(state) {
                    if let NativeSyntax::Markdown(host) = snapshot.syntax() {
                        return host
                            .foldable(start, end)
                            .or_else(|| fold_catalog_markdown_html(state, &snapshot, start, end));
                    }
                    attached = snapshot;
                    attached.syntax()
                } else {
                    let fallbacks = state
                        .context()
                        .facet(&SYNTAX_FALLBACKS)
                        .expect("typed syntax fold fallbacks");
                    return fallbacks
                        .iter()
                        .find_map(|fold| fold(state, start, end))
                        .or_else(|| {
                            markdown_snapshot(state)?.markdown_code_fold(state, start, end)
                        });
                }
            }
        };
        if let NativeSyntax::Cpp(doc) = syntax {
            return doc.foldable_in_state(state, start..end);
        }
        if syntax.has_configured_folds() {
            syntax.foldable_in_scope(state, 0..state.text().len(), start..end)
        } else {
            syntax.foldable(start, end)
        }
    }
}

/// A non-inclusive, atomic range hidden by a fold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fold;

impl RangeValue for Fold {
    fn eq(&self, _other: &Self) -> bool {
        true
    }

    fn start_side(&self) -> i32 {
        1
    }

    fn end_side(&self) -> i32 {
        -1
    }

    fn is_point(&self) -> bool {
        true
    }
}

#[derive(Clone, Debug)]
struct FoldEntry {
    range: Range<usize>,
    prepared: Option<Arc<str>>,
}

/// The folded ranges for one editor snapshot. Nested ranges are retained.
#[derive(Clone, Debug, Default)]
pub struct FoldState {
    ranges: Arc<RangeSet<Fold>>,
    entries: Arc<[FoldEntry]>,
}

impl FoldState {
    pub fn ranges(&self) -> &RangeSet<Fold> {
        &self.ranges
    }

    fn update(&self, update: &StateUpdate<'_>) -> Result<Self, ConfigurationError> {
        let transaction = update.transaction();
        let changes = update.changes();
        let selection_head = transaction
            .selection()
            .map(|selection| selection.main().head());
        let effects = transaction.effects().iter().filter_map(|effect| {
            if let Some(range) = effect.get(fold_effect()) {
                Some((true, range))
            } else {
                effect.get(unfold_effect()).map(|range| (false, range))
            }
        });
        if changes.is_empty() && selection_head.is_none() && effects.clone().next().is_none() {
            return Ok(self.clone());
        }

        let deletions: Vec<_> = if transaction.is_user_event("delete") {
            changes.changed_ranges().map(|(old, _)| old).collect()
        } else {
            Vec::new()
        };
        let mut existing: Vec<_> = self
            .entries
            .iter()
            .filter_map(|entry| {
                if deletions
                    .iter()
                    .any(|deleted| touches(&entry.range, deleted))
                {
                    return None;
                }
                Some(FoldEntry {
                    range: map_range(&entry.range, changes)?,
                    prepared: entry.prepared.clone(),
                })
            })
            .collect();

        // Unfold effects act on existing folds. Add the queued folds only
        // after all effects, as the upstream field does.
        let mut pending = Vec::new();
        for (add, range) in effects {
            if add {
                if !valid_range(update.state().text(), range) {
                    return Err(ConfigurationError::InvalidFieldData);
                }
                if !existing.iter().any(|entry| &entry.range == range) {
                    pending.push(range.clone());
                }
            } else {
                existing.retain(|entry| &entry.range != range);
            }
        }
        if !pending.is_empty() {
            let state = update.state();
            let prepare = placeholder::prepare(state)?;
            existing.extend(pending.into_iter().map(|range| FoldEntry {
                prepared: prepare.as_ref().map(|prepare| prepare(state, &range)),
                range,
            }));
        }
        if let Some(head) = selection_head {
            existing.retain(|entry| !touches(&entry.range, &(head..head)));
        }
        Ok(Self::from_entries(existing))
    }

    fn from_entries(mut entries: Vec<FoldEntry>) -> Self {
        entries.sort_by_key(|entry| (entry.range.start, entry.range.end));
        let ranges = RangeSet::of(
            entries
                .iter()
                .map(|entry| Fold.range(entry.range.start, entry.range.end))
                .collect(),
            false,
        )
        .expect("ordered fold ranges");
        Self {
            ranges: Arc::new(ranges),
            entries: entries.into(),
        }
    }

    fn from_json(value: &Value, text: &Text) -> Result<Self, ConfigurationError> {
        let invalid = ConfigurationError::InvalidFieldData;
        let items = value.as_array().ok_or(invalid)?;
        let (pairs, rest) = items.as_chunks::<2>();
        if !rest.is_empty() {
            return Err(invalid);
        }
        fn position(value: &Value) -> Option<usize> {
            usize::try_from(value.as_u64()?).ok()
        }
        let ranges = pairs
            .iter()
            .map(|[from, to]| {
                let range = position(from).ok_or(invalid)?..position(to).ok_or(invalid)?;
                valid_range(text, &range).then_some(range).ok_or(invalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::from_entries(
            ranges
                .into_iter()
                .map(|range| FoldEntry {
                    range,
                    prepared: None,
                })
                .collect(),
        ))
    }
}

fn touches(range: &Range<usize>, query: &Range<usize>) -> bool {
    range.start < query.end && range.end > query.start
}

fn valid_range(text: &Text, range: &Range<usize>) -> bool {
    !range.is_empty() && text.is_valid_range(range.clone())
}

fn map_range(range: &Range<usize>, changes: &ChangeDesc) -> Option<Range<usize>> {
    let from = changes.map_pos(range.start, Assoc::After, MapMode::Simple)?;
    let to = changes.map_pos(range.end, Assoc::Before, MapMode::Simple)?;
    (from < to).then_some(from..to)
}

static FOLD_EFFECT: LazyLock<StateEffectType<Range<usize>>> =
    LazyLock::new(|| StateEffectType::with_mapping(map_range));
static UNFOLD_EFFECT: LazyLock<StateEffectType<Range<usize>>> =
    LazyLock::new(|| StateEffectType::with_mapping(map_range));
static FOLD_STATE: LazyLock<StateField<FoldState>> =
    LazyLock::new(|| StateField::new(|_| Ok(FoldState::default()), FoldState::update));
static EMPTY: LazyLock<Arc<RangeSet<Fold>>> = LazyLock::new(|| Arc::new(RangeSet::empty()));

/// Queue a fold in the resulting document. The enabled field rejects empty,
/// reversed, out-of-bounds, or non-UTF-8-boundary ranges.
pub fn fold_effect() -> &'static StateEffectType<Range<usize>> {
    &FOLD_EFFECT
}

/// Remove existing folds whose endpoints both match the effect value.
/// Measure the range in the resulting document.
pub fn unfold_effect() -> &'static StateEffectType<Range<usize>> {
    &UNFOLD_EFFECT
}

/// The field definition for native folding state.
pub fn fold_state() -> &'static StateField<FoldState> {
    &FOLD_STATE
}

/// Enable native folding state. This does not install a view or key bindings.
pub fn code_folding() -> Extension {
    fold_state().extension()
}

/// Get the current folds, or an empty set if folding is not enabled.
pub fn folded_ranges(state: &EditorState) -> Arc<RangeSet<Fold>> {
    field(state).map_or_else(|| EMPTY.clone(), |folds| folds.ranges.clone())
}

fn field(state: &EditorState) -> Option<Arc<FoldState>> {
    state
        .context()
        .field(fold_state())
        .expect("valid fold field")
}

/// Serialize folds as a flat array of byte endpoints, sorted by start then end.
/// Unlike the upstream layered traversal, the output uses native range order.
/// Restoration accepts any pair order and validates all ranges against the text.
pub fn fold_json(name: impl Into<String>) -> JsonField {
    fold_state().json(
        name,
        |folds, _| {
            let endpoints: Vec<_> = folds
                .ranges()
                .iter(0)
                .flat_map(|fold| [fold.from, fold.to])
                .collect();
            Ok(json!(endpoints))
        },
        |value, state| FoldState::from_json(value, state.text()),
    )
}
