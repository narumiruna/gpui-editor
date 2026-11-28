use gpui::{Entity, TestAppContext, VisualTestContext};
use gpui_editor_codemirror::{Compartment, theme_one_dark::one_dark_theme};

use super::*;

const TEXT: &str = "<b>[x](y)</b>";
const ROW: &str = "0123456789012345678901234567";

fn setup(cx: &mut TestAppContext) -> (Entity<Editor>, &mut VisualTestContext, usize, Compartment) {
    let source = format!(
        "```js\r\n字🐱\r\n{ROW}\r\n{}\r\n```\n\n```text\n{}\n```",
        "next\n".repeat(22),
        "x".repeat(120)
    );
    let middle = source.find(ROW).unwrap();
    let (editor, cx, theme) = mount(cx, source, middle + 1, middle + 2);
    (editor, cx, middle, theme)
}

fn mount(
    cx: &mut TestAppContext,
    source: String,
    anchor: usize,
    head: usize,
) -> (Entity<Editor>, &mut VisualTestContext, Compartment) {
    let theme = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source, theme.of(one_dark_theme()), window, cx).unwrap()
    });
    cx.simulate_resize(size(px(300.), px(260.)));
    cx.update(|window, cx| {
        bind_keys(cx);
        window.activate_window();
        editor.update(cx, |editor, cx| {
            let fence = editor.markdown().sections()[0].source.clone();
            editor.activate(
                Section::new(SectionKind::DraftCode, fence.clone(), fence),
                false,
                cx,
            );
            editor.set_selection(anchor, head);
            window.focus(&editor.focus);
        });
    });
    cx.run_until_parked();
    (editor, cx, theme)
}

fn show(editor: &Entity<Editor>, cx: &mut VisualTestContext, tip: Tooltip) {
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let snapshot = editor.source_tooltip_snapshot(window).unwrap();
            assert!(editor.show_source_tooltip(&snapshot, tip, TEXT, window, cx));
        });
    });
    cx.run_until_parked();
}

fn frame(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> (PaintedTooltip, Bounds<Pixels>) {
    let body = cx
        .debug_bounds("host-source-tooltip")
        .expect("body painted by GPUI");
    cx.update(|_, cx| {
        (
            editor.read(cx).painted_source_tooltip.clone().unwrap(),
            body,
        )
    })
}

fn absent(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
    cx.run_until_parked();
    // GPUI retains selectors from previous frames in its debug bounds map.
    cx.update(|_, cx| assert!(editor.read(cx).painted_source_tooltip.is_none()));
}

fn close_to(actual: Pixels, expected: Pixels) {
    assert!(
        (f32::from(actual - expected)).abs() < 0.0001,
        "{actual:?} != {expected:?}"
    );
}

fn pinned_arrow_box(above: bool) -> Size<Pixels> {
    #[derive(serde::Deserialize)]
    struct Record {
        name: String,
        arrow: Option<[f32; 4]>,
    }
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/codemirror/tooltip_arrow_browser.jsonl"
    ));
    let name = if above {
        "arrow-above-focused"
    } else {
        "arrow-below"
    };
    let [left, top, right, bottom] = fixture
        .lines()
        .map(|line| serde_json::from_str::<Record>(line).unwrap())
        .find(|row| row.name == name)
        .unwrap()
        .arrow
        .unwrap();
    size(px(right - left), px(bottom - top))
}

