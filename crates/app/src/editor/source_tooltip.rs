//! One explicit, literal host tooltip in the active code draft.

use std::ops::Range;

use gpui::{PathBuilder, Rgba};
use gpui_editor_codemirror::{
    theme_one_dark::TooltipStyle,
    view::tooltips::{
        SHOW_TOOLTIP, Tooltip, TooltipGeometry, TooltipLayout, TooltipMeasurement, TooltipPoint,
        TooltipRect,
    },
};

use super::*;

const MAX_TEXT_BYTES: usize = 512;
const ARROW_HEIGHT: Pixels = px(7.);
const ARROW_WIDTH: Pixels = px(14.);

/// The document revision and editable code body for an explicit host request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceTooltipSnapshot {
    editor: FocusHandle,
    revision: u64,
    body: Range<usize>,
}

pub(super) struct SourceTooltip {
    snapshot: SourceTooltipSnapshot,
    tip: Tooltip,
    text: String,
    state: EditorState,
    layout: TooltipLayout,
}

impl Editor {
    /// Get the current source scope without taking focus or scrolling.
    pub fn source_tooltip_snapshot(&self, window: &Window) -> Option<SourceTooltipSnapshot> {
        Some(SourceTooltipSnapshot {
            editor: self.focus.clone(),
            revision: self.view.revision(),
            body: self.source_tooltip_body(window)?,
        })
    }

    /// Replace the single host tooltip. Text is literal UTF-8, limited to 512
    /// bytes. The optional end must be in the same body; only pos anchors the
    /// box. A hidden or offscreen anchor never paints, even with clip disabled.
    /// This call does not take focus, move the selection, or reveal the anchor.
    pub fn show_source_tooltip(
        &mut self,
        snapshot: &SourceTooltipSnapshot,
        tip: Tooltip,
        text: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let end = tip.end.unwrap_or(tip.pos);
        if text.len() > MAX_TEXT_BYTES
            || text.trim().is_empty()
            || text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
            || !self.source_tooltip_current(snapshot, window)
            || tip.pos < snapshot.body.start
            || end > snapshot.body.end
            || tip.pos > end
            || !self.source_tooltip_boundary(tip.pos)
            || !self.source_tooltip_boundary(end)
        {
            return false;
        }

        // The native placer takes a state. Give it only this host-owned input,
        // with shared text storage and no editor facets, callbacks or history.
        let mut state = EditorState::from_text(self.view.state().text().clone());
        state
            .update(
                Transaction::default().with_effects([SHOW_TOOLTIP.of(Some(tip.clone())).append()]),
            )
            .expect("one standalone tooltip input");
        let layout = self
            .source_tooltip
            .take()
            .filter(|old| old.tip == tip && &old.snapshot == snapshot)
            .map_or_else(TooltipLayout::default, |old| old.layout);
        self.source_tooltip = Some(SourceTooltip {
            snapshot: snapshot.clone(),
            tip,
            text: text.into(),
            state,
            layout,
        });
        cx.notify();
        true
    }

    /// Dismiss the host tooltip without changing any stock overlay.
    pub fn dismiss_source_tooltip(&mut self, cx: &mut Context<Self>) {
        if self.clear_source_tooltip() {
            cx.notify();
        }
    }

    pub(super) fn clear_source_tooltip(&mut self) -> bool {
        self.source_tooltip.take().is_some()
    }

    fn source_tooltip_boundary(&self, pos: usize) -> bool {
        let source = self.source();
        source.is_char_boundary(pos)
            && !(pos > 0 && source.as_bytes().get(pos - 1..=pos) == Some(b"\r\n"))
    }

    fn source_tooltip_body(&self, window: &Window) -> Option<Range<usize>> {
        if !self.input_enabled
            || self.view.state().read_only()
            || self.is_reviewing_changes()
            || self.is_composing()
            || !window.is_window_active()
            || !self.focus.is_focused(window)
        {
            return None;
        }
        Some(self.source_fence_with_unlabeled(true)?.body)
    }

