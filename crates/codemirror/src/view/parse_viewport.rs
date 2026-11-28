use std::sync::Arc;

use crate::{
    Assoc, ChangeDesc, EditError, EditorState, History, MapMode, Text, language::lezer::ParseError,
};

pub(super) trait ViewportConfig: Sized {
    type Job: ViewportJob;

    fn active(state: &EditorState) -> Option<Arc<Self>>;
    fn viewport_job(&self, text: Text) -> Self::Job;
    fn published(state: &EditorState) -> Option<&Arc<Output<Self>>>;
    fn seed<M: Clone>(history: &mut History<M>, output: Arc<Output<Self>>);
    fn commit<M: Clone>(
        history: &mut History<M>,
        output: Arc<Output<Self>>,
    ) -> Result<ChangeDesc, EditError>;
}

pub(super) trait ViewportJob {
    type Output: ViewportOutput;

    fn cutoff_after(&self, start: usize, units: usize) -> Option<usize>;
    fn in_flight_progress(&self) -> Option<usize> {
        None
    }
    fn advance(
        &mut self,
        upto: usize,
        rounds: usize,
    ) -> Result<(usize, Option<Self::Output>), ParseError>;
}

pub(super) trait ViewportOutput {
    fn parsed_to(&self) -> usize;
    fn complete(&self) -> bool;

    fn covers(&self, upto: usize, len: usize) -> bool {
        self.parsed_to() >= upto && (upto < len || self.complete())
    }

    fn extends(&self, old: &Self) -> bool {
        (self.parsed_to(), self.complete()) > (old.parsed_to(), old.complete())
    }
}

type Output<C> = <<C as ViewportConfig>::Job as ViewportJob>::Output;
type IdleStage<C> = (ParseIdleStep, Option<Arc<Output<C>>>);

const START: usize = 3_000;
const START_ROUNDS: usize = 4_096;
const NEAR_END: usize = 1_000;
const AHEAD: usize = 100_000;

/// A caller-held idle work ticket for one view and parse generation.
#[derive(Clone, Debug)]
pub struct ParseIdleWork(pub(super) Arc<()>);

/// Work accepted by the current view during one idle call.
#[derive(Debug, Eq, PartialEq)]
pub struct ParseIdleStep {
    /// The requested coverage in raw UTF-8 bytes.
    pub target: usize,
    /// The number of live parse rounds used.
    pub rounds: usize,
    /// Whether the state now holds a new published snapshot.
    pub published: bool,
}

/// Failure to parse or to commit a ready viewport tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseViewportError {
    /// The parser stopped. The last published tree remains available.
    Parse(ParseError),
    /// A state update failed. The ready stage can be published on retry.
    Commit(EditError),
}

pub(super) struct SyntaxViewport<C: ViewportConfig> {
    pub(super) worker: Option<Worker<C>>,
    pub(super) has_geometry: bool,
}

impl<C: ViewportConfig> Default for SyntaxViewport<C> {
    fn default() -> Self {
        Self {
            worker: None,
            has_geometry: false,
        }
    }
}

impl<C: ViewportConfig> SyntaxViewport<C> {
    pub(super) fn sync<M: Clone>(
        &mut self,
        history: &mut History<M>,
        previous: Option<&EditorState>,
        changes: Option<&ChangeDesc>,
        viewport_end: usize,
    ) {
        let state = history.state();
        let Some(config) = C::active(state) else {
            self.worker = None;
            return;
        };
        let current = self
            .worker
            .as_ref()
            .filter(|worker| Arc::ptr_eq(&worker.config, &config));
        if current.is_some() && changes.is_none() {
            return;
        }
        let restore = current.zip(changes).and_then(|(worker, changes)| {
            let mapped = |at, assoc| changes.map_pos(at, assoc, MapMode::Simple);
            let output = previous
                .and_then(C::published)
                .into_iter()
                .chain(worker.staged.as_ref())
                .max_by_key(|output| (output.parsed_to(), output.complete()));
            if output.is_some_and(|out| out.complete()) {
                return Some(state.text().len());
            }
            let in_flight = worker
                .job
                .as_ref()
                .and_then(ViewportJob::in_flight_progress);
            let through = output
                .map(|out| out.parsed_to())
                .max(in_flight)
                .and_then(|at| mapped(at, Assoc::Before));
            let visible = self
                .has_geometry
                .then(|| mapped(viewport_end, Assoc::After))
                .flatten();
            through.into_iter().chain(visible).max()
        });
        let incoming = C::published(state);
        let mut worker = Worker::new(
            config,
            state.text(),
            incoming.is_some_and(|out| out.complete()),
            restore,
        );
        if incoming.is_none()
            && let Some(initial) = worker.bootstrap()
        {
            C::seed(history, initial);
        }
        self.worker = Some(worker);
    }
    pub(super) fn ensure(
        &mut self,
        upto: usize,
        rounds: usize,
        state: &EditorState,
    ) -> Result<Option<Arc<Output<C>>>, ParseError> {
        if !state.text().is_char_boundary(upto) {
            return Ok(None);
        }
        let published =
            C::published(state).filter(|output| output.covers(upto, state.text().len()));
        let Some(worker) = self.worker.as_mut() else {
            return Ok(published.cloned());
        };
        if published.is_none() {
            worker.advance(upto, rounds)?;
        }
        Ok(worker
            .ready(upto)
            .filter(|stage| published.is_none_or(|out| stage.extends(out)))
            .or(published)
            .cloned())
    }

