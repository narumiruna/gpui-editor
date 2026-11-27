use gpui::{EntityInputHandler, TestAppContext, px, size};

use super::*;
use crate::editor::bind_keys;

#[gpui::test]
fn one_dark_panel_inputs_reconfigure_in_the_goto_line_frame(cx: &mut TestAppContext) {
    use gpui::{Focusable, WindowAppearance};
    use gpui_editor_codemirror::{Compartment, Extension, theme_one_dark::one_dark_theme};

    use crate::editor::one_dark_tests::{configure, panel_input_colors};

    let source = "one\r\ntwo";
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source.into(), slot.of(Extension::default()), window, cx)
            .unwrap()
    });
    cx.simulate_resize(size(px(640.), px(420.)));
    cx.update(|window, cx| {
        assert_eq!(window.appearance(), WindowAppearance::Light);
        bind_keys(cx);
        window.focus(&editor.read(cx).focus);
    });
    cx.simulate_keystrokes("cmd-alt-g");
    cx.run_until_parked();
    let input = cx.update(|_, cx| editor.read(cx).goto_line.as_ref().unwrap().input.clone());
    cx.update(|window, cx| {
        assert_eq!(input.read(cx).text(), "1");
        input.update(cx, |field, cx| {
            let selection = field.selected_text_range(false, window, cx).unwrap();
            assert_eq!(selection.range, 0..1);
            field.replace_and_mark_text_in_range(Some(1..1), "二🦀", Some(1..3), window, cx);
        });
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("goto-line-value").unwrap();
    let probe = |window: &mut Window, cx: &mut App| {
        let field = input.read(cx);
        assert!(field.focus_handle(cx).is_focused(window));
        assert_eq!(field.text(), "1二🦀");
        assert!(field.is_composing());
        input.update(cx, |field, cx| {
            assert_eq!(field.marked_text_range(window, cx), Some(1..4));
            assert_eq!(
                field.selected_text_range(false, window, cx).unwrap().range,
                2..4
            );
            field.bounds_for_range(2..4, bounds, window, cx).unwrap()
        })
    };
    let before = cx.update(probe);
    for (extension, themed) in [(one_dark_theme(), true), (Extension::default(), false)] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, slot.reconfigure(extension), cx)
            });
        });
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("goto-line-value").unwrap(), bounds);
        assert_eq!(cx.update(probe), before);
        cx.update(|_, cx| {
            assert_eq!(
                input.read(cx).painted_colors(),
                panel_input_colors("goto", themed, true)
            );
        });
        cx.simulate_keystrokes("enter escape");
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(editor.goto_line.is_some());
            assert_eq!(editor.source(), source);
            assert_eq!(editor.cursor(), 0);
            assert!(!editor.view.can_undo());
        });
    }
}

#[gpui::test]
fn goto_line_uses_a_utf16_column_and_centers_the_line(cx: &mut TestAppContext) {
    let source: String = (0..100)
        .map(|line| format!("🦀 line{line:03}\r\n"))
        .collect();
    let opened = source.clone();
    let target = source.find("line050").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(opened, window, cx));
    cx.simulate_resize(size(px(700.), px(420.)));
    cx.update(|window, cx| {
        bind_keys(cx);
        window.focus(&editor.read(cx).focus);
    });
    cx.simulate_keystrokes("cmd-alt-g");
    cx.run_until_parked();
    assert!(cx.debug_bounds("goto-line-value").is_some());
    cx.simulate_input("51:3");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(editor.goto_line.is_none());
        assert_eq!(editor.cursor(), target);
        assert_eq!(editor.source(), source);
        assert!(editor.focus.is_focused(window));
        assert!(!editor.view.can_undo());
        let bounds = editor
            .layout
            .as_ref()
            .unwrap()
            .bounds_at(target, false)
            .unwrap();
        let viewport = editor.scroll.bounds();
        assert!(
            (bounds.center().y - viewport.center().y).abs() < px(5.),
            "{bounds:?}, {viewport:?}, {:?}",
            editor.scroll.offset()
        );
    });
}

#[gpui::test]
fn invalid_goto_line_and_input_composition_leave_the_caret_alone(cx: &mut TestAppContext) {
    let (editor, cx) =
        cx.add_window_view(|window, cx| Editor::new("one\r\ntwo".into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        window.focus(&editor.read(cx).focus);
    });
    cx.simulate_keystrokes("cmd-alt-g");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let input = editor.read(cx).goto_line.as_ref().unwrap().input.clone();
        input.update(cx, |field, cx| {
            field.replace_and_mark_text_in_range(None, "二", None, window, cx)
        });
    });
    cx.simulate_keystrokes("escape enter");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let state = editor.read(cx);
        let input = state.goto_line.as_ref().unwrap().input.clone();
        assert_eq!(state.cursor(), 0);
        input.update(cx, |field, cx| field.unmark_text(window, cx));
    });
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(editor.goto_line.is_none());
        assert_eq!(editor.cursor(), 0);
        assert!(editor.focus.is_focused(window));
    });
}
