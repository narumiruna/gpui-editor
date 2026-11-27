use gpui::{Bounds, Pixels, Point, point, px};
use gpui_editor_codemirror::view::{
    FixedScroller, ScrollOffset, ScrollRect, ScrollSnapshot, ScrollSnapshotGeometry,
    ScrollStrategy, ScrollTarget, ScrollView, bidi::Direction,
};

use super::{Editor, RevealAlignment, VERTICAL_PADDING, layout::Layout};

impl Editor {
    pub(super) fn capture_source_scroll(&self) -> Option<ScrollSnapshot> {
        let layout = self.layout.as_ref()?;
        let first = layout.scroll_block_at_height(0.0);
        let offset = self.scroll.offset();
        ScrollSnapshot::capture(
            self.view.state().text(),
            ScrollSnapshotGeometry {
                offset: ScrollOffset {
                    x: f64::from(-offset.x),
                    y: f64::from(-offset.y),
                },
                padding_top: f64::from(VERTICAL_PADDING),
                viewport_from: first.from,
                first_viewport_block: first,
            },
            |height| Some(layout.scroll_block_at_height(height)),
        )
    }

    pub(super) fn restore_source_scroll(&self, layout: &Layout, snapshot: ScrollSnapshot) -> bool {
        let Some(offset) = snapshot.plan_restore(self.view.state().text(), |from| {
            Some(layout.scroll_block_top(from))
        }) else {
            return false;
        };
        // ScrollHandle offsets are negative distances from the document origin.
        self.set_source_scroll(point(-Pixels::from(offset.x), -Pixels::from(offset.y)))
    }

    pub(super) fn reveal_source_bounds(
        &self,
        from: usize,
        bounds: Bounds<Pixels>,
        alignment: RevealAlignment,
    ) -> bool {
        let viewport = self.scroll.bounds();
        if viewport.size.width <= px(8.) || viewport.size.height <= px(8.) {
            return false;
        }
        let (y, y_margin) = match alignment {
            RevealAlignment::Nearest => (ScrollStrategy::Nearest, 4.0),
            RevealAlignment::Center => (ScrollStrategy::Center, 4.0),
            RevealAlignment::Start(margin) => (ScrollStrategy::Start, f64::from(margin)),
        };
        let target = ScrollTarget {
            y,
            y_margin,
            x_margin: 4.0,
            ..ScrollTarget::at(from)
        };
        let view = ScrollView {
            state: self.view.state(),
            viewport: self.view.viewport(),
            focused: self.view.focused(),
        };
        if view.handle_scroll(target) {
            return false;
        }
        let limit = self.scroll.max_offset();
        let geometry = FixedScroller {
            clip: scroll_rect(viewport),
            offset_width: viewport.size.width.into(),
            offset_height: viewport.size.height.into(),
            scale_x: 1.0,
            scale_y: 1.0,
            scroll_x: limit.width > px(0.),
            scroll_y: limit.height > px(0.),
            direction: Direction::Ltr,
        };
        let Some(delta) = view.plan_reveal(target, geometry, |_, _| Some(scroll_rect(bounds)))
        else {
            return false;
        };
        self.set_source_scroll(
            self.scroll.offset() - point(Pixels::from(delta.x), Pixels::from(delta.y)),
        )
    }

    fn set_source_scroll(&self, next: Point<Pixels>) -> bool {
        let max = self.scroll.max_offset();
        let next = point(
            next.x.clamp(-max.width, px(0.)),
            next.y.clamp(-max.height, px(0.)),
        );
        if next == self.scroll.offset() {
            return false;
        }
        self.scroll.set_offset(next);
        true
    }
}

fn scroll_rect(bounds: Bounds<Pixels>) -> ScrollRect {
    ScrollRect {
        left: bounds.left().into(),
        top: bounds.top().into(),
        right: bounds.right().into(),
        bottom: bounds.bottom().into(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use gpui::{TestAppContext, size};
    use gpui_editor_codemirror::{
        Extension, Transaction,
        view::{covered_margins, scroll_handler},
    };

    use super::*;

    #[gpui::test]
    fn source_reveal_runs_handlers_before_default_scrolling(cx: &mut TestAppContext) {
        let source = format!("{}\nLast line", "first line\n\n".repeat(80));
        let from = source.find("Last line").unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.simulate_resize(size(px(400.), px(350.)));
        let intercept = Arc::new(AtomicBool::new(true));
        let calls = Arc::new(AtomicUsize::new(0));
        let providers = Arc::new(AtomicUsize::new(0));
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let count = calls.clone();
                let toggle = intercept.clone();
                let handler = scroll_handler(move |view, target| {
                    assert!(!view.viewport.visible_ranges().is_empty());
                    assert_eq!(target.range.head(), from);
                    count.fetch_add(1, Ordering::Relaxed);
                    toggle.load(Ordering::Relaxed)
                });
                let count = providers.clone();
                let margin = covered_margins(move |_, _| {
                    count.fetch_add(1, Ordering::Relaxed);
                    None
                });
                editor
                    .view
                    .update(
                        Transaction::default()
                            .with_effects([Extension::group([handler, margin]).append()]),
                        None,
                        |_| (None, None),
                    )
                    .unwrap();
                editor.reveal_source(from, cx);
            })
        });
        cx.run_until_parked();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(providers.load(Ordering::Relaxed), 0);
        cx.update(|_, cx| assert_eq!(editor.read(cx).scroll.offset().y, px(0.)));
        intercept.store(false, Ordering::Relaxed);
        cx.update(|_, cx| editor.update(cx, |editor, cx| editor.reveal_source(from, cx)));
        cx.run_until_parked();
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(providers.load(Ordering::Relaxed), 1);
        cx.update(|_, cx| assert!(editor.read(cx).scroll.offset().y < px(0.)));
    }
}
