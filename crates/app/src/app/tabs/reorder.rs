//! Drag preview, edge scrolling, and insertion for the document tab strip.

use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::{
    App, Bounds, Context, Entity, Pixels, Point, Rgba, Task, WeakEntity, Window, canvas, fill,
    point, prelude::*, px, size,
};

use super::super::{Shell, TabId};

const EDGE_WIDTH: Pixels = px(28.);
const SCROLL_STEP: Pixels = px(12.);
pub(super) const SCROLL_TICK: Duration = Duration::from_millis(16);

// This payload is deliberately distinct from sidebar tabs and the sidebar resize grip.
pub(super) struct DocumentDrag(pub(super) TabId);

pub(in crate::app) struct DragPreview {
    shell: WeakEntity<Shell>,
    id: TabId,
    accent: Rgba,
    painted_offset: Rc<Cell<Pixels>>,
    _scroll: Task<()>,
}

impl DocumentDrag {
    pub(super) fn preview(
        &self,
        shell: WeakEntity<Shell>,
        accent: Rgba,
        window: &Window,
        cx: &mut App,
    ) -> Entity<DragPreview> {
        let id = self.0;
        let offset = shell
            .upgrade()
            .map_or(px(0.), |shell| shell.read(cx).tab_scroll.offset().x);
        let preview = cx.new(|cx| DragPreview {
            shell: shell.clone(),
            id,
            accent,
            painted_offset: Rc::new(Cell::new(offset)),
            // GPUI's test window does not request display frames. Use its executor's
            // clock in both environments, and drop the task with the drag preview.
            _scroll: cx.spawn_in(window, {
                let shell = shell.clone();
                async move |_, cx| {
                    loop {
                        cx.background_executor().timer(SCROLL_TICK).await;
                        let active = shell.update_in(cx, |shell, window, cx| {
                            if !cx.has_active_drag()
                                || !shell.can_drag_document_tabs(cx)
                                || shell.tab(id).is_none()
                            {
                                shell.cancel_document_drag(window, cx);
                                return false;
                            }
                            shell.scroll_during_document_drag(window.mouse_position(), cx);
                            true
                        });
                        if !matches!(active, Ok(true)) {
                            break;
                        }
                    }
                }
            }),
        });
        let _ = shell.update(cx, |shell, _| {
            shell.document_drag = Some(preview.downgrade())
        });
        preview
    }
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (shell, id, accent) = (self.shell.clone(), self.id, self.accent);
        let painted_offset = self.painted_offset.clone();
        // The drag layer paints after the strip is laid out. Draw in window coordinates
        // using the latest scroll position, rather than at the drag view's cursor offset.
        canvas(
            |_, _, _| (),
            move |_, (), window, cx| {
                if let Some(shell) = shell.upgrade() {
                    let shell = shell.read(cx);
                    painted_offset.set(shell.tab_scroll.offset().x);
                    if let Some(line) =
                        shell.document_insertion_line(id, window.mouse_position(), cx)
                    {
                        window.paint_quad(fill(line, accent));
                    }
                }
            },
        )
    }
}

impl Shell {
    pub(super) fn can_drag_document_tabs(&self, cx: &App) -> bool {
        self.active.is_none()
            && self.palette.is_none()
            && !self.developer.blocks_document_actions()
            && !self.is_composing(cx)
    }

    fn scroll_during_document_drag(&self, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        let strip = self.tab_scroll.bounds();
        if !strip.contains(&pointer) {
            return;
        }
        let edge = EDGE_WIDTH.min(strip.size.width / 2.);
        let step = if pointer.x < strip.left() + edge {
            SCROLL_STEP
        } else if pointer.x > strip.right() - edge {
            -SCROLL_STEP
        } else {
            return;
        };
        let mut offset = self.tab_scroll.offset();
        let next = (offset.x + step).clamp(-self.tab_scroll.max_offset().width, px(0.));
        if next != offset.x {
            offset.x = next;
            self.tab_scroll.set_offset(offset);
            cx.notify();
        }
    }

    /// Return the slot in the original order, before removing the dragged tab.
    fn document_insertion_slot(&self, pointer: Point<Pixels>, offset: Pixels) -> Option<usize> {
        if !self.tab_scroll.bounds().contains(&pointer) {
            return None;
        }
        // ScrollHandle reports child bounds without applying the scroll offset.
        let x = pointer.x - offset;
        for index in 0..self.tabs.len() {
            if x < self.tab_scroll.bounds_for_item(index)?.center().x {
                return Some(index);
            }
        }
        Some(self.tabs.len())
    }

    pub(super) fn document_insertion_line(
        &self,
        id: TabId,
        pointer: Point<Pixels>,
        cx: &App,
    ) -> Option<Bounds<Pixels>> {
        if !self.can_drag_document_tabs(cx) {
            return None;
        }
        let from = self.tabs.iter().position(|tab| tab.id == id)?;
        let slot = self.document_insertion_slot(pointer, self.tab_scroll.offset().x)?;
        if slot == from || slot == from + 1 {
            return None;
        }
        let x = if slot == self.tabs.len() {
            self.tab_scroll.bounds_for_item(slot - 1)?.right()
        } else {
            self.tab_scroll.bounds_for_item(slot)?.left()
        } + self.tab_scroll.offset().x;
        let strip = self.tab_scroll.bounds();
        let width = px(2.).min(strip.size.width);
        let left = (x - width / 2.).clamp(strip.left(), strip.right() - width);
        Some(Bounds::new(
            point(left, strip.top() + px(4.)),
            size(width, strip.size.height - px(8.)),
        ))
    }

    pub(in crate::app) fn cancel_document_drag(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if self
            .document_drag
            .take()
            .and_then(|drag| drag.upgrade())
            .is_some()
        {
            cx.stop_active_drag(window)
        } else {
            false
        }
    }

    pub(super) fn drop_document_tab(
        &mut self,
        event: &gpui::MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(preview) = self.document_drag.take().and_then(|drag| drag.upgrade()) else {
            return;
        };
        if !cx.stop_active_drag(window) {
            return;
        }
        cx.stop_propagation();
        if self.active.is_some() || !self.allow_document_action(false, cx) {
            return;
        }
        let drag = preview.read(cx);
        let Some(from) = self.tabs.iter().position(|tab| tab.id == drag.id) else {
            return;
        };
        // Mouse-up is hit-tested against the last painted frame. An edge scroll
        // tick can already have updated the offset for the next frame.
        let Some(slot) = self.document_insertion_slot(event.position, drag.painted_offset.get())
        else {
            return;
        };
        let to = slot - usize::from(from < slot);
        if from != to {
            let tab = self.tabs.remove(from);
            self.tabs.insert(to, tab);
            cx.notify();
        }
    }
}
