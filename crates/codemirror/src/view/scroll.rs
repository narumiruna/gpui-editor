//! Host scroll planning adapted from CodeMirror view 6.44.0.
//! See `docs/licenses/codemirror-view.txt` for the upstream MIT notice.

use std::sync::LazyLock;

use super::bidi::Direction;
use crate::{
    Assoc, ChangeDesc, EditorState, Extension, Facet, MapMode, SelectionRange, StateEffectType,
    Text,
};

/// Align a target if it is outside the current viewport, or place it at the
/// specified position on one scroll axis.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ScrollStrategy {
    #[default]
    Nearest,
    Start,
    Center,
    End,
}

/// Distance from the viewport edges for a transaction's automatic caret reveal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorScrollMargin {
    pub x: f64,
    pub y: f64,
}

impl CursorScrollMargin {
    pub const fn uniform(margin: f64) -> Self {
        Self {
            x: margin,
            y: margin,
        }
    }

    pub(super) fn configured(state: &EditorState) -> Self {
        *state
            .context()
            .facet(&CURSOR_SCROLL_MARGIN)
            .expect("typed cursor margins")
    }
}

impl Default for CursorScrollMargin {
    fn default() -> Self {
        Self::uniform(5.0)
    }
}

/// The last input in extension precedence order sets both margins. This
/// does not change explicit scroll effects. A reconfiguration changes the
/// margin for later transactions, not the transaction that installs it.
pub static CURSOR_SCROLL_MARGIN: LazyLock<Facet<CursorScrollMargin, CursorScrollMargin>> =
    LazyLock::new(|| {
        Facet::new(|inputs: &[&CursorScrollMargin]| {
            inputs
                .last()
                .map_or_else(CursorScrollMargin::default, |input| **input)
        })
    });

/// A document range to reveal. Margins are in the host's layout units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollTarget {
    pub range: SelectionRange,
    pub x: ScrollStrategy,
    pub y: ScrollStrategy,
    pub x_margin: f64,
    pub y_margin: f64,
}

impl ScrollTarget {
    pub fn new(range: SelectionRange) -> Self {
        Self {
            range,
            x: ScrollStrategy::Nearest,
            y: ScrollStrategy::Nearest,
            x_margin: 5.0,
            y_margin: 5.0,
        }
    }

    pub fn at(position: usize) -> Self {
        Self::new(SelectionRange::new(position, position))
    }

    /// Plan relative movement for one scroller. `coords_at` reads caret
    /// bounds at a checked UTF-8 byte offset, in the same coordinates as
    /// `scroller.clip`. Use the requested side of a wrap or bidi boundary.
    /// If the anchor has no geometry, reveal the head alone. Return `None`
    /// if the head has no geometry, an input is invalid, or movement is not finite.
    ///
    /// The host applies the result to its current scroll offsets and handles
    /// clamping, rounding, and any scrolling of ancestor elements.
    pub fn plan_reveal(
        self,
        text: &Text,
        scroller: FixedScroller,
        coords_at: impl FnMut(usize, Assoc) -> Option<ScrollRect>,
    ) -> Option<ScrollDelta> {
        self.plan_reveal_with_clip(text, scroller, coords_at, Some)
    }

    pub(super) fn plan_reveal_with_clip(
        self,
        text: &Text,
        scroller: FixedScroller,
        mut coords_at: impl FnMut(usize, Assoc) -> Option<ScrollRect>,
        clip: impl FnOnce(FixedScroller) -> Option<FixedScroller>,
    ) -> Option<ScrollDelta> {
        let target = self.clip(text)?;
        if !scroller.is_valid() || !self.x_margin.is_finite() || !self.y_margin.is_finite() {
            return None;
        }
        let range = target.range;
        let backward = range.head() < range.anchor();
        let head_side = range.assoc().unwrap_or(if range.head() > range.anchor() {
            Assoc::Before
        } else {
            Assoc::After
        });
        let mut rect = coords_at(range.head(), head_side)?;
        if !rect.is_valid() {
            return None;
        }
        let anchor_side = if backward {
            Assoc::Before
        } else {
            Assoc::After
        };
        if !range.is_empty()
            && let Some(anchor) = coords_at(range.anchor(), anchor_side)
        {
            if !anchor.is_valid() {
                return None;
            }
            rect = rect.union(anchor);
            if !rect.is_valid() {
                return None;
            }
        }

        let scroller = clip(scroller)?;
        if !scroller.is_valid() {
            return None;
        }
        let clip = scroller.clip;
        let x_margin = self
            .x_margin
            .clamp(-scroller.offset_width, scroller.offset_width);
        let y_margin = self
            .y_margin
            .clamp(-scroller.offset_height, scroller.offset_height);
        let mut delta = ScrollDelta::default();
        if scroller.scroll_x {
            let movement = match self.x {
                ScrollStrategy::Nearest => {
                    if rect.left < clip.left + x_margin {
                        let near = rect.left - (clip.left + x_margin);
                        if !backward && rect.right > clip.right + near {
                            rect.right - clip.right + x_margin
                        } else {
                            near
                        }
                    } else if rect.right > clip.right - x_margin {
                        let far = rect.right - clip.right + x_margin;
                        if backward && rect.left < clip.left + far {
                            rect.left - (clip.left + x_margin)
                        } else {
                            far
                        }
                    } else {
                        0.0
                    }
                }
                ScrollStrategy::Center => {
                    rect.left + rect.width() / 2.0 - clip.width() / 2.0 - clip.left
                }
                strategy
                    if (strategy == ScrollStrategy::Start)
                        == (scroller.direction == Direction::Ltr) =>
                {
                    rect.left - x_margin - clip.left
                }
                _ => rect.right - clip.width() + x_margin - clip.left,
            };
            delta.x = movement / scroller.scale_x;
        }
        if scroller.scroll_y {
            let movement = match self.y {
                ScrollStrategy::Nearest => {
                    if rect.top < clip.top + y_margin {
                        let near = rect.top - (clip.top + y_margin);
                        if !backward && rect.bottom > clip.bottom + near {
                            rect.bottom - clip.bottom + y_margin
                        } else {
                            near
                        }
                    } else if rect.bottom > clip.bottom - y_margin {
                        let far = rect.bottom - clip.bottom + y_margin;
                        if backward && rect.top - far < clip.top {
                            rect.top - (clip.top + y_margin)
                        } else {
                            far
                        }
                    } else {
                        0.0
                    }
                }
                ScrollStrategy::Center if rect.height() <= clip.height() => {
                    rect.top + rect.height() / 2.0 - clip.height() / 2.0 - clip.top
                }
                strategy
                    if strategy == ScrollStrategy::Start
                        || strategy == ScrollStrategy::Center && backward =>
                {
                    rect.top - y_margin - clip.top
                }
                _ => rect.bottom - clip.height() + y_margin - clip.top,
            };
            delta.y = movement / scroller.scale_y;
        }
        (delta.x.is_finite() && delta.y.is_finite()).then_some(delta)
    }

