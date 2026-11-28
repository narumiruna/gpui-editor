use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};

use crate::{
    Change, ChangeDesc, ChangeSet, ConfigurationError, EditError, EditorState, StateEffectType,
    StateField, Transaction,
};

use super::{
    Chunk, CollapseConfig, DiffConfig, DiffOverride, MergeChunks, MergeProjection, SessionDiff,
    Side,
};

/// Settings for a headless side-by-side merge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MergeViewConfig {
    /// First column in display order. This does not change the document identities.
    pub first: Side,
    /// Destination for revert controls. `None` disables the controls.
    pub revert_to: Option<Side>,
    pub collapse_unchanged: Option<CollapseConfig>,
    pub diff_config: DiffConfig,
}

impl Default for MergeViewConfig {
    fn default() -> Self {
        Self {
            first: Side::A,
            revert_to: None,
            collapse_unchanged: None,
            diff_config: DiffConfig::default(),
        }
    }
}

/// A revert button in the current merge snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevertControl {
    /// Index in the current chunk list. Read the controls again after an edit.
    pub chunk: usize,
    /// Destination of the copy. The other side is the source.
    pub destination: Side,
    /// UTF-8 byte positions at the start of the chunk in A and B.
    pub positions: [usize; 2],
}

/// Two editable states with shared chunks and a fixed-height projection.
///
/// Use `dispatch` to edit either side. There is no DOM, viewport, or built-in
/// undo history. Hidden lines do not block direct transactions, as in upstream.
#[derive(Clone, Debug)]
pub struct MergeView {
    a: EditorState,
    b: EditorState,
    chunks: Arc<[Chunk]>,
    projection: MergeProjection,
    config: MergeViewConfig,
    diff: SessionDiff,
}

impl MergeView {
    /// Use two editor states with any desired extensions already installed.
    pub fn new(a: EditorState, b: EditorState, config: MergeViewConfig) -> Result<Self, EditError> {
        Self::with_diff(a, b, config, SessionDiff::default())
    }

    /// Use an advisory time limit for the initial diff and each rebuilt window.
    /// A zero duration disables the timeout.
    pub fn new_with_timeout(
        a: EditorState,
        b: EditorState,
        config: MergeViewConfig,
        timeout: Duration,
    ) -> Result<Self, EditError> {
        Self::with_diff(
            a,
            b,
            config,
            SessionDiff {
                timeout,
                ..SessionDiff::default()
            },
        )
    }

    /// Use the checked override for the initial chunks and subsequent edits.
    /// Invalid ranges return [`ConfigurationError::InvalidFieldData`] wrapped in
    /// [`EditError::Configuration`].
    pub fn new_with_override(
        a: EditorState,
        b: EditorState,
        config: MergeViewConfig,
        diff_override: DiffOverride,
    ) -> Result<Self, EditError> {
        Self::with_diff(
            a,
            b,
            config,
            SessionDiff {
                diff_override: Some(diff_override),
                ..SessionDiff::default()
            },
        )
    }

    fn with_diff(
        mut a: EditorState,
        mut b: EditorState,
        config: MergeViewConfig,
        diff: SessionDiff,
    ) -> Result<Self, EditError> {
        let chunks = Chunk::build_for_session(a.text(), b.text(), config.diff_config, &diff)?;
        sync_chunks(&mut a, &chunks, Side::A)?;
        sync_chunks(&mut b, &chunks, Side::B)?;
        let projection =
            MergeProjection::new(a.text(), b.text(), &chunks, config.collapse_unchanged);
        Ok(Self {
            a,
            b,
            chunks,
            projection,
            config,
            diff,
        })
    }

