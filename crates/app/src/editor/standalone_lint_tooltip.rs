use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gpui::{Bounds, Context, MouseExitEvent, MouseMoveEvent, Pixels, Point, Task, Window};
use gpui_editor_codemirror::{
    lint::{Diagnostic, lint_gutter_tooltip_at},
    view::{ViewPlugin, ViewUpdate},
};

use super::{Editor, Overlay, layout::Layout};

const MAX_TEXT_BYTES: usize = 512;

struct Hover {
    line_start: usize,
    plan: Option<(String, Duration)>,
    dwell: Option<Task<()>>,
}

#[derive(Default)]
struct HoverState {
    current: RefCell<Option<Hover>>,
    observing_updates: Cell<bool>,
}

#[derive(Clone, Default)]
pub(super) struct StandaloneLintHover(Rc<HoverState>);

impl StandaloneLintHover {
    pub(super) fn clear(&self) -> bool {
        self.0
            .current
            .take()
            .is_some_and(|hover| hover.plan.is_some())
    }
}

impl ViewPlugin for StandaloneLintHover {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        // A filter can read state changed by effects. In particular, the
        // marker set can retain its identity when only its filter changes.
        if update.doc_changed() || !update.effects.is_empty() {
            self.clear();
        }
    }

    fn destroy(&mut self) {
        self.0.observing_updates.set(false);
        self.clear();
    }
}

impl Editor {
    fn standalone_lint_hit(
        &self,
        layout: &Layout,
        pointer: Point<Pixels>,
        window: &Window,
    ) -> Option<(usize, Bounds<Pixels>)> {
        if !self.input_enabled
            || self.is_composing()
            || self.selecting
            || self.rectangle.is_some()
            || !window.is_window_active()
            || !self.focus.is_focused(window)
        {
            return None;
        }
        layout.standalone_lint_hit(self.view.state(), pointer)
    }

    pub(super) fn track_standalone_lint_hover(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hit = self
            .layout
            .as_ref()
            .filter(|_| event.pressed_button.is_none())
            .and_then(|layout| self.standalone_lint_hit(layout, event.position, window));
        let Some((line_start, _)) = hit else {
            if self.standalone_lint_hover.clear() {
                cx.notify();
            }
            return;
        };
        let state = &self.standalone_lint_hover;
        if !state.0.observing_updates.replace(true) {
            self.view.add_plugin(state.clone());
        }
        let mut current = state.0.current.borrow_mut();
        if let Some(hover) = &mut *current
            && hover.line_start == line_start
        {
            if hover.dwell.is_some()
                && let Some((_, delay)) = &hover.plan
            {
                hover.dwell = Some(start_dwell(*delay, window, cx));
            }
            return;
        }
        let was_open = current
            .as_ref()
            .is_some_and(|hover| hover.plan.is_some() && hover.dwell.is_none());
        let plan = lint_gutter_tooltip_at(self.view.state(), line_start)
            .ok()
            .flatten()
            .and_then(|tip| literal_messages(tip.diagnostics).map(|text| (text, tip.delay)));
        let dwell = plan
            .as_ref()
            .map(|(_, delay)| start_dwell(*delay, window, cx));
        *current = Some(Hover {
            line_start,
            plan,
            dwell,
        });
        if was_open {
            cx.notify();
        }
    }

    pub(super) fn standalone_lint_overlay(
        &self,
        layout: &Layout,
        window: &Window,
        show: bool,
    ) -> Option<Overlay> {
        let mut current = self.standalone_lint_hover.0.current.borrow_mut();
        let hover = current.as_ref()?;
        let hit = self.standalone_lint_hit(layout, window.mouse_position(), window);
        let Some((_, anchor)) = hit.filter(|(line, _)| *line == hover.line_start) else {
            *current = None;
            return None;
        };
        if !show || hover.dwell.is_some() {
            return None;
        }
        Some(Overlay {
            text: hover.plan.as_ref()?.0.clone(),
            anchor: Some(anchor),
            viewport: layout.standalone_lint_viewport()?,
            caret: None,
            prefer_above: false,
        })
    }
}

// GPUI's window-exit event does not change its last mouse position or call
// the div's mouse-move listener. Listen without taking or blocking input.
pub(super) fn listen_for_window_exit(editor: &gpui::Entity<Editor>, window: &mut Window) {
    let editor = editor.downgrade();
    window.on_mouse_event(move |_: &MouseExitEvent, phase, _, cx| {
        if phase.capture() {
            let _ = editor.update(cx, |editor, cx| {
                if editor.standalone_lint_hover.clear() {
                    cx.notify();
                }
            });
        }
    });
}

fn start_dwell(delay: Duration, window: &Window, cx: &Context<Editor>) -> Task<()> {
    let timer = cx.background_executor().timer(delay);
    cx.spawn_in(window, async move |editor, cx| {
        timer.await;
        let _ = editor.update_in(cx, |editor, window, cx| {
            let hit = editor.layout.as_ref().and_then(|layout| {
                editor.standalone_lint_hit(layout, window.mouse_position(), window)
            });
            let mut current = editor.standalone_lint_hover.0.current.borrow_mut();
            let Some(hover) = &mut *current else { return };
            if hit.is_some_and(|(line, _)| line == hover.line_start) {
                drop(hover.dwell.take());
                cx.notify();
            } else {
                *current = None;
            }
        });
    })
}

fn literal_messages(diagnostics: Vec<Diagnostic>) -> Option<String> {
    let mut text = String::new();
    for diagnostic in diagnostics {
        let message = diagnostic.message;
        if message.is_empty() {
            continue;
        }
        let gap = usize::from(!text.is_empty());
        let available = MAX_TEXT_BYTES.saturating_sub(text.len() + gap);
        let end = message.floor_char_boundary(available);
        if end == 0 {
            break;
        }
        if gap != 0 {
            text.push('\n');
        }
        text.push_str(&message[..end]);
        if end < message.len() {
            break;
        }
    }
    (!text.is_empty()).then_some(text)
}
