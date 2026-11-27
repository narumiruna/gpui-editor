//! Caller-driven WAST parsing. Jobs and stages are view-local; published
//! trees are part of the editor state, separate from synchronous syntax.

use std::{ops::Range, sync::Arc};

use crate::{
    Assoc, ChangeDesc, EditError, EditorState, History, MapMode, Text,
    language::{
        HighlightSpan, LanguageData,
        lezer::ParseError,
        wast::{WastParseJob, WastParseOutput},
    },
    language_data::{WastConfig, active_wast_viewport},
};

use super::EditorView;

#[cfg(test)]
mod tests;

const START: usize = 3_000;
const START_ROUNDS: usize = 4_096;
const NEAR_END: usize = 1_000;
const AHEAD: usize = 100_000;

/// A caller-held idle work ticket for one view and parse generation.
#[derive(Clone, Debug)]
pub struct WastIdleWork(Arc<()>);

/// Work accepted by the current view during one idle call.
#[derive(Debug, Eq, PartialEq)]
pub struct WastIdleStep {
    /// The requested coverage in raw UTF-8 bytes.
    pub target: usize,
    /// The number of live parse rounds used.
    pub rounds: usize,
    /// Whether the state now holds a new published snapshot.
    pub published: bool,
}

/// Failure to parse or to commit a ready WAST viewport tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WastViewportError {
    /// The parser stopped. The last published tree remains available.
    Parse(ParseError),
    /// A state update failed. The ready stage can be published on retry.
    Commit(EditError),
}

#[derive(Default)]
pub(super) struct WastViewport {
    worker: Option<Worker>,
    pub(super) has_geometry: bool,
}

impl WastViewport {
    pub(super) fn sync<M: Clone>(
        &mut self,
        history: &mut History<M>,
        previous: Option<&EditorState>,
        changes: Option<&ChangeDesc>,
        viewport_end: usize,
    ) {
        let state = history.state();
        let Some(config) = active_wast_viewport(&state.context()) else {
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
                .and_then(|state| state.wast_publication.as_ref())
                .map(|published| &published.output)
                .into_iter()
                .chain(worker.staged.as_ref())
                .max_by_key(|output| (output.parsed_to, output.complete));
            if output.is_some_and(|out| out.complete) {
                return Some(state.text().len());
            }
            let through = output.and_then(|out| mapped(out.parsed_to, Assoc::Before));
            let visible = self
                .has_geometry
                .then(|| mapped(viewport_end, Assoc::After))
                .flatten();
            through.into_iter().chain(visible).max()
        });
        let incoming = state.wast_publication.as_ref().map(|slot| &slot.output);
        let mut worker = Worker::new(
            config,
            state.text(),
            incoming.is_some_and(|out| out.complete),
            restore,
        );
        if incoming.is_none()
            && let Some(initial) = worker.bootstrap()
        {
            history.seed_wast_publication(initial);
        }
        self.worker = Some(worker);
    }
}

struct Worker {
    config: Arc<WastConfig>,
    generation: WastIdleWork,
    len: usize,
    job: Option<WastParseJob>,
    restore: Option<usize>,
    staged: Option<Arc<WastParseOutput>>,
    retry: bool,
    error: Option<ParseError>,
}

impl Worker {
    fn new(config: Arc<WastConfig>, text: &Text, complete: bool, restore: Option<usize>) -> Self {
        let job = (!complete).then(|| config.viewport_job(text.clone()));
        Self {
            config,
            generation: WastIdleWork(Arc::new(())),
            len: text.len(),
            job,
            restore,
            staged: None,
            retry: false,
            error: None,
        }
    }

