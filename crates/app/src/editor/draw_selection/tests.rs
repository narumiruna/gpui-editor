use gpui::{EntityInputHandler, Rgba, TestAppContext, point, px, rgb, size};
use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Change, ChangeSet, Compartment, EditError, EditorSelection,
    Extension, READ_ONLY, StateEffect, Transaction,
    language::bidi_isolates,
    theme_one_dark::one_dark_theme,
    view::{
        Viewport,
        selection_layers::{DrawSelectionOptions, draw_selection},
    },
};

use super::*;
use crate::document::{Section, SectionKind};

fn options(rate: u32, draw_ranges: bool) -> Extension {
    draw_selection(DrawSelectionOptions {
        cursor_blink_rate: Some(rate),
        draw_range_cursor: Some(draw_ranges),
        ..Default::default()
    })
}

fn apply(editor: &mut Editor, transaction: Transaction, cx: &mut Context<Editor>) {
    let active = editor.active.clone();
    editor
        .view
        .update(transaction, active.clone(), |_| (None, active))
        .unwrap();
    cx.notify();
}

fn configure(editor: &mut Editor, effect: StateEffect, cx: &mut Context<Editor>) {
    apply(editor, Transaction::default().with_effects([effect]), cx);
}

fn select(editor: &mut Editor, selection: EditorSelection, cx: &mut Context<Editor>) {
    apply(editor, Transaction::default().with_selection(selection), cx);
}

fn running(editor: &Editor) -> bool {
    editor
        .caret_blink
        .0
        .borrow()
        .as_ref()
        .is_some_and(|blink| blink.task.is_some())
}

fn assert_quads(editor: &Editor, bounds: &[Bounds<Pixels>], color: Rgba) {
    let layout = editor.layout.as_ref().unwrap();
    let actual: Vec<_> = layout
        .painted_carets
        .iter()
        .map(|q| (q.bounds, q.background))
        .collect();
    let expected: Vec<_> = bounds
        .iter()
        .map(|&bounds| (bounds, color.into()))
        .collect();
    assert_eq!(actual, expected);
}

fn assert_heads(editor: &Editor, indices: &[usize], color: Rgba) {
    let selection = editor.view.state().selection();
    let layout = editor.layout.as_ref().unwrap();
    let bounds: Vec<_> = indices
        .iter()
        .map(|&index| {
            let range = selection.ranges()[index];
            let upstream = if index == selection.main_index() {
                editor.upstream
            } else {
                range.assoc() == Some(Assoc::Before)
            };
            layout.bounds_at_selection(range, upstream).unwrap()
        })
        .collect();
    assert_quads(editor, &bounds, color);
}

#[gpui::test]
fn submitted_carets_follow_the_default_clock_and_accepted_selection_sets(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            "Alpha Beta".into(),
            Extension::group([draw_selection(Default::default()), one_dark_theme()]),
            window,
            cx,
        )
        .unwrap()
    });
    let clock = cx.executor();
    cx.update(|window, cx| {
        window.activate_window();
        editor.update(cx, |editor, cx| {
            window.focus(&editor.focus);
            select(editor, EditorSelection::single(3, 3), cx);
        });
    });
    let assert_visible = |cx: &mut gpui::VisualTestContext, visible: bool| {
        cx.update(|_, cx| {
            assert_heads(
                editor.read(cx),
                if visible { &[0] } else { &[] },
                rgb(0x528bff),
            )
        });
    };
    cx.run_until_parked();
    assert_visible(cx, true);
    clock.advance_clock(Duration::from_millis(599));
    assert_visible(cx, true);
    clock.advance_clock(Duration::from_millis(1));
    assert_visible(cx, false);

    // Keep the original phase through a mapped edit, a viewport change, and an unrelated redraw.
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let viewport =
                Viewport::around(editor.view.state().text(), std::iter::once(0..3)).unwrap();
            assert_ne!(editor.view.viewport(), &viewport);
            editor.view.set_viewport_geometry(viewport).unwrap();
            cx.notify();
            let mapped = ChangeSet::single(Change::new(0..0, "new ")).unwrap();
            apply(editor, Transaction::new(mapped), cx);
            assert_eq!(editor.cursor(), 7);
        })
    });
    assert_visible(cx, false);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(
                editor.view.update_checked(
                    Transaction::default().with_selection(EditorSelection::single(0, 0)),
                    None,
                    |_| Err(EditError::InvalidSelection),
                    |_| (None, None),
                ),
                Err(EditError::InvalidSelection)
            );
            assert_eq!(editor.cursor(), 7);
            cx.notify();
        })
    });
    clock.advance_clock(Duration::from_millis(599));
    assert_visible(cx, false);
    clock.advance_clock(Duration::from_millis(1));
    assert_visible(cx, true);
    clock.advance_clock(Duration::from_millis(600));
    assert_visible(cx, false);

    // An accepted selection identical to the current selection still restarts immediately.
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            select(editor, EditorSelection::single(7, 7), cx);
        })
    });
    cx.run_until_parked();
    assert_visible(cx, true);
    clock.advance_clock(Duration::from_millis(599));
    assert_visible(cx, true);
    clock.advance_clock(Duration::from_millis(1));
    assert_visible(cx, false);
}

