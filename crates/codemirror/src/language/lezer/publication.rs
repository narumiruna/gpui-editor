use std::{
    fmt,
    ops::Range,
    sync::{Arc, OnceLock},
};

use crate::{
    ChangeDesc, EditError, EditorState, StateContext,
    language::{HighlightContext, HighlightSpan, lezer::ConfiguredHighlights},
    language_data::{NativeLanguage, active_lr_viewport},
};

use super::LrParseOutput;

pub(crate) struct LrPublication {
    pub(crate) output: Arc<LrParseOutput>,
    highlights: OnceLock<ConfiguredHighlights>,
}

impl fmt::Debug for LrPublication {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LrPublication")
            .field("parsed_to", &self.output.parsed_to)
            .field("complete", &self.output.complete)
            .finish_non_exhaustive()
    }
}

impl LrPublication {
    pub(crate) fn new(output: Arc<LrParseOutput>) -> Arc<Self> {
        Arc::new(Self {
            output,
            highlights: OnceLock::new(),
        })
    }

    pub(crate) fn highlights(
        &self,
        range: Range<usize>,
    ) -> impl Iterator<Item = HighlightSpan<'_>> {
        self.highlights
            .get_or_init(|| ConfiguredHighlights::build(self.output.tree.root()))
            .clipped(range)
    }
}

impl EditorState {
    /// Read the published JSON viewport tree that covers the offset.
    pub fn json_syntax_tree(&self, upto: usize) -> Option<Arc<LrParseOutput>> {
        self.lr_syntax_tree(NativeLanguage::Json, upto)
    }

    /// Read the published Lezer grammar viewport tree that covers the offset.
    pub fn lezer_syntax_tree(&self, upto: usize) -> Option<Arc<LrParseOutput>> {
        self.lr_syntax_tree(NativeLanguage::LezerGrammar, upto)
    }

    /// Get the JSON root highlight scope when a valid nonempty range is covered.
    pub fn json_viewport_highlight_context(
        &self,
        range: &Range<usize>,
    ) -> Option<HighlightContext> {
        self.lr_viewport_highlight_context(NativeLanguage::Json, range)
    }

    /// Get the Lezer grammar root highlight scope when a valid nonempty range is covered.
    pub fn lezer_viewport_highlight_context(
        &self,
        range: &Range<usize>,
    ) -> Option<HighlightContext> {
        self.lr_viewport_highlight_context(NativeLanguage::LezerGrammar, range)
    }

    // Working stages and token overrun do not extend coverage. EOF requires
    // a complete parse. The language check prevents cross-grammar queries.
    pub(crate) fn lr_syntax_tree(
        &self,
        language: NativeLanguage,
        upto: usize,
    ) -> Option<Arc<LrParseOutput>> {
        let output = &self.lr_publication.as_ref()?.output;
        (output.language() == language
            && self.text().is_char_boundary(upto)
            && output.covers(upto, self.text().len()))
        .then(|| Arc::clone(output))
    }

    fn lr_viewport_highlight_context(
        &self,
        language: NativeLanguage,
        range: &Range<usize>,
    ) -> Option<HighlightContext> {
        if range.is_empty() {
            return None;
        }
        self.lr_publication_for_range(language, range)?
            .output
            .tree
            .highlight_context()
    }

    pub(crate) fn lr_publication_for_range(
        &self,
        language: NativeLanguage,
        range: &Range<usize>,
    ) -> Option<&LrPublication> {
        let published = self.lr_publication.as_deref()?;
        (published.output.language() == language
            && self.text().is_valid_range(range.clone())
            && published.output.covers(range.end, self.text().len()))
        .then_some(published)
    }

    pub(crate) fn retain_lr_publication(&mut self, next: &StateContext<'_>) {
        let same_parser = active_lr_viewport(&self.context())
            .is_some_and(|old| active_lr_viewport(next).is_some_and(|new| Arc::ptr_eq(&old, &new)));
        if !same_parser {
            self.lr_publication = None;
        }
    }

    pub(crate) fn commit_lr_publication(
        &mut self,
        output: Arc<LrParseOutput>,
    ) -> Result<ChangeDesc, EditError> {
        let changes = self.refresh_viewport_fields()?;
        self.lr_publication = Some(LrPublication::new(output));
        Ok(changes)
    }
}