    pub(super) fn publish<M: Clone>(
        &mut self,
        output: Arc<Output<C>>,
        history: &mut History<M>,
    ) -> Result<Option<(EditorState, ChangeDesc)>, ParseViewportError> {
        let Some(worker) = self.worker.as_mut() else {
            return Ok(None);
        };
        if !worker
            .staged
            .as_ref()
            .is_some_and(|stage| Arc::ptr_eq(stage, &output))
            || C::published(history.state()).is_some_and(|old| !output.extends(old))
        {
            return Ok(None);
        }
        let previous = history.state().clone();
        let changes = C::commit(history, output.clone()).map_err(|error| {
            worker.retry = true;
            ParseViewportError::Commit(error)
        })?;
        worker.did_publish(&output);
        Ok(Some((previous, changes)))
    }

    pub(super) fn idle_work(
        &self,
        state: &EditorState,
        viewport_end: usize,
    ) -> Option<ParseIdleWork> {
        let worker = self.worker.as_ref()?;
        if !self.has_geometry {
            return None;
        }
        worker.idle_target(viewport_end, C::published(state).map(Arc::as_ref))?;
        Some(worker.generation.clone())
    }

    pub(super) fn idle_stage(
        &mut self,
        work: &ParseIdleWork,
        rounds: usize,
        state: &EditorState,
        viewport_end: usize,
    ) -> Result<Option<IdleStage<C>>, ParseViewportError> {
        let Some(worker) = self
            .worker
            .as_mut()
            .filter(|worker| self.has_geometry && Arc::ptr_eq(&worker.generation.0, &work.0))
        else {
            return Ok(None);
        };
        let Some(target) = worker.idle_target(viewport_end, C::published(state).map(Arc::as_ref))
        else {
            return Ok(None);
        };
        let rounds = worker
            .advance(target, rounds)
            .map_err(ParseViewportError::Parse)?;
        Ok(Some((
            ParseIdleStep {
                target,
                rounds,
                published: false,
            },
            worker.ready(target).cloned(),
        )))
    }
}

pub(super) struct Worker<C: ViewportConfig> {
    pub(super) config: Arc<C>,
    pub(super) generation: ParseIdleWork,
    len: usize,
    pub(super) job: Option<C::Job>,
    restore: Option<usize>,
    pub(super) staged: Option<Arc<Output<C>>>,
    pub(super) retry: bool,
    pub(super) error: Option<ParseError>,
}

impl<C: ViewportConfig> Worker<C> {
    pub(super) fn new(config: Arc<C>, text: &Text, complete: bool, restore: Option<usize>) -> Self {
        let job = (!complete).then(|| config.viewport_job(text.clone()));
        Self {
            config,
            generation: ParseIdleWork(Arc::new(())),
            len: text.len(),
            job,
            restore,
            staged: None,
            retry: false,
            error: None,
        }
    }

    pub(super) fn bootstrap(&mut self) -> Option<Arc<Output<C>>> {
        let start = self
            .job
            .as_ref()?
            .cutoff_after(0, START)
            .unwrap_or(self.len);
        let first = self.restore.map_or(start, |through| through.min(start));
        let rounds = self.advance(first, START_ROUNDS).ok()?;
        let mut output = self.ready(first).cloned();
        if let Some(through) = self.restore.filter(|through| *through > first)
            && self.advance(through, START_ROUNDS - rounds).is_ok()
            && let Some(stage) = self.ready(through)
        {
            output = Some(stage.clone());
        }
        if let Some(output) = &output {
            self.did_publish(output);
        }
        output
    }

    pub(super) fn ready(&self, upto: usize) -> Option<&Arc<Output<C>>> {
        self.staged
            .as_ref()
            .filter(|out| out.covers(upto, self.len))
    }

    pub(super) fn advance(&mut self, upto: usize, rounds: usize) -> Result<usize, ParseError> {
        if self.ready(upto).is_some() {
            return Ok(0);
        }
        let Some(job) = &mut self.job else {
            return self.error.map_or(Ok(0), Err);
        };
        match job.advance(upto, rounds) {
            Ok((rounds, output)) => {
                if let Some(output) = output {
                    if output.complete() {
                        self.job = None;
                    }
                    self.staged = Some(Arc::new(output));
                }
                Ok(rounds)
            }
            Err(error) => {
                self.error = Some(error);
                self.job = None;
                Err(error)
            }
        }
    }

    pub(super) fn did_publish(&mut self, output: &Output<C>) {
        self.staged = None;
        self.retry = false;
        if self
            .restore
            .is_some_and(|through| output.covers(through, self.len))
        {
            self.restore = None;
        }
    }

    pub(super) fn idle_target(
        &self,
        viewport_end: usize,
        published: Option<&Output<C>>,
    ) -> Option<usize> {
        if self.error.is_some() {
            return None;
        }
        if self.retry {
            return self.staged.as_ref().map(|out| out.parsed_to());
        }
        if let Some(restore) = self.restore {
            return Some(restore.max(viewport_end));
        }
        let covers = |at| published.is_some_and(|out| out.covers(at, self.len));
        let target = match &self.job {
            Some(job)
                if !covers(viewport_end)
                    && self.ready(viewport_end).is_none()
                    && job.cutoff_after(viewport_end, NEAR_END).is_some() =>
            {
                viewport_end
            }
            Some(job) => job.cutoff_after(viewport_end, AHEAD).unwrap_or(self.len),
            None => self.len,
        };
        (!covers(target)
            || self
                .ready(target)
                .is_some_and(|stage| published.is_none_or(|out| stage.extends(out))))
        .then_some(target)
    }
}
