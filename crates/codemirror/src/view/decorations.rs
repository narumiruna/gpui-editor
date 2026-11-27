//! Native decoration ranges. All positions are UTF-8 byte offsets.
//!
//! Edge ordering and mapping rules follow `@codemirror/view` 6.44.0. See
//! `docs/licenses/codemirror-view.txt` for the upstream MIT notice. The host
//! draws the decorations. This module does not implement widget rendering or
//! browser layout.

use std::fmt;
use std::ops::Range as ByteRange;
use std::sync::Arc;

use super::MarkDecoration;
use crate::{
    ChangeDesc, ChangeSet, EditError, MapMode, Range, RangeSet, RangeSetError, RangeValue, Text,
};

/// Class for the line that starts at the decorated position.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LineDecoration(String);

impl LineDecoration {
    pub fn new(class: impl Into<String>) -> Self {
        Self(class.into())
    }

    pub fn class(&self) -> &str {
        &self.0
    }
}

/// Position and placement options for a widget.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WidgetSpec {
    /// Order at the widget's position, clamped to `-10_000..=10_000`.
    /// Non-positive values place the widget before the cursor; positive
    /// values place it after the cursor.
    pub side: i32,
    pub block: bool,
    /// Order a block widget with inline widgets on the same side.
    pub inline_order: bool,
}

/// Placement and edge options for a replacement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReplaceSpec {
    pub block: bool,
    /// Include insertions at the edges. By default, block replacements
    /// include both edges and inline replacements exclude them.
    pub inclusive: Option<bool>,
    /// Override `inclusive` at the start.
    pub inclusive_start: Option<bool>,
    /// Override `inclusive` at the end.
    pub inclusive_end: Option<bool>,
}

/// Placement at the decoration's declared byte offsets. A block position
/// does not need to be a line boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockPlacement {
    /// Draw the widget before the text at its position.
    Before,
    /// Draw the widget after the text that ends at its position.
    After,
    /// A block that replaces its range, even if the range is empty.
    Range,
}

/// Decoration content. `W` is a widget description that the native host reads.
#[derive(Debug)]
pub enum DecorationKind<W> {
    Mark(MarkDecoration),
    Line(LineDecoration),
    Widget {
        widget: W,
        spec: WidgetSpec,
    },
    Replace {
        widget: Option<W>,
        spec: ReplaceSpec,
    },
}

/// A shared decoration description. Cloning or mapping a range preserves
/// its decoration instance without cloning widget data.
#[derive(Debug)]
pub struct Decoration<W = ()>(Arc<DecorationKind<W>>);

impl<W> Clone for Decoration<W> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<W> Decoration<W> {
    pub fn mark(mark: MarkDecoration) -> Self {
        Self(Arc::new(DecorationKind::Mark(mark)))
    }

    pub fn line(line: LineDecoration) -> Self {
        Self(Arc::new(DecorationKind::Line(line)))
    }

    pub fn widget(widget: W, spec: WidgetSpec) -> Self {
        Self(Arc::new(DecorationKind::Widget { widget, spec }))
    }

    /// Replace content with a widget, or hide it if `widget` is `None`.
    pub fn replace(widget: Option<W>, spec: ReplaceSpec) -> Self {
        Self(Arc::new(DecorationKind::Replace { widget, spec }))
    }

    pub fn kind(&self) -> &DecorationKind<W> {
        &self.0
    }

    pub fn widget_value(&self) -> Option<&W> {
        match self.kind() {
            DecorationKind::Widget { widget, .. } => Some(widget),
            DecorationKind::Replace { widget, .. } => widget.as_ref(),
            _ => None,
        }
    }

    pub fn block_placement(&self) -> Option<BlockPlacement> {
        match self.kind() {
            DecorationKind::Widget { spec, .. } if spec.block => Some(if spec.side <= 0 {
                BlockPlacement::Before
            } else {
                BlockPlacement::After
            }),
            DecorationKind::Replace { spec, .. } if spec.block => Some(BlockPlacement::Range),
            _ => None,
        }
    }

    /// Test whether two handles share the same decoration instance. Unlike
    /// [`RangeValue::eq`], this does not compare the widget values.
    pub fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Attach this decoration to byte offsets without checking a document.
    /// Marks must not be empty. Widgets and line decorations must be empty.
    /// An empty replacement must include at least one edge.
    pub fn try_range(&self, range: ByteRange<usize>) -> Result<Range<Self>, EditError> {
        if !self.valid_shape(range.start, range.end) {
            return Err(EditError::InvalidRange);
        }
        Ok(Range::new(range.start, range.end, self.clone()))
    }

    fn valid_shape(&self, from: usize, to: usize) -> bool {
        if to < from {
            return false;
        }
        match self.kind() {
            DecorationKind::Mark(_) => from < to,
            DecorationKind::Line(_) | DecorationKind::Widget { .. } => from == to,
            DecorationKind::Replace { .. } => {
                let (start, end) = self.sides();
                from < to || start < 0 || end > 0
            }
        }
    }