    fn source_tooltip_current(&self, snapshot: &SourceTooltipSnapshot, window: &Window) -> bool {
        self.focus == snapshot.editor
            && self.view.revision() == snapshot.revision
            && self.source_tooltip_body(window).as_ref() == Some(&snapshot.body)
    }

    pub(super) fn prepaint_source_tooltip(
        &mut self,
        layout: &Layout,
        viewport: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<PrepaintedTooltip> {
        let tip = self.source_tooltip.as_ref()?;
        if !self.source_tooltip_current(&tip.snapshot, window) {
            self.clear_source_tooltip();
            return None;
        }
        if self.selecting || viewport.size.width <= px(0.) || viewport.size.height <= px(0.) {
            return None;
        }
        let pos = tip.tip.pos;
        if layout.fold_hides(pos) {
            return None;
        }
        // Use a real projected text stop, not a snapped preview edge.
        let anchor = layout.drop_cursor_bounds(pos)?;
        if !viewport.intersects(&anchor) {
            return None;
        }
        let style = PaintStyle::new(
            layout.theme,
            view_style(self.view.state()).map(|view| view.tooltip),
            window,
        );
        let tip = self.source_tooltip.as_mut()?;
        let max_width = viewport.size.width.min(px(440.));
        let space = size(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
        let mut element = style.body(&tip.text, max_width, None);
        let natural = element.layout_as_root(space, window, cx);
        let clip = rect(viewport);
        let placement = tip.layout.position(
            &tip.state,
            TooltipGeometry {
                viewport: clip,
                visible: clip,
                parent_origin: TooltipPoint::default(),
                scale: TooltipPoint { x: 1., y: 1. },
                fixed_left: 0.,
                line_height: DEFAULT_LINE_HEIGHT.into(),
                // The GPUI editor viewport is LTR. Glyph runs can be bidi.
                right_to_left: false,
                in_view: true,
                force_absolute: false,
            },
            |_| {
                TooltipMeasurement::new(
                    Some(rect(anchor)),
                    natural.width.into(),
                    natural.height.into(),
                )
            },
        );
        let (id, placement) = placement.placements.into_iter().next()?;
        debug_assert_eq!(id, tip.tip.id());
        let placement = placement?;
        let origin = point(placement.origin.x.into(), placement.origin.y.into());
        let measured = if let Some(height) = placement.height_limit {
            element = style.body(&tip.text, max_width, Some(height.into()));
            element.layout_as_root(space, window, cx)
        } else {
            natural
        };
        element.prepaint_at(origin, window, cx);
        let body = Bounds::new(origin, measured);
        let arrow = placement
            .arrow_left
            .map(|left| style.arrow(body, left.into(), placement.above));
        Some(PrepaintedTooltip {
            element,
            arrow,
            #[cfg(test)]
            painted: PaintedTooltip {
                anchor,
                viewport,
                above: placement.above,
                arrow_left: placement.arrow_left.map(Pixels::from),
                arrow_box: arrow.map(|arrow| arrow.bounds),
                layers: [None; 2],
                text: tip.text.clone(),
                background: style.background,
                foreground: style.foreground,
            },
        })
    }
}

fn rect(bounds: Bounds<Pixels>) -> TooltipRect {
    TooltipRect {
        left: bounds.left().into(),
        top: bounds.top().into(),
        right: bounds.right().into(),
        bottom: bounds.bottom().into(),
    }
}

struct PaintStyle {
    background: Rgba,
    foreground: Rgba,
    border: Option<Rgba>,
    before: [Rgba; 2],
    after: [Rgba; 2],
}

impl PaintStyle {
    fn new(theme: Theme, style: Option<TooltipStyle>, window: &Window) -> Self {
        if let Some(style) = style {
            return Self {
                background: rgba(style.background.rgba32()),
                foreground: theme.on_tooltip,
                border: None,
                before: [
                    rgba(style.arrow_before.bottom.rgba32()),
                    rgba(style.arrow_before.top.rgba32()),
                ],
                after: [
                    rgba(style.arrow_after.bottom.rgba32()),
                    rgba(style.arrow_after.top.rgba32()),
                ],
            };
        }
        // Keep the pinned base layer structure, but use the app's window colors.
        let light = !Theme::is_system_dark(window);
        Self {
            background: theme.tooltip,
            foreground: theme.on_tooltip,
            border: light.then_some(theme.border),
            before: [if light { theme.border } else { theme.tooltip }; 2],
            after: [if light { theme.tooltip } else { rgba(0) }; 2],
        }
    }

    fn body(&self, text: &str, max_width: Pixels, max_height: Option<Pixels>) -> AnyElement {
        div()
            .max_w(max_width)
            .when_some(max_height, |body, height| body.max_h(height))
            .overflow_hidden()
            .px_3()
            .py_2()
            .text_sm()
            .bg(self.background)
            .text_color(self.foreground)
            .when_some(self.border, |body, color| {
                body.border_1().border_color(color)
            })
            .child(text.to_owned())
            .debug_selector(|| "host-source-tooltip".into())
            .into_any_element()
    }

    fn arrow(&self, body: Bounds<Pixels>, left: Pixels, above: bool) -> Arrow {
        // Native placement supplies arrow-left; do not clamp it with the body.
        // The border, when present, offsets the arrow's positioning box by 1 px.
        let edge = px(if self.border.is_some() { 1. } else { 0. });
        let y = if above {
            body.bottom() - edge
        } else {
            body.top() + edge - ARROW_HEIGHT
        };
        Arrow {
            bounds: Bounds::new(
                point(body.left() + left + edge, y),
                size(ARROW_WIDTH, ARROW_HEIGHT),
            ),
            above,
            colors: [
                self.before[usize::from(above)],
                self.after[usize::from(above)],
            ],
        }
    }
}

#[derive(Clone, Copy)]
struct Arrow {
    bounds: Bounds<Pixels>,
    above: bool,
    colors: [Rgba; 2],
}

impl Arrow {
    fn triangle(&self, inset: Pixels) -> [Point<Pixels>; 3] {
        let base = if self.above {
            self.bounds.top()
        } else {
            self.bounds.bottom()
        };
        let tip = if self.above {
            self.bounds.bottom() - inset
        } else {
            self.bounds.top() + inset
        };
        [
            point(self.bounds.left() + inset, base),
            point(self.bounds.right() - inset, base),
            point(self.bounds.center().x, tip),
        ]
    }
}

pub(super) struct PrepaintedTooltip {
    element: AnyElement,
    arrow: Option<Arrow>,
    #[cfg(test)]
    painted: PaintedTooltip,
}

impl PrepaintedTooltip {
    pub(super) fn paint(
        mut self,
        window: &mut Window,
        cx: &mut App,
        #[cfg(test)] painted: &mut Option<PaintedTooltip>,
    ) {
        // The arrow is outside the body's overflow clip, but still inside the
        // editor scroll mask. Neither layer creates a hitbox or layout node.
        if let Some(arrow) = self.arrow {
            for (index, color) in arrow.colors.into_iter().enumerate() {
                if color.a == 0. {
                    continue;
                }
                let points = arrow.triangle(px(index as f32));
                let mut path = PathBuilder::fill();
                path.add_polygon(&points, true);
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                    #[cfg(test)]
                    {
                        self.painted.layers[index] = Some((points, color));
                    }
                }
            }
        }
        self.element.paint(window, cx);
        #[cfg(test)]
        {
            *painted = Some(self.painted);
        }
    }
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(super) struct PaintedTooltip {
    anchor: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    above: bool,
    arrow_left: Option<Pixels>,
    arrow_box: Option<Bounds<Pixels>>,
    layers: [Option<([Point<Pixels>; 3], Rgba)>; 2],
    text: String,
    background: Rgba,
    foreground: Rgba,
}

#[cfg(test)]
mod tests;
