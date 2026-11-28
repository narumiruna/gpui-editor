//! Undo grouping and mapping adapted from CodeMirror commands 6.11.2.
//! See `docs/licenses/codemirror-commands.txt` for the upstream MIT notice.

use std::sync::{Arc, LazyLock};

use crate::{
    AnnotationType, ChangeDesc, ChangeSet, EditError, EditorSelection, EditorState, StateEffect,
    Transaction,
    collab::collab_state,
    language::{
        lezer::{LrParseOutput, LrPublication},
        wast::{WastParseOutput, WastPublication},
    },
};

mod branch;
mod codec;
mod config;
mod effects;

use branch::{Branch, BranchMapping, Entry};
use config::{Clock, Config};
pub use effects::inverted_effects;

static ISOLATION: LazyLock<AnnotationType<HistoryIsolation>> = LazyLock::new(AnnotationType::new);

enum ReplaySelection {
    Saved,
    Inferred,
}

fn save_opposite_selection(state: &EditorState) -> Option<EditorSelection> {
    // Collaborative events use the same inferred replay as events restored
    // without an opposite selection. Native events keep the host's selection.
    collab_state(state)
        .expect("typed collaboration field")
        .is_none()
        .then(|| state.selection.clone())
}

/// Keep an edit separate from older edits, later edits, or both.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryIsolation {
    Before,
    After,
    Full,
}

impl Transaction {
    pub fn isolate_history(self, isolation: HistoryIsolation) -> Self {
        self.with_annotation(Self::history_isolation_annotation().of(isolation))
    }

    pub fn history_isolation(&self) -> Option<HistoryIsolation> {
        self.annotation(Self::history_isolation_annotation())
            .copied()
    }

    /// Return the annotation type used to isolate a transaction in history.
    pub fn history_isolation_annotation() -> &'static AnnotationType<HistoryIsolation> {
        &ISOLATION
    }
}

#[derive(Clone, Debug, Default)]
enum Group {
    #[default]
    Closed,
    Open,
    Recorded,
}

struct Input {
    isolation: Option<HistoryIsolation>,
    time: Option<u64>,
    composition: bool,
    joinable: bool,
    empty_user_event: bool,
    selection_changed: bool,
    selection_group: Option<(u64, String)>,
    accepted: Option<Transaction>,
}

impl Input {
    fn new(transaction: &Transaction) -> Self {
        Self {
            isolation: transaction.history_isolation(),
            time: transaction.time(),
            composition: transaction.user_event() == Some("input.type.compose"),
            joinable: transaction.user_event().is_none()
                || transaction.is_user_event("input.type")
                || transaction.is_user_event("delete"),
            empty_user_event: transaction.user_event() == Some(""),
            selection_changed: transaction.selection().is_some(),
            selection_group: transaction.time().zip(
                transaction
                    .user_event()
                    .filter(|_| transaction.is_user_event("select"))
                    .map(str::to_owned),
            ),
            accepted: None,
        }
    }

    fn isolate_before(&self) -> bool {
        matches!(
            self.isolation,
            Some(HistoryIsolation::Before | HistoryIsolation::Full)
        )
    }
}

#[derive(Default)]
pub(crate) struct UpdateReport {
    pub changes: Option<ChangeSet>,
    pub selection_set: bool,
    pub scroll_into_view: bool,
    pub effects: Vec<StateEffect>,
}

impl UpdateReport {
    fn effects(transaction: &Transaction, report: bool) -> Vec<StateEffect> {
        if report {
            transaction.effects().to_vec()
        } else {
            Vec::new()
        }
    }
}

/// An editor state with bounded, delta-based undo and redo. Metadata belongs to the view.
#[derive(Debug)]
pub struct History<M> {
    state: EditorState,
    undo: Branch<M>,
    redo: Branch<M>,
    group: Group,
    clock: Clock,
    last_selection_group: Option<(u64, String)>,
}