fn assert_one_dark(
    frame: &PaintedTooltip,
    body: Bounds<Pixels>,
    above: bool,
    expected_left: Pixels,
) {
    assert_eq!(frame.text, TEXT);
    assert_eq!(frame.above, above);
    assert_eq!(frame.background, rgb(0x353a42));
    assert_eq!(frame.foreground, rgb(0xffffff));
    close_to(body.left(), expected_left);
    let pinned = pinned_arrow_box(above);
    let half_width = pinned.width / 2.;
    let left = frame.anchor.left() - body.left() - half_width;
    close_to(frame.arrow_left.unwrap(), left);
    let arrow = frame.arrow_box.unwrap();
    assert_eq!(arrow.size, pinned);
    close_to(arrow.left(), frame.anchor.left() - half_width);
    let (base, tip) = if above {
        close_to(body.bottom(), frame.anchor.top() - pinned.height);
        close_to(arrow.top(), body.bottom());
        (body.bottom(), frame.anchor.top() - px(1.))
    } else {
        close_to(body.top(), frame.anchor.bottom() + pinned.height);
        close_to(arrow.bottom(), body.top());
        (body.top(), frame.anchor.bottom() + px(1.))
    };
    assert_eq!(frame.layers[0], None);
    let (points, color) = frame.layers[1].unwrap();
    assert_eq!(color, rgb(0x353a42));
    for (actual, expected) in points.into_iter().zip([
        point(frame.anchor.left() - px(6.), base),
        point(frame.anchor.left() + px(6.), base),
        point(frame.anchor.left(), tip),
    ]) {
        close_to(actual.x, expected.x);
        close_to(actual.y, expected.y);
    }
}

fn scroll_by(editor: &Entity<Editor>, cx: &mut VisualTestContext, delta: Point<Pixels>) {
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let scroll = editor.scroll.offset() + delta;
            editor.scroll.set_offset(scroll);
            cx.notify();
        });
    });
    cx.run_until_parked();
}

fn editor_geometry(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
) -> impl Eq + std::fmt::Debug + use<> {
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        let layout = editor.layout.as_ref().unwrap();
        (
            editor.source().to_owned(),
            editor.view.revision(),
            editor.view.state().selection().clone(),
            editor.view.undo_depth(),
            editor.scroll.bounds(),
            editor.scroll.offset(),
            editor.scroll.max_offset(),
            layout.origin,
            editor.focus.is_focused(window),
        )
    })
}

#[gpui::test]
fn live_frames_place_and_paint_the_opt_in_arrow(cx: &mut TestAppContext) {
    let (editor, cx, middle, theme) = setup(cx);
    let before = editor_geometry(&editor, cx);
    let mut tip = Tooltip::new(middle + 10);
    tip.arrow = true;
    tip.above = true;
    show(&editor, cx, tip.clone());
    let (above, body) = frame(&editor, cx);
    assert_one_dark(&above, body, true, above.anchor.left() - px(14.));
    assert_eq!(editor_geometry(&editor, cx), before);

    tip.above = false;
    show(&editor, cx, tip.clone());
    let (below, body) = frame(&editor, cx);
    assert_one_dark(&below, body, false, below.anchor.left() - px(14.));

    tip.arrow = false;
    show(&editor, cx, tip.clone());
    let (plain, plain_body) = frame(&editor, cx);
    assert_eq!(plain.arrow_left, None);
    assert_eq!(plain.arrow_box, None);
    assert_eq!(plain.layers, [None; 2]);
    close_to(plain_body.left(), plain.anchor.left());
    close_to(plain_body.top(), plain.anchor.bottom());
    assert_eq!(plain_body.size, body.size);
    assert_eq!(editor_geometry(&editor, cx), before);

    tip.arrow = true;
    tip.above = true;
    show(&editor, cx, tip.clone());
    // Remove the installed theme while keeping the same tooltip ID and request.
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor
                .view
                .update(
                    Transaction::default().with_effects([theme.reconfigure(Extension::default())]),
                    editor.active.clone(),
                    |_| (None, None),
                )
                .unwrap();
            cx.notify();
        });
    });
    cx.run_until_parked();
    let (fallback, body) = frame(&editor, cx);
    assert_eq!(fallback.anchor, above.anchor);
    assert_eq!(fallback.background, rgb(0x1e293b));
    assert_eq!(fallback.foreground, rgb(0xffffff));
    let arrow = fallback.arrow_box.unwrap();
    close_to(arrow.left(), fallback.anchor.left() - px(6.));
    close_to(arrow.top(), body.bottom() - px(1.));
    assert_eq!(arrow.size, size(px(14.), px(7.)));
    for (index, color) in [rgb(0xd3dce9), rgb(0x1e293b)].into_iter().enumerate() {
        let (points, actual) = fallback.layers[index].unwrap();
        assert_eq!(actual, color);
        let inset = px(index as f32);
        close_to(points[0].x, arrow.left() + inset);
        close_to(points[1].x, arrow.right() - inset);
        close_to(points[0].y, arrow.top());
        close_to(points[2].y, arrow.bottom() - inset);
    }
    assert_eq!(editor_geometry(&editor, cx), before);
}

