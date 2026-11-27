use gpui::{Modifiers, TestAppContext};
use gpui_editor_codemirror::{Compartment, Precedence, StateEffect, view::placeholder_extension};

use super::*;

fn configure(editor: &mut Editor, effect: StateEffect, cx: &mut Context<Editor>) {
    editor
        .view
        .update(Transaction::default().with_effects([effect]), None, |_| {
            (None, None)
        })
        .unwrap();
    cx.notify();
}

fn assert_painted(editor: &Editor, expected: &[&str]) {
    // The retained layout comes from the document canvas paint callback.
    let layout = editor.layout.as_ref().unwrap();
    let actual: Vec<_> = layout
        .painted_placeholders()
        .map(|(text, _)| text)
        .collect();
    assert_eq!(actual, expected);
}

fn empty_geometry(
    editor: &Editor,
) -> (Bounds<Pixels>, Bounds<Pixels>, Size<Pixels>, Point<Pixels>) {
    assert!(editor.source().is_empty());
    let layout = editor.layout.as_ref().unwrap();
    let [row] = layout.rows.as_slice() else {
        panic!("one empty document row");
    };
    assert!(row.items.is_empty());
    assert_eq!(row.width, px(0.));
    (
        layout.bounds_at(0, false).unwrap(),
        editor.scroll.bounds(),
        editor.scroll.max_offset(),
        editor.scroll.offset(),
    )
}

#[gpui::test]
fn placeholder_opt_in_and_reconfigure_paint_native_widget_order(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
    cx.simulate_resize(size(px(400.), px(240.)));
    cx.run_until_parked();
    let config = Compartment::new();
    let (geometry, selection) = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_painted(editor, &[]);
            let geometry = empty_geometry(editor);
            let selection = editor.view.state().selection().clone();
            // The same extension twice contributes one widget. The highest
            // precedence empty label contributes no visual widget.
            let shared = placeholder_extension("second");
            configure(
                editor,
                config
                    .of(Extension::group([
                        placeholder_extension("last").with_precedence(Precedence::Low),
                        shared.clone(),
                        shared,
                        placeholder_extension("").with_precedence(Precedence::High),
                    ]))
                    .append(),
                cx,
            );
            (geometry, selection)
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.focus.is_focused(window));
            assert_painted(editor, &["second", "last"]);
            assert_eq!(empty_geometry(editor), geometry);
            let layout = editor.layout.as_ref().unwrap();
            let origins: Vec<_> = layout.painted_placeholders().map(|(_, at)| at).collect();
            assert_eq!(origins[0], point(geometry.0.right(), geometry.0.top()));
            assert_eq!(origins[1].y, origins[0].y);
            assert!(origins[1].x > origins[0].x);
            configure(editor, config.reconfigure(placeholder_extension("new")), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_painted(editor, &["new"]);
            configure(editor, config.reconfigure(Extension::default()), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_painted(editor, &[]);
        assert_eq!(empty_geometry(editor), geometry);
        assert_eq!(editor.view.state().selection(), &selection);
        assert!(!editor.view.can_undo());
        assert!(!editor.view.can_undo_selection());
    });
}

#[gpui::test]
fn typing_hides_placeholder_and_undo_paints_the_current_configuration(cx: &mut TestAppContext) {
    let config = Compartment::new();
    let initial = config.of(placeholder_extension("original"));
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(String::new(), initial, window, cx).unwrap()
    });
    cx.update(|window, cx| {
        bind_keys(cx);
        window.activate_window();
        window.focus(&editor.read(cx).focus);
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert_painted(editor.read(cx), &["original"]));
    cx.simulate_input("x");
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), "x");
            assert_eq!(editor.selection(), 1..1);
            assert_painted(editor, &[]);
            configure(
                editor,
                config.reconfigure(placeholder_extension("replacement")),
                cx,
            );
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert_painted(editor.read(cx), &[]));
    cx.simulate_keystrokes("cmd-z");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(editor.focus.is_focused(window));
        assert!(editor.view.focused());
        assert_eq!(editor.source(), "");
        assert_eq!(editor.selection(), 0..0);
        assert_painted(editor, &["replacement"]);
    });
}

#[gpui::test]
fn placeholder_paint_preserves_empty_mouse_hits_and_ime(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
    cx.simulate_resize(size(px(240.), px(200.)));
    cx.run_until_parked();
    let config = Compartment::new();
    let (geometry, hit) = cx.update(|window, cx| {
        window.activate_window();
        editor.update(cx, |editor, cx| {
            let geometry = empty_geometry(editor);
            let hit = geometry.0.origin + point(px(40.), px(10.));
            let layout = editor.layout.as_ref().unwrap();
            assert_eq!(layout.position_at(hit), (0, false));
            assert_eq!(editor.character_index_for_point(hit, window, cx), Some(0));
            assert_eq!(
                editor.bounds_for_range(0..0, geometry.1, window, cx),
                Some(geometry.0)
            );
            configure(
                editor,
                config
                    .of(placeholder_extension(
                        "[plain](note.md)\r\nnext\nlast\u{2028}end",
                    ))
                    .append(),
                cx,
            );
            (geometry, hit)
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            // The single visual row is clipped in this narrow viewport.
            assert_painted(editor, &["[plain](note.md) next last end"]);
            assert_eq!(empty_geometry(editor), geometry);
            let layout = editor.layout.as_ref().unwrap();
            assert!(layout.item_at(hit).is_none());
            assert_eq!(layout.position_at(hit), (0, false));
            assert_eq!(editor.character_index_for_point(hit, window, cx), Some(0));
            assert_eq!(
                editor.bounds_for_range(0..0, geometry.1, window, cx),
                Some(geometry.0)
            );
        });
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.focus.is_focused(window));
            assert_eq!(editor.selection(), 0..0);
            assert_eq!(
                editor.bounds_for_range(0..0, geometry.1, window, cx),
                Some(geometry.0)
            );
            assert_eq!(editor.marked_text_range(window, cx), None);
            let mut actual = None;
            assert_eq!(
                editor
                    .text_for_range(0..5, &mut actual, window, cx)
                    .as_deref(),
                Some("")
            );
            assert_eq!(actual, Some(0..0));
            editor.replace_and_mark_text_in_range(None, "👩‍💻", Some(2..5), window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_painted(editor, &[]);
            assert_eq!(editor.source(), "👩‍💻");
            assert_eq!(editor.marked_text_range(window, cx), Some(0..5));
            assert_eq!(
                editor.selected_text_range(false, window, cx).unwrap().range,
                2..5
            );
            configure(
                editor,
                config.reconfigure(placeholder_extension("restored hint")),
                cx,
            );
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_painted(editor, &[]);
            assert_eq!(editor.marked_text_range(window, cx), Some(0..5));
            assert_eq!(
                editor.selected_text_range(false, window, cx).unwrap().range,
                2..5
            );
            editor.replace_and_mark_text_in_range(None, "", None, window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.focus.is_focused(window));
            assert_eq!(editor.selection(), 0..0);
            assert_eq!(editor.marked_text_range(window, cx), None);
            assert_eq!(empty_geometry(editor), geometry);
            assert_painted(editor, &["restored hint"]);
        });
    });
}
