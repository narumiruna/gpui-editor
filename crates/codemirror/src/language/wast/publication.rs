use std::{
    fmt,
    ops::Range,
    sync::{Arc, OnceLock},
};

use crate::{
    Assoc, ChangeDesc, EditError, EditorState, StateContext,
    language::{HighlightContext, HighlightSpan, LanguageData},
    language_data::active_wast_viewport,
};

use super::{ConfiguredHighlights, WastDataNodes, WastParseOutput};

/// Shared by states that hold the same parse result. The indexes depend only
/// on the tree, not on a view, the state facets, or the current parse job.
pub(crate) struct WastPublication {
    pub(crate) output: Arc<WastParseOutput>,
    highlights: OnceLock<ConfiguredHighlights>,
    data_nodes: OnceLock<WastDataNodes>,
}

impl fmt::Debug for WastPublication {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WastPublication")
            .field("parsed_to", &self.output.parsed_to)
            .field("complete", &self.output.complete)
            .finish_non_exhaustive()
    }
}

impl WastPublication {
    pub(crate) fn new(output: Arc<WastParseOutput>) -> Arc<Self> {
        Arc::new(Self {
            output,
            highlights: OnceLock::new(),
            data_nodes: OnceLock::new(),
        })
    }

    #[cfg(test)]
    pub(crate) fn has_data_nodes(&self) -> bool {
        self.data_nodes.get().is_some()
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

impl WastParseOutput {
    pub(crate) fn covers(&self, upto: usize, len: usize) -> bool {
        self.parsed_to >= upto && (upto < len || self.complete)
    }
}

impl EditorState {
    /// Read the published WAST viewport tree at a raw UTF-8 byte boundary.
    /// EOF requires a complete parse. A token can extend past the reported
    /// coverage. This does not read staged output or synchronous attachments.
    pub fn wast_syntax_tree(&self, upto: usize) -> Option<Arc<WastParseOutput>> {
        let output = &self.wast_publication.as_ref()?.output;
        (self.text().is_char_boundary(upto) && output.covers(upto, self.text().len()))
            .then(|| Arc::clone(output))
    }

    /// Read the root context for a nonempty raw UTF-8 byte range in the
    /// published viewport tree. Staged trees and token overrun do not extend
    /// coverage. This does not use the configured parser as a fallback.
    pub fn wast_viewport_highlight_context(
        &self,
        range: &Range<usize>,
    ) -> Option<HighlightContext> {
        if range.is_empty() {
            return None;
        }
        self.wast_publication_for_range(range)?
            .output
            .tree
            .wast_highlight_context()
    }

    pub(crate) fn wast_publication_for_range(
        &self,
        range: &Range<usize>,
    ) -> Option<&WastPublication> {
        let published = self.wast_publication.as_deref()?;
        (self.text().is_valid_range(range.clone())
            && published.output.covers(range.end, self.text().len()))
        .then_some(published)
    }

    pub(crate) fn wast_viewport_data(
        &self,
        position: usize,
        side: Assoc,
    ) -> Vec<Arc<LanguageData>> {
        let text = self.text();
        if !text.is_char_boundary(position)
            || (position > 0
                && position < text.len()
                && text.range_eq(position - 1..position + 1, "\r\n"))
        {
            return Vec::new();
        }
        let Some(config) = active_wast_viewport(&self.context()) else {
            return Vec::new();
        };
        config.root_node_type().configured_data_at(self, || {
            let published = self.wast_publication.as_ref()?;
            Some(
                published
                    .data_nodes
                    .get_or_init(|| WastDataNodes::new(&published.output.tree))
                    .resolve_at(position, side),
            )
        })
    }

    pub(crate) fn retain_wast_publication(&mut self, next: &StateContext<'_>) {
        let same_parser = active_wast_viewport(&self.context()).is_some_and(|old| {
            active_wast_viewport(next).is_some_and(|new| Arc::ptr_eq(&old, &new))
        });
        if !same_parser {
            self.wast_publication = None;
        }
    }

    // A resolved native publication is not a host edit: no filters, extenders,
    // effects, reconfiguration, or selection normalization. Fields still run
    // and can reject it before either the new field values or the tree is set.
    pub(crate) fn commit_wast_publication(
        &mut self,
        output: Arc<WastParseOutput>,
    ) -> Result<ChangeDesc, EditError> {
        let changes = self.refresh_viewport_fields()?;
        self.wast_publication = Some(WastPublication::new(output));
        Ok(changes)
    }
}