    pub fn state(&self, side: Side) -> &EditorState {
        match side {
            Side::A => &self.a,
            Side::B => &self.b,
        }
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    pub fn projection(&self) -> &MergeProjection {
        &self.projection
    }

    pub fn config(&self) -> MergeViewConfig {
        self.config
    }

    /// Apply settings. A changed collapse setting rebuilds the hidden ranges.
    /// A changed diff setting takes effect on the next document edit when no
    /// override is active. Use [`Self::reconfigure_collapse`] to replace an equal
    /// collapse setting.
    pub fn reconfigure(&mut self, config: MergeViewConfig) {
        if self.config.collapse_unchanged != config.collapse_unchanged {
            self.reconfigure_collapse(config.collapse_unchanged);
        }
        self.config = config;
    }

    /// Replace the override without changing the states, chunks, or projection.
    /// On the next accepted document edit, use this override for the rebuilt
    /// windows. `None` restores the configured native diff.
    pub fn reconfigure_diff_override(&mut self, diff_override: Option<DiffOverride>) {
        self.diff.diff_override = diff_override;
    }

    /// Set the advisory timeout for rebuilt windows on later edits. This does
    /// not rebuild existing chunks, and an installed override ignores it.
    pub fn reconfigure_diff_timeout(&mut self, timeout: Duration) {
        self.diff.timeout = timeout;
    }

    /// Replace the collapse setting and rebuild the hidden ranges, even when
    /// the new setting equals the old one. This discards prior expansions but
    /// preserves both editor states and the current chunks.
    pub fn reconfigure_collapse(&mut self, config: Option<CollapseConfig>) {
        self.projection = MergeProjection::new(self.a.text(), self.b.text(), &self.chunks, config);
        self.config.collapse_unchanged = config;
    }

    /// Expand a hidden range and its peer at the current byte position.
    pub fn expand(&mut self, side: Side, byte_start: usize) -> bool {
        self.projection.expand(side, byte_start)
    }

    pub fn dispatch(&mut self, side: Side, transaction: Transaction) -> Result<(), EditError> {
        self.dispatch_batch(side, [transaction])
    }

    /// Apply transactions for one side in order. Each transaction uses the state
    /// left by the previous one. Update chunks once from the accepted changes.
    /// An error leaves both states, chunks, and projection unchanged.
    pub fn dispatch_batch(
        &mut self,
        side: Side,
        transactions: impl IntoIterator<Item = Transaction>,
    ) -> Result<(), EditError> {
        let mut target = self.state(side).clone();
        let mut changes: Option<ChangeDesc> = None;
        for transaction in transactions {
            let length = target.text().len();
            if let Some(edit) = target.apply(transaction)? {
                let next = edit.forward.desc(length)?;
                changes = Some(match changes {
                    Some(previous) => previous.compose(&next)?,
                    None => next,
                });
            }
        }
        let mut peer = self.state(side.other()).clone();
        let mut chunks = self.chunks.clone();
        let projection = if let Some(changes) = changes.filter(|changes| !changes.is_empty()) {
            let (a, b) = match side {
                Side::A => (target.text(), peer.text()),
                Side::B => (peer.text(), target.text()),
            };
            chunks = Chunk::update_for_session(
                &chunks,
                a,
                b,
                side,
                &changes,
                self.config.diff_config,
                &self.diff,
            )?;
            Some(self.projection.update(a, b, &chunks, side, &changes))
        } else {
            None
        };
        sync_chunks(&mut target, &chunks, side)?;
        sync_chunks(&mut peer, &chunks, side.other())?;
        (self.a, self.b) = match side {
            Side::A => (target, peer),
            Side::B => (peer, target),
        };
        self.chunks = chunks;
        if let Some(projection) = projection {
            self.projection = projection;
        }
        Ok(())
    }

    /// List one control per chunk when enabled, independent of the viewport.
    pub fn revert_controls(&self) -> impl Iterator<Item = RevertControl> + '_ {
        self.config.revert_to.into_iter().flat_map(|destination| {
            self.chunks
                .iter()
                .enumerate()
                .map(move |(index, chunk)| RevertControl {
                    chunk: index,
                    destination,
                    positions: [chunk.range(Side::A).start, chunk.range(Side::B).start],
                })
        })
    }

    /// Activate the revert control for the current chunk index. Return `false`
    /// if no such control exists. The destination filters can reject the edit.
    pub fn revert(&mut self, index: usize) -> Result<bool, EditError> {
        let (Some(destination), Some(chunk)) = (self.config.revert_to, self.chunks.get(index))
        else {
            return Ok(false);
        };
        let source = destination.other();
        let insert = self
            .state(source)
            .text()
            .slice_string(chunk.range(source))
            .ok_or(EditError::InvalidRange)?;
        let changes = ChangeSet::single(Change::new(chunk.range(destination), insert))?;
        self.dispatch(
            destination,
            Transaction::new(changes).with_user_event("revert"),
        )?;
        Ok(true)
    }
}

static SET_CHUNKS: LazyLock<StateEffectType<MergeChunks>> = LazyLock::new(StateEffectType::new);

static SIDE_BY_SIDE_CHUNKS: LazyLock<StateField<MergeChunks>> = LazyLock::new(|| {
    StateField::new(
        |_| Err(ConfigurationError::MissingField),
        |old, update| {
            Ok(update
                .transaction()
                .effects()
                .iter()
                .find_map(|effect| effect.get(&SET_CHUNKS))
                .unwrap_or(old)
                .clone())
        },
    )
});

pub(super) fn get_chunks(state: &EditorState) -> Result<Option<MergeChunks>, ConfigurationError> {
    Ok(state
        .context()
        .field(&SIDE_BY_SIDE_CHUNKS)?
        .map(|value| (*value).clone()))
}

fn sync_chunks(
    state: &mut EditorState,
    chunks: &Arc<[Chunk]>,
    side: Side,
) -> Result<(), EditError> {
    let value = (chunks.clone(), side);
    let effect = match get_chunks(state)? {
        Some((current, current_side)) if current_side == side && Arc::ptr_eq(&current, chunks) => {
            return Ok(());
        }
        Some(_) => SET_CHUNKS.of(value),
        None => SIDE_BY_SIDE_CHUNKS
            .init(move |_| Ok(value.clone()))
            .append(),
    };
    state.update(
        Transaction::default()
            .with_effects([effect])
            .without_filtering(),
    )
}
