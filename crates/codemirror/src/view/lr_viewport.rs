//! Caller-driven JSON and Lezer grammar viewport parsing.

use std::{ops::Range, sync::Arc};

use crate::{
    ChangeDesc, EditError, EditorState, History, Text,
    language::{
        HighlightSpan,
        lezer::{LrParseJob, LrParseOutput, ParseError},
    },
    language_data::{LrConfig, NativeLanguage, active_lr_viewport},
};

use super::{
    EditorView,
    parse_viewport::{
        ParseIdleStep, ParseIdleWork, ParseViewportError, SyntaxViewport, ViewportConfig,
        ViewportJob, ViewportOutput,
    },
};

pub use super::parse_viewport::{
    ParseIdleStep as JsonIdleStep, ParseIdleStep as LezerIdleStep, ParseIdleWork as JsonIdleWork,
    ParseIdleWork as LezerIdleWork, ParseViewportError as JsonViewportError,
    ParseViewportError as LezerViewportError,
};

impl ViewportConfig for LrConfig {
    type Job = LrParseJob;

    fn active(state: &EditorState) -> Option<Arc<Self>> {
        active_lr_viewport(&state.context())
    }

    fn published(state: &EditorState) -> Option<&Arc<LrParseOutput>> {
        state.lr_publication.as_ref().map(|slot| &slot.output)
    }

    fn seed<M: Clone>(history: &mut History<M>, output: Arc<LrParseOutput>) {
        history.seed_lr_publication(output);
    }

    fn commit<M: Clone>(
        history: &mut History<M>,
        output: Arc<LrParseOutput>,
    ) -> Result<ChangeDesc, EditError> {
        history.commit_lr_publication(output)
    }

    fn viewport_job(&self, text: Text) -> Self::Job {
        LrConfig::viewport_job(self, text)
    }
}

impl ViewportJob for LrParseJob {
    type Output = LrParseOutput;

    fn cutoff_after(&self, start: usize, units: usize) -> Option<usize> {
        LrParseJob::cutoff_after(self, start, units)
    }

    fn in_flight_progress(&self) -> Option<usize> {
        LrParseJob::in_flight_progress(self)
    }

    fn advance(
        &mut self,
        upto: usize,
        rounds: usize,
    ) -> Result<(usize, Option<Self::Output>), ParseError> {
        let step = self.advance_viewport(upto, rounds)?;
        Ok((step.rounds, step.output))
    }
}

impl ViewportOutput for LrParseOutput {
    fn parsed_to(&self) -> usize {
        self.parsed_to
    }
    fn complete(&self) -> bool {
        self.complete
    }
}

pub(super) type LrViewport = SyntaxViewport<LrConfig>;

impl<M: Clone> EditorView<M> {
    /// Read the state's published JSON tree through a raw UTF-8 offset.
    pub fn json_syntax_tree(&self, upto: usize) -> Option<Arc<LrParseOutput>> {
        self.state().json_syntax_tree(upto)
    }