impl<M: Clone> History<M> {
    /// Create empty history. Without [`EditorState::history_config`] or a
    /// custom predicate, keep the native limit of 100 groups per branch.
    pub fn new(state: EditorState) -> Self {
        Self {
            state,
            undo: Branch::default(),
            redo: Branch::default(),
            group: Group::Closed,
            clock: Clock::Reset,
            last_selection_group: None,
        }
    }

    pub fn state(&self) -> &EditorState {
        &self.state
    }

    // Keep both branches private until all steps of a headless input succeed.
    pub(crate) fn staging_copy(&self) -> Self {
        Self {
            state: self.state.clone(),
            undo: self.undo.clone(),
            redo: self.redo.clone(),
            group: self.group.clone(),
            clock: self.clock,
            last_selection_group: self.last_selection_group.clone(),
        }
    }

    // The bounded initial tree is part of view attachment or the already
    // accepted state change. It is not another field or history update.
    pub(crate) fn seed_wast_publication(&mut self, output: Arc<WastParseOutput>) {
        debug_assert!(self.state.wast_publication.is_none());
        self.state.wast_publication = Some(WastPublication::new(output));
    }

    pub(crate) fn commit_wast_publication(
        &mut self,
        output: Arc<WastParseOutput>,
    ) -> Result<ChangeDesc, EditError> {
        self.state.commit_wast_publication(output)
    }

    pub(crate) fn seed_lr_publication(&mut self, output: Arc<LrParseOutput>) {
        debug_assert!(self.state.lr_publication.is_none());
        self.state.lr_publication = Some(LrPublication::new(output));
    }

    pub(crate) fn commit_lr_publication(
        &mut self,
        output: Arc<LrParseOutput>,
    ) -> Result<ChangeDesc, EditError> {
        self.state.commit_lr_publication(output)
    }

    /// Return the number of edit or reversible-effect groups available to undo.
    /// Selection-only steps do not count.
    pub fn undo_depth(&self) -> usize {
        self.undo.entries.len()
    }

    /// Return the number of edit or reversible-effect groups available to redo.
    pub fn redo_depth(&self) -> usize {
        self.redo.entries.len()
    }

    pub fn can_undo(&self) -> bool {
        self.undo_depth() != 0
    }

    pub fn can_redo(&self) -> bool {
        self.redo_depth() != 0
    }

    /// Return whether the next undo-selection command can replay a selection
    /// without replaying a change event.
    pub fn can_undo_selection(&self) -> bool {
        !self.undo.selections().is_empty()
    }

    /// Return whether the next redo-selection command can replay a selection
    /// without replaying a change event.
    pub fn can_redo_selection(&self) -> bool {
        !self.redo.selections().is_empty()
    }

    /// Begin a group of subsequent edits. The group may be empty.
    pub fn start_group(&mut self) {
        self.break_timed_groups(Clock::Boundary);
        self.group = Group::Open;
    }

    pub fn end_group(&mut self) {
        self.close_group(Clock::Boundary);
    }

    fn close_group(&mut self, clock: Clock) {
        self.break_timed_groups(clock);
        self.group = Group::Closed;
    }

    fn isolate(&mut self, config: Option<&Config>) {
        let clock = self.clock.isolated(config);
        if !matches!(clock, Clock::At(0)) {
            self.break_timed_groups(clock);
        }
        if matches!(self.group, Group::Recorded) {
            self.group = Group::Open;
        }
    }

    fn break_timed_groups(&mut self, clock: Clock) {
        self.clock = clock;
        self.last_selection_group = None;
    }

    /// Set the selection without adding a history entry. Use [`Self::update`]
    /// with an explicit selection to record selection history.
    pub fn set_selection(&mut self, selection: EditorSelection) -> Result<(), EditError> {
        self.state.set_selection(selection)?;
        self.break_timed_groups(Clock::Boundary);
        Ok(())
    }

