use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, ChangeDesc, ChangeSet, ConfigurationError, EditError, EditorState, Extension, MapMode,
    StateEffect, StateEffectType, StateField, StateUpdate, Text,
};

use super::{
    Chunk, CollapseConfig, DiffConfig, Side,
    projection::{build_collapsed_ranges, update_collapsed_range},
};

mod actions;
pub use actions::{
    MergeControl, MergeControlKind, accept_chunk, go_to_next_chunk, go_to_previous_chunk,
    reject_chunk,
};

/// Settings for a headless unified merge. The editor document is side B.
#[derive(Clone, Debug)]
pub struct UnifiedMergeConfig {
    /// The initial original document (side A). Its line endings are preserved.
    pub original: Text,
    pub diff_config: DiffConfig,
    /// List per-chunk accept and reject controls. Defaults to `true` and does
    /// not affect programmatic commands.
    pub merge_controls: bool,
    /// Hide unchanged lines in side B. Defaults to `None` (all lines visible).
    pub collapse_unchanged: Option<CollapseConfig>,
}

impl UnifiedMergeConfig {
    pub fn new(original: Text) -> Self {
        Self {
            original,
            diff_config: DiffConfig::default(),
            merge_controls: true,
            collapse_unchanged: None,
        }
    }
}

/// A snapshot of the original document, changed chunks, and collapsed ranges.
#[derive(Clone, Debug)]
pub struct UnifiedMerge {
    original: Text,
    chunks: Arc<[Chunk]>,
    config: DiffConfig,
    merge_controls: bool,
    collapsed: Arc<[Range<usize>]>,
}

impl UnifiedMerge {
    pub fn original(&self) -> &Text {
        &self.original
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    /// Hidden ranges in the editor document, in UTF-8 bytes. The ranges
    /// exclude the last hidden line's ending and do not block edits or selections.
    pub fn collapsed_ranges(&self) -> &[Range<usize>] {
        &self.collapsed
    }

    /// Expand the range that starts at `byte_start` in the transaction's
    /// resulting editor document. If no range starts there, do nothing.
    pub fn expand_effect(byte_start: usize) -> StateEffect {
        EXPAND_UNCHANGED.of(byte_start)
    }

    /// List the controls for the current chunks, with accept before reject.
    /// An application can use each control's position with the corresponding
    /// command. Read a new snapshot after a document or configuration change.
    pub fn controls(&self) -> impl Iterator<Item = MergeControl> + '_ {
        let chunks = if self.merge_controls {
            self.chunks.as_ref()
        } else {
            &[]
        };
        chunks.iter().flat_map(|chunk| {
            [MergeControlKind::Accept, MergeControlKind::Reject].map(|kind| MergeControl {
                kind,
                position: chunk.range(Side::B).start,
            })
        })
    }

    fn update(&self, update: &StateUpdate<'_>) -> Result<Self, ConfigurationError> {
        let mut next = self.clone();
        // Use the same effect for the original document and its change description.
        // Each effect is a replacement computed from the start state's original.
        let original_edit = update
            .transaction()
            .effects()
            .iter()
            .rev()
            .find_map(|effect| effect.get(&UPDATE_ORIGINAL));
        if let Some(edit) = original_edit {
            if self.original != edit.before {
                return Err(ConfigurationError::InvalidFieldData);
            }
            next.original = edit.after.clone();
            if !edit.changes.is_empty() {
                next.chunks = Chunk::update(
                    &next.chunks,
                    &next.original,
                    update.start().text(),
                    Side::A,
                    &edit.changes,
                    self.config,
                )
                .into();
            }
        }
        if !update.changes().is_empty() {
            next.chunks = Chunk::update(
                &next.chunks,
                &next.original,
                update.state().text(),
                Side::B,
                update.changes(),
                self.config,
            )
            .into();
        }
        next.update_collapsed_ranges(self, update);
        Ok(next)
    }