    pub(super) fn map(self, changes: &ChangeDesc) -> Option<Self> {
        Some(Self {
            range: self.range.map(changes).ok()?,
            ..self
        })
    }

    pub(super) fn clip(self, text: &Text) -> Option<Self> {
        let span = self.range.range();
        if span.end > text.len() {
            Some(Self {
                range: SelectionRange::new(text.len(), text.len()),
                ..self
            })
        } else {
            text.is_valid_range(span).then_some(self)
        }
    }
}

/// A mapped scroll request. The host reads it from an accepted view update
/// and uses its layout to reveal the requested source range.
pub static SCROLL_INTO_VIEW: LazyLock<StateEffectType<ScrollTarget>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|target: &ScrollTarget, changes| target.map(changes))
});

/// The signed position of the editor's own scroller, in its local units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollOffset {
    pub x: f64,
    pub y: f64,
}

impl ScrollOffset {
    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// Geometry of a host line block. `from` is a byte offset and `top` is in
/// document-relative coordinates, excluding top padding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollBlock {
    pub from: usize,
    pub top: f64,
}

impl ScrollBlock {
    fn is_valid(self, text: &Text) -> bool {
        self.top.is_finite() && text.is_valid_range(self.from..self.from)
    }
}

/// Geometry to capture the current scroll position. Use the layout's viewport
/// start and its first line block, not necessarily the first fully visible line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollSnapshotGeometry {
    pub offset: ScrollOffset,
    pub padding_top: f64,
    pub viewport_from: usize,
    pub first_viewport_block: ScrollBlock,
}

/// A scroll position relative to a document anchor, made with
/// `EditorView.scrollSnapshot` rules. This does not retain layout or a view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollSnapshot {
    anchor: usize,
    line_offset: f64,
    x: f64,
}

impl ScrollSnapshot {
    /// Read the block at `offset.y + 12 - padding_top`. If it precedes the
    /// viewport, use the first viewport block unless its top is more than
    /// 200 units below `offset.y`.
    ///
    /// The host supplies the block lookup, including its rules for wrapping,
    /// folds, and offscreen positions. Invalid or missing geometry returns `None`.
    pub fn capture(
        text: &Text,
        geometry: ScrollSnapshotGeometry,
        line_at_height: impl FnOnce(f64) -> Option<ScrollBlock>,
    ) -> Option<Self> {
        let first = geometry.first_viewport_block;
        let offset = geometry.offset;
        if !offset.is_finite()
            || !geometry.padding_top.is_finite()
            || geometry.padding_top < 0.0
            || !text.is_valid_range(geometry.viewport_from..geometry.viewport_from)
            || !first.is_valid(text)
        {
            return None;
        }
        let height = offset.y + 12.0 - geometry.padding_top;
        let first_offset = first.top - offset.y;
        if !height.is_finite() || !first_offset.is_finite() {
            return None;
        }
        let block = line_at_height(height)?;
        if !block.is_valid(text) {
            return None;
        }
        let block = if block.from >= geometry.viewport_from || first_offset > 200.0 {
            block
        } else {
            first
        };
        let line_offset = block.top - offset.y;
        line_offset.is_finite().then_some(Self {
            anchor: block.from,
            line_offset,
            x: offset.x,
        })
    }