    fn bootstrap(&mut self) -> Option<Arc<WastParseOutput>> {
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

    fn ready(&self, upto: usize) -> Option<&Arc<WastParseOutput>> {
        self.staged
            .as_ref()
            .filter(|out| out.covers(upto, self.len))
    }

    fn advance(&mut self, upto: usize, rounds: usize) -> Result<usize, ParseError> {
        if self.ready(upto).is_some() {
            return Ok(0);
        }
        let Some(job) = &mut self.job else {
            return self.error.map_or(Ok(0), Err);
        };
        match job.advance(upto, rounds) {
            Ok(step) => {
                if let Some(output) = step.output {
                    if output.complete {
                        self.job = None;
                    }
                    self.staged = Some(Arc::new(output));
                }
                Ok(step.rounds)
            }
            Err(error) => {
                self.error = Some(error);
                self.job = None;
                Err(error)
            }
        }
    }

    fn did_publish(&mut self, output: &WastParseOutput) {
        self.staged = None;
        self.retry = false;
        if self
            .restore
            .is_some_and(|through| output.covers(through, self.len))
        {
            self.restore = None;
        }
    }

    fn idle_target(
        &self,
        viewport_end: usize,
        published: Option<&WastParseOutput>,
    ) -> Option<usize> {
        if self.error.is_some() {
            return None;
        }
        if self.retry {
            return self.staged.as_ref().map(|out| out.parsed_to);
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

impl<M: Clone> EditorView<M> {
    /// Read the state's published WAST viewport tree, not the working stage.
    pub fn wast_syntax_tree(&self, upto: usize) -> Option<Arc<WastParseOutput>> {
        self.state().wast_syntax_tree(upto)
    }

    /// Highlight a raw UTF-8 range using the published viewport tree. Return
    /// `None` for invalid boundaries or if the published prefix does not cover
    /// the full range. Staged trees and token overrun do not extend coverage.
    pub fn wast_viewport_highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        Some(
            self.state()
                .wast_publication_for_range(&range)?
                .highlights(range),
        )
    }

    /// WAST language data in facet order at an original UTF-8 byte boundary.
    /// Positional rules read only the published tree, not staged output. A
    /// token in that tree can extend past its reported coverage. Beyond its
    /// end, predicates receive the root node. Invalid offsets, including CRLF
    /// interiors, or inactive viewport parsers return an empty list. It does not
    /// include independently installed language-data providers.
    pub fn wast_viewport_data_at(&self, position: usize, side: Assoc) -> Vec<Arc<LanguageData>> {
        self.state().wast_viewport_data(position, side)
    }

    /// Check staged or published coverage without parsing or publishing.
    /// EOF requires a complete parse. Positions use raw UTF-8 byte boundaries.
    pub fn wast_syntax_tree_available(&self, upto: usize) -> bool {
        self.state().text().is_char_boundary(upto)
            && (self
                .wast_viewport
                .worker
                .as_ref()
                .is_some_and(|worker| worker.ready(upto).is_some())
                || self.wast_syntax_tree(upto).is_some())
    }

    /// Return a ready tree or try to stage a tree through `upto` without
    /// publishing. Use raw UTF-8 byte boundaries. Return `None` if no viewport
    /// parser is installed or coverage is not ready. The round quota is not a
    /// time limit.
    pub fn ensure_wast_syntax_tree(
        &mut self,
        upto: usize,
        rounds: usize,
    ) -> Result<Option<Arc<WastParseOutput>>, ParseError> {
        if !self.state().text().is_char_boundary(upto) {
            return Ok(None);
        }
        let published = self.wast_syntax_tree(upto);
        let Some(worker) = self.wast_viewport.worker.as_mut() else {
            return Ok(published);
        };
        if published.is_none() {
            worker.advance(upto, rounds)?;
        }
        Ok(worker
            .ready(upto)
            .filter(|stage| published.as_ref().is_none_or(|out| stage.extends(out)))
            .cloned()
            .or(published))
    }

    /// Ensure coverage and commit a newer ready snapshot, if any. A zero
    /// round quota can commit an existing stage. Return whether the requested
    /// prefix is published. A commit error keeps both the old state and stage.
    pub fn force_wast_parsing(
        &mut self,
        upto: usize,
        rounds: usize,
    ) -> Result<bool, WastViewportError> {
        let Some(output) = self
            .ensure_wast_syntax_tree(upto, rounds)
            .map_err(WastViewportError::Parse)?
        else {
            return Ok(false);
        };
        self.publish_wast_stage(output)?;
        Ok(true)
    }

    fn publish_wast_stage(
        &mut self,
        output: Arc<WastParseOutput>,
    ) -> Result<bool, WastViewportError> {
        let Some(worker) = self.wast_viewport.worker.as_mut() else {
            return Ok(false);
        };
        if !worker
            .staged
            .as_ref()
            .is_some_and(|stage| Arc::ptr_eq(stage, &output))
            || self
                .history
                .state()
                .wast_publication
                .as_ref()
                .is_some_and(|old| !output.extends(&old.output))
        {
            return Ok(false);
        }
        let previous = self.history.state().clone();
        let changes = self
            .history
            .commit_wast_publication(output.clone())
            .map_err(|error| {
                worker.retry = true;
                WastViewportError::Commit(error)
            })?;
        worker.did_publish(&output);
        self.notify(
            &previous,
            &changes,
            &self.viewport.clone(),
            false,
            false,
            &[],
        );
        Ok(true)
    }

    /// Get an idle ticket only when reported geometry leaves work to do.
    /// The caller is responsible for scheduling and painting new output.
    pub fn wast_idle_work(&self) -> Option<WastIdleWork> {
        let worker = self.wast_viewport.worker.as_ref()?;
        if !self.wast_viewport.has_geometry {
            return None;
        }
        self.wast_idle_target(worker)?;
        Some(worker.generation.clone())
    }

    fn wast_idle_target(&self, worker: &Worker) -> Option<usize> {
        worker.idle_target(
            self.viewport.interval().end,
            self.state()
                .wast_publication
                .as_ref()
                .map(|slot| slot.output.as_ref()),
        )
    }

    /// Work toward the current measured viewport, then ahead of it in a
    /// later call. Run at most `rounds` live rounds, plus staging or EOF
    /// output work. A ready stage that failed to commit is retried before
    /// advancing the parser. Return `None` for stale work or when no work is due.
    pub fn run_wast_idle(
        &mut self,
        work: &WastIdleWork,
        rounds: usize,
    ) -> Result<Option<WastIdleStep>, WastViewportError> {
        let Some(worker) = self.wast_viewport.worker.as_ref().filter(|worker| {
            self.wast_viewport.has_geometry && Arc::ptr_eq(&worker.generation.0, &work.0)
        }) else {
            return Ok(None);
        };
        let Some(target) = self.wast_idle_target(worker) else {
            return Ok(None);
        };
        let worker = self.wast_viewport.worker.as_mut().expect("current worker");
        let rounds = worker
            .advance(target, rounds)
            .map_err(WastViewportError::Parse)?;
        let published = match worker.ready(target).cloned() {
            Some(output) => self.publish_wast_stage(output)?,
            None => false,
        };
        Ok(Some(WastIdleStep {
            target,
            rounds,
            published,
        }))
    }

    /// The parse error that stopped the current generation, if any. The
    /// last published prefix stays available. There is no fallback parse.
    pub fn wast_parse_error(&self) -> Option<ParseError> {
        self.wast_viewport.worker.as_ref()?.error
    }
}
