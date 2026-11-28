//! Collaboration with a central authority.
//!
//! The update transforms are adapted from `@codemirror/collab` 6.1.1. See
//! `docs/licenses/codemirror-collab.txt` for the upstream MIT notice.
//!
//! # Authority batches and recovery
//!
//! The pinned upstream (`collab` 6.1.1 with `state` 6.7.6) is not invariant to
//! batch boundaries. For example, start with `abc` and queue a local insertion
//! of `X` at 0. Accept `b` inserting `X` at 0, then `c` replacing `0..2` with
//! `YY`, in that order. Both authority paths end at `YYbc` before the echo:
//!
//! - Rebasing the stale submission over `[b, c]` in one call inserts `X` at 0
//!   (`XYYbc`). A client that receives `[b, c]` in one call agrees.
//! - Rebasing over `[b]` and then `[c]` inserts `X` at 2 (`YYXbc`). A client
//!   that receives the same two batches agrees.
//!
//! Upstream acknowledges by client ID alone. A sequential client that receives
//! the first echo advances to version 3 and clears its queue, but stays at
//! `YYXbc`. The reverse mismatch also clears the queue and leaves the wrong
//! text. Upstream also accepts an echo with the right range but different
//! bytes: with `abc` and a local insertion of `!` at 3, it accepts `?` at 3
//! and keeps `abc!` while the authority has `abc?`.
//!
//! Rust rejects an echo that does not match the pending edit on the client's
//! path. Equal resulting text alone is not enough: different edit
//! positions can change the mapping of selections, effects, and undo events.
//! For example, if `c` inserts `X` rather than `YY` in the example above,
//! both paths produce `XXbc`, but the echo positions are 0 and 1.
//!
//! Batch boundaries can also change shared effects when the edit geometry
//! agrees. For example, queue only a mark at 1 in `abc` that tracks deletion
//! with [`crate::MapMode::TrackDel`]. If `b` and `c` each delete `0..1`, the
//! mark stays at 0 after two separate receipts but is dropped by a single
//! receipt. Both paths have an empty pending edit against `c`. Rust checks
//! echoed edits, not shared effect values. The host must preserve the effect
//! policy and cannot use a successful text acknowledgement to verify effects.
//!
//! A host can avoid this conflict by accepting submissions only at their exact
//! base version. If a stale submission was not accepted, the host can pull and
//! submit the client's current rebased queue at its new version. When rebasing
//! a stale submission, match the client's batch boundaries from that base.
//! If an incompatible echo was already accepted, do not discard the failed
//! receipt or blindly resubmit. Recover from a matching checkpoint and replay
//! the authority batches, or reload authority text and version and explicitly
//! reconcile unresolved local edits. The host must track which submissions the
//! authority accepted. A snapshot reload does not preserve native undo history.
//!
//! # Native API differences
//!
//! Rust uses UTF-8 byte coordinates and a checked `usize` authority version.
//! `receive_updates` requires the batch's starting version. Hosts must supply
//! the authority, serialize updates, and decode shared effects into local types.
//! A description-only rebase can check edit geometry but not inserted text.
//!
//! Collaboration keeps the geometry of same-text replacements for local
//! publication, rebasing, and selection mapping. This includes edits that
//! install collaboration in the same transaction. Ordinary edits without
//! collaboration still discard replacements that do not change the text.
//!
//! Wire changes can contain separate deletions that share a boundary. A receipt
//! preserves that boundary for effect mapping when no nonidentity transform has
//! normalized the changes. Full authority rebase can normalize those deletions
//! even over an effect-only local update, unlike description-only rebase.
//! Composition uses the individual wire sections as input. Joining them before
//! composition can change where a pending edit lands. An echo must match
//! individual deletion boundaries, not just the resulting text.
//!
//! Even when both sides process the same batch, a concrete authority rebase
//! can produce a different echo from a description-only rebase. For example,
//! start with `a` and queue an effect-only update, then insert `!` at 0. In
//! one batch, a peer inserts `X` and `B` in separate wire sections at 0,
//! the authority acknowledges the effect-only update, and another peer
//! inserts `Z` after `X` and deletes `Ba`. The concrete rebase inserts `!`
//! before `XZ`; the description-only rebase and the client put it after `XZ`.
//! Upstream clears the queue for either echo. Rust rejects the concrete echo
//! without changing the document, version, or queue, whether the echo arrives
//! in that batch or in a separate receipt after the prefix. The compatible
//! description-only echo leaves both sides at `XZ!`.
//!
//! Inbound effects map through the composition of the remaining pending edits.
//! For example, with two pending front deletions in `abc`, a confirmed
//! `TrackDel` mark at 1 is dropped. Mapping the mark through each deletion
//! separately would keep it at 0.

