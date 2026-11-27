//! Scroll interception and covered edges for one native scroller.
//! Adapted from CodeMirror view 6.44.0. See `docs/licenses/codemirror-view.txt`.

use std::sync::{Arc, LazyLock};

use crate::{Assoc, EditorState, Extension, Facet};

use super::{
    FixedScroller, ScrollDelta, ScrollRect, ScrollTarget, ViewException, Viewport, log_exception,
};

/// State and document viewport for the layout in which a scroll will run.
/// The host supplies the viewport and current focus. Callbacks must not
/// initiate editor updates.
#[derive(Clone, Copy)]
pub struct ScrollView<'a> {
    pub state: &'a EditorState,
    pub viewport: &'a Viewport,
    pub focused: bool,
}

type Handler =
    Arc<dyn Fn(ScrollView<'_>, ScrollTarget) -> Result<bool, ViewException> + Send + Sync>;
static SCROLL_HANDLERS: LazyLock<Facet<Handler, Vec<Handler>>> = LazyLock::new(Facet::collect);

/// Register a handler for ordinary scroll reveals, in extension precedence
/// order. Return `true` to suppress the default reveal and later handlers.
pub fn scroll_handler(
    handler: impl Fn(ScrollView<'_>, ScrollTarget) -> bool + Send + Sync + 'static,
) -> Extension {
    try_scroll_handler(move |view, target| Ok(handler(view, target)))
}

/// Register a handler that can fail. Report errors to the view exception sink
/// and continue with the next handler. Rust panics are not intercepted.
pub fn try_scroll_handler(
    handler: impl Fn(ScrollView<'_>, ScrollTarget) -> Result<bool, ViewException>
    + Send
    + Sync
    + 'static,
) -> Extension {
    SCROLL_HANDLERS.of(Arc::new(handler))
}

/// Absolute physical edges of the visible area, in the same units and
/// coordinate space as the client bounds of this scroller. For example, a
/// left overlay supplies the coordinate of its right edge as `left`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CoveredMargins {
    pub left: Option<f64>,
    pub top: Option<f64>,
    pub right: Option<f64>,
    pub bottom: Option<f64>,
}

/// Insets from a known reference rectangle, in physical axes. These are
/// distinct from the per-target or automatic caret reveal margins.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollMargins {
    pub left: Option<f64>,
    pub top: Option<f64>,
    pub right: Option<f64>,
    pub bottom: Option<f64>,
}

impl CoveredMargins {
    /// Convert legacy `scrollMargins` insets into absolute covered edges.
    /// The host supplies the reference bounds for its scroller in clip
    /// coordinates. Upstream uses the intersection of the scroll parent's
    /// bounding box and the window. This does not discover any parent or
    /// apply margins to another scroller. Negative insets are allowed.
    pub fn from_insets(bounds: ScrollRect, insets: ScrollMargins) -> Option<Self> {
        if !bounds.is_valid() {
            return None;
        }
        let edges = Self {
            left: insets.left.map(|left| bounds.left + left),
            top: insets.top.map(|top| bounds.top + top),
            right: insets.right.map(|right| bounds.right - right),
            bottom: insets.bottom.map(|bottom| bounds.bottom - bottom),
        };
        edges.is_finite().then_some(edges)
    }

    fn is_finite(self) -> bool {
        [self.left, self.top, self.right, self.bottom]
            .into_iter()
            .flatten()
            .all(f64::is_finite)
    }

    fn intersect(self, clip: ScrollRect) -> ScrollRect {
        ScrollRect {
            left: self.left.map_or(clip.left, |left| clip.left.max(left)),
            top: self.top.map_or(clip.top, |top| clip.top.max(top)),
            right: self.right.map_or(clip.right, |right| clip.right.min(right)),
            bottom: self
                .bottom
                .map_or(clip.bottom, |bottom| clip.bottom.min(bottom)),
        }
    }
}

type Provider = Arc<dyn Fn(ScrollView<'_>, ScrollRect) -> Option<CoveredMargins> + Send + Sync>;
static COVERED_MARGINS: LazyLock<Facet<Provider, Vec<Provider>>> = LazyLock::new(Facet::collect);

/// Register the geometry of an overlay that covers part of this scroller.
/// Each provider receives the original client bounds, without any covered
/// edges applied. Return `None` when the overlay does not cover the scroller.
/// Providers combine by intersection, not addition.
pub fn covered_margins(
    provider: impl Fn(ScrollView<'_>, ScrollRect) -> Option<CoveredMargins> + Send + Sync + 'static,
) -> Extension {
    COVERED_MARGINS.of(Arc::new(provider))
}

impl ScrollView<'_> {
    /// Intercept an ordinary reveal before looking up geometry. The host
    /// must not call this for snapshot restorations. Return `true` when a
    /// handler accepts the request. This does not change a scroll offset.
    pub fn handle_scroll(self, target: ScrollTarget) -> bool {
        let Some(target) = target.clip(self.state.text()) else {
            return false;
        };
        let Ok(handlers) = self.state.context().facet(&SCROLL_HANDLERS) else {
            return false;
        };
        for handler in handlers.iter() {
            match handler(self, target) {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => {
                    log_exception(self.state, &error.with_context("scroll handler"));
                }
            }
        }
        false
    }

    /// Plan a default reveal after the host has checked [`Self::handle_scroll`].
    /// Run the covered-edge providers after looking up the target geometry.
    /// For the geometry and result units, see [`ScrollTarget::plan_reveal`].
    pub fn plan_reveal(
        self,
        target: ScrollTarget,
        scroller: FixedScroller,
        coords_at: impl FnMut(usize, Assoc) -> Option<ScrollRect>,
    ) -> Option<ScrollDelta> {
        target.plan_reveal_with_clip(self.state.text(), scroller, coords_at, |scroller| {
            self.clip_scroller(scroller)
        })
    }

    /// Clip one scroller to its covered edges for a default reveal. Pass
    /// unclipped client bounds in `scroller.clip`. Do not call this after a
    /// handler accepts the reveal or for a snapshot restoration.
    /// Return `None` if the input bounds or an edge are not finite, or if
    /// the intersection crosses its edges. A zero extent is valid.
    pub fn clip_scroller(self, mut scroller: FixedScroller) -> Option<FixedScroller> {
        let bounds = scroller.clip;
        if !bounds.is_valid() {
            return None;
        }
        let providers = self.state.context().facet(&COVERED_MARGINS).ok()?;
        let mut finite = true;
        for edges in providers
            .iter()
            .filter_map(|provider| provider(self, bounds))
        {
            finite &= edges.is_finite();
            scroller.clip = edges.intersect(scroller.clip);
        }
        (finite && scroller.clip.is_valid()).then_some(scroller)
    }
}
