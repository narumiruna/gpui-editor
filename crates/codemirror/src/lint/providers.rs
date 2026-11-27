use std::{
    future::{Future, ready},
    ops::Range,
    pin::Pin,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use crate::{EditorState, Extension, Facet, StateContext, Transaction, view::ViewUpdate};

use super::{
    DEFAULT_LINT_DELAY, Diagnostic, DocumentVersion, LintRun, LintState, LintTooltipHide, extension,
};

const BATCH_DELAY: Duration = Duration::from_millis(200);
const EARLY_WAKE: Duration = Duration::from_millis(10);

pub type LintResult = Result<Vec<Diagnostic>, String>;
pub type LintFuture = Pin<Box<dyn Future<Output = LintResult> + Send>>;
type RunSource = dyn Fn(&EditorState) -> LintFuture + Send + Sync;

/// A provider called with the state at the start of a run.
#[derive(Clone)]
pub struct LintSource(Arc<RunSource>);

impl LintSource {
    pub fn new(run: impl Fn(&EditorState) -> Vec<Diagnostic> + Send + Sync + 'static) -> Self {
        Self::asynchronous(move |state| ready(Ok(run(state))))
    }

    /// Copy anything the future needs from the state before returning it.
    pub fn asynchronous<F>(run: impl Fn(&EditorState) -> F + Send + Sync + 'static) -> Self
    where
        F: Future<Output = LintResult> + Send + 'static,
    {
        Self(Arc::new(move |state| Box::pin(run(state))))
    }
}

/// A list filter. It can remove, reorder, or replace diagnostics.
/// Do not read the field that is applying the filter from within the callback.
pub type DiagnosticFilter =
    Arc<dyn Fn(Vec<Diagnostic>, &StateContext<'_>) -> Vec<Diagnostic> + Send + Sync>;
pub type NeedsRefresh = Arc<dyn Fn(&ViewUpdate<'_>) -> bool + Send + Sync>;

/// Settings shared by all configured providers.
#[derive(Clone, Default)]
pub struct LintConfig {
    /// Use the longest explicit delay, or 750 ms if none is specified.
    pub delay: Option<Duration>,
    /// Schedule a run if any predicate accepts the view update.
    pub needs_refresh: Option<NeedsRefresh>,
    /// Apply configured list filters in extension order, before native predicates.
    pub marker_filter: Option<DiagnosticFilter>,
    /// Filter the hover list without changing the stored diagnostics or markers.
    pub tooltip_filter: Option<DiagnosticFilter>,
    /// Override lint hover dismissal for an accepted transaction.
    /// Return `None` to use the default policy.
    pub hide_on: Option<LintTooltipHide>,
    /// Open on nonempty publications and close when the diagnostics are cleared.
    pub auto_panel: bool,
}

pub(super) struct LintInput {
    source: Option<LintSource>,
    pub(super) config: LintConfig,
}

type LintInputs = Facet<Arc<LintInput>, Vec<Arc<LintInput>>>;
pub(super) static LINTERS: LazyLock<LintInputs> = LazyLock::new(Facet::collect);

/// The registered linter inputs, in extension precedence order. Equality
/// compares the identity of the registrations, not their callbacks or values.
#[derive(Clone)]
pub struct LintConfiguration(Arc<Vec<Arc<LintInput>>>);

impl LintConfiguration {
    pub fn get(state: &EditorState) -> Self {
        Self(
            state
                .context()
                .facet(&LINTERS)
                .expect("valid lint configuration"),
        )
    }

    pub fn has_sources(&self) -> bool {
        self.0.iter().any(|input| input.source.is_some())
    }
}

impl PartialEq for LintConfiguration {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || self.0.len() == other.0.len()
                && self
                    .0
                    .iter()
                    .zip(other.0.iter())
                    .all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

impl Eq for LintConfiguration {}

pub(super) fn combine_callbacks<'a, T: ?Sized + 'a>(
    mut callbacks: impl Iterator<Item = &'a Arc<T>>,
) -> impl Iterator<Item = &'a Arc<T>> {
    // Upstream skips identical inputs until the first combination creates a
    // different callback. After that, keep the rest in order.
    let first = callbacks.next();
    first.into_iter().chain(
        callbacks
            .skip_while(move |callback| first.is_some_and(|first| Arc::ptr_eq(first, callback))),
    )
}

/// Install lint state and register a provider. `None` only adds configuration.
/// Use a [`LintSession`] to drive the registered providers without a runtime.
pub fn linter(source: Option<LintSource>, config: LintConfig) -> Extension {
    Extension::group([
        extension(),
        LINTERS.of(Arc::new(LintInput { source, config })),
    ])
}

#[derive(Clone, Copy)]
struct Timer {
    since: Instant,
    delay: Duration,
}

impl Timer {
    fn remaining(self, now: Instant) -> Duration {
        self.delay
            .saturating_sub(now.saturating_duration_since(self.since))
    }
}

struct Pending {
    wake: Timer,
    due: Timer,
}

struct Observed {
    version: DocumentVersion,
    configuration: LintConfiguration,
}

/// The caller-driven scheduler for the [`linter`] extensions of one editor.
/// The host feeds updates, wakes the scheduler, and executes the returned
/// futures. A run can complete while another run for the same editor is pending.
#[derive(Default)]
pub struct LintSession {
    observed: Option<Observed>,
    pending: Option<Pending>,
    fallback: Option<LintSource>,
}

impl LintSession {
    /// Use a host source only when no provider is registered. The configured
    /// delay and refresh predicates also apply to the fallback.
    pub fn with_fallback(source: LintSource) -> Self {
        Self {
            fallback: Some(source),
            ..Self::default()
        }
    }

    pub fn new(state: &EditorState, now: Instant) -> Self {
        let mut session = Self::default();
        session.observe(state, now);
        session
    }

    /// Call after each editor update to track document and configuration changes.
    /// Use [`Self::observe_update`] instead if using `needs_refresh` predicates.
    pub fn observe(&mut self, state: &EditorState, now: Instant) {
        self.observe_inner(state, now, None);
    }

    /// Observe a native view update, including non-document refresh requests.
    pub fn observe_update(&mut self, update: &ViewUpdate<'_>, now: Instant) {
        self.observe_inner(update.state, now, Some(update));
    }

    fn observe_inner(
        &mut self,
        state: &EditorState,
        now: Instant,
        update: Option<&ViewUpdate<'_>>,
    ) {
        let configuration = LintConfiguration::get(state);
        let inputs = &configuration.0;
        let Some(lint) =
            LintState::get(state).filter(|_| !inputs.is_empty() || self.fallback.is_some())
        else {
            self.observed = None;
            self.pending = None;
            return;
        };
        let changed = self.observed.as_ref().is_none_or(|old| {
            !old.version.matches(&lint.version) || old.configuration != configuration
        });
        if changed
            || update.is_some_and(|update| {
                combine_callbacks(
                    inputs
                        .iter()
                        .filter_map(|input| input.config.needs_refresh.as_ref()),
                )
                .any(|refresh| refresh(update))
            })
        {
            let delay = inputs
                .iter()
                .filter_map(|input| input.config.delay)
                .max()
                .unwrap_or(DEFAULT_LINT_DELAY);
            let due = Timer { since: now, delay };
            if let Some(pending) = &mut self.pending {
                pending.due = due;
            } else {
                self.pending = Some(Pending { wake: due, due });
            }
        }
        self.observed = Some(Observed {
            version: lint.version.clone(),
            configuration,
        });
    }

    /// Time until the next scheduler wake, if one is pending.
    pub fn wait_time(&self, now: Instant) -> Option<Duration> {
        Some(self.pending.as_ref()?.wake.remaining(now))
    }

    /// On a scheduled wake, start due work and return the futures in provider
    /// order. Deliver each result to the batch with its index in this list.
    pub fn take_due(
        &mut self,
        state: &EditorState,
        now: Instant,
    ) -> Option<(LintBatch, Vec<LintFuture>)> {
        self.observe(state, now);
        let pending = self.pending.as_mut()?;
        if !pending.wake.remaining(now).is_zero() {
            return None;
        }
        if pending.due.remaining(now) > EARLY_WAKE {
            pending.wake = pending.due;
            return None;
        }
        self.start(state)
    }

    fn start(&mut self, state: &EditorState) -> Option<(LintBatch, Vec<LintFuture>)> {
        self.pending.take()?;
        let run = LintRun::start(state)?;
        let configuration = &self.observed.as_ref()?.configuration;
        let futures: Vec<_> = if configuration.has_sources() {
            configuration
                .0
                .iter()
                .filter_map(|input| Some((input.source.as_ref()?.0)(state)))
                .collect()
        } else {
            vec![(self.fallback.as_ref()?.0)(state)]
        };
        let remaining = futures.len();
        Some((
            LintBatch {
                run,
                finished: vec![false; remaining],
                remaining,
                diagnostics: Vec::new(),
                timer: None,
            },
            futures,
        ))
    }
}

/// Start pending work immediately. Do not start another run if none is pending.
pub fn force_linting(
    session: &mut LintSession,
    state: &EditorState,
    now: Instant,
) -> Option<(LintBatch, Vec<LintFuture>)> {
    session.observe(state, now);
    session.start(state)
}

/// Coalesce results from one run in completion order. Publish immediately when
/// every provider succeeds, or after a 200 ms gap between successful results.
/// Later results can publish a new batch that includes the earlier results.
pub struct LintBatch {
    run: LintRun,
    finished: Vec<bool>,
    remaining: usize,
    diagnostics: Vec<Diagnostic>,
    timer: Option<Timer>,
}

impl LintBatch {
    /// Limit each publication to this range in the run's document snapshot.
    /// See [`LintRun::restrict_to`].
    pub fn restrict_to(&mut self, range: Range<usize>) {
        self.run.restrict_to(range);
    }

    /// Report a completed future. An invalid or duplicate index is ignored.
    /// A provider error is returned to the host and does not reset the timer.
    /// Deliver results and timer wakes in event order.
    pub fn complete(
        &mut self,
        index: usize,
        result: LintResult,
        now: Instant,
    ) -> Result<Option<Transaction>, String> {
        let Some(finished) = self.finished.get_mut(index) else {
            return Ok(None);
        };
        if std::mem::replace(finished, true) {
            return Ok(None);
        }
        self.diagnostics.extend(result?);
        self.remaining -= 1;
        if self.remaining == 0 {
            self.timer = None;
            Ok(Some(
                self.run
                    .clone()
                    .finish(std::mem::take(&mut self.diagnostics)),
            ))
        } else {
            self.timer = Some(Timer {
                since: now,
                delay: BATCH_DELAY,
            });
            Ok(None)
        }
    }

    pub fn wait_time(&self, now: Instant) -> Option<Duration> {
        Some(self.timer?.remaining(now))
    }

    /// Take the partial publication when the batch timer fires.
    pub fn take_due(&mut self, now: Instant) -> Option<Transaction> {
        if !self.wait_time(now)?.is_zero() {
            return None;
        }
        self.timer = None;
        Some(self.run.clone().finish(self.diagnostics.clone()))
    }
}
