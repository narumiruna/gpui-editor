use std::{
    fmt,
    sync::{Arc, OnceLock, Weak},
};

use crate::{EditorSelection, StateContext, Text, Transaction, extension::ResolvedValues};

use super::{Collab, CollabError, Update};

/// The accepted local transaction and its state before and after application.
#[derive(Debug)]
pub struct TransactionContext {
    transaction: Transaction,
    start_text: Text,
    start_selection: EditorSelection,
    new_text: Text,
    new_selection: EditorSelection,
    states: OnceLock<OriginStates>,
}

#[derive(Debug)]
struct OriginStates {
    start: ResolvedValues,
    result: ResolvedValues,
    start_collab: Option<QueueSnapshot>,
    result_collab: Option<QueueSnapshot>,
}

impl TransactionContext {
    pub(super) fn new(
        transaction: Transaction,
        start: StateContext<'_>,
        state: StateContext<'_>,
    ) -> Self {
        Self {
            transaction,
            start_text: start.text().clone(),
            start_selection: start.selection().clone(),
            new_text: state.text().clone(),
            new_selection: state.selection().clone(),
            states: OnceLock::new(),
        }
    }

    pub(super) fn is_captured(&self) -> bool {
        self.states.get().is_some()
    }

    pub(super) fn capture(
        &self,
        start: ResolvedValues,
        result: ResolvedValues,
        start_collab: Option<QueueSnapshot>,
        result_collab: Option<QueueSnapshot>,
    ) {
        self.states.get_or_init(|| OriginStates {
            start,
            result,
            start_collab,
            result_collab,
        });
    }

    pub fn transaction(&self) -> &Transaction {
        &self.transaction
    }

    pub fn start_text(&self) -> &Text {
        &self.start_text
    }

    pub fn start_selection(&self) -> &EditorSelection {
        &self.start_selection
    }

    pub fn new_text(&self) -> &Text {
        &self.new_text
    }

    /// The applied selection, including an implicit selection mapped over the edit.
    pub fn new_selection(&self) -> &EditorSelection {
        &self.new_selection
    }

    /// Read the resolved start fields and facets. For the start queue, use
    /// [`Self::start_collab_state`]. While selecting effects, use the live
    /// state views from [`super::SharedEffectsContext`] instead.
    pub fn start_state(&self) -> Option<StateContext<'_>> {
        Some(
            self.states
                .get()?
                .start
                .context(&self.start_text, &self.start_selection),
        )
    }

    /// Read the resolved result fields and facets. See [`Self::start_state`].
    pub fn state(&self) -> Option<StateContext<'_>> {
        Some(
            self.states
                .get()?
                .result
                .context(&self.new_text, &self.new_selection),
        )
    }

    /// After acceptance, read the recording session at the transaction start.
    /// For a standalone `Collab`, this is the session rather than an editor field.
    pub fn start_collab_state(&self) -> Option<OriginCollabState<'_>> {
        Some(OriginCollabState(self.states.get()?.start_collab.as_ref()))
    }

    /// After acceptance, read the recording session at the transaction result.
    /// The queue includes the new local update. For a synthetic text group,
    /// this is the result before the grouping receipt replaces its queue.
    pub fn collab_state(&self) -> Option<OriginCollabState<'_>> {
        Some(OriginCollabState(self.states.get()?.result_collab.as_ref()))
    }
}

/// A read-only view of collaboration at a queued transaction's start or result.
/// Later receipts, reconfiguration, and editor disposal do not change this view.
#[derive(Clone, Copy)]
pub struct OriginCollabState<'a>(Option<&'a QueueSnapshot>);

impl OriginCollabState<'_> {
    /// Return the send queue at this state, including the origins of its updates.
    pub fn sendable_updates(&self) -> Result<Vec<Update>, CollabError> {
        let snapshot = self.0.ok_or(CollabError::NotConfigured)?;
        let mut updates = snapshot.history.updates();
        if let Some((entry, origin)) = &snapshot.appended {
            let mut entry = entry.clone();
            // This view borrows the context that owns the snapshot.
            entry.origin = Some(origin.upgrade().expect("borrowed origin"));
            updates.push(entry);
        }
        Ok(updates)
    }

    pub fn synced_version(&self) -> Result<usize, CollabError> {
        Ok(self.0.ok_or(CollabError::NotConfigured)?.version)
    }

    /// The effective client ID, or an empty string if collaboration was disabled.
    pub fn client_id(&self) -> &str {
        self.0.map_or("", |snapshot| &snapshot.client_id)
    }
}

pub(super) struct QueueSnapshot {
    client_id: String,
    version: usize,
    history: PendingHistory,
    // A result can show the update that owns it without retaining itself.
    appended: Option<(Update, Weak<TransactionContext>)>,
}

