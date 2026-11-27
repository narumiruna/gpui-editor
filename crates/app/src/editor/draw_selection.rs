//! Paint and animate opt-in carets in the editor's measured layout.

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{App, BackgroundExecutor, Bounds, Context, Pixels, Task, Window, fill, rgba};
use gpui_editor_codemirror::{
    Assoc, EditorState, SelectionRange,
    view::{EditorView, ViewPlugin, ViewUpdate, selection_layers::installed_draw_config},
};

use super::{Editor, Layout};

#[derive(Clone, Default)]
pub(super) struct CaretBlink(Rc<RefCell<Option<Blink>>>);

struct Blink {
    phase: Phase,
    task: Option<Task<()>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Phase {
    start: Instant,
    period: Duration,
}

impl Phase {
    fn offset(self, now: Instant) -> Duration {
        let nanos = now.saturating_duration_since(self.start).as_nanos() % self.period.as_nanos();
        Duration::from_nanos(u64::try_from(nanos).expect("cycle configured as u32 milliseconds"))
    }

    fn visible(self, now: Instant) -> bool {
        self.offset(now) < self.period / 2
    }

    fn time_to_edge(self, now: Instant) -> Duration {
        let offset = self.offset(now);
        let half = self.period / 2;
        if offset < half {
            half - offset
        } else {
            self.period - offset
        }
    }
}

impl CaretBlink {
    pub(super) fn install<M>(view: &mut EditorView<M>, cx: &Context<Editor>) -> Self
    where
        M: Clone,
    {
        let blink = Self::default();
        view.add_plugin(Updates {
            blink: blink.clone(),
            clock: cx.background_executor().clone(),
        });
        blink
    }

    pub(super) fn cancel(&self) {
        self.0.borrow_mut().take();
    }
}

struct Updates {
    blink: CaretBlink,
    clock: BackgroundExecutor,
}

impl ViewPlugin for Updates {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        let mut running = self.blink.0.borrow_mut();
        let Some(blink) = running.as_mut() else {
            return;
        };
        if blink_period(update.state) != Some(blink.phase.period) {
            // An incompatible reconfiguration or new ineligible selection cancels
            // the task now. The editor starts a new phase on its next render if needed.
            *running = None;
        } else if update.selection_set {
            // Reset even for an explicit selection identical to the old selection.
            blink.phase.start = self.clock.now();
            blink.task = None;
        }
    }
}

fn blink_period(state: &EditorState) -> Option<Duration> {
    let config = installed_draw_config(state)?;
    (config.cursor_blink_rate != 0
        && state
            .selection()
            .ranges()
            .iter()
            .any(|range| eligible(*range, config.draw_range_cursor)))
    .then(|| Duration::from_millis(config.cursor_blink_rate.into()))
}

fn eligible(range: SelectionRange, draw_range_cursor: bool) -> bool {
    range.is_empty() || draw_range_cursor
}

impl Editor {
    fn caret_period(&self, window: &Window) -> Option<Duration> {
        if window.is_window_active()
            && self.focus.is_focused(window)
            && !self.is_reviewing_changes()
        {
            blink_period(self.view.state())
        } else {
            None
        }
    }

    pub(super) fn sync_caret_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(period) = self.caret_period(window) else {
            self.caret_blink.cancel();
            return;
        };
        let mut running = self.caret_blink.0.borrow_mut();
        let now = cx.background_executor().now();
        let blink = running.get_or_insert_with(|| Blink {
            phase: Phase { start: now, period },
            task: None,
        });
        if blink.task.is_some() {
            return;
        }
        let phase = blink.phase;
        blink.task = Some(cx.spawn_in(window, async move |editor, cx| {
            loop {
                let clock = cx.background_executor();
                clock.timer(phase.time_to_edge(clock.now())).await;
                let active = editor.update_in(cx, |editor, window, cx| {
                    if !editor
                        .caret_blink
                        .0
                        .borrow()
                        .as_ref()
                        .is_some_and(|blink| blink.phase == phase)
                    {
                        return false;
                    }
                    if editor.caret_period(window) != Some(phase.period) {
                        editor.caret_blink.cancel();
                        return false;
                    }
                    cx.notify();
                    true
                });
                if !matches!(active, Ok(true)) {
                    break;
                }
            }
        }));
    }
}

pub(super) fn paint(
    editor: &Editor,
    layout: &mut Layout,
    primary: Option<Bounds<Pixels>>,
    clip: Bounds<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    if !editor.focus.is_focused(window) {
        return;
    }
    let state = editor.view.state();
    let config = installed_draw_config(state);
    if config.is_some()
        && (!window.is_window_active()
            || editor
                .caret_blink
                .0
                .borrow()
                .as_ref()
                .is_some_and(|blink| !blink.phase.visible(cx.background_executor().now())))
    {
        return;
    }

    let selection = state.selection();
    let draw_range_cursor = config.is_some_and(|config| config.draw_range_cursor);
    let primary = primary.filter(|_| eligible(selection.main(), draw_range_cursor));
    let secondary = selection
        .ranges()
        .iter()
        .enumerate()
        .filter_map(|(index, range)| {
            (index != selection.main_index() && eligible(*range, draw_range_cursor))
                .then(|| layout.bounds_at_selection(*range, range.assoc() == Some(Assoc::Before)))
                .flatten()
        });
    let color = layout
        .theme
        .view
        .map_or(layout.theme.accent, |style| rgba(style.cursor.rgba32()));
    #[cfg(test)]
    let mut painted = Vec::new();
    for bounds in primary
        .into_iter()
        .chain(secondary)
        .filter(|bounds| bounds.intersects(&clip))
    {
        let quad = fill(bounds, color);
        #[cfg(test)]
        let submitted = quad.clone();
        window.paint_quad(quad);
        #[cfg(test)]
        painted.push(submitted);
    }
    #[cfg(test)]
    {
        layout.painted_carets = painted;
    }
}

#[cfg(test)]
mod tests;
