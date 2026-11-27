use gpui::{EntityInputHandler, Rgba, TestAppContext};
use gpui_editor_codemirror::{
    Assoc, Compartment, EditorSelection, Extension, READ_ONLY, StateEffect, Transaction,
    theme_one_dark::one_dark_theme,
    view::selection_layers::{drop_cursor, drop_cursor_position, set_drop_cursor},
};

use super::*;
use crate::document::{BlockPreview, Section};

fn apply(editor: &mut Editor, effect: StateEffect, cx: &mut gpui::Context<Editor>) {
    editor.active = editor
        .view
        .update(
            Transaction::default()
                .with_effects([effect])
                .with_add_to_history(false),
            editor.active.clone(),
            |_| (None, editor.active.clone()),
        )
        .unwrap();
    cx.notify();
}

fn track(editor: &mut Editor, position: Option<usize>, cx: &mut gpui::Context<Editor>) {
    let effect = set_drop_cursor(editor.view.state(), position)
        .unwrap()
        .unwrap();
    apply(editor, effect, cx);
}

fn assert_paint(editor: &Editor, expected: Option<(usize, Rgba)>) {
    let layout = editor.layout.as_ref().unwrap();
    match (&layout.painted_drop_cursor, expected) {
        (None, None) => {}
        (Some(quad), Some((offset, color))) => {
            let caret = EditorSelection::cursor(offset, Some(Assoc::After), None, None);
            let bounds = layout.bounds_at_selection(caret, false).unwrap();
            assert_eq!(quad.bounds.left(), bounds.left() - px(0.6));
            assert_eq!(quad.bounds.top(), bounds.top());
            assert_eq!(quad.bounds.size, size(px(1.2), bounds.size.height));
            assert_eq!(quad.background, color.into());
        }
        (quad, expected) => panic!(
            "submitted {:?}, expected {expected:?}",
            quad.as_ref().map(|q| q.bounds)
        ),
    }
}

#[gpui::test]
fn drop_cursor_paints_only_an_opted_in_marker_and_tracks_native_mapping(cx: &mut TestAppContext) {
    let source = "plain 👩‍💻 text";
    let target = source.find("text").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    let tracking = Compartment::new();
    let theme = Compartment::new();
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_paint(editor, None);
            assert!(
                set_drop_cursor(editor.view.state(), Some(target))
                    .unwrap()
                    .is_none()
            );
            editor.set_selection(1, 3);
            apply(editor, tracking.of(drop_cursor()).append(), cx);
            apply(editor, theme.of(one_dark_theme()).append(), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.focus.is_focused(window));
            assert_paint(editor, None);
            track(editor, Some(target), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_paint(editor, Some((target, rgb(0x528bff))));
            assert_eq!(editor.source(), source);
            assert_eq!(editor.selection(), 1..3);
            assert_eq!(editor.view.revision(), 0);
            assert!(!editor.view.can_undo());
            assert!(!editor.view.can_undo_selection());
            assert!(editor.replace(0..0, "new\n", cx));
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(drop_cursor_position(editor.view.state()), Some(target + 4));
            assert_paint(editor, Some((target + 4, rgb(0x528bff))));
            apply(editor, theme.reconfigure(Extension::default()), cx);
            editor.replay_history(false, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), source);
            assert_eq!(editor.selection(), 1..3);
            assert!(!editor.view.can_undo());
            assert!(editor.view.can_redo());
            assert_paint(editor, Some((target, rgb(0x000000))));
            track(editor, None, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_paint(editor, None);
            track(editor, Some(target), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_paint(editor, Some((target, rgb(0x000000))));
            apply(editor, tracking.reconfigure(Extension::default()), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(drop_cursor_position(editor.read(cx).view.state()), None);
        assert_paint(editor.read(cx), None);
    });
}

#[gpui::test]
fn drop_cursor_suppresses_unprojected_and_protected_positions(cx: &mut TestAppContext) {
    let source = "## Title ##\n\nplain `inline` and 👩‍💻\n\n```json\n{\n  \"word\": true\n}\n```\n";
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.simulate_resize(size(px(600.), px(600.)));
    cx.update(|_, cx| editor.update(cx, |editor, cx| apply(editor, drop_cursor().append(), cx)));
    let body = source.find("word").unwrap();
    let fence = source.find("json").unwrap();
    for target in [
        source.find(" ##").unwrap() + 2,  // Hidden heading markup.
        source.find("inline").unwrap(),   // Visible but read-only text.
        source.find('\u{200d}').unwrap(), // Inside one grapheme.
        body,                             // Read-only preview.
    ] {
        cx.update(|_, cx| editor.update(cx, |editor, cx| track(editor, Some(target), cx)));
        cx.run_until_parked();
        cx.update(|_, cx| {
            assert_eq!(
                drop_cursor_position(editor.read(cx).view.state()),
                Some(target)
            );
            assert_paint(editor.read(cx), None);
        });
    }
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let code = editor
                .markdown()
                .sections()
                .into_iter()
                .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                .unwrap()
                .source;
            editor.activate(
                Section::new(SectionKind::DraftCode, code.clone(), code),
                false,
                cx,
            );
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_paint(editor, Some((body, rgb(0x000000))));
            track(editor, Some(fence), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_paint(editor, None);
            editor.set_selection(body, body);
            assert!(editor.toggle_source_fold(cx));
            track(editor, Some(body), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(editor.layout.as_ref().unwrap().fold_hides(body));
        assert_paint(editor, None);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn drop_cursor_respects_current_viewport_input_policy_and_composition(cx: &mut TestAppContext) {
    let middle = "middle\n".repeat(35);
    let wide = "wide preview ".repeat(12);
    let source = format!("first\n{middle}\n```text\n{wide}\n```");
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
    let config = Compartment::new();
    cx.simulate_resize(size(px(400.), px(170.)));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            apply(
                editor,
                Extension::group([drop_cursor(), config.of(Extension::default())]).append(),
                cx,
            );
            track(editor, Some(1), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_paint(editor, Some((1, rgb(0x000000))));
            let max = editor.scroll.max_offset();
            assert!(max.width > px(20.));
            editor.scroll.set_offset(point(-max.width, px(0.)));
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(drop_cursor_position(editor.view.state()), Some(1));
            assert_paint(editor, None);
            editor
                .scroll
                .set_offset(point(px(0.), -editor.scroll.max_offset().height));
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(drop_cursor_position(editor.view.state()), Some(1));
            assert_paint(editor, None);
            editor.scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_paint(editor, Some((1, rgb(0x000000))));
            editor.set_input_enabled(false, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_paint(editor, None);
            editor.set_input_enabled(true, cx);
            apply(editor, config.reconfigure(READ_ONLY.of(true)), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_paint(editor, None);
            apply(editor, config.reconfigure(Extension::default()), cx);
            editor.replace_and_mark_text_in_range(Some(0..0), "é", None, window, cx);
            assert!(editor.is_composing());
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(drop_cursor_position(editor.view.state()), Some(3));
            assert_paint(editor, None);
            editor.unmark_text(window, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_paint(editor.read(cx), Some((3, rgb(0x000000))));
        assert_eq!(editor.read(cx).source(), format!("é{source}"));
    });
}

#[gpui::test]
fn drop_cursor_paints_an_empty_document_caret(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            apply(editor, drop_cursor().append(), cx);
            track(editor, Some(0), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert_paint(editor.read(cx), Some((0, rgb(0x000000)))));
}