use std::{borrow::Cow, fmt, ops::Range, sync::Arc};

use crate::{
    ChangeDesc, ChangeSet, EditError, EditorState, StateContext, StateEffect, Text, Transaction,
};

mod extension;
mod origin;
pub use extension::{
    CollabConfig, collab, collab_state, get_client_id, get_synced_version,
    prepare_text_update_group, receive_updates, sendable_updates,
};
pub(crate) use extension::{capture_origin_states, validate_receipt};
pub use origin::{OriginCollabState, TransactionContext};
use origin::{PendingHistory, QueueSnapshot};

/// Whether a transaction is marked as coming from another source.
pub fn is_remote(transaction: &Transaction) -> bool {
    transaction.is_remote()
}

/// The transaction and both state views while selecting shared effects.
/// The result view includes computed fields, facets, and the result client
/// ID. It cannot read the result queue or version while that queue is being built.
pub struct SharedEffectsContext<'a> {
    transaction: &'a TransactionContext,
    start: StateContext<'a>,
    result: StateContext<'a>,
    start_collab: &'a Collab,
    result_client_id: &'a str,
}

impl SharedEffectsContext<'_> {
    pub fn transaction(&self) -> &Transaction {
        self.transaction.transaction()
    }

    pub fn start(&self) -> &StateContext<'_> {
        &self.start
    }

    pub fn state(&self) -> &StateContext<'_> {
        &self.result
    }

    /// Read the queue, client ID, and version before the transaction.
    pub fn start_collab(&self) -> &Collab {
        self.start_collab
    }

    /// Read the configured client ID after the transaction, including reconfiguration.
    pub fn result_client_id(&self) -> &str {
        self.result_client_id
    }
}

/// An update measured against a document of a known byte length. Equality
/// compares client ID and edit contents, and uses identity for effect values.
#[derive(Clone, Debug)]
pub struct Update {
    client_id: String,
    edit: VersionedChange,
    effects: Vec<StateEffect>,
    origin: Option<Arc<TransactionContext>>,
}

impl PartialEq for Update {
    fn eq(&self, other: &Self) -> bool {
        self.client_id == other.client_id
            && self.edit == other.edit
            && self.effects.len() == other.effects.len()
            && self
                .effects
                .iter()
                .zip(&other.effects)
                .all(|(left, right)| left.same_value(right))
    }
}

impl Eq for Update {}

impl Update {
    pub fn new(
        client_id: impl Into<String>,
        changes: ChangeSet,
        old_length: usize,
    ) -> Result<Self, EditError> {
        Ok(Self {
            client_id: client_id.into(),
            edit: VersionedChange::new(changes, old_length)?,
            effects: Vec::new(),
            origin: None,
        })
    }

    /// Supply decoded shared effects from a trusted peer or authority. Effects
    /// describe positions after this update's changes. Local policy and transport
    /// must decide which effect types can be shared.
    pub fn with_effects(mut self, effects: impl IntoIterator<Item = StateEffect>) -> Self {
        self.effects.extend(effects);
        self
    }

    pub fn effects(&self) -> &[StateEffect] {
        &self.effects
    }

    /// The accepted local transaction for an update from the local queue.
    /// Its positions are unchanged by receipt rebasing. Updates mapped by an
    /// authority rebase have no origin.
    pub fn origin(&self) -> Option<&Transaction> {
        self.origin_context().map(TransactionContext::transaction)
    }