    /// The reference position in UTF-8 bytes, before optional clipping.
    pub fn anchor(self) -> usize {
        self.anchor
    }

    /// Map the anchor before an insertion at that position. Use a description
    /// of changes to the snapshot's document. A description has only lengths;
    /// `clip` or `plan_restore` checks the destination UTF-8 boundary.
    pub fn map(self, changes: &ChangeDesc) -> Option<Self> {
        Some(Self {
            anchor: changes.map_pos(self.anchor, Assoc::Before, MapMode::Simple)?,
            ..self
        })
    }

    /// Clip to a shorter document's end. Reject positions inside a UTF-8 scalar.
    pub fn clip(self, text: &Text) -> Option<Self> {
        let anchor = self.anchor.min(text.len());
        text.is_valid_range(anchor..anchor)
            .then_some(Self { anchor, ..self })
    }

    /// Plan an absolute scroll offset. `line_top_at` reads the current line
    /// block top at the clipped anchor, in the same units as during capture.
    /// Apply this only to the editor's scroller, not to its ancestors. The host
    /// clamps the offset to its scroll limits. An unclamped offset can be negative.
    pub fn plan_restore(
        self,
        text: &Text,
        line_top_at: impl FnOnce(usize) -> Option<f64>,
    ) -> Option<ScrollOffset> {
        let anchor = self.clip(text)?.anchor;
        let offset = ScrollOffset {
            x: self.x,
            y: line_top_at(anchor)? - self.line_offset,
        };
        offset.is_finite().then_some(offset)
    }
}

/// A mapped request to restore a captured scroll position after layout.
/// A snapshot is distinct from an ordinary [`SCROLL_INTO_VIEW`] request.
pub static SCROLL_SNAPSHOT: LazyLock<StateEffectType<ScrollSnapshot>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|snapshot: &ScrollSnapshot, changes| snapshot.map(changes))
});

/// Physical edges of a caret or clip rectangle. Edges and extents must be
/// finite and ordered. Zero width or height is allowed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollRect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl ScrollRect {
    fn width(self) -> f64 {
        self.right - self.left
    }

    fn height(self) -> f64 {
        self.bottom - self.top
    }

    pub(super) fn is_valid(self) -> bool {
        let (w, h) = (self.width(), self.height());
        w.is_finite() && h.is_finite() && w >= 0.0 && h >= 0.0
    }

    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

/// Measurements for revealing a target in a single, fixed scroller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedScroller {
    /// Client bounds (without scrollbars) intersected with covered-edge bounds.
    /// If the intersection has crossed edges, the planner returns `None`.
    pub clip: ScrollRect,
    /// Full, unscaled box dimensions, used to limit the target's margins.
    pub offset_width: f64,
    pub offset_height: f64,
    /// Positive layout-to-clip coordinate scales. Use 1 for unscaled axes.
    pub scale_x: f64,
    pub scale_y: f64,
    /// Whether each axis has both overflow and user-scrollable behavior.
    pub scroll_x: bool,
    pub scroll_y: bool,
    /// Base text direction. This controls horizontal start and end alignment.
    pub direction: Direction,
}

impl FixedScroller {
    fn is_valid(self) -> bool {
        self.clip.is_valid()
            && self.offset_width.is_finite()
            && self.offset_width >= 0.0
            && self.offset_height.is_finite()
            && self.offset_height >= 0.0
            && self.scale_x.is_finite()
            && self.scale_x > 0.0
            && self.scale_y.is_finite()
            && self.scale_y > 0.0
    }
}

/// Requested movement, before the host clamps or rounds it, in unscaled
/// scroll offset units. Zero means unchanged on that axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollDelta {
    pub x: f64,
    pub y: f64,
}

static SCROLL_PAST_END: LazyLock<Facet<(), bool>> =
    LazyLock::new(|| Facet::new(|inputs| !inputs.is_empty()));

/// Enable bottom padding so the last line can scroll to the top of the editor.
/// Use only in an editor with its own scroll viewport. The host measures the
/// viewport and applies the padding from [`EndScrollPadding`].
pub fn scroll_past_end() -> Extension {
    SCROLL_PAST_END.of(())
}

/// The host retains one measurement per view. As in CodeMirror view 6.44.0,
/// padding starts at 1,000 pixels and keeps its last value when the viewport
/// is smaller than one line plus the document's top padding.
#[derive(Debug, Default)]
pub struct EndScrollPadding(Option<f64>);

impl EndScrollPadding {
    /// Return bottom padding in the same pixel units as the input geometry.
    /// `None` indicates that the extension is disabled. An unmeasured or
    /// invalid viewport does not change the retained value.
    pub fn update(
        &mut self,
        state: &EditorState,
        viewport_height: f64,
        line_height: f64,
        top_padding: f64,
    ) -> Option<f64> {
        if !*state.context().facet(&SCROLL_PAST_END).ok()? {
            self.0 = None;
            return None;
        }
        let height = viewport_height - line_height - top_padding - 0.5;
        let previous = self.0.get_or_insert(1000.0);
        if height.is_finite() && height >= 0.0 {
            *previous = height;
        }
        Some(*previous)
    }
}