#[gpui::test]
fn live_frames_flip_clamp_and_hide_without_revealing_source(cx: &mut TestAppContext) {
    let (editor, cx, middle, _) = setup(cx);
    let rightmost = cx.update(|_, cx| {
        let layout = editor.read(cx).layout.as_ref().unwrap();
        (middle..=middle + ROW.len())
            .filter_map(|pos| Some((layout.drop_cursor_bounds(pos)?.left(), pos)))
            .max()
            .unwrap()
            .1
    });
    let mut tip = Tooltip::new(rightmost);
    tip.arrow = true;
    tip.above = true;
    show(&editor, cx, tip.clone());
    let (right, body) = frame(&editor, cx);
    assert_one_dark(&right, body, true, right.viewport.right() - body.size.width);
    assert!(right.arrow_left.unwrap() > px(7.));

    tip.pos = middle;
    show(&editor, cx, tip.clone());
    let (unclamped, _) = frame(&editor, cx);
    scroll_by(
        &editor,
        cx,
        point(
            unclamped.viewport.left() + px(6.) - unclamped.anchor.left(),
            px(0.),
        ),
    );
    let (left, body) = frame(&editor, cx);
    assert_one_dark(&left, body, true, left.viewport.left());
    close_to(left.arrow_left.unwrap(), px(-1.));

    scroll_by(
        &editor,
        cx,
        point(px(0.), left.viewport.top() + px(3.) - left.anchor.top()),
    );
    let (flipped, body) = frame(&editor, cx);
    assert_one_dark(&flipped, body, false, left.viewport.left());
    close_to(flipped.anchor.top(), flipped.viewport.top() + px(3.));

    tip.strict_side = true;
    let before = editor_geometry(&editor, cx);
    show(&editor, cx, tip.clone());
    absent(&editor, cx);
    assert_eq!(editor_geometry(&editor, cx), before);

    scroll_by(&editor, cx, point(px(0.), -px(80.)));
    let before = editor_geometry(&editor, cx);
    tip.strict_side = false;
    tip.clip = false;
    show(&editor, cx, tip);
    absent(&editor, cx);
    assert_eq!(editor_geometry(&editor, cx), before);
}

