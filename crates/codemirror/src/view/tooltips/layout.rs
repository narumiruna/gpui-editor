//! Position tooltip boxes using geometry measured by the host.

use std::collections::{HashMap, HashSet};

use super::{
    ActiveTooltip, TooltipId, TooltipParentId, TooltipPosition, active_tooltips, tooltip_settings,
};
use crate::EditorState;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TooltipPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TooltipRect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// Geometry in viewport coordinates. Supply finite values, ordered rectangles,
/// positive scales and line height, and nonnegative box sizes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipGeometry {
    pub viewport: TooltipRect,
    /// The scroll rectangle with covered margins removed.
    pub visible: TooltipRect,
    pub parent_origin: TooltipPoint,
    pub scale: TooltipPoint,
    pub fixed_left: f64,
    pub line_height: f64,
    pub right_to_left: bool,
    pub in_view: bool,
    /// The host detects conditions that prevent fixed positioning.
    pub force_absolute: bool,
}

/// Measurements for a single positioned box, including the shared hover host.
/// Measure natural size, even when a previous layout restricted the height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipMeasurement {
    /// The host can override the text anchor rectangle. `None` hides the box.
    pub anchor: Option<TooltipRect>,
    pub width: f64,
    pub height: f64,
    pub offset: TooltipPoint,
    pub overlap: bool,
    pub resize: bool,
}

impl TooltipMeasurement {
    pub fn new(anchor: Option<TooltipRect>, width: f64, height: f64) -> Self {
        Self {
            anchor,
            width,
            height,
            offset: TooltipPoint::default(),
            overlap: false,
            resize: true,
        }
    }
}

/// Coordinates and height in the selected positioning space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipPlacement {
    pub origin: TooltipPoint,
    pub above: bool,
    pub height_limit: Option<f64>,
    pub arrow_left: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TooltipPositions {
    pub position: TooltipPosition,
    pub parent: Option<TooltipParentId>,
    pub space: TooltipRect,
    /// Facet order. A placement of `None` hides the box.
    pub placements: Vec<(TooltipId, Option<TooltipPlacement>)>,
}

/// Keep one layout per view to retain side flips and the fallback to absolute
/// positioning across measure passes.
#[derive(Default)]
pub struct TooltipLayout {
    above: HashMap<TooltipId, bool>,
    made_absolute: bool,
}

impl TooltipLayout {
    pub fn position(
        &mut self,
        state: &EditorState,
        geometry: TooltipGeometry,
        mut measure: impl FnMut(&ActiveTooltip) -> TooltipMeasurement,
    ) -> TooltipPositions {
        let settings = tooltip_settings(state);
        let config = settings.config;
        let active = active_tooltips(state);
        let ids: HashSet<_> = active.iter().map(|tip| tip.tooltip.id()).collect();
        self.above.retain(|id, _| ids.contains(id));
        let mut position = config.position.unwrap_or_default();
        if position == TooltipPosition::Fixed && geometry.in_view && !active.is_empty() {
            self.made_absolute |= geometry.force_absolute;
        }
        let (origin, scale) = if self.made_absolute || position == TooltipPosition::Absolute {
            position = TooltipPosition::Absolute;
            (geometry.parent_origin, geometry.scale)
        } else {
            (
                TooltipPoint {
                    x: geometry.fixed_left,
                    y: 0.0,
                },
                TooltipPoint { x: 1.0, y: 1.0 },
            )
        };
        let space = settings
            .measure_space
            .as_ref()
            .map(|measure| measure(state, &geometry))
            .or(config.tooltip_space)
            .unwrap_or(geometry.viewport);
        let mut others = Vec::new();
        let placements = active
            .iter()
            .map(|tip| {
                let id = tip.tooltip.id();
                let above = self.above.entry(id).or_insert(tip.tooltip.above);
                if !geometry.in_view {
                    return (id, None);
                }
                let placement = place(tip, measure(tip), &geometry, space, above, &mut others).map(
                    |placement| TooltipPlacement {
                        origin: TooltipPoint {
                            x: (placement.origin.x - origin.x) / scale.x,
                            y: (placement.origin.y - origin.y) / scale.y,
                        },
                        height_limit: placement.height_limit.map(|height| height / scale.y),
                        arrow_left: placement.arrow_left.map(|left| left / scale.x),
                        ..placement
                    },
                );
                (id, placement)
            })
            .collect();
        TooltipPositions {
            position,
            parent: config.parent,
            space,
            placements,
        }
    }
}

fn place(
    active: &ActiveTooltip,
    size: TooltipMeasurement,
    geometry: &TooltipGeometry,
    space: TooltipRect,
    above: &mut bool,
    others: &mut Vec<TooltipRect>,
) -> Option<TooltipPlacement> {
    const ARROW_SIZE: f64 = 7.0;
    const ARROW_OFFSET: f64 = 14.0;
    let tip = &active.tooltip;
    let pos = size.anchor?;
    let visible = geometry.visible;
    if tip.clip
        && (pos.bottom <= visible.top.max(space.top)
            || pos.top >= visible.bottom.min(space.bottom)
            || pos.right < visible.left.max(space.left) - 0.1
            || pos.left > visible.right.min(space.right) + 0.1)
    {
        return None;
    }
    let arrow_height = if tip.arrow { ARROW_SIZE } else { 0.0 };
    let arrow_offset = if tip.arrow { ARROW_OFFSET } else { 0.0 };
    let left = if size.width > space.right - space.left {
        if geometry.right_to_left {
            space.right - size.width
        } else {
            space.left
        }
    } else if geometry.right_to_left {
        (pos.left - size.width + arrow_offset - size.offset.x)
            .max(space.left)
            .min(space.right - size.width)
    } else {
        (pos.left - arrow_offset + size.offset.x)
            .min(space.right - size.width)
            .max(space.left)
    };
    let room_above = pos.top - space.top;
    let room_below = space.bottom - pos.bottom;
    let room = if *above { room_above } else { room_below };
    if !tip.strict_side
        && room < size.height + arrow_height + size.offset.y
        && *above == (room_below > room_above)
    {
        *above = !*above;
    }
    let room = (if *above { room_above } else { room_below }) - arrow_height;
    let height_limit = if size.resize && room < size.height {
        if room < geometry.line_height {
            return None;
        }
        Some(room)
    } else {
        None
    };
    let height = height_limit.unwrap_or(size.height);
    let mut top = if *above {
        pos.top - height - arrow_height - size.offset.y
    } else {
        pos.bottom + arrow_height + size.offset.y
    };
    let right = left + size.width;
    if !size.overlap {
        for rect in others.iter() {
            if rect.left < right
                && rect.right > left
                && rect.top < top + height
                && rect.bottom > top
            {
                top = if *above {
                    rect.top - height - 2.0 - arrow_height
                } else {
                    rect.bottom + arrow_height + 2.0
                };
            }
        }
        others.push(TooltipRect {
            left,
            top,
            right,
            bottom: top + height,
        });
    }
    let arrow_left = tip.arrow.then(|| {
        let x = if geometry.right_to_left {
            -size.offset.x
        } else {
            size.offset.x
        };
        pos.left + x - (left + ARROW_OFFSET - ARROW_SIZE)
    });
    Some(TooltipPlacement {
        origin: TooltipPoint { x: left, y: top },
        above: *above,
        height_limit,
        arrow_left,
    })
}