    fn update_collapsed_ranges(&mut self, previous: &Self, update: &StateUpdate<'_>) {
        let expanded = update
            .transaction()
            .effects()
            .iter()
            .filter_map(|effect| effect.get(&EXPAND_UNCHANGED));
        let chunks_changed = !Arc::ptr_eq(&self.chunks, &previous.chunks);
        if self.collapsed.is_empty() || (!chunks_changed && expanded.clone().next().is_none()) {
            return;
        }
        let changes = (!update.changes().is_empty()).then_some(update.changes());
        self.collapsed = self
            .collapsed
            .iter()
            .filter_map(|range| {
                if chunks_changed {
                    update_collapsed_range(
                        update.state().text(),
                        &self.chunks,
                        Side::B,
                        range,
                        changes,
                    )
                } else {
                    Some(range.clone())
                }
            })
            .filter(|range| !expanded.clone().any(|&at| at == range.start))
            .collect();
    }
}

static EXPAND_UNCHANGED: LazyLock<StateEffectType<usize>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|at, changes| {
        changes.map_pos(*at, Assoc::Before, MapMode::Simple)
    })
});

struct OriginalEdit {
    before: Text,
    after: Text,
    changes: ChangeDesc,
}

static UPDATE_ORIGINAL: LazyLock<StateEffectType<OriginalEdit>> =
    LazyLock::new(StateEffectType::new);

static UNIFIED_MERGE: LazyLock<StateField<UnifiedMerge>> = LazyLock::new(|| {
    StateField::new(
        |_| Err(ConfigurationError::MissingField),
        UnifiedMerge::update,
    )
});

/// Install unified merge state. The extension does not render a view.
/// Chunks update from the actual document changes after transaction filtering.
/// Installing a new extension rebuilds the collapsed ranges. Edits can remove
/// ranges but do not add them back, and keeping the same extension preserves expansions.
pub fn unified_merge_view(config: UnifiedMergeConfig) -> Extension {
    UNIFIED_MERGE.init(move |state| {
        let chunks: Arc<_> =
            Chunk::build_with_config(&config.original, state.text(), config.diff_config).into();
        let collapsed = config
            .collapse_unchanged
            .map_or_else(Arc::default, |collapse| {
                build_collapsed_ranges(state.text(), &chunks, Side::B, collapse).into()
            });
        Ok(UnifiedMerge {
            original: config.original.clone(),
            chunks,
            config: config.diff_config,
            merge_controls: config.merge_controls,
            collapsed,
        })
    })
}

/// Read the unified merge snapshot, or `None` if the extension is absent.
pub fn unified_merge_state(
    state: &EditorState,
) -> Result<Option<Arc<UnifiedMerge>>, ConfigurationError> {
    state.context().field(&UNIFIED_MERGE)
}

/// Changed chunks and the editor's side in the comparison.
pub type MergeChunks = (Arc<[Chunk]>, Side);

/// Return the changed chunks and the editor's side, if configured.
/// The editor is always side B in a unified merge.
pub fn get_chunks(state: &EditorState) -> Result<Option<MergeChunks>, ConfigurationError> {
    if let Some(chunks) = super::side_by_side::get_chunks(state)? {
        return Ok(Some(chunks));
    }
    Ok(unified_merge_state(state)?.map(|merge| (merge.chunks.clone(), Side::B)))
}

/// Return the original document, if configured.
pub fn get_original_doc(state: &EditorState) -> Result<Option<Text>, ConfigurationError> {
    Ok(unified_merge_state(state)?.map(|merge| merge.original.clone()))
}

/// Prepare a change to the original document for a unified merge transaction.
/// The changes use the current original document's byte coordinates, independent
/// of any changes to the editor document. If a transaction has more than one
/// original document effect, the last one supplies both the document and its
/// change description. Upstream 6.13.0 can use different effects for those
/// values. If the original text no longer matches, the last effect rejects the
/// transaction.
pub fn original_doc_change_effect(
    state: &EditorState,
    mut changes: ChangeSet,
) -> Result<StateEffect, EditError> {
    let before = get_original_doc(state)?.ok_or(ConfigurationError::MissingField)?;
    changes.check(&before)?;
    changes.remove_noops(&before);
    let description = changes.desc(before.len())?;
    let mut after = before.clone();
    changes.apply(&mut after);
    Ok(UPDATE_ORIGINAL.of(OriginalEdit {
        before,
        after,
        changes: description,
    }))
}
