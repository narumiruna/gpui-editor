//! Headless tooltips from `@codemirror/view` 6.44.0.
//! See `docs/licenses/codemirror-view.txt` for the upstream MIT notice.
//!
//! The host owns content, pointer hit testing, source calls, and rendering.
//! It dispatches hover effects and measures the active tooltips in facet order.

use std::{
    collections::HashSet,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{EditorState, Extension, Facet, StateEffect, StateEffectType};

mod hover;
mod layout;

pub use hover::{
    CLOSE_HOVER_TOOLTIPS, HoverActive, HoverHide, HoverOptions, HoverPointer, HoverRequest,
    HoverSession, HoverSide, HoverSourceId, HoverTarget, HoverTooltip, HoverUntil,
    close_hover_tooltip, close_hover_tooltips, has_hover_tooltips, hover_tooltip,
};
pub use layout::{
    TooltipGeometry, TooltipLayout, TooltipMeasurement, TooltipPlacement, TooltipPoint,
    TooltipPositions, TooltipRect,
};

fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("tooltip IDs exhausted")
}

/// Stable identity for native content. Keep the ID when changing a tooltip's
/// anchor or options. The first active occurrence of an ID supplies its data.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TooltipId(u64);

impl TooltipId {
    /// All active hover sections share this positioned container.
    pub const HOVER_HOST: Self = Self(0);
}

/// An anchor and display options. Positions are UTF-8 byte offsets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tooltip {
    id: TooltipId,
    pub pos: usize,
    pub end: Option<usize>,
    pub above: bool,
    pub strict_side: bool,
    pub arrow: bool,
    pub clip: bool,
}

impl Tooltip {
    pub fn new(pos: usize) -> Self {
        Self::with_id(TooltipId(next_id()), pos)
    }

    fn with_id(id: TooltipId, pos: usize) -> Self {
        Self {
            id,
            pos,
            end: None,
            above: false,
            strict_side: false,
            arrow: false,
            clip: true,
        }
    }

    pub fn id(&self) -> TooltipId {
        self.id
    }
}

/// Direct tooltip inputs, in extension precedence order. Use a computed input
/// if an anchor must change with editor state. Constant inputs are not mapped.
pub static SHOW_TOOLTIP: LazyLock<Facet<Option<Tooltip>, Vec<Option<Tooltip>>>> =
    LazyLock::new(Facet::collect);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveTooltip {
    pub tooltip: Tooltip,
    /// Present only for the shared hover host, in extension order.
    pub sections: Option<Arc<Vec<Tooltip>>>,
}

pub fn active_tooltips(state: &EditorState) -> Vec<ActiveTooltip> {
    let context = state.context();
    let mut seen = HashSet::new();
    context
        .facet(&SHOW_TOOLTIP)
        .expect("valid tooltip list")
        .iter()
        .flatten()
        .filter(|tooltip| seen.insert(tooltip.id()))
        .map(|tooltip| ActiveTooltip {
            tooltip: tooltip.clone(),
            sections: (tooltip.id() == TooltipId::HOVER_HOST).then(|| {
                context
                    .facet(&hover::HOVER_SECTIONS)
                    .expect("valid hover sections")
            }),
        })
        .collect()
}

/// Look up an active tooltip container by ID. This returns data, not a widget.
/// Get the sections of the hover host by using [`TooltipId::HOVER_HOST`].
pub fn get_tooltip(state: &EditorState, id: TooltipId) -> Option<ActiveTooltip> {
    active_tooltips(state)
        .into_iter()
        .find(|active| active.tooltip.id() == id)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TooltipPosition {
    /// Use viewport coordinates.
    #[default]
    Fixed,
    /// Use coordinates relative to the tooltip parent.
    Absolute,
}

/// An alternate container known by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TooltipParentId(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TooltipConfig {
    pub position: Option<TooltipPosition>,
    pub parent: Option<TooltipParentId>,
    /// Override the viewport bounds supplied for each measure pass.
    pub tooltip_space: Option<TooltipRect>,
}

type MeasureSpace = dyn Fn(&EditorState, &TooltipGeometry) -> TooltipRect + Send + Sync;

#[derive(Clone)]
struct TooltipSettings {
    config: TooltipConfig,
    measure_space: Option<Arc<MeasureSpace>>,
}

static TOOLTIP_CONFIG: LazyLock<Facet<TooltipSettings, TooltipSettings>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&TooltipSettings]| {
        let space = inputs
            .iter()
            .find(|input| input.config.tooltip_space.is_some() || input.measure_space.is_some());
        TooltipSettings {
            config: TooltipConfig {
                position: inputs.iter().find_map(|input| input.config.position),
                parent: inputs.iter().find_map(|input| input.config.parent),
                tooltip_space: space.and_then(|input| input.config.tooltip_space),
            },
            measure_space: space.and_then(|input| input.measure_space.clone()),
        }
    })
});

/// Configure tooltip positioning. The first defined value per option wins.
/// Defaults are fixed positioning, the editor parent, and the measured viewport.
pub fn tooltips(config: TooltipConfig) -> Extension {
    TOOLTIP_CONFIG.of(TooltipSettings {
        config,
        measure_space: None,
    })
}

/// Resolve the available tooltip space from the current state and measured
/// geometry. The first configured space wins, whether fixed or measured.
/// Return a finite rectangle with ordered edges in viewport coordinates.
pub fn tooltip_space(
    measure: impl Fn(&EditorState, &TooltipGeometry) -> TooltipRect + Send + Sync + 'static,
) -> Extension {
    TOOLTIP_CONFIG.of(TooltipSettings {
        config: TooltipConfig::default(),
        measure_space: Some(Arc::new(measure)),
    })
}

fn tooltip_settings(state: &EditorState) -> Arc<TooltipSettings> {
    state
        .context()
        .facet(&TOOLTIP_CONFIG)
        .expect("valid tooltip config")
}

/// Return the configured fixed options without invoking a measurement callback.
pub fn tooltip_config(state: &EditorState) -> TooltipConfig {
    tooltip_settings(state).config
}

/// Ask the host to measure and position active tooltips again.
pub static REPOSITION_TOOLTIPS: LazyLock<StateEffectType<()>> = LazyLock::new(StateEffectType::new);

pub fn reposition_tooltips() -> StateEffect {
    REPOSITION_TOOLTIPS.of(())
}