    fn sides(&self) -> (i32, i32) {
        match self.kind() {
            DecorationKind::Mark(mark) => (mark.start_side(), mark.end_side()),
            DecorationKind::Line(_) => (-200_000_000, -200_000_000),
            DecorationKind::Widget { spec, .. } => {
                let side = spec.side.clamp(-10_000, 10_000);
                let offset = match (spec.block && !spec.inline_order, side > 0) {
                    (true, true) => 300_000_000,
                    (true, false) => -400_000_000,
                    (false, true) => 100_000_000,
                    (false, false) => -100_000_000,
                };
                (offset + side, offset + side)
            }
            DecorationKind::Replace { spec, .. } => {
                let inclusive = spec.inclusive.unwrap_or(spec.block);
                let start = match (spec.inclusive_start.unwrap_or(inclusive), spec.block) {
                    (true, true) => -300_000_000,
                    (true, false) => -1,
                    (false, _) => 500_000_000,
                };
                let end = match (spec.inclusive_end.unwrap_or(inclusive), spec.block) {
                    (true, true) => 200_000_000,
                    (true, false) => 1,
                    (false, _) => -600_000_000,
                };
                // Inclusive replacements surround marks with the same edges.
                (start - 1, end + 1)
            }
        }
    }
}

impl<W: PartialEq> RangeValue for Decoration<W> {
    fn eq(&self, other: &Self) -> bool {
        if self.same_instance(other) {
            return true;
        }
        match (self.kind(), other.kind()) {
            (DecorationKind::Mark(a), DecorationKind::Mark(b)) => RangeValue::eq(a, b),
            (DecorationKind::Line(a), DecorationKind::Line(b)) => a == b,
            (DecorationKind::Widget { .. }, DecorationKind::Widget { .. })
            | (DecorationKind::Replace { .. }, DecorationKind::Replace { .. }) => {
                self.sides() == other.sides()
                    && self.block_placement() == other.block_placement()
                    && self.widget_value() == other.widget_value()
            }
            _ => false,
        }
    }

    fn start_side(&self) -> i32 {
        self.sides().0
    }

    fn end_side(&self) -> i32 {
        self.sides().1
    }

    fn map_mode(&self) -> MapMode {
        match self.kind() {
            DecorationKind::Line(_) => MapMode::TrackBefore,
            _ if self.block_placement().is_some() => {
                if self.start_side() < 0 {
                    MapMode::TrackBefore
                } else {
                    MapMode::TrackAfter
                }
            }
            _ => MapMode::TrackDel,
        }
    }

    fn is_point(&self) -> bool {
        !matches!(self.kind(), DecorationKind::Mark(_))
    }
}

/// Invalid decoration offsets, range shape, or input order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecorationSetError {
    InvalidRange,
    UnsortedRanges,
}

impl fmt::Display for DecorationSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange => f.write_str("invalid decoration range"),
            Self::UnsortedRanges => RangeSetError::UnsortedRanges.fmt(f),
        }
    }
}

impl std::error::Error for DecorationSetError {}

/// Validated decoration ranges for a document.
#[derive(Debug)]
pub struct DecorationSet<W = ()>(RangeSet<Decoration<W>>);

impl<W> DecorationSet<W> {
    pub const fn empty() -> Self {
        Self(RangeSet::empty())
    }

    pub fn as_range_set(&self) -> &RangeSet<Decoration<W>> {
        &self.0
    }

    /// Iterate in range order over decorations that end at or after `from`.
    pub fn iter(&self, from: usize) -> impl Iterator<Item = &Range<Decoration<W>>> {
        self.0.iter(from)
    }
}

impl<W> Clone for DecorationSet<W> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<W> Default for DecorationSet<W> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<W: PartialEq> DecorationSet<W> {
    /// Check the range shapes, document bounds, and UTF-8 boundaries. When
    /// `sort` is false, the ranges must be ordered by start and start side.
    /// A line decoration only affects a line that starts at its position.
    pub fn new(
        text: &Text,
        ranges: Vec<Range<Decoration<W>>>,
        sort: bool,
    ) -> Result<Self, DecorationSetError> {
        if ranges.iter().any(|range| {
            !text.is_valid_range(range.from..range.to)
                || !range.value.valid_shape(range.from, range.to)
        }) {
            return Err(DecorationSetError::InvalidRange);
        }
        RangeSet::of(ranges, sort)
            .map(Self)
            .map_err(|error| match error {
                RangeSetError::ReversedRange => DecorationSetError::InvalidRange,
                RangeSetError::UnsortedRanges => DecorationSetError::UnsortedRanges,
            })
    }

    /// Map through accepted changes for the document that owns these ranges.
    /// Use the resolved changes, not the unfiltered input to a transaction.
    /// This uses [`RangeSet::map`], which drops fully covered chunks before
    /// mapping the surviving decoration ranges.
    pub fn map(&self, changes: &ChangeSet) -> Self {
        Self(self.0.map(changes))
    }

    /// Map through the accepted change description in a
    /// [`ViewUpdate`](crate::view::ViewUpdate). The description must refer to
    /// the document that owns these decorations.
    pub fn map_desc(&self, changes: &ChangeDesc) -> Result<Self, EditError> {
        self.0.map_desc(changes).map(Self)
    }
}