#[gpui::test]
fn host_route_rejects_unowned_and_stale_requests_and_uses_editor_dismissal(
    cx: &mut TestAppContext,
) {
    let (editor, cx, middle, _) = setup(cx);
    let mut tip = Tooltip::new(middle);
    tip.arrow = true;
    let snapshot = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let snapshot = editor.source_tooltip_snapshot(window).unwrap();
            let other = SHOW_TOOLTIP.of(Some(tip.clone())).append();
            editor
                .view
                .update(
                    Transaction::default().with_effects([other]),
                    editor.active.clone(),
                    |_| (None, None),
                )
                .unwrap();
            for pos in [
                0,
                editor.source().find('🐱').unwrap() + 1,
                middle - 1,
                usize::MAX,
            ] {
                let mut invalid = tip.clone();
                invalid.pos = pos;
                assert!(!editor.show_source_tooltip(&snapshot, invalid, TEXT, window, cx));
            }
            let mut invalid = tip.clone();
            invalid.end = Some(snapshot.body.end + 1);
            assert!(!editor.show_source_tooltip(&snapshot, invalid, TEXT, window, cx));
            for text in ["", "a\u{0}", &"X".repeat(MAX_TEXT_BYTES + 1)] {
                assert!(!editor.show_source_tooltip(&snapshot, tip.clone(), text, window, cx));
            }
            cx.notify();
            snapshot
        })
    });
    absent(&editor, cx);
    show(&editor, cx, tip.clone());
    let baseline = editor_geometry(&editor, cx);
    cx.simulate_keystrokes("escape");
    absent(&editor, cx);
    assert_eq!(editor_geometry(&editor, cx), baseline);

    show(&editor, cx, tip.clone());
    cx.update(|window, _| window.blur());
    absent(&editor, cx);
    cx.update(|window, cx| {
        assert!(editor.read(cx).source_tooltip_snapshot(window).is_none());
        window.focus(&editor.read(cx).focus);
    });
    absent(&editor, cx);

    show(&editor, cx, tip.clone());
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.replace_and_mark_text_in_range(None, "é", None, window, cx);
            assert!(editor.is_composing());
            assert!(!editor.show_source_tooltip(&snapshot, tip.clone(), TEXT, window, cx));
            assert!(editor.source_tooltip_snapshot(window).is_none());
        });
    });
    absent(&editor, cx);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.finish_composition();
            assert!(editor.source_tooltip_snapshot(window).is_some());
            assert!(!editor.show_source_tooltip(&snapshot, tip.clone(), TEXT, window, cx));
        });
    });
    show(&editor, cx, tip.clone());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.replace(middle..middle, "x", cx));
        });
    });
    absent(&editor, cx);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let snapshot = editor.source_tooltip_snapshot(window).unwrap();
            editor.finish_active();
            assert!(!editor.show_source_tooltip(&snapshot, tip, TEXT, window, cx));
            assert!(editor.source_tooltip_snapshot(window).is_none());
        });
    });
}

#[gpui::test]
fn stock_message_paints_at_identical_bounds_with_a_host_request(cx: &mut TestAppContext) {
    let (editor, cx, middle, _) = setup(cx);
    let message = "Cannot edit this Markdown range";
    let set_message = |cx: &mut VisualTestContext, text: Option<&str>| {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.message = text.map(str::to_owned);
                cx.notify();
            });
        });
        cx.run_until_parked();
    };
    set_message(cx, Some(message));
    let bounds = cx.debug_bounds("stock-editor-overlay").unwrap();
    let before = editor_geometry(&editor, cx);
    let mut tip = Tooltip::new(middle);
    tip.arrow = true;
    show(&editor, cx, tip);
    absent(&editor, cx);
    assert_eq!(cx.debug_bounds("stock-editor-overlay"), Some(bounds));
    assert_eq!(editor_geometry(&editor, cx), before);
    set_message(cx, None);
    cx.update(|_, cx| assert!(editor.read(cx).prepainted_overlay.is_none()));
    let _ = frame(&editor, cx);
}