#[gpui::test]
fn native_focus_activation_and_review_stop_and_restart_the_clock(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions("Draft".into(), options(100, true), window, cx).unwrap()
    });
    let clock = cx.executor();
    cx.update(|window, cx| {
        window.activate_window();
        window.focus(&editor.read(cx).focus);
    });
    let check = |cx: &mut gpui::VisualTestContext, scheduled, visible| {
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(running(editor), scheduled);
            assert_heads(
                editor,
                if visible { &[0] } else { &[] },
                editor.layout.as_ref().unwrap().theme.accent,
            );
        });
    };
    cx.run_until_parked();
    check(cx, true, true);
    clock.advance_clock(Duration::from_millis(50));
    check(cx, true, false);
    cx.update(|window, _| window.blur());
    check(cx, false, false);
    clock.advance_clock(Duration::from_secs(1));
    check(cx, false, false);
    cx.update(|window, cx| window.focus(&editor.read(cx).focus));
    check(cx, true, true);
    clock.advance_clock(Duration::from_millis(50));
    check(cx, true, false);
    cx.deactivate_window();
    check(cx, false, false);
    clock.advance_clock(Duration::from_secs(1));
    check(cx, false, false);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    check(cx, true, true);
    clock.advance_clock(Duration::from_millis(50));
    check(cx, true, false);

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| editor.review_changes("Saved", window, cx))
    });
    clock.advance_clock(Duration::from_secs(1));
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(!running(editor));
        assert!(editor.is_reviewing_changes());
        assert!(editor.layout.is_none());
    });
    cx.update(|_, cx| editor.update(cx, Editor::dismiss_merge_review));
    check(cx, true, true);
    clock.advance_clock(Duration::from_millis(50));
    check(cx, true, false);
}

#[gpui::test]
fn reconfigure_preserves_phase_and_only_eligible_heads_schedule(cx: &mut TestAppContext) {
    let config = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            "abcdefghijklmnop".into(),
            Extension::group([
                config.of(Extension::default()),
                ALLOW_MULTIPLE_SELECTIONS.of(true),
            ]),
            window,
            cx,
        )
        .unwrap()
    });
    let clock = cx.executor();
    let mixed = || {
        EditorSelection::new(
            vec![
                SelectionRange::new(0, 2),
                SelectionRange::new(9, 5),
                SelectionRange::new(12, 12),
            ],
            1,
        )
        .unwrap()
    };
    cx.update(|window, cx| {
        window.activate_window();
        editor.update(cx, |editor, cx| {
            window.focus(&editor.focus);
            select(editor, mixed(), cx);
        });
    });
    let check = |cx: &mut gpui::VisualTestContext, scheduled, indices: &[usize]| {
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(running(editor), scheduled);
            assert_heads(
                editor,
                indices,
                editor.layout.as_ref().unwrap().theme.accent,
            );
        });
    };
    clock.advance_clock(Duration::from_secs(1));
    check(cx, false, &[2]); // Absent: a steady empty secondary, no range heads.
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            configure(editor, config.reconfigure(options(100, false)), cx);
        })
    });
    check(cx, true, &[2]);
    clock.advance_clock(Duration::from_millis(50));
    check(cx, true, &[]);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            configure(editor, config.reconfigure(options(100, true)), cx);
        })
    });
    check(cx, true, &[]); // Eligibility changed, but an eligible head and the rate remain.
    clock.advance_clock(Duration::from_millis(50));
    check(cx, true, &[1, 0, 2]); // The primary is reversed and precedes the secondary quads.

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            configure(editor, config.reconfigure(options(0, true)), cx);
        })
    });
    clock.advance_clock(Duration::from_secs(1));
    check(cx, false, &[1, 0, 2]);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            select(
                editor,
                EditorSelection::new(
                    vec![SelectionRange::new(0, 2), SelectionRange::new(9, 5)],
                    1,
                )
                .unwrap(),
                cx,
            );
            configure(editor, config.reconfigure(options(100, false)), cx);
        })
    });
    clock.advance_clock(Duration::from_secs(1));
    check(cx, false, &[]);
    cx.update(|_, cx| editor.update(cx, |editor, cx| select(editor, mixed(), cx)));
    check(cx, true, &[2]);
    clock.advance_clock(Duration::from_millis(50));
    check(cx, true, &[]);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            configure(editor, config.reconfigure(Extension::default()), cx);
        })
    });
    clock.advance_clock(Duration::from_secs(1));
    check(cx, false, &[2]);
}