impl QueueSnapshot {
    pub(super) fn new(session: &Collab, owner: Option<&Arc<TransactionContext>>) -> Self {
        let (history, appended) = if let Some(owner) = owner {
            let Some(Entries::Append(prefix, entry)) = session.pending_history.0.as_deref() else {
                unreachable!("capturing the result of a local append");
            };
            let mut entry = entry.clone();
            assert!(
                entry
                    .origin
                    .take()
                    .is_some_and(|origin| Arc::ptr_eq(&origin, owner))
            );
            (prefix.clone(), Some((entry, Arc::downgrade(owner))))
        } else {
            (session.pending_history.clone(), None)
        };
        Self {
            client_id: session.client_id.clone(),
            version: session.version,
            history,
            appended,
        }
    }
}

impl fmt::Debug for QueueSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueueSnapshot")
            .field("client_id", &self.client_id)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

// Appends share the old prefix instead of pinning a copy of the live vector
// at each origin. A receipt starts a new history from its transformed queue.
#[derive(Clone, Default)]
pub(super) struct PendingHistory(Option<Arc<Entries>>);

enum Entries {
    Base(Arc<Vec<Update>>),
    Append(PendingHistory, Update),
}

impl PendingHistory {
    pub(super) fn base(pending: &Arc<Vec<Update>>) -> Self {
        Self((!pending.is_empty()).then(|| Arc::new(Entries::Base(pending.clone()))))
    }

    pub(super) fn append(&mut self, update: Update) {
        let prefix = Self(self.0.take());
        self.0 = Some(Arc::new(Entries::Append(prefix, update)));
    }

    fn updates(&self) -> Vec<Update> {
        let mut suffix = Vec::new();
        let mut next = self.0.as_deref();
        while let Some(entries) = next {
            match entries {
                Entries::Append(prefix, update) => {
                    suffix.push(update.clone());
                    next = prefix.0.as_deref();
                }
                Entries::Base(updates) => {
                    suffix.extend(updates.iter().rev().cloned());
                    break;
                }
            }
        }
        suffix.reverse();
        suffix
    }
}

impl Drop for PendingHistory {
    fn drop(&mut self) {
        let Some(entries) = self.0.take().and_then(Arc::into_inner) else {
            return;
        };
        // A retained origin can be the last owner of a long history. Release
        // unique prefixes and the origins they own without recursing on them.
        let mut work = Vec::new();
        release_entries(entries, &mut work);
        while let Some(entries) = work.pop() {
            if let Some(entries) = Arc::into_inner(entries) {
                release_entries(entries, &mut work);
            }
        }
    }
}

fn release_entries(entries: Entries, work: &mut Vec<Arc<Entries>>) {
    match entries {
        Entries::Append(mut prefix, update) => {
            work.extend(prefix.0.take());
            release_update(update, work);
        }
        Entries::Base(updates) => {
            if let Some(updates) = Arc::into_inner(updates) {
                for update in updates {
                    release_update(update, work);
                }
            }
        }
    }
}

fn release_update(mut update: Update, work: &mut Vec<Arc<Entries>>) {
    if let Some(mut origin) = update.origin.take().and_then(Arc::into_inner)
        && let Some(states) = origin.states.take()
    {
        for mut snapshot in [states.start_collab, states.result_collab]
            .into_iter()
            .flatten()
        {
            work.extend(snapshot.history.0.take());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EditorState, StateEffectType};

    #[test]
    fn retaining_the_tail_keeps_earlier_origins_but_not_live_vectors() {
        const COUNT: usize = 8192;
        let mut editor = EditorState::new("abc");
        let effect = StateEffectType::new();
        let mut session =
            Collab::new("a", 7, &editor).with_shared_effects(move |_| vec![effect.of(())]);
        for _ in 0..COUNT {
            session
                .apply_local(&mut editor, Transaction::default())
                .unwrap();
            // Appending retains the history, not the backing live vector.
            assert_eq!(Arc::strong_count(&session.pending), 1);
        }
        let first = Arc::downgrade(session.pending[0].origin.as_ref().unwrap());
        let tail = session.pending.last().unwrap().clone();
        let last = Arc::downgrade(tail.origin.as_ref().unwrap());
        let echoes = session.pending.clone();
        session.receive_updates(&mut editor, 7, &echoes).unwrap();
        assert!(session.sendable_updates().is_empty());
        drop(echoes);
        drop(session);
        drop(editor);
        assert!(first.upgrade().is_some());
        let view = tail.origin_context().unwrap().collab_state().unwrap();
        assert_eq!(view.synced_version(), Ok(7));
        let snapshot = view.sendable_updates().unwrap();
        assert_eq!(snapshot.len(), COUNT);
        assert!(Weak::ptr_eq(
            &first,
            &Arc::downgrade(snapshot[0].origin.as_ref().unwrap())
        ));
        assert!(Arc::ptr_eq(
            snapshot.last().unwrap().origin.as_ref().unwrap(),
            tail.origin.as_ref().unwrap()
        ));
        drop(snapshot);
        drop(tail);
        assert!(first.upgrade().is_none());
        assert!(last.upgrade().is_none());
    }
}