#[gpui::test]
fn stock_diagnostic_and_completion_keep_priority_and_geometry(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::lint::{
        DEFAULT_LINT_DELAY, Diagnostic, LintSource, Severity, linter,
    };

    let source = "```css\np {\n    col\n}\n```\n";
    let from = source.find("col").unwrap();
    let head = from + 3;
    use crate::editor::one_dark_tests::{OverlayText, capture_overlay_text, captured_overlay_text};

    let (editor, cx, theme) = mount(cx, source.into(), head, head);
    let diagnostic = "Stock diagnostic";
    let clock = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let source = LintSource::new(move |_| {
                vec![Diagnostic::new(from..head, Severity::Warning, diagnostic)]
            });
            editor
                .view
                .update(
                    Transaction::default()
                        .with_effects([linter(Some(source), Default::default()).append()]),
                    editor.active.clone(),
                    |_| (None, None),
                )
                .unwrap();
            cx.notify();
        });
        cx.background_executor().clone()
    });
    cx.run_until_parked();
    clock.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    capture_overlay_text();
    cx.simulate_keystrokes("f8");
    cx.run_until_parked();
    let diagnostic_text = captured_overlay_text();
    let pins = include_str!("../../../tests/fixtures/codemirror/tooltip_arrow_browser.jsonl");
    let pinned = pins
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|row| row["name"] == "stock-lint-F8")
        .unwrap();
    assert_eq!(pinned["bodyStyle"]["color"], "rgb(255, 255, 255)");
    let diagnostic_bounds = cx.debug_bounds("stock-editor-overlay").unwrap();
    let before = editor_geometry(&editor, cx);
    let mut tip = Tooltip::new(head);
    tip.arrow = true;
    show(&editor, cx, tip.clone());
    absent(&editor, cx);
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).prepainted_overlay.as_deref(),
            Some(diagnostic)
        );
    });
    assert_eq!(
        cx.debug_bounds("stock-editor-overlay"),
        Some(diagnostic_bounds)
    );
    assert_eq!(editor_geometry(&editor, cx), before);

    cx.simulate_keystrokes("escape");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.move_to(head, false, cx);
        });
    });
    capture_overlay_text();
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    clock.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    clock.advance_clock(Duration::from_millis(75));
    cx.run_until_parked();
    let completion_bounds = cx.debug_bounds("source-completion: color").unwrap();
    let completion_text = captured_overlay_text();
    let pins: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/codemirror/theme_one_dark_chromium.json"
    ))
    .unwrap();
    let roles = &pins["viewRoles"];
    let expected = |name: &str| {
        let hex = u32::from_str_radix(roles[name].as_str().unwrap(), 16).unwrap();
        gpui::Hsla::from(rgba(hex))
    };
    assert!(!diagnostic_text.is_empty());
    assert!(
        diagnostic_text.iter().all(|&(role, color)| {
            role == OverlayText::Anchored && color == rgb(0xffffff).into()
        }),
        "{diagnostic_text:?}"
    );
    assert!(
        completion_text.contains(&(
            OverlayText::Completion,
            expected("completion.unselected.color")
        )),
        "{completion_text:?}"
    );
    assert!(
        completion_text.contains(&(
            OverlayText::SelectedCompletion,
            expected("completion.selected.color")
        )),
        "{completion_text:?}"
    );
    let before = editor_geometry(&editor, cx);
    show(&editor, cx, tip);
    absent(&editor, cx);
    cx.update(|_, cx| assert!(editor.read(cx).prepainted_overlay.is_none()));
    assert_eq!(
        cx.debug_bounds("source-completion: color"),
        Some(completion_bounds)
    );
    assert_eq!(editor_geometry(&editor, cx), before);

    capture_overlay_text();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor
                .view
                .update(
                    Transaction::default().with_effects([
                        theme.reconfigure(gpui_editor_codemirror::Extension::default())
                    ]),
                    editor.active.clone(),
                    |_| (None, None),
                )
                .unwrap();
            cx.notify();
        });
    });
    cx.run_until_parked();
    let fallback = captured_overlay_text();
    assert!(
        fallback.contains(&(OverlayText::Completion, rgb(0x172336).into())),
        "{fallback:?}"
    );
    let restored = cx.debug_bounds("source-completion: color").unwrap();
    // Removing One Dark restores the stock one-pixel popup border.
    assert_eq!(restored.left(), completion_bounds.left() + px(1.));
    assert_eq!(restored.right(), completion_bounds.right() - px(1.));
    assert_eq!(restored.size.height, completion_bounds.size.height);
    assert_eq!(editor_geometry(&editor, cx), before);
}