    /// Apply an edit and let the caller derive the final selection and metadata from
    /// the resulting state. An invalid transaction or final selection leaves no edit.
    pub fn update(
        &mut self,
        transaction: Transaction,
        before: M,
        finish: impl FnOnce(&EditorState) -> (Option<EditorSelection>, M),
    ) -> Result<M, EditError> {
        self.update_reporting(transaction, before, finish, false)
            .map(|(metadata, _)| metadata)
    }

    pub(crate) fn update_reporting(
        &mut self,
        transaction: Transaction,
        before: M,
        finish: impl FnOnce(&EditorState) -> (Option<EditorSelection>, M),
        report: bool,
    ) -> Result<(M, UpdateReport), EditError> {
        let transaction = self.state.filter_transaction(transaction)?;
        self.update_resolved_reporting(transaction, before, finish, report)
    }

    // The caller must apply filters and extenders once before this step.
    pub(crate) fn update_resolved_reporting(
        &mut self,
        transaction: Transaction,
        before: M,
        finish: impl FnOnce(&EditorState) -> (Option<EditorSelection>, M),
        report: bool,
    ) -> Result<(M, UpdateReport), EditError> {
        self.apply_resolved(transaction, before, finish, report, ReplaySelection::Saved)
    }

    pub(crate) fn update_resolved_input_step(
        &mut self,
        transaction: Transaction,
        metadata: &M,
    ) -> Result<UpdateReport, EditError> {
        self.apply_resolved(
            transaction,
            metadata.clone(),
            |_| (None, metadata.clone()),
            true,
            ReplaySelection::Inferred,
        )
        .map(|(_, report)| report)
    }

    fn apply_resolved(
        &mut self,
        transaction: Transaction,
        before: M,
        finish: impl FnOnce(&EditorState) -> (Option<EditorSelection>, M),
        report: bool,
        replay: ReplaySelection,
    ) -> Result<(M, UpdateReport), EditError> {
        let scroll_into_view = report && transaction.scroll_into_view();
        let accepted_effects = UpdateReport::effects(&transaction, report);
        let mut input = Input::new(&transaction);
        let record = transaction.add_to_history();
        if record
            && (!transaction.effects().is_empty()
                || self
                    .state
                    .configured_history()
                    .as_ref()
                    .as_ref()
                    .is_some_and(Config::needs_transaction))
        {
            // An effect can install the predicate that handles this edit.
            input.accepted = Some(transaction.clone());
        }
        let length = self.state.text.len();
        let before_selection = self.state.selection.clone();
        let before_extensions = self.state.extensions.clone();
        let before_wast_publication = self.state.wast_publication.clone();
        let before_lr_publication = self.state.lr_publication.clone();
        let (edit, effects) = effects::apply(&mut self.state, transaction, record)?;
        let config = self.state.configured_history();
        let config = config.as_ref().as_ref();
        if config.is_some_and(Config::needs_transaction)
            && let Some(transaction) = &mut input.accepted
        {
            transaction.changes = edit
                .as_ref()
                .map_or_else(ChangeSet::default, |edit| edit.forward.clone());
        }
        let (selection, after) = finish(&self.state);
        input.selection_changed |= selection.is_some();
        let selection_set = input.selection_changed;
        let result = (|| {
            if let Some(selection) = selection {
                self.state.set_selection(selection)?;
            }
            if record {
                let entry = Entry {
                    changes: edit
                        .as_ref()
                        .map_or_else(ChangeSet::default, |edit| edit.inverse.clone()),
                    effects,
                    selection: before_selection.clone(),
                    opposite_selection: match replay {
                        ReplaySelection::Saved => save_opposite_selection(&self.state),
                        ReplaySelection::Inferred => None,
                    },
                    before,
                    after: after.clone(),
                    selections_after: Vec::new(),
                    mapped: None,
                };
                self.record(entry, self.state.text.len(), input, config)
            } else {
                self.map_branches(
                    edit.as_ref().map(|edit| &edit.forward),
                    length,
                    input,
                    config,
                )
            }
        })();
        if let Err(error) = result {
            if let Some(edit) = edit {
                edit.inverse.apply(&mut self.state.text);
            }
            self.state.selection = before_selection;
            self.state.extensions = before_extensions;
            self.state.wast_publication = before_wast_publication;
            self.state.lr_publication = before_lr_publication;
            return Err(error);
        }
        let changes = edit.filter(|_| report).map(|edit| edit.forward);
        Ok((
            after,
            UpdateReport {
                changes,
                selection_set,
                scroll_into_view,
                effects: accepted_effects,
            },
        ))
    }

