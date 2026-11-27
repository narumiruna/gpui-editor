//! Hover sources, accepted results, and pointer timing.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use super::{SHOW_TOOLTIP, Tooltip, TooltipId, next_id};
use crate::{
    Assoc, EditorState, Extension, Facet, MapMode, StateEffect, StateEffectType, StateField,
    StateUpdate,
};

const HOVER_TIME: Duration = Duration::from_millis(300);
const RESTART_TIME: Duration = Duration::from_millis(20);

/// A pure transaction predicate that dismisses a programmatically opened hover.
pub type HoverUntil = Arc<dyn Fn(&StateUpdate<'_>) -> bool + Send + Sync>;
/// A pure predicate that dismisses one tooltip before position mapping.
pub type HoverHide = Arc<dyn Fn(&StateUpdate<'_>, &Tooltip) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct HoverOptions {
    /// Hide on a document change or an explicit selection, even if it is unchanged.
    /// Upstream 6.44.0 also treats its `"touch"` setting as enabled.
    pub hide_on_change: bool,
    pub hide_on: Option<HoverHide>,
    /// A zero duration uses the default of 300 ms.
    pub hover_time: Duration,
}

impl Default for HoverOptions {
    fn default() -> Self {
        Self {
            hide_on_change: false,
            hide_on: None,
            hover_time: HOVER_TIME,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HoverSourceId(u64);

#[derive(Clone, Default)]
pub struct HoverActive {
    tooltips: Vec<Tooltip>,
    until: Option<HoverUntil>,
}

impl HoverActive {
    pub fn tooltips(&self) -> &[Tooltip] {
        &self.tooltips
    }

    pub fn is_locked(&self) -> bool {
        self.until.is_some() && !self.tooltips.is_empty()
    }
}

#[derive(Clone)]
struct Source {
    id: HoverSourceId,
    active: StateField<HoverActive>,
    set: StateEffectType<HoverActive>,
    hover_time: Duration,
}

impl Source {
    fn active(&self, state: &EditorState) -> Option<Arc<HoverActive>> {
        state
            .context()
            .field(&self.active)
            .expect("valid hover state")
    }
}

/// A source registration. Keep the registration to read its active results or
/// to target activation and close effects. The host supplies the source itself.
#[derive(Clone)]
pub struct HoverTooltip {
    source: Source,
    extension: Extension,
}

type Sources = Facet<Source, Vec<Source>>;
static SOURCES: LazyLock<Sources> = LazyLock::new(Facet::collect);

type HoverSections = Facet<Vec<Tooltip>, Vec<Tooltip>>;
pub(super) static HOVER_SECTIONS: LazyLock<HoverSections> = LazyLock::new(|| {
    Facet::new(|inputs: &[&Vec<Tooltip>]| {
        inputs
            .iter()
            .flat_map(|input| input.iter())
            .cloned()
            .collect()
    })
});

static HOVER_HOST: LazyLock<Extension> = LazyLock::new(|| {
    SHOW_TOOLTIP.compute(|state| {
        let sections = state.facet(&HOVER_SECTIONS)?;
        let Some(first) = sections.first() else {
            return Ok(None);
        };
        let mut host = Tooltip::with_id(TooltipId::HOVER_HOST, first.pos);
        host.above = first.above;
        host.pos = sections.iter().map(|section| section.pos).min().unwrap();
        host.end = sections
            .iter()
            .map(|section| section.end.unwrap_or(section.pos))
            .max();
        host.arrow = sections.iter().any(|section| section.arrow);
        Ok(Some(host))
    })
});

type Close = StateEffectType<Option<HoverSourceId>>;
static CLOSE: LazyLock<Close> = LazyLock::new(StateEffectType::new);

/// A transaction effect that closes every active hover source.
pub static CLOSE_HOVER_TOOLTIPS: LazyLock<StateEffect> = LazyLock::new(|| CLOSE.of(None));

pub fn close_hover_tooltips() -> StateEffect {
    CLOSE_HOVER_TOOLTIPS.clone()
}

pub fn close_hover_tooltip(source: &HoverTooltip) -> StateEffect {
    CLOSE.of(Some(source.id()))
}

pub fn has_hover_tooltips(state: &EditorState) -> bool {
    !state
        .context()
        .facet(&HOVER_SECTIONS)
        .expect("valid hover sections")
        .is_empty()
}

pub fn hover_tooltip(options: HoverOptions) -> HoverTooltip {
    let id = HoverSourceId(next_id());
    let set = StateEffectType::<HoverActive>::new();
    let effect = set.clone();
    let hover_time = if options.hover_time.is_zero() {
        HOVER_TIME
    } else {
        options.hover_time
    };
    let active = StateField::new(
        |_| Ok(HoverActive::default()),
        move |previous, update| {
            let mut value = previous.clone();
            let tr = update.transaction();
            let changed = !update.changes().is_empty();
            if !value.tooltips.is_empty() {
                if (options.hide_on_change && (changed || tr.selection().is_some()))
                    || value.until.as_ref().is_some_and(|until| until(update))
                {
                    value.tooltips.clear();
                } else if let Some(hide) = &options.hide_on {
                    value.tooltips.retain(|tip| !hide(update, tip));
                }
            }
            if changed {
                let changes = update.changes();
                value.tooltips.retain_mut(|tip| {
                    let Some(pos) = changes.map_pos(tip.pos, Assoc::Before, MapMode::TrackDel)
                    else {
                        return false;
                    };
                    tip.pos = pos;
                    tip.end = tip
                        .end
                        .and_then(|end| changes.map_pos(end, Assoc::Before, MapMode::Simple));
                    true
                });
            }
            for e in tr.effects() {
                if let Some(results) = e.get(&effect) {
                    value = results.clone();
                    value.tooltips.retain(|tip| {
                        update
                            .state()
                            .text()
                            .is_valid_range(tip.pos..tip.end.unwrap_or(tip.pos))
                    });
                } else if e
                    .get(&CLOSE)
                    .is_some_and(|scope| scope.is_none_or(|scope| scope == id))
                {
                    value.tooltips.clear();
                }
            }
            if value.tooltips.is_empty() {
                value.until = None;
            }
            Ok(value)
        },
    );
    let source = Source {
        id,
        active,
        set,
        hover_time,
    };
    let extension = Extension::group([
        source.active.extension(),
        SOURCES.of(source.clone()),
        HOVER_SECTIONS.from_field(source.active.clone(), |active| active.tooltips.clone()),
        HOVER_HOST.clone(),
    ]);
    HoverTooltip { source, extension }
}

impl HoverTooltip {
    pub fn id(&self) -> HoverSourceId {
        self.source.id
    }

    pub fn extension(&self) -> Extension {
        self.extension.clone()
    }

    pub fn active(&self, state: &EditorState) -> Option<Arc<HoverActive>> {
        self.source.active(state)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HoverSide {
    Before,
    After,
}

/// The host resolves a text hit, including its side in document order. Use
/// `After` at the start of a widget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HoverTarget {
    pub pos: usize,
    pub side: HoverSide,
}

/// Hit tests for keeping an open tooltip. The host uses an exact hit for a
/// point and a nearest position inside the editor's content bounds for a range.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HoverPointer {
    pub position: Option<usize>,
    pub range_position: Option<usize>,
    /// Includes the hover host's arrow and a four pixel margin.
    pub over_tooltip: bool,
}

/// Send each request to its source. Return it with the source result, whether
/// the source runs immediately or asynchronously.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HoverRequest {
    pub source: HoverSourceId,
    pub target: HoverTarget,
    token: u64,
}

struct Pending {
    request: HoverRequest,
    until: Option<HoverUntil>,
    deferred: bool,
}

#[derive(Default)]
struct SourceRun {
    check_at: Option<Instant>,
    restart_at: Option<Instant>,
    pending: Option<Pending>,
    cancelled_pending: bool,
    watch_leave: bool,
}

impl SourceRun {
    fn start(
        &mut self,
        source: HoverSourceId,
        target: HoverTarget,
        until: Option<HoverUntil>,
    ) -> HoverRequest {
        let request = HoverRequest {
            source,
            target,
            token: next_id(),
        };
        self.pending = Some(Pending {
            request,
            until,
            deferred: false,
        });
        request
    }
}

/// Per-view hover timing. The host uses a monotonic clock and supplies a fresh
/// hit at the last pointer location each time it polls. Process each request
/// in order: complete synchronous calls immediately and defer asynchronous
/// calls. Dispatch returned effects and report each accepted view update.
/// Drop this session with its view.
#[derive(Default)]
pub struct HoverSession {
    runs: HashMap<HoverSourceId, SourceRun>,
    last_move: Option<Instant>,
}

impl HoverSession {
    fn sync(&mut self, state: &EditorState) -> Arc<Vec<Source>> {
        let sources = state
            .context()
            .facet(&SOURCES)
            .expect("valid hover sources");
        self.runs
            .retain(|id, _| sources.iter().any(|source| source.id == *id));
        for source in sources.iter() {
            self.runs.entry(source.id).or_default();
        }
        sources
    }

    /// Call after an accepted transaction or view geometry/focus update.
    /// An unfinished request becomes stale and retries from the pointer hit.
    pub fn view_updated(&mut self, state: &EditorState, now: Instant) {
        self.sync(state);
        for run in self.runs.values_mut() {
            let cancelled = std::mem::take(&mut run.cancelled_pending);
            let deferred = run.pending.as_ref().is_some_and(|pending| pending.deferred);
            if deferred {
                run.pending = None;
            }
            if cancelled || deferred {
                run.restart_at = Some(now + RESTART_TIME);
            }
        }
    }

    pub fn pointer_move(
        &mut self,
        state: &EditorState,
        now: Instant,
        pointer: HoverPointer,
    ) -> Vec<StateEffect> {
        self.last_move = Some(now);
        let mut effects = Vec::new();
        for source in self.sync(state).iter() {
            let run = self.runs.get_mut(&source.id).unwrap();
            run.check_at.get_or_insert(now + source.hover_time);
            let active = source.active(state).expect("enabled hover source");
            let first = active.tooltips.first();
            if (first.is_some() && !active.is_locked() && !pointer.over_tooltip)
                || run.pending.is_some()
            {
                let (pos, end) = if let Some(tip) = first {
                    (tip.pos, tip.end.unwrap_or(tip.pos))
                } else {
                    let pos = run.pending.as_ref().unwrap().request.target.pos;
                    (pos, pos)
                };
                let outside = if pos == end {
                    pointer.position != Some(pos)
                } else {
                    !pointer
                        .range_position
                        .is_some_and(|hit| (pos..=end).contains(&hit))
                };
                if outside {
                    effects.push(source.set.of(HoverActive::default()));
                    // Upstream dispatches before it clears the pending call;
                    // the accepted update therefore also schedules a retry.
                    run.cancelled_pending |=
                        run.pending.take().is_some_and(|pending| pending.deferred);
                }
            }
        }
        effects
    }

    /// Stop dwell checks. Like the upstream plugin, this does not cancel an
    /// unfinished source call. Report whether the pointer entered the host box.
    pub fn pointer_leave(&mut self, state: &EditorState, into_tooltip: bool) -> Vec<StateEffect> {
        let mut effects = Vec::new();
        for source in self.sync(state).iter() {
            let run = self.runs.get_mut(&source.id).unwrap();
            run.check_at = None;
            let active = source.active(state).expect("enabled hover source");
            if !active.tooltips.is_empty() && !active.is_locked() {
                if into_tooltip {
                    run.watch_leave = true;
                } else {
                    effects.push(source.set.of(HoverActive::default()));
                }
            }
        }
        effects
    }

    pub fn tooltip_leave(&mut self, state: &EditorState, into_editor: bool) -> Vec<StateEffect> {
        let mut effects = Vec::new();
        for source in self.sync(state).iter() {
            let run = self.runs.get_mut(&source.id).unwrap();
            if std::mem::take(&mut run.watch_leave) && !into_editor {
                let active = source.active(state).expect("enabled hover source");
                if !active.tooltips.is_empty() && !active.is_locked() {
                    effects.push(source.set.of(HoverActive::default()));
                }
            }
        }
        effects
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.runs
            .values()
            .flat_map(|run| [run.check_at, run.restart_at])
            .flatten()
            .min()
    }

    pub fn poll(
        &mut self,
        state: &EditorState,
        now: Instant,
        hit: Option<HoverTarget>,
    ) -> Vec<HoverRequest> {
        let mut requests = Vec::new();
        let hit = hit.filter(|hit| state.text().is_char_boundary(hit.pos));
        for source in self.sync(state).iter() {
            let run = self.runs.get_mut(&source.id).unwrap();
            let restart = run.restart_at.is_some_and(|at| now >= at);
            let mut dwell = false;
            if run.check_at.is_some_and(|at| now >= at) {
                run.check_at = None;
                let active = source.active(state).expect("enabled hover source");
                if active.tooltips.is_empty() {
                    let at = self.last_move.unwrap() + source.hover_time;
                    if now < at {
                        run.check_at = Some(at);
                    } else {
                        dwell = true;
                    }
                }
            }
            if restart || dwell {
                run.restart_at = None;
                if let Some(hit) = hit {
                    requests.push(run.start(source.id, hit, None));
                }
            }
        }
        requests
    }

    /// Start a locked request for a specific registered source, or for all
    /// sources if none matches. `until` can close it on a later transaction.
    pub fn activate_hover(
        &mut self,
        state: &EditorState,
        target: HoverTarget,
        source: Option<&HoverTooltip>,
        until: Option<HoverUntil>,
    ) -> Vec<HoverRequest> {
        let sources = self.sync(state);
        if !state.text().is_char_boundary(target.pos) {
            return Vec::new();
        }
        let selected = source
            .map(HoverTooltip::id)
            .filter(|id| self.runs.contains_key(id));
        let until = until.unwrap_or_else(|| Arc::new(|_| false));
        sources
            .iter()
            .filter(|source| selected.is_none_or(|id| source.id == id))
            .map(|source| {
                self.runs
                    .get_mut(&source.id)
                    .unwrap()
                    .start(source.id, target, Some(until.clone()))
            })
            .collect()
    }

    /// Mark an asynchronous call before continuing to the next source. Returns
    /// `false` if this request is no longer current.
    pub fn defer(&mut self, request: HoverRequest) -> bool {
        let pending = self
            .runs
            .get_mut(&request.source)
            .and_then(|run| run.pending.as_mut());
        let Some(pending) = pending.filter(|pending| pending.request == request) else {
            return false;
        };
        pending.deferred = true;
        true
    }

    /// Return an effect only for a current, nonempty result. Pass an empty
    /// collection for a null result or a source error. The host owns errors.
    /// Result anchors are in the document coordinates at the time of the request.
    pub fn complete(
        &mut self,
        state: &EditorState,
        request: HoverRequest,
        tooltips: Vec<Tooltip>,
    ) -> Option<StateEffect> {
        let sources = self.sync(state);
        let source = sources.iter().find(|source| source.id == request.source)?;
        let run = self.runs.get_mut(&request.source)?;
        if run.pending.as_ref()?.request != request {
            return None;
        }
        let pending = run.pending.take().unwrap();
        (!tooltips.is_empty()).then(|| {
            source.set.of(HoverActive {
                tooltips,
                until: pending.until,
            })
        })
    }
}
