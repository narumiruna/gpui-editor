use std::{ops::Range, sync::Arc};

use crate::{
    EditError, RangeSet,
    language::{BidiIsolateOptions, bidi::isolate_marks_for},
    view::{MarkDecoration, Viewport, bidi::Direction},
};

use super::{ISOLATE, Node, WastDocument, WastIsolation, WastNodeTypes};

#[derive(Debug)]
pub(super) struct IsolateRanges(Vec<Isolate>);

#[derive(Debug)]
struct Isolate {
    range: Range<usize>,
    prefix_end: usize,
    direction: Option<Direction>,
}

impl IsolateRanges {
    pub(super) fn new(nodes: &[Node], node_types: &WastNodeTypes) -> Self {
        let mut prefix_end = 0;
        Self(
            nodes
                .iter()
                .filter_map(|node| {
                    let direction = (*node_types.for_kind(node.kind).prop(&ISOLATE)?)?;
                    if node.range.is_empty() {
                        return None;
                    }
                    prefix_end = prefix_end.max(node.range.end);
                    Some(Isolate {
                        range: node.range.clone(),
                        prefix_end,
                        direction: match direction {
                            WastIsolation::Auto => None,
                            WastIsolation::Ltr => Some(Direction::Ltr),
                            WastIsolation::Rtl => Some(Direction::Rtl),
                        },
                    })
                })
                .collect(),
        )
    }

    fn touching(
        &self,
        visible: Range<usize>,
    ) -> impl Iterator<Item = (Range<usize>, Option<Direction>)> + '_ {
        // A parent can start before the viewport and end after any of its
        // children. Search by the largest end so far, not by individual ends.
        let start = self
            .0
            .partition_point(|node| node.prefix_end < visible.start);
        self.0[start..]
            .iter()
            .take_while(move |node| node.range.start <= visible.end)
            .filter(move |node| node.range.end >= visible.start)
            .map(|node| (node.range.clone(), node.direction))
    }
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
        isolate_marks_for(
            &self.text,
            self.syntax_lines(),
            viewport,
            options,
            |range| self.isolate_ranges.touching(range),
        )
    }
}