    fn joins(
        &self,
        previous: &Entry<M>,
        entry: &Entry<M>,
        length: usize,
        input: &Input,
        config: Option<&Config>,
    ) -> bool {
        // A group must not change the replay mode of an older event.
        if previous.opposite_selection.is_some() != entry.opposite_selection.is_some() {
            return false;
        }
        let isolate = input.isolate_before();
        let clock = match self.group {
            Group::Recorded => return !isolate,
            Group::Open => return false,
            Group::Closed if isolate && config.is_none() => return false,
            Group::Closed if isolate => self.clock.isolated(config),
            Group::Closed => self.clock,
        };
        let joinable = input.joinable || (config.is_some() && input.empty_user_event);
        if previous.changes.is_empty() || !joinable {
            return false;
        }
        let timed = previous.selections_after.is_empty() && clock.within(input.time, config) && {
            let adjacent = previous.touches(entry, length);
            config.map_or(adjacent, |policy| {
                policy.joins(input.accepted.as_ref(), adjacent)
            })
        };
        timed || input.composition && clock.joins_composition(input.time, config)
    }

    fn record(
        &mut self,
        mut entry: Entry<M>,
        length: usize,
        input: Input,
        config: Option<&Config>,
    ) -> Result<(), EditError> {
        let has_event = !entry.is_empty();
        let join = has_event
            && self
                .undo
                .entries
                .back()
                .is_some_and(|previous| self.joins(previous, &entry, length, &input, config));
        if join {
            entry = self
                .undo
                .entries
                .back()
                .expect("joining requires a previous event")
                .join(entry, length)?;
        }
        if input.isolate_before() {
            self.isolate(config);
        }
        if has_event {
            self.undo
                .store(entry, join, config.map(|cfg| cfg.min_depth));
            if matches!(self.group, Group::Open) {
                self.group = Group::Recorded;
            }
            self.redo = Branch::default();
            self.clock = Clock::recorded(input.time);
            self.last_selection_group = None;
        } else {
            if matches!(self.group, Group::Recorded) {
                // Keep the final host metadata and any saved opposite
                // selection for an explicit group, even after a no-op edit.
                let previous = self.undo.entries.back_mut().expect("recorded group");
                if let Some(selection) = &mut previous.opposite_selection {
                    selection.clone_from(&self.state.selection);
                }
                previous.after = entry.after;
            } else if input.selection_changed {
                self.record_selection(entry.selection, input.time, input.selection_group, config);
            }
        }
        if matches!(
            input.isolation,
            Some(HistoryIsolation::After | HistoryIsolation::Full)
        ) {
            self.isolate(config);
        }
        Ok(())
    }

    fn record_selection(
        &mut self,
        before: EditorSelection,
        time: Option<u64>,
        group: Option<(u64, String)>,
        config: Option<&Config>,
    ) {
        let join =
            self.last_selection_group
                .as_ref()
                .zip(group.as_ref())
                .is_some_and(|((previous, previous_event), (now, event))| {
                    previous_event == event
                        && Clock::At(*previous).within(Some(*now), config)
                        && self.undo.selections().last().is_some_and(|last| {
                            last.ranges().len() == before.ranges().len()
                                && last.ranges().iter().zip(before.ranges()).all(|(a, b)| {
                                    (a.anchor() == a.head()) == (b.anchor() == b.head())
                                })
                        })
                });
        if !join {
            self.undo.add_selection(before);
            self.last_selection_group = group;
            if config.is_some() {
                self.clock = Clock::recorded(time);
            }
        }
    }

