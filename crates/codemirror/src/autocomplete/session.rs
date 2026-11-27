//! Native completion source scheduling and result lifetimes.
//! Based on `@codemirror/autocomplete` 6.20.4. See `docs/licenses/codemirror-autocomplete.txt`.

use std::{
    cmp::Ordering as SortOrder,
    collections::{HashMap, HashSet},
    ops::Range,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use crate::{
    AnnotationType, Assoc, ChangeDesc, EditError, EditorState, MapMode, RangeTouch, Transaction,
};

use super::{
    CompletionResult, MatchMode, insert_completion_text,
    ranking::{compare_completions, rank},
};

mod callbacks;
mod context;
mod result;
mod source;
use callbacks::{ACTIVATION, ActivateOnCompletion, COMPARISON, CompareCompletions};
pub use callbacks::{activate_on_completion, compare_completion_ties};
pub use context::{CompletionNode, QueryToken, if_in, if_not_in};
pub use result::{CompletionItem, SourceResult};
pub use source::{ListSource, QueryMatch, complete_any_word};

const CANCELLED: u8 = 1;
const ABORT_ON_CHANGE: u8 = 2;
const EXPLICIT_DELAY: Duration = Duration::from_millis(50);
const MAX_UPDATES: usize = 50;
const MIN_ABORT_TIME: Duration = Duration::from_secs(1);

static PICKED_COMPLETION: LazyLock<AnnotationType<CompletionItem>> =
    LazyLock::new(AnnotationType::new);

/// Identifies the item picked by an `input.complete` transaction. Native text
/// picks attach this annotation. Custom apply functions can attach it themselves.
pub fn picked_completion() -> &'static AnnotationType<CompletionItem> {
    &PICKED_COMPLETION
}

/// The host uses this key to identify a registered provider across updates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceId(pub u64);

/// A cancellable request. Run the source on this immutable document snapshot.
#[derive(Clone, Debug)]
pub struct CompletionQuery {
    source: SourceId,
    serial: u64,
    state: EditorState,
    pub position: usize,
    pub explicit: bool,
    flags: Arc<AtomicU8>,
}

impl CompletionQuery {
    pub fn source(&self) -> SourceId {
        self.source
    }

    pub fn state(&self) -> &EditorState {
        &self.state
    }

    pub fn is_cancelled(&self) -> bool {
        self.flags.load(Ordering::Acquire) & CANCELLED != 0
    }

    /// Ask the host to cancel this query on the next document change.
    pub fn abort_on_document_change(&self) {
        self.flags.fetch_or(ABORT_ON_CHANGE, Ordering::Release);
    }

    fn wants_abort_on_change(&self) -> bool {
        self.flags.load(Ordering::Acquire) & ABORT_ON_CHANGE != 0
    }

    fn cancel(&self) {
        self.flags.fetch_or(CANCELLED, Ordering::Release);
    }