    /// Highlight the requested bytes if the published JSON prefix covers them.
    pub fn json_viewport_highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.lr_viewport_highlights(NativeLanguage::Json, range)
    }

    /// Check staged or published JSON coverage without parse work.
    pub fn json_syntax_tree_available(&self, upto: usize) -> bool {
        self.lr_syntax_tree_available(NativeLanguage::Json, upto)
    }

    /// Stage JSON coverage through a raw byte offset without publishing it.
    pub fn ensure_json_syntax_tree(
        &mut self,
        upto: usize,
        rounds: usize,
    ) -> Result<Option<Arc<LrParseOutput>>, ParseError> {
        self.ensure_lr_syntax_tree(NativeLanguage::Json, upto, rounds)
    }

    /// Ensure and publish JSON coverage. A zero quota can publish a ready stage.
    pub fn force_json_parsing(
        &mut self,
        upto: usize,
        rounds: usize,
    ) -> Result<bool, JsonViewportError> {
        self.force_lr_parsing(NativeLanguage::Json, upto, rounds)
    }

    /// Get an idle ticket for measured JSON viewport work.
    pub fn json_idle_work(&self) -> Option<JsonIdleWork> {
        self.lr_idle_work(NativeLanguage::Json)
    }

    /// Work toward the current JSON viewport. Return `None` for a stale ticket
    /// or when no work remains. The quota does not include staging work.
    pub fn run_json_idle(
        &mut self,
        work: &JsonIdleWork,
        rounds: usize,
    ) -> Result<Option<JsonIdleStep>, JsonViewportError> {
        self.run_lr_idle(NativeLanguage::Json, work, rounds)
    }

    /// Return the error that stopped the current JSON job.
    pub fn json_parse_error(&self) -> Option<ParseError> {
        self.lr_parse_error(NativeLanguage::Json)
    }

    /// Read the state's published Lezer grammar tree through a raw UTF-8 offset.
    pub fn lezer_syntax_tree(&self, upto: usize) -> Option<Arc<LrParseOutput>> {
        self.state().lezer_syntax_tree(upto)
    }

    /// Highlight the requested bytes if the published Lezer grammar prefix covers them.
    pub fn lezer_viewport_highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        self.lr_viewport_highlights(NativeLanguage::LezerGrammar, range)
    }

    /// Check staged or published Lezer grammar coverage without parse work.
    pub fn lezer_syntax_tree_available(&self, upto: usize) -> bool {
        self.lr_syntax_tree_available(NativeLanguage::LezerGrammar, upto)
    }

    /// Stage Lezer grammar coverage through a raw byte offset without publishing it.
    pub fn ensure_lezer_syntax_tree(
        &mut self,
        upto: usize,
        rounds: usize,
    ) -> Result<Option<Arc<LrParseOutput>>, ParseError> {
        self.ensure_lr_syntax_tree(NativeLanguage::LezerGrammar, upto, rounds)
    }

    /// Ensure and publish Lezer grammar coverage. A zero quota can publish a ready stage.
    pub fn force_lezer_parsing(
        &mut self,
        upto: usize,
        rounds: usize,
    ) -> Result<bool, LezerViewportError> {
        self.force_lr_parsing(NativeLanguage::LezerGrammar, upto, rounds)
    }

    /// Get an idle ticket for measured Lezer grammar viewport work.
    pub fn lezer_idle_work(&self) -> Option<LezerIdleWork> {
        self.lr_idle_work(NativeLanguage::LezerGrammar)
    }

    /// Work toward the current Lezer grammar viewport. Return `None` for a stale ticket
    /// or when no work remains. The quota does not include staging work.
    pub fn run_lezer_idle(
        &mut self,
        work: &LezerIdleWork,
        rounds: usize,
    ) -> Result<Option<LezerIdleStep>, LezerViewportError> {
        self.run_lr_idle(NativeLanguage::LezerGrammar, work, rounds)
    }

    /// Return the error that stopped the current Lezer grammar job.
    pub fn lezer_parse_error(&self) -> Option<ParseError> {
        self.lr_parse_error(NativeLanguage::LezerGrammar)
    }

    fn runs_lr_viewport(&self, language: NativeLanguage) -> bool {
        self.lr_viewport
            .worker
            .as_ref()
            .is_some_and(|worker| worker.config.language() == language)
    }

    fn lr_viewport_highlights(
        &self,
        language: NativeLanguage,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        Some(
            self.state()
                .lr_publication_for_range(language, &range)?
                .highlights(range),
        )
    }

    fn lr_syntax_tree_available(&self, language: NativeLanguage, upto: usize) -> bool {
        self.runs_lr_viewport(language)
            && self.state().text().is_char_boundary(upto)
            && (self
                .lr_viewport
                .worker
                .as_ref()
                .is_some_and(|worker| worker.ready(upto).is_some())
                || self.state().lr_syntax_tree(language, upto).is_some())
    }

    fn ensure_lr_syntax_tree(
        &mut self,
        language: NativeLanguage,
        upto: usize,
        rounds: usize,
    ) -> Result<Option<Arc<LrParseOutput>>, ParseError> {
        if !self.runs_lr_viewport(language) {
            return Ok(None);
        }
        self.lr_viewport.ensure(upto, rounds, self.history.state())
    }

    fn force_lr_parsing(
        &mut self,
        language: NativeLanguage,
        upto: usize,
        rounds: usize,
    ) -> Result<bool, ParseViewportError> {
        let Some(output) = self
            .ensure_lr_syntax_tree(language, upto, rounds)
            .map_err(ParseViewportError::Parse)?
        else {
            return Ok(false);
        };
        self.publish_lr_stage(output)?;
        Ok(true)
    }

    fn publish_lr_stage(&mut self, output: Arc<LrParseOutput>) -> Result<bool, ParseViewportError> {
        let Some((previous, changes)) = self.lr_viewport.publish(output, &mut self.history)? else {
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

    fn lr_idle_work(&self, language: NativeLanguage) -> Option<ParseIdleWork> {
        if !self.runs_lr_viewport(language) {
            return None;
        }
        self.lr_viewport
            .idle_work(self.state(), self.viewport.interval().end)
    }

    fn run_lr_idle(
        &mut self,
        language: NativeLanguage,
        work: &ParseIdleWork,
        rounds: usize,
    ) -> Result<Option<ParseIdleStep>, ParseViewportError> {
        if !self.runs_lr_viewport(language) {
            return Ok(None);
        }
        let Some((mut step, output)) = self.lr_viewport.idle_stage(
            work,
            rounds,
            self.history.state(),
            self.viewport.interval().end,
        )?
        else {
            return Ok(None);
        };
        if let Some(output) = output {
            step.published = self.publish_lr_stage(output)?;
        }
        Ok(Some(step))
    }

    fn lr_parse_error(&self, language: NativeLanguage) -> Option<ParseError> {
        if !self.runs_lr_viewport(language) {
            return None;
        }
        self.lr_viewport.worker.as_ref()?.error
    }
}

#[cfg(test)]
mod tests;