    fn map_branches(
        &mut self,
        changes: Option<&ChangeSet>,
        length: usize,
        input: Input,
        config: Option<&Config>,
    ) -> Result<(), EditError> {
        if let Some(changes) = changes {
            let mapping = changes.desc(length)?;
            let undo = BranchMapping::new(&self.undo, 0, mapping.clone())?;
            let redo = BranchMapping::new(&self.redo, 0, mapping)?;
            if undo.apply(&mut self.undo) && matches!(self.group, Group::Recorded) {
                self.group = Group::Open;
            }
            redo.apply(&mut self.redo);
        }
        if input.isolate_before() {
            self.isolate(config);
        }
        Ok(())
    }

    /// Replay an undo step even when the editor state is read-only.
    /// For a user command, use [`Self::undo_command`].
    pub fn undo(&mut self) -> Option<M> {
        self.undo_reporting(false).map(|(metadata, _)| metadata)
    }

    pub(crate) fn undo_reporting(&mut self, report: bool) -> Option<(M, UpdateReport)> {
        self.replay(false, None, report)
    }

    /// Replay a redo step even when the editor state is read-only.
    /// For a user command, use [`Self::redo_command`].
    pub fn redo(&mut self) -> Option<M> {
        self.redo_reporting(false).map(|(metadata, _)| metadata)
    }

    pub(crate) fn redo_reporting(&mut self, report: bool) -> Option<(M, UpdateReport)> {
        self.replay(true, None, report)
    }

    /// Run the headless undo command. It does not edit a read-only state.
    pub fn undo_command(&mut self) -> Option<M> {
        if self.state.read_only() {
            return None;
        }
        self.undo()
    }

    /// Run the headless redo command. It does not edit a read-only state.
    pub fn redo_command(&mut self) -> Option<M> {
        if self.state.read_only() {
            return None;
        }
        self.redo()
    }

    /// Undo the latest selection change, or fall back to [`Self::undo`].
    /// This command is also available when the state is read-only.
    /// Selection replay retains the caller's `metadata`.
    pub fn undo_selection(&mut self, metadata: M) -> Option<M> {
        self.undo_selection_reporting(metadata, false)
            .map(|(metadata, _)| metadata)
    }

    pub(crate) fn undo_selection_reporting(
        &mut self,
        metadata: M,
        report: bool,
    ) -> Option<(M, UpdateReport)> {
        self.replay(false, Some(metadata), report)
    }

    /// Redo the latest selection change, or fall back to [`Self::redo`].
    /// This command is also available when the state is read-only.
    /// Selection replay retains the caller's `metadata`.
    pub fn redo_selection(&mut self, metadata: M) -> Option<M> {
        self.redo_selection_reporting(metadata, false)
            .map(|(metadata, _)| metadata)
    }

    pub(crate) fn redo_selection_reporting(
        &mut self,
        metadata: M,
        report: bool,
    ) -> Option<(M, UpdateReport)> {
        self.replay(true, Some(metadata), report)
    }

