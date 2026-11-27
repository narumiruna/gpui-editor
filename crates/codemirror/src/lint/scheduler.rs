use std::{
    ops::Range,
    sync::LazyLock,
    time::{Duration, Instant},
};

use crate::{EditorState, StateEffectType, Text, Transaction};

use super::{Diagnostic, DocumentVersion, LintState};

/// The delay used by [`LintScheduler::new`].
pub const DEFAULT_LINT_DELAY: Duration = Duration::from_millis(750);

/// A document snapshot to send to a diagnostic worker.
#[derive(Clone, Debug)]
pub struct LintRun {
    text: Text,
    version: DocumentVersion,
    scope: PublicationScope,
}

impl LintRun {
    /// Capture the current document, or return `None` when lint state is disabled.
    pub fn start(state: &EditorState) -> Option<Self> {
        Some(Self {
            text: state.text().clone(),
            version: LintState::get(state)?.version.clone(),
            scope: PublicationScope::Document,
        })
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    /// Keep only valid UTF-8 ranges inside `range`, before and after filters.
    /// Offsets are absolute in the captured text. An invalid scope admits no diagnostics.
    pub fn restrict_to(&mut self, range: Range<usize>) {
        self.scope = if self.text.is_valid_range(range.clone()) {
            PublicationScope::Range(range)
        } else {
            PublicationScope::Discard
        };
    }

    /// Clear this snapshot's diagnostics without running publication filters.
    pub fn discard(mut self) -> Transaction {
        self.scope = PublicationScope::Discard;
        self.finish(Vec::new())
    }

    /// Build a publication transaction. The lint field ignores it if the
    /// document changed before it is applied, even if the text was restored.
    /// Selection-only updates do not make results stale. Runs of the same
    /// document version can publish in completion order.
    pub fn finish(self, mut diagnostics: Vec<Diagnostic>) -> Transaction {
        if !matches!(self.scope, PublicationScope::Document) {
            diagnostics.retain(|diagnostic| {
                self.scope.accepts(&diagnostic.range)
                    && self.text.is_valid_range(diagnostic.range.clone())
            });
        }
        Transaction::default().with_effects([PUBLISH_DIAGNOSTICS.of(Publication {
            version: self.version,
            diagnostics,
            scope: self.scope,
        })])
    }
}

#[derive(Clone, Debug)]
pub(super) enum PublicationScope {
    Document,
    Range(Range<usize>),
    Discard,
}

impl PublicationScope {
    pub(super) fn accepts(&self, range: &Range<usize>) -> bool {
        match self {
            Self::Document => true,
            Self::Range(scope) => scope.start <= range.start && range.end <= scope.end,
            Self::Discard => false,
        }
    }
}

pub(super) struct Publication {
    pub(super) version: DocumentVersion,
    pub(super) diagnostics: Vec<Diagnostic>,
    pub(super) scope: PublicationScope,
}

pub(super) static PUBLISH_DIAGNOSTICS: LazyLock<StateEffectType<Publication>> =
    LazyLock::new(StateEffectType::new);

/// A caller-driven debounce timer. The caller runs workers and passes their
/// results to [`LintRun::finish`]. No background timer or executor is required.
#[derive(Debug)]
pub struct LintScheduler {
    delay: Duration,
    observed: Option<DocumentVersion>,
    pending_since: Option<Instant>,
}

impl LintScheduler {
    pub fn new(state: &EditorState, now: Instant) -> Self {
        Self::with_delay(state, now, DEFAULT_LINT_DELAY)
    }

    pub fn with_delay(state: &EditorState, now: Instant, delay: Duration) -> Self {
        let mut scheduler = Self {
            delay,
            observed: None,
            pending_since: None,
        };
        scheduler.observe(state, now);
        scheduler
    }

    /// Call after each editor update to reset the timer on document changes.
    /// Disabling the lint extension clears any pending work.
    pub fn observe(&mut self, state: &EditorState, now: Instant) {
        if let Some(lint) = LintState::get(state) {
            if self
                .observed
                .as_ref()
                .is_none_or(|old| !old.matches(&lint.version))
            {
                self.observed = Some(lint.version.clone());
                self.pending_since = Some(now);
            }
        } else {
            self.observed = None;
            self.pending_since = None;
        }
    }

    /// Request a run after the delay without changing the document. For
    /// example, call this after a source or its configuration changes.
    pub fn refresh(&mut self, now: Instant) {
        if self.observed.is_some() {
            self.pending_since = Some(now);
        }
    }

    /// Take a pending run only when the configured delay has elapsed.
    pub fn take_due(&mut self, state: &EditorState, now: Instant) -> Option<LintRun> {
        self.observe(state, now);
        if now.saturating_duration_since(self.pending_since?) < self.delay {
            return None;
        }
        self.pending_since = None;
        LintRun::start(state)
    }

    /// Take a pending run immediately. Does not schedule an additional run.
    pub fn force(&mut self, state: &EditorState, now: Instant) -> Option<LintRun> {
        self.observe(state, now);
        self.pending_since.take()?;
        LintRun::start(state)
    }
}