    /// The original local transaction and snapshots, unchanged by receipt rebasing.
    /// Updates created by [`Update::new`] do not have an origin.
    pub fn origin_context(&self) -> Option<&TransactionContext> {
        self.origin.as_deref()
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn changes(&self) -> &ChangeSet {
        &self.edit.changes
    }

    /// The change geometry, including individual boundaries in wire edits.
    pub fn change_desc(&self) -> &ChangeDesc {
        &self.edit.desc
    }

    /// Check the old length and UTF-8 boundaries before applying this update.
    pub fn apply_to(&self, text: &mut Text) -> Result<(), EditError> {
        self.edit.check_length(text.len())?;
        self.edit.changes.apply_to(text)
    }

    fn with_mapped_edit(
        &self,
        edit: VersionedChange,
        through: &ChangeDesc,
        keep_origin: bool,
    ) -> Self {
        Self {
            client_id: self.client_id.clone(),
            edit,
            effects: StateEffect::map_all(self.effects(), through),
            origin: if keep_origin {
                self.origin.clone()
            } else {
                None
            },
        }
    }
}

/// An invalid collaboration operation. Errors do not change the session or editor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollabError {
    Edit(EditError),
    UnexpectedVersion { expected: usize, received: usize },
    VersionOverflow,
    UnexpectedAcknowledgement,
    MismatchedAcknowledgement,
    StateMismatch,
    SharedEffectsInTextGroup,
    NotConfigured,
}

impl From<EditError> for CollabError {
    fn from(error: EditError) -> Self {
        Self::Edit(error)
    }
}

/// A client session for one editor and one authority.
///
/// Give each active client a distinct ID. Apply all document edits to the editor
/// through the session. Direct selection-only transactions are permitted.
/// Use [`collab`] instead to include local edits and undo with an editor's state.
#[derive(Clone)]
pub struct Collab {
    client_id: String,
    version: usize,
    synced_text: Text,
    optimistic_text: Text,
    pending: Arc<Vec<Update>>,
    pending_history: PendingHistory,
    share: Option<Arc<SharedEffects>>,
    identity: Arc<()>,
    generation: Arc<()>,
}