    fn matches(&self, other: &Self) -> bool {
        self.source == other.source
            && self.serial == other.serial
            && Arc::ptr_eq(&self.flags, &other.flags)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CompletionConfig {
    pub activate_on_typing: bool,
    pub typing_delay: Duration,
    pub update_sync_time: Duration,
    pub interaction_delay: Duration,
    pub select_on_open: bool,
    pub match_mode: MatchMode,
}

impl Default for CompletionConfig {
    fn default() -> Self {
        Self {
            activate_on_typing: true,
            typing_delay: Duration::from_millis(100),
            update_sync_time: Duration::from_millis(100),
            interaction_delay: Duration::from_millis(75),
            select_on_open: true,
            match_mode: MatchMode::Fuzzy,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionStatus {
    Inactive,
    Pending,
    Active,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionStep {
    Option,
    /// The number of options visible in the host's list.
    Page(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionOptionId {
    source: SourceId,
    serial: u64,
    revision: u64,
    index: usize,
}

impl CompletionOptionId {
    pub fn source(self) -> SourceId {
        self.source
    }
}

pub struct SessionOption<'a> {
    pub id: CompletionOptionId,
    pub item: &'a CompletionItem,
    pub range: Range<usize>,
    /// Byte ranges in the display label, or in the label if there is no display label.
    pub match_ranges: Vec<Range<usize>>,
    pub score: i64,
}

#[derive(Clone)]
struct Patch {
    before: usize,
    state: EditorState,
    changes: ChangeDesc,
    kind: Interaction,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Interaction {
    Typing,
    Backward,
    Other,
    Reset,
}

impl Interaction {
    fn simple(self) -> bool {
        matches!(self, Self::Typing | Self::Backward)
    }
}

struct Active {
    result: SourceResult,
    range: Range<usize>,
    limit: usize,
    explicit: bool,
    serial: u64,
    revision: u64,
}

impl Active {
    fn new(query: &CompletionQuery, result: SourceResult) -> Result<Self, EditError> {
        let range = result_range(&result, query.state())?;
        let limit = query
            .position
            .min(range.start.saturating_add(usize::from(!query.explicit)));
        Ok(Self {
            result,
            range,
            limit,
            explicit: query.explicit,
            serial: query.serial,
            revision: 0,
        })
    }

    fn update(mut self, patch: &Patch, activate: bool) -> Phase {
        let retry = || {
            if activate {
                Phase::Pending(false)
            } else {
                Phase::Idle
            }
        };
        if patch.kind == Interaction::Reset
            || patch.kind == Interaction::Other
                && patch.changes.touches_range(self.range.clone()) != RangeTouch::None
        {
            return retry();
        }
        if !patch.changes.is_empty()
            && let Some(map) = &self.result.map
        {
            let Some(mapped) = map(&self.result, &patch.changes) else {
                return retry();
            };
            self.set_result(mapped);
        }
        let Some(start) = patch
            .changes
            .map_pos(self.range.start, Assoc::Before, MapMode::Simple)
        else {
            return retry();
        };
        let Some(end) = patch
            .changes
            .map_pos(self.range.end, Assoc::After, MapMode::Simple)
        else {
            return retry();
        };
        let head = cursor(&patch.state);
        if patch.state.text().slice(start..end).is_none()
            || patch.kind.simple()
                && (head > end
                    || patch.kind == Interaction::Backward
                        && (patch.before == self.range.start || head < self.limit))
        {
            return retry();
        }
        self.range = start..end;
        self.limit = patch
            .changes
            .map_pos(self.limit, Assoc::Before, MapMode::Simple)
            .unwrap_or(start);
        if !patch.kind.simple() || self.result.valid_in(&patch.state, self.range.clone()) {
            return Phase::Active(self);
        }
        if let Some(update) = &self.result.update
            && let Some(result) = update(&self.result, self.range.clone(), &patch.state)
            && let Ok(range) = result_range(&result, &patch.state)
        {
            self.set_result(result);
            self.range = range;
            return Phase::Active(self);
        }
        Phase::Pending(self.explicit)
    }

    fn set_result(&mut self, result: SourceResult) {
        if !Arc::ptr_eq(&self.result.options, &result.options) {
            self.revision = self.revision.wrapping_add(1);
        }
        self.result = result;
    }
}

fn result_range(result: &SourceResult, state: &EditorState) -> Result<Range<usize>, EditError> {
    let range = result.from..result.to.unwrap_or_else(|| cursor(state));
    state
        .text()
        .slice(range.clone())
        .ok_or(EditError::InvalidRange)?;
    Ok(range)
}

struct Running {
    query: CompletionQuery,
    started: Duration,
    patches: Vec<Patch>,
    finished: Option<Option<Active>>,
}

enum Phase {
    Idle,
    Pending(bool),
    Running(Box<Running>),
    Active(Active),
}

impl Phase {
    fn pending(&self) -> bool {
        matches!(self, Self::Pending(_) | Self::Running(_))
    }

    fn cancel(&self) {
        if let Self::Running(running) = self {
            running.query.cancel();
        }
    }
}

struct Source {
    id: SourceId,
    phase: Phase,
}

/// Coordinates with caller-owned timers and async providers. All durations must
/// come from the same monotonic clock. A host must forward accepted editor
/// updates before asking the session to produce new completion transactions.
pub struct CompletionSession {
    config: CompletionConfig,
    activate_on_completion: Option<Box<ActivateOnCompletion>>,
    compare_completions: Option<Box<CompareCompletions>>,
    state: EditorState,
    sources: Vec<Source>,
    next_serial: u64,
    query_due: Option<Duration>,
    publish_due: Option<Duration>,
    shown_at: Option<Duration>,
    selected: Option<CompletionOptionId>,
    manually_selected: bool,
}

impl CompletionSession {
    pub fn new(
        state: &EditorState,
        config: CompletionConfig,
        sources: impl IntoIterator<Item = SourceId>,
    ) -> Self {
        let mut session = Self {
            config,
            activate_on_completion: None,
            compare_completions: None,
            state: state.clone(),
            sources: Vec::new(),
            next_serial: 0,
            query_due: None,
            publish_due: None,
            shown_at: None,
            selected: None,
            manually_selected: false,
        };
        session.set_sources(sources, Duration::ZERO);
        session
    }

    /// Reactivate completion after picks accepted by `update_after_apply` for
    /// which `activate` returns true. This does not depend on typing activation.
    pub fn with_activate_on_completion(
        mut self,
        activate: impl Fn(&CompletionItem) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.activate_on_completion = Some(Box::new(activate));
        self
    }

    /// Set the order of options with equal scores, before adjacent duplicate
    /// removal. Return `Equal` to retain source order. Set this before starting
    /// a session.
    pub fn with_compare_completions(
        mut self,
        compare: impl Fn(&CompletionItem, &CompletionItem) -> SortOrder + Send + Sync + 'static,
    ) -> Self {
        self.compare_completions = Some(Box::new(compare));
        self
    }

    /// Replace the language's available sources. Retain entries with unchanged IDs.
    pub fn set_sources(&mut self, sources: impl IntoIterator<Item = SourceId>, now: Duration) {
        let active = self.status() != CompletionStatus::Inactive;
        let mut old: HashMap<_, _> = self
            .sources
            .drain(..)
            .map(|source| (source.id, source.phase))
            .collect();
        let mut seen = HashSet::new();
        self.sources = sources
            .into_iter()
            .filter(|id| seen.insert(*id))
            .map(|id| Source {
                id,
                phase: old.remove(&id).unwrap_or(if active {
                    Phase::Pending(false)
                } else {
                    Phase::Idle
                }),
            })
            .collect();
        for remaining in old.values() {
            remaining.cancel();
        }
        self.schedule(now);
        self.refresh_visibility(now);
    }

    pub fn state(&self) -> &EditorState {
        &self.state
    }

    pub fn status(&self) -> CompletionStatus {
        if self.sources.iter().any(|item| item.phase.pending()) {
            CompletionStatus::Pending
        } else if self.shown_at.is_some() {
            CompletionStatus::Active
        } else {
            CompletionStatus::Inactive
        }
    }

    /// Start afresh, including when the user explicitly requests completion.
    pub fn start(&mut self, explicit: bool, now: Duration) {
        self.close();
        for source in &mut self.sources {
            source.phase = Phase::Pending(explicit);
        }
        self.schedule(now);
    }

    /// Request explicit completion if the session has a registered source.
    pub fn start_completion(&mut self, now: Duration) -> bool {
        if self.sources.is_empty() {
            return false;
        }
        self.start(true, now);
        true
    }

    /// Close a visible list or cancel pending requests. Return whether handled.
    pub fn close_completion(&mut self) -> bool {
        if self.status() == CompletionStatus::Inactive {
            return false;
        }
        self.close();
        true
    }

    pub fn close(&mut self) {
        for source in &mut self.sources {
            source.phase.cancel();
            source.phase = Phase::Idle;
        }
        self.query_due = None;
        self.publish_due = None;
        self.shown_at = None;
        self.selected = None;
        self.manually_selected = false;
    }

    /// Replay a committed edit. `changes` describes the actual accepted edit,
    /// not an unfiltered proposal. A selection set by typing or deleting is not
    /// treated as a separate caret motion.
    pub fn update(
        &mut self,
        state: &EditorState,
        changes: &ChangeDesc,
        selection_set: bool,
        user_event: Option<&str>,
        now: Duration,
    ) -> Result<(), EditError> {
        self.record_update(state, changes, selection_set, user_event, None, now)
            .map(|_| ())
    }

    /// Forward a successfully applied transaction, including its pick annotation.
    /// Pass the final proposal (after host planning) and the resolved transaction
    /// (after editor filters). For example, capture `resolved` in the check of
    /// `EditorView::update_checked`, then call this only after it succeeds.
    /// A filtered pick does not activate completion. Filters must preserve the
    /// pick, event, changes, selection, and proposed effects. Do not forward
    /// rejected transactions. Return whether the pick asked the session to reactivate.
    pub fn update_after_apply(
        &mut self,
        state: &EditorState,
        proposed: &Transaction,
        resolved: &Transaction,
        now: Duration,
    ) -> Result<bool, EditError> {
        let proposal = proposed.annotation(picked_completion());
        let picked = proposal.filter(|item| {
            resolved.annotation(picked_completion()) == Some(*item)
                && proposed.user_event() == resolved.user_event()
                && proposed.changes() == resolved.changes()
                && proposed.selection() == resolved.selection()
                && resolved.effects().len() >= proposed.effects().len()
                && proposed
                    .effects()
                    .iter()
                    .zip(resolved.effects())
                    .all(|(a, b)| a.same_value(b))
        });
        let event = if proposal.is_some() && picked.is_none() {
            None
        } else {
            resolved.user_event()
        };
        let mut changes = resolved.changes().clone();
        changes.remove_noops(self.state.text());
        self.record_update(
            state,
            &changes.desc(self.state.text().len())?,
            resolved.selection().is_some(),
            event,
            picked,
            now,
        )
    }

    fn record_update(
        &mut self,
        state: &EditorState,
        changes: &ChangeDesc,
        selection_set: bool,
        user_event: Option<&str>,
        picked: Option<&CompletionItem>,
        now: Duration,
    ) -> Result<bool, EditError> {
        if changes.length() != self.state.text().len() || changes.new_length() != state.text().len()
        {
            return Err(EditError::MismatchedLength);
        }
        let matches_event = |expected| {
            user_event.is_some_and(|name: &str| {
                name == expected
                    || name
                        .strip_prefix(expected)
                        .is_some_and(|tail| tail.starts_with('.'))
            })
        };
        let mut reactivate = false;
        if matches_event("input.complete")
            && let Some(item) = picked
        {
            let configured = ACTIVATION.read(state)?;
            reactivate = self
                .activate_on_completion
                .as_deref()
                .or(configured.as_deref())
                .is_some_and(|activate| activate(item));
        }
        if !reactivate && changes.is_empty() && !selection_set {
            self.state = state.clone();
            return Ok(false);
        }
        let kind = if reactivate {
            Interaction::Reset
        } else if matches_event("input.type") {
            Interaction::Typing
        } else if matches_event("delete.backward") {
            Interaction::Backward
        } else if selection_set {
            Interaction::Reset
        } else {
            Interaction::Other
        };
        let activate = reactivate || kind == Interaction::Typing && self.config.activate_on_typing;
        let patch = Patch {
            before: cursor(&self.state),
            state: state.clone(),
            changes: changes.clone(),
            kind,
        };
        for source in &mut self.sources {
            let old = std::mem::replace(&mut source.phase, Phase::Idle);
            source.phase = match old {
                Phase::Active(active) => active.update(&patch, activate),
                Phase::Running(mut running) => {
                    let reset = kind == Interaction::Reset
                        || !changes.is_empty()
                            && (!kind.simple() || running.query.wants_abort_on_change())
                        || running.patches.len() >= MAX_UPDATES
                            && now.saturating_sub(running.started) > MIN_ABORT_TIME;
                    if reset {
                        running.query.cancel();
                        pending_after_reset(running.query.explicit, &patch, activate)
                    } else {
                        running.patches.push(patch.clone());
                        Phase::Running(running)
                    }
                }
                Phase::Pending(explicit) => pending_after_reset(explicit, &patch, activate),
                Phase::Idle if activate => Phase::Pending(false),
                Phase::Idle => Phase::Idle,
            };
        }
        self.state = state.clone();
        self.schedule(now);
        self.refresh_visibility(now);
        Ok(reactivate)
    }

    /// The next timer deadline, if pending requests or results need attention.
    pub fn deadline(&self) -> Option<Duration> {
        self.query_due.into_iter().chain(self.publish_due).min()
    }

    /// Publish any ready results and start the requests whose delay elapsed.
    pub fn take_queries(&mut self, now: Duration) -> Vec<CompletionQuery> {
        if self.publish_due.is_some_and(|due| now >= due) {
            self.publish(now);
        }
        if self.query_due.is_none_or(|due| now < due) {
            return Vec::new();
        }
        self.query_due = None;
        let mut ready = Vec::new();
        for source in &mut self.sources {
            let Phase::Pending(explicit) = source.phase else {
                continue;
            };
            let query = CompletionQuery {
                source: source.id,
                serial: self.next_serial,
                state: self.state.clone(),
                position: cursor(&self.state),
                explicit,
                flags: Arc::new(AtomicU8::new(0)),
            };
            self.next_serial = self.next_serial.wrapping_add(1);
            source.phase = Phase::Running(Box::new(Running {
                query: query.clone(),
                started: now,
                patches: Vec::new(),
                finished: None,
            }));
            ready.push(query);
        }
        ready
    }

    /// Resolve a source. Return `false` for a request that is obsolete, foreign,
    /// or already finished. Pass `None` for no candidates.
    pub fn finish_query(
        &mut self,
        query: &CompletionQuery,
        result: Option<SourceResult>,
        now: Duration,
    ) -> Result<bool, EditError> {
        let Some(current) = self.sources.iter_mut().find(|item| item.id == query.source) else {
            return Ok(false);
        };
        let Phase::Running(running) = &mut current.phase else {
            return Ok(false);
        };
        if !running.query.matches(query) || running.finished.is_some() || query.is_cancelled() {
            return Ok(false);
        }
        let validation = result.map(|result| Active::new(query, result)).transpose();
        let error = match validation {
            Ok(result) => {
                running.finished = Some(result);
                None
            }
            Err(error) => {
                running.finished = Some(None);
                Some(error)
            }
        };
        if !self.sources.iter().any(
            |source| matches!(&source.phase, Phase::Running(running) if running.finished.is_none()),
        ) {
            self.publish(now);
        } else if self.publish_due.is_none() {
            self.publish_due = Some(now.saturating_add(self.config.update_sync_time));
        }
        error.map_or(Ok(true), Err)
    }

    /// Close active completion after a provider error. Ignore obsolete errors.
    pub fn fail_query(&mut self, query: &CompletionQuery) -> bool {
        let active = self.sources.iter().any(|source| {
            matches!(&source.phase, Phase::Running(running) if running.finished.is_none() && running.query.matches(query))
        });
        if active {
            self.close();
        }
        active
    }

    fn publish(&mut self, now: Duration) {
        self.publish_due = None;
        for source in &mut self.sources {
            let Phase::Running(running) = &mut source.phase else {
                continue;
            };
            let Some(result) = running.finished.take() else {
                continue;
            };
            let mut phase = result.map_or(Phase::Idle, Phase::Active);
            let returned = matches!(phase, Phase::Active(_));
            for patch in &running.patches {
                let activate = patch.kind == Interaction::Typing && self.config.activate_on_typing;
                phase = match phase {
                    Phase::Active(result) => result.update(patch, activate),
                    Phase::Pending(explicit) => pending_after_reset(explicit, patch, activate),
                    _ if activate => Phase::Pending(false),
                    _ => Phase::Idle,
                };
            }
            source.phase = if returned && matches!(phase, Phase::Idle) {
                Phase::Pending(running.query.explicit)
            } else {
                phase
            };
        }
        if self
            .sources
            .iter()
            .any(|item| matches!(item.phase, Phase::Pending(_)))
        {
            self.query_due = Some(now);
        }
        self.refresh_visibility(now);
    }

    fn schedule(&mut self, now: Duration) {
        let due = if self
            .sources
            .iter()
            .any(|item| matches!(item.phase, Phase::Pending(true)))
        {
            Some(EXPLICIT_DELAY)
        } else if self
            .sources
            .iter()
            .any(|item| matches!(item.phase, Phase::Pending(false)))
        {
            Some(self.config.typing_delay)
        } else {
            None
        };
        self.query_due = due.map(|delay| now.saturating_add(delay));
        if !self.sources.iter().any(
            |source| matches!(&source.phase, Phase::Running(running) if running.finished.is_some()),
        ) {
            self.publish_due = None;
        }
    }

    fn refresh_visibility(&mut self, now: Duration) {
        let first_and_kept = self.options().ok().and_then(|options| {
            let first = options.first()?.id;
            let kept = self.selected.filter(|id| {
                self.manually_selected && options.iter().any(|option| option.id == *id)
            });
            Some((first, kept))
        });
        if let Some((first, kept)) = first_and_kept {
            self.shown_at.get_or_insert(now);
            self.manually_selected = kept.is_some();
            self.selected = kept.or(self.config.select_on_open.then_some(first));
        } else {
            self.selected = None;
            self.manually_selected = false;
            // A pending refresh makes the old options unavailable but keeps
            // the list's original time for the interaction delay.
            if !self.sources.iter().any(|item| item.phase.pending()) {
                self.shown_at = None;
                for source in &mut self.sources {
                    source.phase = Phase::Idle;
                }
            }
        }
    }

    pub fn options(&self) -> Result<Vec<SessionOption<'_>>, EditError> {
        let configured = COMPARISON.read(&self.state)?;
        let compare = self
            .compare_completions
            .as_deref()
            .or(configured.as_deref());
        let active: Vec<_> = self
            .sources
            .iter()
            .filter_map(|source| match &source.phase {
                Phase::Active(active) => Some((source.id, active)),
                _ => None,
            })
            .collect();
        let borrowed: Vec<Vec<_>> = active
            .iter()
            .map(|(_, item)| {
                item.result
                    .options
                    .iter()
                    .map(CompletionItem::as_completion)
                    .collect()
            })
            .collect();
        let mut owners = HashMap::new();
        let results: Vec<_> = active
            .iter()
            .zip(&borrowed)
            .map(|((source, active), list)| {
                for (index, completion) in list.iter().enumerate() {
                    let id = CompletionOptionId {
                        source: *source,
                        serial: active.serial,
                        revision: active.revision,
                        index,
                    };
                    owners.insert(
                        std::ptr::from_ref(completion),
                        (
                            id,
                            &active.result.options[index],
                            &active.result.match_ranges,
                        ),
                    );
                }
                CompletionResult {
                    range: active.range.clone(),
                    options: list,
                    filter: active.result.filter,
                }
            })
            .collect();
        let ranked = rank(
            self.state.text(),
            &results,
            self.config.match_mode,
            |a, b| {
                if let Some(compare) = compare {
                    compare(
                        owners[&std::ptr::from_ref(a)].1,
                        owners[&std::ptr::from_ref(b)].1,
                    )
                } else {
                    compare_completions(a, b)
                }
            },
            |completion, matched| {
                let (_, item, hook) = owners[&std::ptr::from_ref(completion)];
                hook.as_ref()
                    .map_or_else(Vec::new, |hook| hook(item, matched))
            },
        )?;
        Ok(ranked
            .into_iter()
            .map(|row| {
                let (id, item, _) = owners[&std::ptr::from_ref(row.completion)];
                SessionOption {
                    id,
                    item,
                    range: row.range,
                    match_ranges: row.match_ranges,
                    score: row.score,
                }
            })
            .collect())
    }

    pub fn selected(&self) -> Option<CompletionOptionId> {
        self.selected
    }

    /// Read the selected option, even in a read-only editor.
    pub fn selected_completion(&self) -> Result<Option<SessionOption<'_>>, EditError> {
        let Some(id) = self.selected else {
            return Ok(None);
        };
        Ok(self.options()?.into_iter().find(|option| option.id == id))
    }

    /// Read the selected index in the current ranked list.
    pub fn selected_completion_index(&self) -> Result<Option<usize>, EditError> {
        let Some(id) = self.selected else {
            return Ok(None);
        };
        Ok(self.options()?.iter().position(|option| option.id == id))
    }

    /// Select an index without waiting for the keyboard interaction delay.
    /// Pass `None` to clear the selection. Return false if there is no change
    /// or the index is out of range.
    pub fn set_selected_completion(&mut self, index: Option<usize>) -> Result<bool, EditError> {
        let next = if let Some(index) = index {
            let options = self.options()?;
            let Some(option) = options.get(index) else {
                return Ok(false);
            };
            Some(option.id)
        } else {
            None
        };
        if self.selected == next {
            return Ok(false);
        }
        self.selected = next;
        self.manually_selected =
            index.is_some_and(|index| index != 0 || !self.config.select_on_open);
        Ok(true)
    }

    /// Move the selection after the minimum interaction delay. Option motion
    /// wraps around, and page motion stops at the first or last option.
    pub fn move_selection(
        &mut self,
        forward: bool,
        step: CompletionStep,
        now: Duration,
    ) -> Result<bool, EditError> {
        if !self.can_interact(now) {
            return Ok(false);
        }
        let list = self.options()?;
        if list.is_empty() {
            return Ok(false);
        }
        let last = list.len() - 1;
        let current = list
            .iter()
            .position(|option| Some(option.id) == self.selected);
        let next = match (current, forward, step) {
            (None, true, _) => 0,
            (None, false, _) => last,
            (Some(at), true, CompletionStep::Option) => {
                if at == last {
                    0
                } else {
                    at + 1
                }
            }
            (Some(at), false, CompletionStep::Option) => at.checked_sub(1).unwrap_or(last),
            (Some(at), true, CompletionStep::Page(visible)) => at
                .saturating_add(visible.saturating_sub(1).max(2))
                .min(last),
            (Some(at), false, CompletionStep::Page(visible)) => {
                at.saturating_sub(visible.saturating_sub(1).max(2))
            }
        };
        self.selected = Some(list[next].id);
        self.manually_selected = next != 0 || !self.config.select_on_open;
        Ok(true)
    }

    fn can_interact(&self, now: Duration) -> bool {
        self.shown_at
            .is_some_and(|at| now.saturating_sub(at) >= self.config.interaction_delay)
    }

    /// Build a text completion edit if the option remains available after the
    /// minimum interaction delay. Do not apply it to a different snapshot.
    pub fn accept(
        &self,
        id: CompletionOptionId,
        now: Duration,
    ) -> Result<Option<Transaction>, EditError> {
        self.accept_with(id, now, insert_picked_completion)
    }

    /// Build an applied result for an eligible option. The host can supply a
    /// snippet expansion or another native edit instead of a plain insertion.
    /// For a custom completion transaction, attach `picked_completion()` and
    /// set the `input.complete` user event to enable pick activation.
    pub fn accept_with<T>(
        &self,
        id: CompletionOptionId,
        now: Duration,
        apply: impl FnOnce(&EditorState, SessionOption<'_>) -> Result<T, EditError>,
    ) -> Result<Option<T>, EditError> {
        if !self.can_interact(now) {
            return Ok(None);
        }
        self.accept_pointer_with(id, apply)
    }

    /// Accept an option the user clicks in the visible list. Pointer picks
    /// skip the keyboard interaction delay, but still require an active,
    /// current option and a writable state.
    pub fn accept_pointer_with<T>(
        &self,
        id: CompletionOptionId,
        apply: impl FnOnce(&EditorState, SessionOption<'_>) -> Result<T, EditError>,
    ) -> Result<Option<T>, EditError> {
        self.find_option(id)?
            .map(|option| apply(&self.state, option))
            .transpose()
    }

    pub fn accept_selected(&self, now: Duration) -> Result<Option<Transaction>, EditError> {
        match self.selected {
            Some(id) => self.accept(id, now),
            None => Ok(None),
        }
    }

    /// Accept before the host inserts a typed commit character. An item's
    /// character list replaces, rather than extends, the source's list.
    pub fn commit(
        &self,
        id: CompletionOptionId,
        character: char,
    ) -> Result<Option<Transaction>, EditError> {
        self.commit_with(id, character, insert_picked_completion)
    }

    /// Like `commit`, with a host-provided edit builder for custom application.
    pub fn commit_with<T>(
        &self,
        id: CompletionOptionId,
        character: char,
        apply: impl FnOnce(&EditorState, SessionOption<'_>) -> Result<T, EditError>,
    ) -> Result<Option<T>, EditError> {
        if character.len_utf16() != 1 {
            return Ok(None);
        }
        let Some(option) = self.find_option(id)? else {
            return Ok(None);
        };
        let active = self
            .sources
            .iter()
            .find_map(|source| match &source.phase {
                Phase::Active(active) if option.id.source == source.id => Some(active),
                _ => None,
            })
            .expect("ranked option has an active source");
        if !option
            .item
            .commit_characters
            .as_ref()
            .unwrap_or(&active.result.commit_characters)
            .contains(&character)
        {
            return Ok(None);
        }
        apply(&self.state, option).map(Some)
    }

    fn find_option(&self, id: CompletionOptionId) -> Result<Option<SessionOption<'_>>, EditError> {
        if self.state.read_only() {
            return Ok(None);
        }
        Ok(self.options()?.into_iter().find(|item| item.id == id))
    }
}

impl Drop for CompletionSession {
    fn drop(&mut self) {
        self.close();
    }
}

fn insert_picked_completion(
    state: &EditorState,
    option: SessionOption<'_>,
) -> Result<Transaction, EditError> {
    insert_completion_text(
        state,
        option.range,
        option.item.insert.as_deref().unwrap_or(&option.item.label),
    )
    .map(|transaction| transaction.with_annotation(picked_completion().of(option.item.clone())))
}

fn pending_after_reset(explicit: bool, patch: &Patch, activate: bool) -> Phase {
    if patch.kind == Interaction::Reset
        || patch.kind == Interaction::Other
            && patch.changes.touches_range(patch.before..patch.before) != RangeTouch::None
    {
        if activate {
            Phase::Pending(false)
        } else {
            Phase::Idle
        }
    } else {
        Phase::Pending(explicit)
    }
}

fn cursor(state: &EditorState) -> usize {
    state.selection().main().range().start
}
