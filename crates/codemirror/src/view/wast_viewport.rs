//! Caller-driven WAST parsing. Jobs and stages are view-local; published
//! trees are part of the editor state, separate from synchronous syntax.

use std::{ops::Range, sync::Arc};

use crate::{
    Assoc, ChangeDesc, EditError, EditorState, History, Text,
    language::{
        HighlightSpan, LanguageData,
        lezer::ParseError,
        wast::{WastParseJob, WastParseOutput},
    },
    language_data::{WastConfig, active_wast_viewport},
};

use super::{
    EditorView,
    parse_viewport::{SyntaxViewport, ViewportConfig, ViewportJob, ViewportOutput},
};

#[cfg(test)]
mod tests;

pub use super::parse_viewport::{
    ParseIdleStep as WastIdleStep, ParseIdleWork as WastIdleWork,
    ParseViewportError as WastViewportError,
};

impl ViewportConfig for WastConfig {
    type Job = WastParseJob;

    fn active(state: &EditorState) -> Option<Arc<Self>> {
        active_wast_viewport(&state.context())
    }

    fn published(state: &EditorState) -> Option<&Arc<WastParseOutput>> {
        state.wast_publication.as_ref().map(|slot| &slot.output)
    }

    fn seed<M: Clone>(history: &mut History<M>, output: Arc<WastParseOutput>) {
        history.seed_wast_publication(output);
    }

    fn commit<M: Clone>(
        history: &mut History<M>,
        output: Arc<WastParseOutput>,
    ) -> Result<ChangeDesc, EditError> {
        history.commit_wast_publication(output)
    }

    fn viewport_job(&self, text: Text) -> Self::Job {
        WastConfig::viewport_job(self, text)
    }
}

impl ViewportJob for WastParseJob {
    type Output = WastParseOutput;

    fn cutoff_after(&self, start: usize, units: usize) -> Option<usize> {
        WastParseJob::cutoff_after(self, start, units)
    }

    fn in_flight_progress(&self) -> Option<usize> {
        WastParseJob::in_flight_progress(self)
    }

    fn advance(
        &mut self,
        upto: usize,
        rounds: usize,
    ) -> Result<(usize, Option<Self::Output>), ParseError> {
        let step = WastParseJob::advance(self, upto, rounds)?;
        Ok((step.rounds, step.output))
    }
}

impl ViewportOutput for WastParseOutput {
    fn parsed_to(&self) -> usize {
        self.parsed_to
    }
    fn complete(&self) -> bool {
        self.complete
    }
}

pub(super) type WastViewport = SyntaxViewport<WastConfig>;

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
        self.wast_viewport
            .ensure(upto, rounds, self.history.state())
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
        let Some((previous, changes)) = self.wast_viewport.publish(output, &mut self.history)?
        else {
            return Ok(false);
        };
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
        self.wast_viewport
            .idle_work(self.state(), self.viewport.interval().end)
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
        let Some((mut step, output)) = self.wast_viewport.idle_stage(
            work,
            rounds,
            self.history.state(),
            self.viewport.interval().end,
        )?
        else {
            return Ok(None);
        };
        if let Some(output) = output {
            step.published = self.publish_wast_stage(output)?;
        }
        Ok(Some(step))
    }

    /// The parse error that stopped the current generation, if any. The
    /// last published prefix stays available. There is no fallback parse.
    pub fn wast_parse_error(&self) -> Option<ParseError> {
        self.wast_viewport.worker.as_ref()?.error
    }
}