#[gpui::test]
fn measured_wrap_bounds_remain_available_while_blink_hides_the_range_head(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions("a".repeat(100), options(100, true), window, cx).unwrap()
    });
    cx.simulate_resize(size(px(230.), px(600.)));
    let clock = cx.executor();
    let head = cx.update(|window, cx| {
        window.activate_window();
        editor.update(cx, |editor, cx| {
            let layout = editor.layout.as_ref().unwrap();
            let first = &layout.rows[0];
            let (head, upstream) =
                layout.position_at(layout.origin + point(first.width + px(2.), first.height / 2.));
            assert!(upstream);
            window.focus(&editor.focus);
            select(editor, EditorSelection::single(head + 1, head), cx);
            editor.upstream = true;
            head
        })
    });
    cx.run_until_parked();
    let (upper, lower) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        let layout = editor.layout.as_ref().unwrap();
        let upper = layout.bounds_at(head, true).unwrap();
        let lower = layout.bounds_at(head, false).unwrap();
        assert!(upper.top() < lower.top());
        assert_quads(editor, &[upper], layout.theme.accent);
        (upper, lower)
    });
    clock.advance_clock(Duration::from_millis(50));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let layout = editor.layout.as_ref().unwrap();
            assert_quads(editor, &[], layout.theme.accent);
            assert_eq!(
                editor.bounds_for_range(head..head, editor.scroll.bounds(), window, cx),
                Some(upper)
            );
            editor.upstream = false;
            cx.notify();
        })
    });
    clock.advance_clock(Duration::from_millis(50));
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_quads(
            editor,
            &[lower],
            editor.layout.as_ref().unwrap().theme.accent,
        );
    });
}

#[gpui::test]
fn composing_and_read_only_do_not_add_caret_paint_gates(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions("Draft".into(), options(0, true), window, cx).unwrap()
    });
    cx.update(|window, cx| {
        window.activate_window();
        editor.update(cx, |editor, cx| {
            window.focus(&editor.focus);
            editor.replace_and_mark_text_in_range(None, "猫", Some(0..1), window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.is_composing());
            assert_eq!(editor.selection(), 0.."猫".len());
            assert_heads(editor, &[0], editor.layout.as_ref().unwrap().theme.accent);
            editor.unmark_text(window, cx);
            configure(editor, READ_ONLY.of(true).append(), cx);
            editor.replace_text_in_range(None, "X", window, cx);
            assert_eq!(editor.source(), "猫Draft");
        });
    });
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(editor.view.state().read_only());
        assert_heads(editor, &[0], editor.layout.as_ref().unwrap().theme.accent);
        assert!(!running(editor));
    });
}

#[gpui::test]
fn drawn_carets_follow_measured_bidi_stops_or_the_standard_layout(cx: &mut TestAppContext) {
    const MIXED: &str = "(module (data \"ABC אב 🦀\") ;; عرب";
    let source = format!("```wast\n{MIXED}\n{MIXED}\n)\n```\n");
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.clone(),
            Extension::group([
                options(0, true),
                bidi_isolates(false),
                ALLOW_MULTIPLE_SELECTIONS.of(true),
            ]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(470.), px(400.)));
    let (first, second) = cx.update(|window, cx| {
        window.activate_window();
        editor.update(cx, |editor, cx| {
            let fence = 0..source.len();
            editor.activate(
                Section::new(SectionKind::DraftCode, fence.clone(), fence),
                false,
                cx,
            );
            let body = editor.source_fence().unwrap().body;
            let first = body.start + 19;
            let second = first + MIXED.len() + 1;
            window.focus(&editor.focus);
            select(
                editor,
                EditorSelection::new(
                    vec![
                        EditorSelection::range(first + 2, first, None, None, Some(Assoc::After)),
                        EditorSelection::cursor(second, Some(Assoc::Before), None, None),
                    ],
                    0,
                )
                .unwrap(),
                cx,
            );
            (first, second)
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let layout = editor.layout.as_ref().unwrap();
        let at = |head, side| {
            layout
                .bounds_at_selection(EditorSelection::cursor(head, Some(side), None, None), false)
                .unwrap()
        };
        for head in [first, second] {
            let measured = layout.uses_bidi_carets(head);
            #[cfg(target_os = "macos")]
            assert!(measured, "TextKit must supply caret stops for this fixture");
            if measured {
                assert!(at(head, Assoc::After).left() - at(head, Assoc::Before).left() > px(5.));
            } else {
                let standard = layout.bounds_at(head, false).unwrap();
                assert_eq!(at(head, Assoc::After), standard);
                assert_eq!(at(head, Assoc::Before), standard);
            }
        }
        assert_quads(
            editor,
            &[at(first, Assoc::After), at(second, Assoc::Before)],
            layout.theme.accent,
        );
        assert!(!running(editor));
        assert_eq!(editor.source(), source);
    });
}
