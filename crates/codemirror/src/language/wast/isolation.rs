use std::sync::Arc;

use crate::{
    EditError, RangeSet,
    language::{BidiIsolateOptions, bidi::IsolateRanges},
    view::{MarkDecoration, Viewport},
};

use super::{ISOLATE, Node, WastDocument, WastNodeTypes};

pub(super) fn index(nodes: &[Node], node_types: &WastNodeTypes) -> IsolateRanges {
    IsolateRanges::new(nodes.iter().filter_map(|node| {
        let direction = (*node_types.for_kind(node.kind).prop(&ISOLATE)?)?;
        Some((node.range.clone(), direction))
    }))
}

impl WastDocument {
    /// Build visible bidi isolation marks from the configured node-type
    /// properties. The viewport selects full node ranges without clipping.
    /// Return [`EditError::InvalidRange`] for invalid UTF-8 viewport endpoints.
    ///
    /// Install the result in [`crate::view::bidi::BIDI_ISOLATED_RANGES`] at
    /// lowest precedence, and use it as outer marks when painting the same
    /// text snapshot. Refresh after changes to the document, view, or direction.
    pub fn bidi_isolate_marks(
        &self,
        viewport: &Viewport,
        options: BidiIsolateOptions,
    ) -> Result<Arc<RangeSet<MarkDecoration>>, EditError> {
        self.isolate_ranges
            .marks(&self.text, self.syntax_lines(), viewport, options)
    }
}