type SharedEffects = dyn Fn(&SharedEffectsContext<'_>) -> Vec<StateEffect> + Send + Sync;

impl fmt::Debug for Collab {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Collab")
            .field("client_id", &self.client_id)
            .field("version", &self.version)
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

impl Collab {
    /// Start at an authority version with its text already loaded in `state`.
    pub fn new(client_id: impl Into<String>, start_version: usize, state: &EditorState) -> Self {
        Self::from_text(client_id.into(), start_version, state.text())
    }

    fn from_text(client_id: String, start_version: usize, text: &Text) -> Self {
        Self {
            client_id,
            version: start_version,
            synced_text: text.clone(),
            optimistic_text: text.clone(),
            pending: Arc::new(Vec::new()),
            pending_history: PendingHistory::default(),
            share: None,
            identity: Arc::new(()),
            generation: Arc::new(()),
        }
    }

    /// Select effects to send from each accepted local transaction. By default
    /// the session shares only document changes. Unselected effects stay local.
    pub fn with_shared_effects(
        self,
        share: impl Fn(&Transaction) -> Vec<StateEffect> + Send + Sync + 'static,
    ) -> Self {
        self.with_shared_effects_context(move |context| share(context.transaction()))
    }

    /// Select effects using the accepted transaction and its text and selection
    /// snapshots. This replaces any previously configured sharing callback.
    pub fn with_shared_effects_context(
        self,
        share: impl Fn(&TransactionContext) -> Vec<StateEffect> + Send + Sync + 'static,
    ) -> Self {
        self.with_shared_effects_state(move |context| share(context.transaction))
    }

    /// Select effects using both state views of the accepted transaction.
    /// The queued update keeps the resolved fields and facets available to the host.
    pub fn with_shared_effects_state(
        mut self,
        share: impl Fn(&SharedEffectsContext<'_>) -> Vec<StateEffect> + Send + Sync + 'static,
    ) -> Self {
        self.share = Some(Arc::new(share));
        self
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn synced_version(&self) -> usize {
        self.version
    }

    /// Updates in order from the synced version. Reading them does not acknowledge them.
    pub fn sendable_updates(&self) -> &[Update] {
        &self.pending
    }

    /// Apply a local transaction and queue its accepted changes or shared effects.
    /// Selection-only transactions with no shared effects are not queued.
    pub fn apply_local(
        &mut self,
        state: &mut EditorState,
        transaction: Transaction,
    ) -> Result<(), CollabError> {
        self.check_state(state.text())?;
        let mut accepted = state.filter_transaction(transaction)?;
        let mut candidate = state.clone();
        let edit = candidate.apply_with_replacement_policy(accepted.clone(), true)?;
        accepted.changes = edit.map_or_else(ChangeSet::default, |edit| edit.forward);
        let before = QueueSnapshot::new(self, None);
        if let Some(origin) = self.record(accepted, state.context(), candidate.context(), None)? {
            origin.capture(
                extension::origin_values(&state.extensions),
                extension::origin_values(&candidate.extensions),
                Some(before),
                Some(QueueSnapshot::new(self, Some(&origin))),
            );
        }
        *state = candidate;
        Ok(())
    }

    fn record(
        &mut self,
        transaction: Transaction,
        start: StateContext<'_>,
        state: StateContext<'_>,
        result_client_id: Option<&str>,
    ) -> Result<Option<Arc<TransactionContext>>, EditError> {
        let mut recorded = None;
        if !transaction.changes().is_empty() || self.share.is_some() {
            let origin = Arc::new(TransactionContext::new(transaction, start, state));
            let effects = self.share.as_ref().map_or_else(Vec::new, |share| {
                share(&SharedEffectsContext {
                    transaction: &origin,
                    start,
                    result: state,
                    start_collab: self,
                    result_client_id: result_client_id.unwrap_or(&self.client_id),
                })
            });
            let changes = origin.transaction().changes();
            if !changes.is_empty() || !effects.is_empty() {
                let mut update =
                    Update::new(&self.client_id, changes.clone(), self.optimistic_text.len())?
                        .with_effects(effects);
                update.origin = Some(origin.clone());
                self.pending_history.append(update.clone());
                Arc::make_mut(&mut self.pending).push(update);
                self.generation = Arc::new(());
                recorded = Some(origin);
            }
        }
        self.optimistic_text = state.text().clone();
        Ok(recorded)
    }

    fn replace_pending(&mut self, pending: Vec<Update>) {
        self.pending = Arc::new(pending);
        self.pending_history = PendingHistory::base(&self.pending);
    }

    /// Apply contiguous authoritative updates, acknowledge matching local updates,
    /// and rebase any remaining local updates. The first update must start at
    /// `from_version`, which must equal the current synced version.
    ///
    /// Edits echoed for this client must match the next pending edit after mapping over
    /// earlier changes in the batch. If an authority and a client mapped the same
    /// edit through different batches and got different results, this method returns
    /// [`CollabError::MismatchedAcknowledgement`] instead of silently diverging.
    pub fn receive_updates(
        &mut self,
        state: &mut EditorState,
        from_version: usize,
        updates: &[Update],
    ) -> Result<(), CollabError> {
        if let Some(receipt) = self.prepare(state.text(), from_version, updates)? {
            if let Some(transaction) = receipt.transaction {
                let accepted = state.filter_transaction(transaction)?;
                state.apply_with_replacement_policy(accepted, true)?;
            }
            *self = receipt.next;
        }
        Ok(())
    }

    fn prepare(
        &self,
        text: &Text,
        from_version: usize,
        updates: &[Update],
    ) -> Result<Option<Receipt>, CollabError> {
        if from_version != self.version {
            return Err(CollabError::UnexpectedVersion {
                expected: self.version,
                received: from_version,
            });
        }
        self.check_state(text)?;
        if updates.is_empty() {
            return Ok(None);
        }
        let version = self
            .version
            .checked_add(updates.len())
            .ok_or(CollabError::VersionOverflow)?;
        let mut synced_text = self.synced_text.clone();
        for update in updates {
            update.apply_to(&mut synced_text)?;
        }
        let Reconciled {
            pending,
            remote,
            effects,
        } = reconcile(&self.pending, updates, Some(&self.client_id))?;
        let mut next = self.clone();
        next.version = version;
        next.synced_text = synced_text;
        next.replace_pending(pending);
        next.generation = Arc::new(());
        let transaction = if let Some(remote) = remote {
            remote.check_length(text.len())?;
            remote.changes.apply_to(&mut next.optimistic_text)?;
            Some(
                Transaction::new(remote.changes)
                    .with_effects(effects)
                    .with_remote(true)
                    .with_add_to_history(false)
                    .without_filtering(),
            )
        } else {
            None
        };
        Ok(Some(Receipt { next, transaction }))
    }

    fn check_state(&self, text: &Text) -> Result<(), CollabError> {
        if text != &self.optimistic_text {
            return Err(CollabError::StateMismatch);
        }
        Ok(())
    }
}

struct Receipt {
    next: Collab,
    transaction: Option<Transaction>,
}

/// Rebase submitted updates over the authority's accepted suffix since the
/// submission's base version. Drop matching updates already in that suffix.
/// The caller must select the correct suffix, in authority order. Both lists
/// must start at the same document and use consecutive update coordinates.
/// Call this once per batch if the client received that suffix in separate
/// batches. Each result is the submission for the start of the next batch.
///
/// This function checks lengths and echoes, but does not have the document text.
/// Use [`Update::apply_to`] to validate returned changes against the current text.
pub fn rebase_updates(updates: &[Update], over: &[Update]) -> Result<Vec<Update>, CollabError> {
    if let Some(first) = updates.first().or_else(|| over.first()) {
        let length = first.change_desc().length();
        check_chain(updates, length)?;
        check_chain(over, length)?;
    }
    if updates.is_empty() || over.is_empty() {
        return Ok(updates.to_vec());
    }
    Ok(reconcile(updates, over, None)?.pending)
}

/// The client and shape of one update already accepted by the authority.
#[derive(Clone, Copy, Debug)]
pub struct UpdateDescription<'a> {
    pub client_id: &'a str,
    pub changes: &'a ChangeDesc,
}

/// Rebase a submission when the authority retained only change descriptions
/// in its history. Acknowledgements must match the mapped geometry. Splits
/// in unchanged spans and insertions at the same position are permitted.
/// Inserted bytes cannot be checked without the full updates.
pub fn rebase_updates_described(
    mut pending: &[Update],
    over: &[UpdateDescription<'_>],
) -> Result<Vec<Update>, CollabError> {
    let Some(first) = pending
        .first()
        .map(Update::change_desc)
        .or_else(|| over.first().map(|item| item.changes))
    else {
        return Ok(Vec::new());
    };
    let mut length = first.length();
    check_chain(pending, length)?;
    let mut remote: Option<ChangeDesc> = None;
    for update in over {
        if update.changes.length() != length {
            return Err(EditError::MismatchedLength.into());
        }
        length = update.changes.new_length();
        if let Some((next, rest)) = pending.split_first()
            && next.client_id() == update.client_id
        {
            let expected = match &remote {
                Some(through) if !through.is_empty() => {
                    Cow::Owned(next.edit.desc.map(through, false)?)
                }
                _ => Cow::Borrowed(next.change_desc()),
            };
            if !matches_echo_geometry(&expected, update.changes) {
                return Err(CollabError::MismatchedAcknowledgement);
            }
            if let Some(through) = &mut remote {
                *through = through.map(&next.edit.desc, true)?;
            }
            pending = rest;
        } else {
            remote = Some(match remote {
                Some(previous) => previous.compose(update.changes)?,
                None => update.changes.clone(),
            });
        }
    }
    let Some(mut remote) = remote else {
        return Ok(pending.to_vec());
    };
    let mut rebased = Vec::with_capacity(pending.len());
    for update in pending {
        let edit = update.edit.map(&remote, false)?;
        remote = remote.map(&update.edit.desc, true)?;
        rebased.push(update.with_mapped_edit(edit, &remote, false));
    }
    Ok(rebased)
}

fn matches_echo_geometry(expected: &ChangeDesc, echo: &ChangeDesc) -> bool {
    expected == echo
        || expected.length() == echo.length()
            && expected.new_length() == echo.new_length()
            && echo_ranges(expected).eq(echo_ranges(echo))
}

fn echo_ranges(desc: &ChangeDesc) -> impl Iterator<Item = (Range<usize>, Range<usize>)> + '_ {
    // Ignore splits in gaps and same-position insertions, but not splits in
    // replacements or deletions. Those boundaries can affect position mapping.
    let mut ranges = desc.changed_ranges().peekable();
    std::iter::from_fn(move || {
        let (old, mut new) = ranges.next()?;
        if old.is_empty() {
            while let Some((_, part)) = ranges.next_if(|(range, _)| range == &old) {
                new.end = part.end;
            }
        }
        Some((old, new))
    })
}

fn check_chain(updates: &[Update], mut length: usize) -> Result<(), EditError> {
    for update in updates {
        update.edit.check_length(length)?;
        length = update.change_desc().new_length();
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct VersionedChange {
    changes: ChangeSet,
    desc: ChangeDesc,
}

impl PartialEq for VersionedChange {
    fn eq(&self, other: &Self) -> bool {
        self.desc == other.desc && self.changes.changes().eq(other.changes.changes())
    }
}

impl Eq for VersionedChange {}

impl VersionedChange {
    fn new(changes: ChangeSet, old_length: usize) -> Result<Self, EditError> {
        let desc = changes.desc(old_length)?;
        Ok(Self { changes, desc })
    }

    fn check_length(&self, old_length: usize) -> Result<(), EditError> {
        if self.desc.length() != old_length {
            return Err(EditError::MismatchedLength);
        }
        Ok(())
    }

    fn map(&self, over: &ChangeDesc, before: bool) -> Result<Self, EditError> {
        self.check_length(over.length())?;
        if over.is_empty() {
            return Ok(self.clone());
        }
        Self::new(self.changes.map_desc(over, before)?, over.new_length())
    }

    fn map_over_local(&mut self, local: &Self, receiving: bool) -> Result<(), EditError> {
        if receiving {
            *self = self.map(&local.desc, true)?;
        } else {
            // Full authority rebase runs the transform even over an identity.
            self.check_length(local.desc.length())?;
            *self = Self::new(
                self.changes.map_desc(&local.desc, true)?,
                local.desc.new_length(),
            )?;
        }
        Ok(())
    }

    fn compose(&self, after: &Self) -> Result<Self, EditError> {
        after.check_length(self.desc.new_length())?;
        let length = self.desc.length();
        Self::new(self.changes.compose(&after.changes, length)?, length)
    }

    fn matches_echo(&self, echo: &Self) -> bool {
        fn inserted_bytes(changes: &ChangeSet) -> impl Iterator<Item = u8> + '_ {
            changes.changes().flat_map(|(_, text)| text.bytes())
        }
        matches_echo_geometry(&self.desc, &echo.desc)
            && inserted_bytes(&self.changes).eq(inserted_bytes(&echo.changes))
    }
}

struct Reconciled {
    pending: Vec<Update>,
    remote: Option<VersionedChange>,
    effects: Vec<StateEffect>,
}

fn reconcile(
    mut pending: &[Update],
    accepted: &[Update],
    own_id: Option<&str>,
) -> Result<Reconciled, CollabError> {
    let receiving = own_id.is_some();
    let mut remote: Option<VersionedChange> = None;
    let mut effects = Vec::new();
    for update in accepted {
        // Upstream maps shared effects even across effect-only updates.
        if !effects.is_empty() {
            effects = StateEffect::map_all(&effects, update.change_desc());
        }
        if let Some((next, rest)) = pending.split_first()
            && next.client_id == update.client_id
        {
            let matches = match &remote {
                Some(changes) => next
                    .edit
                    .map(&changes.desc, false)?
                    .matches_echo(&update.edit),
                None => next.edit.matches_echo(&update.edit),
            };
            if !matches {
                return Err(CollabError::MismatchedAcknowledgement);
            }
            if let Some(changes) = &mut remote {
                changes.map_over_local(&next.edit, receiving)?;
            }
            pending = rest;
        } else if own_id == Some(update.client_id()) {
            return Err(CollabError::UnexpectedAcknowledgement);
        } else {
            if receiving {
                effects.extend_from_slice(update.effects());
            }
            remote = Some(match remote {
                Some(changes) => changes.compose(&update.edit)?,
                None => update.edit.clone(),
            });
        }
    }

    let mut rebased = Vec::with_capacity(pending.len());
    for update in pending {
        if let Some(changes) = &mut remote {
            let edit = update.edit.map(&changes.desc, false)?;
            changes.map_over_local(&update.edit, receiving)?;
            let mut mapped = update.with_mapped_edit(edit, &changes.desc, receiving);
            if let Some(client_id) = own_id {
                // A receive uses the current configuration for mapped entries.
                client_id.clone_into(&mut mapped.client_id);
            }
            rebased.push(mapped);
        } else {
            rebased.push(update.clone());
        }
    }
    if !effects.is_empty()
        && let Some(first) = rebased.first()
    {
        let mut mapped = Cow::Borrowed(first.change_desc());
        for next in &rebased[1..] {
            mapped = Cow::Owned(mapped.compose(next.change_desc())?);
        }
        effects = StateEffect::map_all(&effects, &mapped);
    }
    Ok(Reconciled {
        pending: rebased,
        remote,
        effects,
    })
}
