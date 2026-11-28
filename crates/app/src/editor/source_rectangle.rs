use gpui::{Context, Modifiers, MouseDownEvent, Pixels, Point, Window, px};
use gpui_editor_codemirror::{
    Text, Transaction,
    view::{
        keymap::KeyModifiers,
        selection_layers::{RectangleHit, RectangleStartEvent, RectangularSelection},
    },
};

use super::Editor;

pub(super) struct RectangleDrag {
    selection: RectangularSelection,
    lines: Text,
    revision: u64,
    width: Pixels,
    multiple: bool,
}

impl Editor {
    pub(super) fn start_source_rectangle(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !event.modifiers.alt || self.is_composing() || self.has_source_snippet() {
            return false;
        }
        if !self
            .layout
            .as_ref()
            .is_some_and(|layout| layout.source_frame_contains(event.position))
        {
            return false;
        }
        let text = window.text_system();
        let Some(width) = text
            .advance(text.resolve_font(&window.text_style().font()), px(16.), '0')
            .ok()
            .map(|advance| advance.width)
            .filter(|width| *width > px(0.))
        else {
            return false;
        };
        let Some(hit) = self.rectangle_hit(event.position, width) else {
            return false;
        };
        let Some(fence) = self.source_fence() else {
            return false;
        };
        let projected = self.source_body_line_projection();
        let state = self.view.state();
        let lines = if let Some(projected) = projected {
            let Ok(lines) = state.text().replace_text(fence.body, &projected) else {
                return false;
            };
            lines
        } else {
            state.text().clone()
        };
        let modifiers = rectangle_modifiers(event.modifiers);
        let Ok(Some(selection)) = RectangularSelection::start_with_lines(
            state,
            RectangleStartEvent {
                button: 0,
                modifiers,
            },
            hit,
            &lines,
        ) else {
            return false;
        };
        self.rectangle = Some(RectangleDrag {
            selection,
            lines,
            revision: self.view.revision(),
            width,
            multiple: modifiers.meta,
        });
        self.selecting = false;
        window.focus(&self.focus);
        self.drag_source_rectangle(event.position, cx)
    }

    pub(super) fn drag_source_rectangle(
        &mut self,
        point: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(drag) = &self.rectangle else {
            return false;
        };
        if self.view.revision() != drag.revision {
            self.rectangle = None;
            return true;
        }
        let Some(hit) = self.rectangle_hit(point, drag.width) else {
            return true;
        };
        let Ok(selection) = drag.selection.selection_with_lines(
            self.view.state(),
            Some(hit),
            drag.multiple,
            &drag.lines,
        ) else {
            return true;
        };
        let Some(scope) = self.source_fence().map(|fence| fence.body) else {
            return true;
        };
        if selection.ranges().iter().any(|range| {
            let range = range.range();
            range.start < scope.start || range.end > scope.end
        }) {
            return true;
        }
        self.record_selection_transaction(
            Transaction::default()
                .with_selection(selection)
                .with_user_event("select.pointer"),
            self.active.clone(),
            cx,
        );
        self.preferred_x = None;
        self.upstream = false;
        cx.notify();
        true
    }

    fn rectangle_hit(&self, point: Point<Pixels>, width: Pixels) -> Option<RectangleHit> {
        let scope = self.source_fence()?.body;
        let layout = self.layout.as_ref()?;
        let position = layout.index_at(point).clamp(scope.start, scope.end);
        let line_end_column = layout.bounds_at(scope.start, false).map(|reference| {
            (f32::from((reference.left() - point.x).abs()) / f32::from(width)).round() as usize
        });
        Some(RectangleHit {
            position,
            line_end_column,
        })
    }
}

fn rectangle_modifiers(modifiers: Modifiers) -> KeyModifiers {
    KeyModifiers {
        alt: modifiers.alt,
        control: modifiers.control,
        meta: modifiers.platform,
        shift: modifiers.shift,
    }
}
