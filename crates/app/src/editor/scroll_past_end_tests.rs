use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase};
use gpui_editor_codemirror::{
    Compartment, StateEffect,
    view::{SCROLL_SNAPSHOT, scroll_past_end},
};

use super::*;

fn apply_effects(
    editor: &mut Editor,
    effects: impl IntoIterator<Item = StateEffect>,
    cx: &mut Context<Editor>,
) {
    editor
        .view
        .update(Transaction::default().with_effects(effects), None, |_| {
            (None, None)
        })
        .unwrap();
    cx.notify();
}

fn assert_scroll_extent(editor: &Editor, expected: Pixels) {
    // GPUI rounds layout bounds to whole pixels; the policy uses half pixels.
    let max = editor.scroll.max_offset();
    assert_eq!(max.width, px(0.));
    assert!((max.height - expected).abs() <= px(0.5), "{max:?}");
}

#[gpui::test]
fn scroll_past_end_preserves_source_hits_and_restores_a_scroll_snapshot(cx: &mut TestAppContext) {
    let source = "one\r\nsecond\r\n[last](last.md)";
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.simulate_resize(size(px(400.), px(240.)));
    cx.run_until_parked();
    let config = Compartment::new();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_scroll_extent(editor, px(0.));
            editor.set_selection(2, 1);
            apply_effects(editor, [config.of(scroll_past_end()).append()], cx);
        });
    });
    cx.run_until_parked();
    let viewport = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_scroll_extent(editor, px(51.5));
        assert_eq!(editor.scroll.offset().y, px(0.));
        editor.scroll.bounds()
    });
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-240.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    let (snapshot, offset) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        let offset = editor.scroll.offset().y;
        assert_eq!(offset, -editor.scroll.max_offset().height);
        let layout = editor.layout.as_ref().unwrap();
        let last = layout.rows.last().unwrap();
        let last_top = layout.origin.y + last.y;
        assert!((last_top - viewport.top() - px(VERTICAL_PADDING)).abs() <= px(1.));
        let x = layout.origin.x + px(1.);
        assert!(Editor::hovered_link(layout, point(x, last_top + px(1.))).is_some());
        let blank = point(x, viewport.bottom() - px(1.));
        assert!(layout.item_at(blank).is_none());
        assert_eq!(
            layout.position_at(blank),
            layout.position_at(point(x, last_top + px(1.)))
        );
        (editor.capture_source_scroll().unwrap(), offset)
    });

    cx.simulate_resize(size(px(400.), px(300.)));
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.scroll.bounds().size.height, px(300.));
            assert_scroll_extent(editor, px(51.5));
            assert_eq!(editor.scroll.offset().y, offset);
            apply_effects(editor, [config.reconfigure(Extension::default())], cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_scroll_extent(editor, px(0.));
            assert_eq!(editor.scroll.offset().y, px(0.));
            apply_effects(
                editor,
                [
                    config.reconfigure(scroll_past_end()),
                    SCROLL_SNAPSHOT.of(snapshot),
                ],
                cx,
            );
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.scroll.offset().y, offset);
        assert_eq!(editor.source(), source);
        assert_eq!(
            editor.view.state().selection(),
            &EditorSelection::single(2, 1)
        );
        assert!(!editor.view.can_undo());
        assert!(!editor.view.can_undo_selection());
    });
}

#[gpui::test]
fn scroll_past_end_preserves_composition_and_protected_markdown(cx: &mut TestAppContext) {
    let fence = format!("```text\r\n{}\r\n```", "wide preview ".repeat(12));
    let source = format!("before\r\n\r\n{fence}");
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source, window, cx));
    cx.simulate_resize(size(px(400.), px(240.)));
    cx.update(|window, cx| window.focus(&editor.read(cx).focus));
    cx.run_until_parked();
    let (selection, horizontal) = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let horizontal = editor.scroll.max_offset().width;
            assert!(horizontal > px(0.));
            editor.replace_and_mark_text_in_range(Some(0..0), "👩‍💻", Some(5..5), window, cx);
            assert!(editor.is_composing());
            apply_effects(editor, [scroll_past_end().append()], cx);
            (editor.view.state().selection().clone(), horizontal)
        })
    });
    cx.simulate_resize(size(px(400.), px(300.)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), format!("👩‍💻before\r\n\r\n{fence}"));
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.marked_text_range(window, cx), Some(0..5));
            assert_eq!(editor.scroll.max_offset().width, horizontal);
            assert!(editor.scroll.max_offset().height > px(0.));
        });
    });
}

#[gpui::test]
fn scroll_past_end_retains_padding_at_small_heights_without_growing_the_viewport(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new("one row".into(), window, cx));
    cx.simulate_resize(size(px(400.), px(40.)));
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            apply_effects(editor, [scroll_past_end().append()], cx);
        });
    });
    cx.run_until_parked();
    for (height, max) in [(40., 1010.), (240., 0.), (40., 199.5), (51., 0.)] {
        cx.simulate_resize(size(px(400.), px(height)));
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.scroll.bounds().size.height, px(height));
            assert_scroll_extent(editor, px(max));
            assert_eq!(editor.scroll.offset().y, px(0.));
            assert_eq!(editor.source(), "one row");
            assert_eq!(editor.selection(), 0..0);
        });
    }
}