    fn replay(
        &mut self,
        redo: bool,
        metadata: Option<M>,
        report: bool,
    ) -> Option<(M, UpdateReport)> {
        let (source, target, event) = if redo {
            (&mut self.redo, &mut self.undo, "redo")
        } else {
            (&mut self.undo, &mut self.redo, "undo")
        };
        if let Some(metadata) = metadata
            && !source.selections().is_empty()
        {
            let accepted =
                Self::replay_selection(&mut self.state, source, target, event, &metadata, report)?;
            self.close_group(Clock::Reset);
            return Some((metadata, accepted));
        }
        let entry = source.entries.back()?;
        let reverse_selection = entry.reverse_selection(self.state.text.len()).ok()?;
        let rest = entry
            .mapped
            .clone()
            .map(|mapping| BranchMapping::new(source, 1, mapping))
            .transpose()
            .ok()?;
        let transaction = self
            .state
            .filter_transaction(
                Transaction::new(entry.changes.clone())
                    .with_selection(entry.selection.clone())
                    .with_effects(entry.effects.iter().cloned())
                    .with_user_event(event)
                    .with_scroll_into_view()
                    .without_filtering(),
            )
            .ok()?;
        let before = self.state.selection.clone();
        let scroll_into_view = report && transaction.scroll_into_view();
        let accepted_effects = UpdateReport::effects(&transaction, report);
        let (edit, effects) = effects::apply(&mut self.state, transaction, true).ok()?;
        let mut entry = source.entries.pop_back().expect("replay requires an entry");
        if let Some(rest) = rest {
            rest.apply(source);
        }
        let (forward, inverse) = edit.map_or((None, ChangeSet::default()), |edit| {
            (report.then_some(edit.forward), edit.inverse)
        });
        entry.changes = inverse;
        entry.effects = effects;
        entry.mapped = None;
        entry.selections_after.clear();
        let previous = std::mem::replace(&mut entry.selection, reverse_selection);
        if entry.opposite_selection.is_some() {
            entry.opposite_selection = Some(previous);
        }
        let metadata = if redo { &entry.after } else { &entry.before }.clone();
        if entry.is_empty() {
            target.add_selection(before);
        } else {
            let config = self.state.configured_history();
            target.store(
                entry,
                false,
                config.as_ref().as_ref().map(|cfg| cfg.min_depth),
            );
        }
        self.close_group(Clock::Reset);
        Some((
            metadata,
            UpdateReport {
                changes: forward,
                selection_set: true,
                scroll_into_view,
                effects: accepted_effects,
            },
        ))
    }

    fn replay_selection(
        state: &mut EditorState,
        source: &mut Branch<M>,
        target: &mut Branch<M>,
        event: &str,
        metadata: &M,
        report: bool,
    ) -> Option<UpdateReport> {
        let selections = source.selections();
        let transaction = state
            .filter_transaction(
                Transaction::default()
                    .with_selection(selections.last()?.clone())
                    .with_user_event(format!("select.{event}"))
                    .with_scroll_into_view(),
            )
            .ok()?;
        let before = state.selection.clone();
        let scroll_into_view = report && transaction.scroll_into_view();
        let accepted_effects = UpdateReport::effects(&transaction, report);
        let (edit, effects) = effects::apply(state, transaction, true).ok()?;
        let (forward, inverse) = edit.map_or((None, ChangeSet::default()), |edit| {
            (report.then_some(edit.forward), edit.inverse)
        });
        if inverse.is_empty() && effects.is_empty() {
            target.add_selection(before);
        } else {
            // A filter or extender can add changes or reversible effects.
            let config = state.configured_history();
            target.store(
                Entry {
                    changes: inverse,
                    effects,
                    selection: selections[0].clone(),
                    opposite_selection: save_opposite_selection(state),
                    before: metadata.clone(),
                    after: metadata.clone(),
                    selections_after: Vec::new(),
                    mapped: None,
                },
                false,
                config.as_ref().as_ref().map(|cfg| cfg.min_depth),
            );
        }
        source.pop_selection();
        Some(UpdateReport {
            changes: forward,
            selection_set: true,
            scroll_into_view,
            effects: accepted_effects,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Change;

    #[test]
    fn stores_only_changed_bytes_for_a_small_edit() {
        let mut history = History::new(EditorState::new("a".repeat(1_000_000)));
        let changes = ChangeSet::single(Change::new(123..124, "longer")).unwrap();
        history
            .update(Transaction::new(changes), (), |_| (None, ()))
            .unwrap();
        assert_eq!(history.undo.entries[0].changes.payload_len(), 1);
        assert_eq!(history.undo(), Some(()));
        assert_eq!(history.redo.entries[0].changes.payload_len(), 6);
    }
}
