use std::{
    future::Future,
    pin::pin,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Poll, Waker},
};

use gpui::{EntityInputHandler, Modifiers, TestAppContext};
use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Change, ChangeSet, Compartment, Extension, SelectionRange,
    StateEffect, Transaction,
    language::fold::{
        FoldPlaceholderConfig, code_folding_with_placeholders, fold_effect, fold_gutter,
        fold_service, folded_ranges,
    },
    language_data::{LANGUAGES, LanguageDescription, json_extension, lezer_extension},
    lint::{Diagnostic, lint_gutter, set_diagnostics_effect},
    setup::basic_setup,
    theme_one_dark::one_dark_theme,
    transaction_filter,
    view::{
        ViewHighlight,
        gutters::{GutterMarker, LineNumberConfig, highlight_active_line_gutter},
        keymap::KeyPlatform,
    },
};
use serde::Deserialize;

use super::*;
use crate::{
    document::{BlockPreview, Section, SectionKind},
    editor::{HORIZONTAL_PADDING, one_dark_tests::painted_runs},
};

const CODE: &str = "Before\r\n\r\n```json\r\n{\r\n  \"word\": \"字\",\r\n  \"list\": [1, 2]\r\n}\r\n```\r\n\r\nAfter";

fn enter_code(editor: &mut Editor, cx: &mut Context<Editor>) {
    let range = editor
        .markdown()
        .sections()
        .into_iter()
        .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
        .unwrap()
        .source;
    editor.activate(
        Section::new(SectionKind::DraftCode, range.clone(), range),
        false,
        cx,
    );
}

fn configure(editor: &mut Editor, effect: StateEffect, cx: &mut Context<Editor>) {
    let active = editor.active.clone();
    editor
        .view
        .update(
            Transaction::default().with_effects([effect]),
            active.clone(),
            |_| (None, active),
        )
        .unwrap();
    cx.notify();
}

fn frame(editor: &Editor) -> (&Layout, &SourceGutter) {
    let layout = editor.layout.as_ref().unwrap();
    let gutter = layout
        .source_frame
        .as_ref()
        .unwrap()
        .gutter
        .as_ref()
        .unwrap();
    (layout, gutter)
}

fn row_hit(editor: &Editor, start: usize, first: bool) -> Point<Pixels> {
    let (layout, gutter) = frame(editor);
    let row = layout
        .rows
        .iter()
        .find(|row| {
            let Some((number, is_first)) = gutter.line_at_row(row) else {
                return false;
            };
            is_first == first
                && gutter.state.text().line(number).unwrap().range.start + gutter.body.start
                    == start
        })
        .unwrap();
    layout.origin + gutter.fold_bounds(row).center()
}

fn assert_only_fold(editor: &Editor, range: Range<usize>) {
    assert_eq!(editor.source_fold_ranges(), std::slice::from_ref(&range));
}

fn markers(editor: &Editor) -> Vec<(usize, FoldGutterState, &str)> {
    frame(editor)
        .1
        .folds
        .as_ref()
        .unwrap()
        .markers
        .iter()
        .map(|(&start, marker)| {
            (
                start,
                marker.state,
                marker.marker.content.as_deref().unwrap(),
            )
        })
        .collect()
}

#[gpui::test]
fn fold_gutter_opt_in_click_interception_and_reconfiguration(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CODE.into(), window, cx));
    cx.simulate_resize(size(px(420.), px(420.)));
    cx.run_until_parked();
    let open = CODE.find('{').unwrap();
    let close = CODE.find('}').unwrap();
    let end = CODE.find("  \"list\"").unwrap();
    let config = Compartment::new();
    let intercept = Arc::new(AtomicBool::new(true));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let stock = FoldGutterConfig {
        open_text: "hide".into(),
        closed_text: "show".into(),
        handle_click: Some(Arc::new({
            let calls = calls.clone();
            let intercept = intercept.clone();
            move |state, line| {
                assert_eq!(state.text().as_str().len(), CODE.len());
                calls.lock().unwrap().push(line.clone());
                intercept.load(Ordering::Relaxed)
            }
        })),
        ..Default::default()
    };
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.layout.as_ref().unwrap().source_frame.is_none());
            enter_code(editor, cx);
            let word = CODE.find("word").unwrap();
            assert!(editor.replace(word..word + 4, "name", cx));
            // Keep an existing selection inside the range, with nonempty undo history.
            editor.set_selection(word + 3, word);
        })
    });
    cx.run_until_parked();
    let (numbers_width, selection, revision) = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let (_, gutter) = frame(editor);
            assert!(gutter.folds.is_none());
            let width = gutter.width;
            let selection = editor.view.state().selection().clone();
            let revision = editor.view.revision();
            configure(
                editor,
                config
                    .of(Extension::group([
                        fold_gutter(stock),
                        fold_service(move |_, from, to| {
                            ((from..to) == (open..open + 1)).then_some(open + 1..end)
                        }),
                    ]))
                    .append(),
                cx,
            );
            (width, selection, revision)
        })
    });
    cx.run_until_parked();
    let (hit, width) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        let (_, gutter) = frame(editor);
        assert_eq!(gutter.numbers_width, numbers_width);
        assert!(gutter.width > numbers_width);
        assert_eq!(markers(editor), [(open, FoldGutterState::Open, "hide")]);
        (row_hit(editor, open, true), gutter.width)
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| assert!(editor.read(cx).source_fold_ranges().is_empty()));
    assert_eq!(
        *calls.lock().unwrap(),
        std::slice::from_ref(&(open..open + 1))
    );
    intercept.store(false, Ordering::Relaxed);
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    let hit = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_only_fold(editor, open + 1..end);
        assert_eq!(markers(editor), [(open, FoldGutterState::Closed, "show")]);
        assert_eq!(frame(editor).1.width, width);
        assert_eq!(editor.view.state().selection(), &selection);
        row_hit(editor, open, true)
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        *calls.lock().unwrap(),
        [open..open + 1, open..open + 1, open..open + 1]
    );
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.source_fold_ranges().is_empty());
            configure(
                editor,
                config.reconfigure(fold_gutter(FoldGutterConfig {
                    render_marker: Some(Arc::new(|_, kind| {
                        GutterMarker::text(match kind {
                            FoldGutterState::Open => "custom hide",
                            FoldGutterState::Closed => "custom show",
                        })
                    })),
                    ..Default::default()
                })),
                cx,
            );
        })
    });
    cx.run_until_parked();
    let hit = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(
            markers(editor),
            [(open, FoldGutterState::Open, "custom hide")]
        );
        row_hit(editor, open, true)
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_only_fold(editor, open + 1..close);
            assert_eq!(
                markers(editor),
                [(open, FoldGutterState::Closed, "custom show")]
            );
            configure(editor, config.reconfigure(Extension::default()), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let (_, gutter) = frame(editor);
        assert!(gutter.folds.is_none());
        assert_eq!(gutter.width, numbers_width);
        assert_only_fold(editor, open + 1..close);
        assert_eq!(editor.source(), CODE.replacen("word", "name", 1));
        assert_eq!(editor.view.state().selection(), &selection);
        assert_eq!(editor.view.revision(), revision);
        assert_eq!(editor.view.undo_depth(), 1);
        assert_eq!(editor.view.redo_depth(), 0);
        assert!(!editor.view.can_undo_selection());
    });
}

#[gpui::test]
fn fold_gutter_first_row_hit_does_not_act_during_composition(cx: &mut TestAppContext) {
    let source = "```js\r\nconst some_long_name_here = {\r\n  word: '字',\r\n};\r\n```";
    let start = source.find("const").unwrap();
    let open = source.find('{').unwrap();
    let close = source.find('}').unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.simulate_resize(size(px(214.), px(430.)));
    let calls = Arc::new(Mutex::new(Vec::new()));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            enter_code(editor, cx);
            window.focus(&editor.focus);
            configure(
                editor,
                fold_gutter(FoldGutterConfig {
                    handle_click: Some(Arc::new({
                        let calls = calls.clone();
                        move |_, line| {
                            calls.lock().unwrap().push(line.clone());
                            false
                        }
                    })),
                    ..Default::default()
                })
                .append(),
                cx,
            );
        })
    });
    cx.run_until_parked();
    let continuation = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(markers(editor), [(start, FoldGutterState::Open, "⌄")]);
        row_hit(editor, start, false)
    });
    cx.simulate_click(continuation, Modifiers::none());
    cx.run_until_parked();
    assert!(calls.lock().unwrap().is_empty());
    let (first, selection) = cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            let hit = row_hit(editor, start, true);
            let head = source.find('字').unwrap();
            editor.set_selection(head, head);
            editor.marked = Some(head..head + '字'.len_utf8());
            (hit, editor.view.state().selection().clone())
        })
    });
    cx.simulate_click(first, Modifiers::none());
    cx.run_until_parked();
    assert!(calls.lock().unwrap().is_empty());
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.is_composing());
            assert!(editor.source_fold_ranges().is_empty());
            assert_eq!(editor.view.state().selection(), &selection);
            editor.finish_composition();
        })
    });
    cx.simulate_click(first, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_only_fold(editor, open + 1..close);
        assert_eq!(editor.view.state().selection(), &selection);
        assert!(!editor.view.can_undo());
        assert_eq!(editor.source(), source);
    });
    assert_eq!(
        *calls.lock().unwrap(),
        std::slice::from_ref(&(start..open + 1))
    );
}

#[gpui::test]
fn fold_gutter_rejects_invalid_open_closed_and_filtered_effects(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CODE.into(), window, cx));
    cx.simulate_resize(size(px(600.), px(430.)));
    let open = CODE.find('{').unwrap();
    let close = CODE.find('}').unwrap();
    let outside = CODE.rfind("```").unwrap() + 1;
    let word = CODE.find('字').unwrap();
    let config = Compartment::new();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            enter_code(editor, cx);
            editor.set_selection(open, open);
            configure(editor, fold_gutter(Default::default()).append(), cx);
            configure(editor, config.of(Extension::default()).append(), cx);
        })
    });
    cx.run_until_parked();
    for rejected in [
        0..close,
        open + 1..outside,
        open + 1..word + 1,
        open + 2..close,
    ] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(
                    editor,
                    config.reconfigure(fold_service(move |_, from, _| {
                        (from == open).then(|| rejected.clone())
                    })),
                    cx,
                )
            })
        });
        cx.run_until_parked();
        let hit = cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(markers(editor).is_empty());
            row_hit(editor, open, true)
        });
        cx.simulate_click(hit, Modifiers::none());
        cx.run_until_parked();
        cx.update(|_, cx| assert!(folded_ranges(editor.read(cx).view.state()).is_empty()));
    }
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            configure(
                editor,
                config.reconfigure(transaction_filter(move |_, tr| {
                    if tr
                        .effects()
                        .iter()
                        .any(|effect| effect.get(fold_effect()).is_some())
                    {
                        return Ok(Transaction::new(ChangeSet::single(Change::new(0..1, "X"))?));
                    }
                    Ok(tr)
                })),
                cx,
            );
        })
    });
    cx.run_until_parked();
    let hit = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(markers(editor), [(open, FoldGutterState::Open, "⌄")]);
        row_hit(editor, open, true)
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.source_fold_ranges().is_empty());
            assert_eq!(editor.source(), CODE);
            configure(editor, config.reconfigure(Extension::default()), cx);
            configure(editor, fold_effect().of(open + 1..outside), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(markers(editor).is_empty());
        assert_eq!(hit, row_hit(editor, open, true));
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(folded_ranges(editor.view.state()).len(), 1);
        assert!(editor.source_fold_ranges().is_empty());
        assert_eq!(editor.source(), CODE);
        assert_eq!(editor.cursor(), open);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn fold_gutter_projects_wast_lone_cr_rows_without_editing_bytes(cx: &mut TestAppContext) {
    let source = "```wat\r\n(module\r  (func\r    nop)\r)\r\n```";
    let open = source.find("(module").unwrap();
    let func = source.find("  (func").unwrap();
    let close = source.rfind(')').unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.simulate_resize(size(px(600.), px(420.)));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            enter_code(editor, cx);
            editor.set_selection(open, open);
            configure(editor, fold_gutter(Default::default()).append(), cx);
        })
    });
    cx.run_until_parked();
    let hit = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(
            markers(editor),
            [
                (open, FoldGutterState::Open, "⌄"),
                (func, FoldGutterState::Open, "⌄")
            ]
        );
        row_hit(editor, open, true)
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(markers(editor), [(open, FoldGutterState::Closed, "›")]);
        assert_only_fold(editor, open + 1..close);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}

// Recorded in Chrome 158.0.8127.37 with SHA-512-verified CodeMirror pins.
const STANDALONE: &str = "const label = 'café'; // this logical line wraps across several measured display rows\n\nconst end = 42;\n";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StandaloneFrame<'a> {
    path: &'a str,
    mode: &'a str,
    focused: bool,
    body_active: Vec<usize>,
    body_fill: Option<&'a str>,
    slots: Vec<&'a str>,
    content_left: f32,
    width: Option<f32>,
    spacer: Option<&'a str>,
    base_foreground: Option<&'a str>,
    base_background: Option<&'a str>,
    border: Option<&'a str>,
    #[serde(default, borrow)]
    rows: Vec<StandaloneRow<'a>>,
}

#[derive(Deserialize)]
struct StandaloneRow<'a> {
    label: &'a str,
    active: bool,
    color: &'a str,
    fill: &'a str,
    height: f32,
    blank: bool,
}

fn fixture_color(hex: &str) -> Rgba {
    rgba(u32::from_str_radix(hex, 16).unwrap())
}

fn assert_standalone_paint(
    layout: &Layout,
    text: &Text,
    viewport_width: Pixels,
    frame: &StandaloneFrame<'_>,
    window: &Window,
) -> Pixels {
    let paints = &layout.painted_active_lines;
    assert!(layout.source_frame.is_none());
    let gutter = layout.standalone_gutter.as_ref();
    let preset_only = frame.mode == "browser-basic-setup-only";
    assert_eq!(gutter.is_some(), frame.width.is_some() && !preset_only);
    let inset = gutter.map_or(px(0.), StandaloneGutter::inset);
    let measure_line = |number| {
        let line = text.line(number).unwrap();
        let first = layout.bounds_at(line.range.start, false).unwrap();
        let last = layout.bounds_at(line.range.end, false).unwrap();
        assert_eq!(first.left(), layout.origin.x + inset);
        assert_eq!(
            layout.index_at(first.origin + point(px(0.), px(1.))),
            line.range.start
        );
        assert_eq!(
            layout.line_block(line.range.start),
            Some(line.range.clone())
        );
        (line.range, first.top(), last.bottom())
    };
    let expected_body = frame.body_active.iter().flat_map(|&number| {
        let (_, top, bottom) = measure_line(number);
        let color = fixture_color(frame.body_fill.unwrap());
        layout.rows.iter().filter_map(move |row| {
            let y = layout.origin.y + row.y;
            (top <= y && y < bottom).then_some((
                Bounds::new(
                    point(layout.origin.x + inset, y),
                    size(viewport_width - inset, row.height),
                ),
                color,
            ))
        })
    });
    let assert_quads = |painted: &[gpui::PaintQuad], expected: &[(Bounds<Pixels>, Rgba)]| {
        assert_eq!(
            painted.len(),
            expected.len(),
            "{} {}",
            frame.path,
            frame.mode
        );
        for (quad, (bounds, color)) in painted.iter().zip(expected) {
            assert_eq!(quad.bounds, *bounds);
            assert_eq!(quad.background, (*color).into());
        }
    };
    assert_quads(&paints.content, &expected_body.collect::<Vec<_>>());
    for row in &layout.rows {
        assert!(row.width <= viewport_width);
        assert_eq!(row.items.first().map_or(row.width, |item| item.left), inset);
    }
    let Some(gutter) = gutter else {
        assert!(paints.gutter.is_empty());
        assert!(paints.ordinary_gutter.is_empty());
        assert!(paints.gutter_labels.is_empty());
        assert!(paints.standalone_gutter_border.is_none());
        if !preset_only {
            assert!(frame.slots.is_empty());
            assert_eq!(frame.content_left, 0.);
        }
        return inset;
    };
    assert_eq!(frame.slots, ["cm-gutter cm-lineNumbers"]);
    let themed = frame.mode != "theme-removed";
    let foreground = fixture_color(frame.base_foreground.unwrap());
    let background = fixture_color(frame.base_background.unwrap());
    // Unthemed GPUI retains its native colors, not the browser's light palette.
    let (foreground, background) = if themed {
        (foreground, background)
    } else {
        assert_eq!(foreground, rgba(0x6c6c6cff));
        assert_eq!(background, rgba(0xf5f5f5ff));
        (layout.theme.muted, layout.theme.preview)
    };
    let last = layout.rows.last().unwrap();
    let height = last.y + last.height;
    assert_quads(
        &paints.ordinary_gutter,
        &[(
            Bounds::new(layout.origin, size(gutter.width, height)),
            background,
        )],
    );
    let border = if themed {
        assert_eq!(frame.border, Some("0px"));
        assert_eq!(frame.content_left, frame.width.unwrap());
        None
    } else {
        assert_eq!(frame.border, Some("1px"));
        assert_eq!(frame.content_left, frame.width.unwrap() + 1.);
        Some((
            Bounds::new(
                layout.origin + point(gutter.width, px(0.)),
                size(px(1.), height),
            ),
            layout.theme.border,
        ))
    };
    assert_quads(
        paints.standalone_gutter_border.as_slice(),
        border.as_slice(),
    );
    let spacer = shape_label(window, frame.spacer.unwrap().into(), foreground);
    assert_eq!(gutter.width, spacer.width + LABEL_PAD * 2.);
    assert_eq!(text.line_count(), frame.rows.len());
    assert_eq!(paints.gutter_labels.len(), frame.rows.len());
    let mut active = Vec::new();
    for (i, (expected, label)) in frame.rows.iter().zip(&paints.gutter_labels).enumerate() {
        let (range, top, bottom) = measure_line(i + 1);
        assert_eq!(range.is_empty(), expected.blank);
        assert_eq!(label.text.as_ref(), expected.label);
        assert_eq!(expected.color, frame.base_foreground.unwrap());
        assert_eq!(painted_runs(label, foreground), [(0..label.len(), false)]);
        if i == 0 {
            assert!(expected.height > frame.rows[1].height);
            assert!(bottom - top > layout.rows[0].height);
        }
        if expected.active {
            active.push((
                Bounds::new(
                    point(layout.origin.x, top),
                    size(gutter.width, bottom - top),
                ),
                fixture_color(expected.fill),
            ));
        } else {
            assert_eq!(expected.fill, "00000000");
        }
    }
    assert_quads(&paints.gutter, &active);
    inset
}

#[gpui::test]
fn standalone_gutter_matches_pinned_line_number_roles(cx: &mut TestAppContext) {
    let frames: Vec<StandaloneFrame<'_>> = include_str!("standalone_gutter_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(frames.len(), 28);
    for path in ["sample.txt", "sample.js"] {
        let [theme, content, active, numbers] = std::array::from_fn(|_| Compartment::new());
        let (editor, cx) = cx.add_window_view(|window, cx| {
            let mut editor = Editor::with_native_extensions(
                STANDALONE.replacen("42", "43", 1),
                Extension::group([
                    ALLOW_MULTIPLE_SELECTIONS.of(true),
                    theme.of(one_dark_theme()),
                    content.of(Extension::default()),
                    active.of(Extension::default()),
                    numbers.of(Extension::default()),
                ]),
                window,
                cx,
            )
            .unwrap();
            editor.set_path(Some(path.into()), None);
            editor
        });
        cx.simulate_resize(size(px(228.), px(500.)));
        let blank = STANDALONE.find("\n\n").unwrap() + 1;
        let mut selection = EditorSelection::new(
            vec![
                SelectionRange::new(6, 11),
                SelectionRange::new(blank, blank),
            ],
            0,
        )
        .unwrap();
        let (mut source, mut baseline) = (STANDALONE.to_owned(), px(0.));
        let mut history = cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                window.activate_window();
                window.focus(&editor.focus);
                let digit = source.find("42").unwrap();
                assert!(editor.replace(digit..digit + 2, "42", cx));
                editor.view.set_selection(selection.clone()).unwrap();
                (editor.view.revision(), editor.view.undo_depth())
            })
        });
        assert_eq!(history.1, 1);
        for frame in frames.iter().filter(|frame| frame.path == path) {
            cx.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    let effect = match frame.mode {
                        "theme-only" => None,
                        "both-blurred" => {
                            window.blur();
                            None
                        }
                        "active-gutter-no-numbers" | "both-focused" => {
                            Some(active.reconfigure(highlight_active_line_gutter()))
                        }
                        "numbers" => {
                            configure(editor, active.reconfigure(Extension::default()), cx);
                            Some(numbers.reconfigure(line_numbers(Default::default())))
                        }
                        "active-body-only" => {
                            Some(content.reconfigure(ViewHighlight::ActiveLine.extension()))
                        }
                        "active-gutter-moved-to-trailing-blank" => {
                            selection = EditorSelection::single(source.len(), source.len());
                            editor.view.set_selection(selection.clone()).unwrap();
                            Some(content.reconfigure(Extension::default()))
                        }
                        "theme-removed" => Some(theme.reconfigure(Extension::default())),
                        "theme-restored" => Some(theme.reconfigure(one_dark_theme())),
                        "custom-labels" => {
                            Some(numbers.reconfigure(line_numbers(LineNumberConfig {
                                format_number: Some(Arc::new(|n, _| format!("Line {n:02}"))),
                            })))
                        }
                        "decimal-restored" => {
                            Some(numbers.reconfigure(line_numbers(Default::default())))
                        }
                        "ten-lines" => {
                            configure(editor, active.reconfigure(Extension::default()), cx);
                            let extra = "a\nb\nc\nd\ne\nf\n";
                            assert!(editor.replace(source.len()..source.len(), extra, cx));
                            source.push_str(extra);
                            editor.view.set_selection(selection.clone()).unwrap();
                            history = (editor.view.revision(), editor.view.undo_depth());
                            None
                        }
                        "numbers-removed" => Some(numbers.reconfigure(Extension::default())),
                        "browser-basic-setup-only" => {
                            let preset =
                                basic_setup::<()>(editor.view.state().clone(), KeyPlatform::Mac)
                                    .unwrap();
                            let headless = preset.gutters(1..=10, None).unwrap();
                            assert_eq!(
                                headless
                                    .iter()
                                    .map(|slot| format!("cm-gutter {}", slot.class))
                                    .collect::<Vec<_>>(),
                                frame.slots
                            );
                            let mut both = preset.view().state().clone();
                            both.update(Transaction::default().with_effects([
                                numbers.reconfigure(line_numbers(Default::default())),
                            ]))
                            .unwrap();
                            assert_eq!(gutter_slots(&both, 1..=10).unwrap(), headless);
                            assert!(standalone_line_numbers(&both, 1..=10).unwrap().is_some());
                            editor.view.set_state(preset.view().state().clone());
                            history = (editor.view.revision(), 0);
                            cx.notify();
                            None
                        }
                        mode => panic!("unexpected browser step: {mode}"),
                    };
                    if let Some(effect) = effect {
                        configure(editor, effect, cx);
                    }
                })
            });
            cx.run_until_parked();
            let hit = cx.update(|window, cx| {
                let editor = editor.read(cx);
                assert_eq!(editor.source(), source);
                assert_eq!(editor.view.state().selection(), &selection);
                assert_eq!((editor.view.revision(), editor.view.undo_depth()), history);
                assert_eq!(editor.view.redo_depth(), 0);
                assert_eq!(editor.focus.is_focused(window), frame.focused);
                let layout = editor.layout.as_ref().unwrap();
                let viewport = editor.scroll.bounds().size.width - px(2. * HORIZONTAL_PADDING);
                let inset = assert_standalone_paint(
                    layout,
                    editor.view.state().text(),
                    viewport,
                    frame,
                    window,
                );
                match frame.mode {
                    "numbers" => baseline = inset,
                    "active-body-only" | "both-focused" | "both-blurred" | "theme-restored"
                    | "decimal-restored" => assert_eq!(inset, baseline),
                    "theme-removed" => assert_eq!(inset, baseline + px(1.)),
                    "custom-labels" | "ten-lines" => assert!(inset > baseline),
                    _ => (),
                }
                (frame.mode == "both-blurred")
                    .then(|| layout.origin + point(inset / 2., layout.rows[1].y + px(1.)))
            });
            if let Some(hit) = hit {
                cx.simulate_click(hit, Modifiers::none());
                cx.run_until_parked();
                cx.update(|window, cx| {
                    let editor = editor.read(cx);
                    assert!(!editor.focus.is_focused(window));
                    assert_eq!(editor.view.state().selection(), &selection);
                    assert!(!editor.selecting);
                });
            }
        }
    }
}

#[gpui::test]
fn standalone_active_gutter_paints_fold_and_lint_lanes_without_numbers(cx: &mut TestAppContext) {
    let selected = JSON.find("a long line").unwrap();
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            JSON.into(),
            Extension::group([
                json_extension(),
                one_dark_theme(),
                highlight_active_line_gutter(),
                slot.of(Extension::default()),
            ]),
            window,
            cx,
        )
        .unwrap();
        editor.set_path(Some("sample.json".into()), None);
        editor.set_selection(selected, selected);
        editor
    });
    cx.simulate_resize(size(px(228.), px(440.)));
    for lanes in 0..=3 {
        let extensions = Extension::group(
            [
                (1, fold_gutter(Default::default())),
                (2, lint_gutter(Default::default())),
            ]
            .into_iter()
            .filter_map(|(flag, extension)| (lanes & flag != 0).then_some(extension)),
        );
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| configure(e, slot.reconfigure(extensions), cx))
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            let highlights = &layout.painted_active_lines.gutter;
            if lanes == 0 {
                assert!(layout.standalone_gutter.is_none());
                assert!(highlights.is_empty());
                return;
            }
            let (_, gutter) = standalone_fold_lane(editor);
            assert!(gutter.numbers_width.is_none());
            assert_eq!(highlights.len(), 1);
            let line = editor.view.state().text().line_at(selected).unwrap();
            let first = layout
                .rows
                .iter()
                .position(|row| row.start == line.range.start)
                .unwrap();
            let last = layout.rows[first..]
                .iter()
                .find(|row| !row.soft_end)
                .unwrap();
            let height = last.y + last.height - layout.rows[first].y;
            assert!(height > layout.rows[first].height);
            let expected = Bounds::new(
                layout.origin + point(px(0.), layout.rows[first].y),
                size(gutter.width, height),
            );
            assert_eq!(highlights[0].bounds, expected);
            assert_eq!(editor.view.state().selection().main().head(), selected);
            assert_eq!(editor.source(), JSON);
            assert_eq!((editor.view.revision(), editor.view.undo_depth()), (0, 0));
        });
    }
}

#[gpui::test]
fn standalone_gutter_crlf_hits_and_suppression(cx: &mut TestAppContext) {
    let source = STANDALONE.replace('\n', "\r\n");
    let blank = source.find("\r\n\r\n").unwrap() + 2;
    let trailing = source.len();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            source.clone(),
            Extension::group([
                one_dark_theme(),
                line_numbers(Default::default()),
                highlight_active_line_gutter(),
            ]),
            window,
            cx,
        )
        .unwrap();
        editor.set_path(Some("crlf.js".into()), None);
        editor
    });
    cx.simulate_resize(size(px(228.), px(420.)));
    cx.run_until_parked();
    let read_paint = |editor: &Editor, active, offset| {
        let layout = editor.layout.as_ref().unwrap();
        let painted = &layout.painted_active_lines;
        let gutter = layout.standalone_gutter.as_ref().unwrap();
        assert_eq!(
            painted
                .gutter_labels
                .iter()
                .map(|label| label.text.as_ref())
                .collect::<Vec<_>>(),
            ["1", "2", "3", "4"]
        );
        assert_eq!(painted.ordinary_gutter.len(), 1);
        assert_eq!(painted.gutter.len(), usize::from(active));
        let caret = layout.bounds_at(offset, false).unwrap();
        assert!(caret.left() >= layout.origin.x + gutter.inset());
        assert_eq!(
            layout.index_at(caret.origin + point(px(0.), px(1.))),
            offset
        );
        caret.origin + point(px(0.), px(1.))
    };
    let click = cx.update(|_, cx| read_paint(editor.read(cx), true, trailing));
    cx.simulate_click(click, Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.cursor(), trailing);
            assert!(editor.focus.is_focused(window));
            editor.set_input_enabled(false, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(read_paint(editor, false, trailing), click);
            assert_eq!(editor.source(), source);
            editor.set_input_enabled(true, cx);
            let units = source[..blank].encode_utf16().count();
            editor.replace_and_mark_text_in_range(Some(units..units), "仮", None, window, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.marked, Some(blank..blank + "仮".len()));
            assert_eq!(
                editor.source(),
                source.replacen("\r\n\r\n", "\r\n仮\r\n", 1)
            );
            read_paint(editor, false, blank);
            editor.unmark_text(window, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        read_paint(editor, true, blank);
        assert_eq!(editor.view.undo_depth(), 1);
    });
}

const JSON: &str = "{\r\n  \"fold\": {\r\n    \"word\": \"猫😀\",\r\n\r\n    \"list\": [\r\n      0\r\n    ]\r\n  },\r\n  \"keep\": \"a long line that wraps across several displayed rows\",\r\n  \"tail\": 2\r\n}\r\n";

fn standalone_fold_lane(editor: &Editor) -> (&Layout, &StandaloneGutter) {
    let layout = editor.layout.as_ref().unwrap();
    (layout, layout.standalone_gutter.as_ref().unwrap())
}

fn standalone_fold_hit(editor: &Editor, start: usize) -> Point<Pixels> {
    let (layout, gutter) = standalone_fold_lane(editor);
    let row = layout.rows.iter().find(|row| row.start == start).unwrap();
    layout.origin + gutter.fold_bounds(row).center()
}

#[gpui::test]
fn standalone_fold_gutter_requires_matching_attachment(cx: &mut TestAppContext) {
    for (path, source, extensions, preset, expected) in [
        (
            "sample.json",
            JSON,
            Extension::group([json_extension(), fold_gutter(Default::default())]),
            false,
            true,
        ),
        (
            "sample.txt",
            JSON,
            Extension::group([json_extension(), fold_gutter(Default::default())]),
            false,
            false,
        ),
        (
            "sample.json",
            JSON,
            fold_gutter(Default::default()),
            false,
            false,
        ),
        (
            "sample.json",
            JSON,
            Extension::group([lezer_extension(), fold_gutter(Default::default())]),
            false,
            false,
        ),
        (
            "sample.js",
            "function f() {\n  return 1;\n}\n",
            Extension::group([
                script_catalog_extension("TypeScript"),
                fold_gutter(Default::default()),
            ]),
            false,
            false,
        ),
        (
            "sample.js",
            "function f() {\n  return 1;\n}\n",
            script_catalog_extension("JavaScript"),
            true,
            false,
        ),
        ("sample.json", JSON, json_extension(), true, false),
        ("sample.json", JSON, json_extension(), false, false),
        (
            "sample.json",
            "{\r  \"word\": \"猫😀\"\r}",
            Extension::group([json_extension(), fold_gutter(Default::default())]),
            false,
            false,
        ),
    ] {
        let (editor, window) = cx.add_window_view(|window, cx| {
            let mut editor = Editor::with_native_extensions(
                source.into(),
                Extension::group([extensions, one_dark_theme()]),
                window,
                cx,
            )
            .unwrap();
            editor.set_path(Some(path.into()), None);
            if preset {
                let setup =
                    basic_setup::<()>(editor.view.state().clone(), KeyPlatform::Mac).unwrap();
                editor.view.set_state(setup.view().state().clone());
            }
            editor
        });
        window.run_until_parked();
        window.update(|_, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            assert_eq!(layout.standalone_gutter.is_some(), expected);
            assert!(layout.painted_active_lines.gutter_labels.is_empty());
            assert_eq!(
                !layout
                    .painted_active_lines
                    .standalone_fold_labels
                    .is_empty(),
                expected
            );
            assert!(editor.source_fold_ranges().is_empty());
        });
    }
}

fn script_catalog_extension(name: &str) -> Extension {
    let entry = LanguageDescription::match_name(LANGUAGES, name, false).unwrap();
    let mut load = pin!(entry.catalog_support_load().unwrap().load());
    let Poll::Ready(Ok(support)) = load
        .as_mut()
        .poll(&mut std::task::Context::from_waker(Waker::noop()))
    else {
        panic!("missing native {name} support");
    };
    support.extension().unwrap()
}

#[gpui::test]
fn standalone_script_gutter_replays_pinned_folds_and_mapping(cx: &mut TestAppContext) {
    #[derive(Deserialize)]
    struct Case {
        mode: String,
        suffixes: Vec<String>,
        source: String,
        eol: String,
        before: Vec<[usize; 3]>,
        folded: Vec<[usize; 2]>,
        mapped: Vec<[usize; 2]>,
        after: Vec<[usize; 3]>,
    }

    let fixtures =
        include_str!("../../../../tests/fixtures/codemirror/standalone_javascript_folds.jsonl");
    let marker_starts = |editor: &Editor| {
        standalone_fold_lane(editor)
            .1
            .folds
            .as_ref()
            .unwrap()
            .markers
            .keys()
            .copied()
            .collect::<Vec<_>>()
    };
    for fixture in fixtures.lines() {
        let Case {
            mode,
            suffixes,
            source,
            eol,
            before,
            folded,
            mapped,
            after,
        } = serde_json::from_str(fixture).unwrap();
        let syntax = script_catalog_extension(&mode);
        for suffix in suffixes {
            let (editor, cx) = cx.add_window_view(|window, cx| {
                let mut editor = Editor::with_native_extensions(
                    source.clone(),
                    Extension::group([syntax.clone(), fold_gutter(Default::default())]),
                    window,
                    cx,
                )
                .unwrap();
                editor.set_path(Some(format!("sample.{suffix}").into()), None);
                editor
            });
            cx.run_until_parked();
            let hits = cx.update(|_, cx| {
                let editor = editor.read(cx);
                assert_eq!(
                    marker_starts(editor),
                    before.iter().map(|row| row[0]).collect::<Vec<_>>()
                );
                before
                    .iter()
                    .map(|row| standalone_fold_hit(editor, row[0]))
                    .collect::<Vec<_>>()
            });
            for (&hit, &[start, from, to]) in hits.iter().zip(&before).rev() {
                cx.simulate_click(hit, Modifiers::none());
                cx.run_until_parked();
                let unfold = cx.update(|_, cx| {
                    let editor = editor.read(cx);
                    assert_only_fold(editor, from..to);
                    assert!(editor.layout.as_ref().unwrap().fold_hides(from + 1));
                    assert_eq!(editor.source(), source);
                    assert_eq!(editor.view.undo_depth(), 0);
                    standalone_fold_hit(editor, start)
                });
                if from != folded[0][0] {
                    cx.simulate_click(unfold, Modifiers::none());
                    cx.run_until_parked();
                }
            }
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| assert!(editor.replace(0..0, &eol, cx)));
            });
            cx.run_until_parked();
            let unfold = cx.update(|_, cx| {
                let editor = editor.read(cx);
                assert_only_fold(editor, mapped[0][0]..mapped[0][1]);
                assert_eq!(marker_starts(editor), [after[0][0]]);
                standalone_fold_hit(editor, after[0][0])
            });
            cx.simulate_click(unfold, Modifiers::none());
            cx.run_until_parked();
            cx.update(|_, cx| {
                let editor = editor.read(cx);
                assert!(editor.source_fold_ranges().is_empty());
                assert_eq!(
                    marker_starts(editor),
                    after.iter().map(|row| row[0]).collect::<Vec<_>>()
                );
                assert_eq!(editor.source(), format!("{eol}{source}"));
                assert_eq!(editor.view.undo_depth(), 1);
            });
        }
    }
}

#[gpui::test]
fn standalone_json_fold_click_elides_without_focus_or_history_change(cx: &mut TestAppContext) {
    let config = Compartment::new();
    let fold = JSON.find("  \"fold\"").unwrap();
    let open = fold + "  \"fold\": ".len();
    let close = JSON.find("},").unwrap();
    let keep = JSON.find("  \"keep\"").unwrap();
    let tail = JSON.find("  \"tail\"").unwrap();
    let hidden = open + 1..close;
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            JSON.replacen("2\r\n", "3\r\n", 1),
            Extension::group([
                json_extension(),
                line_numbers(Default::default()),
                // Keep the fold state installed if the host removes its gutter.
                code_folding_with_placeholders(FoldPlaceholderConfig {
                    text: Some("folded\nJSON".into()),
                    ..Default::default()
                }),
                config.of(fold_gutter(FoldGutterConfig {
                    open_text: "−".into(),
                    closed_text: "Expand".into(),
                    ..Default::default()
                })),
            ]),
            window,
            cx,
        )
        .unwrap();
        editor.set_path(Some("sample.json".into()), None);
        editor
    });
    cx.simulate_resize(size(px(340.), px(700.)));
    let (other, selection, history) = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let digit = tail + "  \"tail\": ".len();
            assert!(editor.replace(digit..digit + 1, "2", cx));
            editor.set_user_selection(keep + 6, keep + 3, "select.pointer", cx);
            let other = cx.focus_handle();
            window.focus(&other);
            (
                other,
                editor.view.state().selection().clone(),
                (
                    editor.view.revision(),
                    editor.view.undo_depth(),
                    editor.view.redo_depth(),
                    editor.view.can_undo_selection(),
                    editor.view.can_redo_selection(),
                ),
            )
        })
    });
    assert_eq!(history.1, 1);
    assert!(history.3);
    cx.run_until_parked();
    let assert_preserved = |editor: &Editor, window: &Window| {
        assert!(other.is_focused(window));
        assert!(!editor.selecting);
        assert_eq!(editor.source(), JSON);
        assert_eq!(editor.view.state().selection(), &selection);
        assert_eq!(
            (
                editor.view.revision(),
                editor.view.undo_depth(),
                editor.view.redo_depth(),
                editor.view.can_undo_selection(),
                editor.view.can_redo_selection()
            ),
            history
        );
    };
    let (hit, width, count, x, inert_hits) = cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert_preserved(editor, window);
        let (layout, gutter) = standalone_fold_lane(editor);
        assert_eq!(
            gutter
                .folds
                .as_ref()
                .unwrap()
                .markers
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [fold, JSON.find("    \"list\"").unwrap()]
        );
        let wrapped = layout
            .rows
            .iter()
            .find(|row| keep < row.start && row.start < tail)
            .unwrap();
        let hit = standalone_fold_hit(editor, fold);
        let blank = JSON.find("\r\n\r\n").unwrap() + 2;
        (
            hit,
            gutter.inset(),
            layout.rows.len(),
            layout.bounds_at(keep, false).unwrap().left(),
            [
                layout.origin + point(gutter.numbers_width.unwrap() / 2., hit.y - layout.origin.y),
                standalone_fold_hit(editor, blank),
                layout.origin + gutter.fold_bounds(wrapped).center(),
            ],
        )
    });
    for hit in inert_hits {
        cx.simulate_click(hit, Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            assert_preserved(editor, window);
            assert!(editor.source_fold_ranges().is_empty());
        });
    }
    for unfold_by_label in [false, true] {
        cx.simulate_click(hit, Modifiers::none());
        cx.run_until_parked();
        let unfold = cx.update(|window, cx| {
            let editor = editor.read(cx);
            assert_preserved(editor, window);
            assert_only_fold(editor, hidden.clone());
            let (layout, gutter) = standalone_fold_lane(editor);
            assert_eq!(gutter.inset(), width);
            assert!(layout.rows.len() < count);
            assert_eq!(layout.painted_active_lines.standalone_fold_labels.iter().map(|label| label.text.as_ref()).collect::<Vec<_>>(), ["Expand"]);
            assert_eq!(layout.painted_active_lines.gutter_labels.iter().map(|label| label.text.as_ref()).collect::<Vec<_>>(), ["1", "2", "9", "10", "11", "12"]);
            assert!(layout.fold_hides(JSON.find('猫').unwrap()));
            assert!(editor.view.viewport().visible_ranges().iter().all(|range| range.end <= hidden.start || hidden.end <= range.start));
            let caret = layout.bounds_at(keep, false).unwrap();
            assert_eq!(caret.left(), x);
            assert_eq!(layout.index_at(caret.origin + point(px(0.), px(1.))), keep);
            let (row, item) = layout.rows.iter().find_map(|row| {
                row.items.iter().find(|item| matches!(&item.paint, crate::editor::layout::ItemPaint::Fold(label) if label.text.as_ref() == " folded JSON ")).map(|item| (row, item))
            }).unwrap();
            assert_eq!(item.source, hidden);
            assert!(item.atomic);
            if unfold_by_label {
                layout.origin + point(item.left + item.width / 2., row.y + row.height / 2.)
            } else {
                standalone_fold_hit(editor, fold)
            }
        });
        cx.simulate_click(unfold, Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            assert_preserved(editor, window);
            assert!(editor.source_fold_ranges().is_empty());
            assert_eq!(editor.layout.as_ref().unwrap().rows.len(), count);
        });
    }
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_only_fold(editor, hidden.clone());
            configure(editor, config.reconfigure(Extension::default()), cx);
            // The old painted marker is stale, but the lane still consumes its click.
            assert!(editor.mouse_down_fold_gutter(hit, cx));
            assert_eq!(
                folded_ranges(editor.view.state())
                    .iter(0)
                    .map(|fold| fold.from..fold.to)
                    .collect::<Vec<_>>(),
                std::slice::from_ref(&hidden)
            );
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert_preserved(editor, window);
        let (layout, gutter) = standalone_fold_lane(editor);
        assert!(gutter.folds.is_none());
        assert!(gutter.inset() < width);
        assert!(!layout.fold_hides(JSON.find('猫').unwrap()));
        assert!(
            layout
                .painted_active_lines
                .standalone_fold_labels
                .is_empty()
        );
    });
}

#[gpui::test]
fn standalone_json_fold_wrapped_suffix_has_no_gutter_target(cx: &mut TestAppContext) {
    let source = "{\r\n  \"value\": \"猫\"\r\n}\r\n";
    let close = source.find('}').unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            source.into(),
            Extension::group([
                json_extension(),
                line_numbers(Default::default()),
                fold_gutter(Default::default()),
                code_folding_with_placeholders(FoldPlaceholderConfig {
                    text: Some("W".repeat(30).into()),
                    ..Default::default()
                }),
            ]),
            window,
            cx,
        )
        .unwrap();
        editor.set_path(Some("sample.json".into()), None);
        editor
    });
    cx.simulate_resize(size(px(230.), px(350.)));
    cx.run_until_parked();
    let hit = cx.update(|_, cx| standalone_fold_hit(editor.read(cx), 0));
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    let suffix = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_only_fold(editor, 1..close);
        let (layout, gutter) = standalone_fold_lane(editor);
        let index = layout
            .rows
            .iter()
            .position(|row| row.start == close)
            .unwrap();
        assert!(layout.rows[index - 1].soft_end);
        assert_eq!(
            gutter
                .folds
                .as_ref()
                .unwrap()
                .markers
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [0]
        );
        assert_eq!(
            layout
                .painted_active_lines
                .gutter_labels
                .iter()
                .map(|label| label.text.as_ref())
                .collect::<Vec<_>>(),
            ["1", "4"]
        );
        standalone_fold_hit(editor, close)
    });
    cx.simulate_click(suffix, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_only_fold(editor, 1..close);
        assert_eq!(editor.cursor(), 0);
        assert_eq!(editor.source(), source);
    });
}

fn lint_paint(editor: &Editor) -> &BTreeMap<usize, StandaloneLintPaint> {
    standalone_fold_lane(editor)
        .1
        .lint
        .as_ref()
        .map(|slot| &slot.painted)
        .unwrap()
}

fn lint_rows(editor: &Editor) -> Vec<(usize, Severity)> {
    lint_paint(editor)
        .iter()
        .map(|(&start, paint)| (start, paint.severity))
        .collect()
}

#[gpui::test]
fn standalone_lint_gutter_reserves_width_and_measures_passive_crlf_markers(
    cx: &mut TestAppContext,
) {
    let source = STANDALONE.replace('\n', "\r\n") + &"tail\r\n".repeat(12);
    let blank = source.find("\r\n\r\n").unwrap() + 2;
    let end = source.find("const end").unwrap();
    let hint = source.find("tail").unwrap();
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            source.replacen("42", "43", 1),
            slot.of(Extension::default()),
            window,
            cx,
        )
        .unwrap();
        editor.set_path(Some("sample.txt".into()), None);
        editor
    });
    cx.simulate_resize(size(px(228.), px(300.)));
    let (other, selection, revision) = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let digit = source.find("42").unwrap();
            assert!(editor.replace(digit..digit + 2, "42", cx));
            editor.set_user_selection(digit + 2, digit, "select.pointer", cx);
            let other = cx.focus_handle();
            window.focus(&other);
            (
                other,
                editor.view.state().selection().clone(),
                editor.view.revision(),
            )
        })
    });
    cx.run_until_parked();
    let assert_passive = |editor: &Editor, window: &Window| {
        assert!(other.is_focused(window));
        assert!(!editor.selecting);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.view.state().selection(), &selection);
        assert_eq!(editor.view.revision(), revision);
        assert_eq!(editor.view.undo_depth(), 1);
        assert_eq!(editor.view.redo_depth(), 0);
        assert!(editor.view.can_undo_selection());
        assert!(!editor.view.can_redo_selection());
        assert!(editor.prepainted_overlay.is_none());
    };
    let publish = |editor: &mut Editor, cx: &mut Context<Editor>| {
        let middle_crlf = blank - 1;
        let diagnostics = [
            (source.find('é').unwrap(), Severity::Info),
            (middle_crlf, Severity::Error),
            (blank, Severity::Info),
            (end + 6, Severity::Warning),
            (hint, Severity::Hint),
        ]
        .into_iter()
        .map(|(at, severity)| Diagnostic::new(at..at, severity, "diagnostic"))
        .collect();
        configure(editor, set_diagnostics_effect().of(diagnostics), cx);
    };
    let unindented = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let layout = editor.layout.as_ref().unwrap();
            assert!(layout.standalone_gutter.is_none());
            let left = layout.bounds_at(0, false).unwrap().left();
            publish(editor, cx);
            left
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.layout.as_ref().unwrap().standalone_gutter.is_none());
            configure(
                editor,
                slot.reconfigure(lint_gutter(Default::default())),
                cx,
            );
        })
    });
    cx.run_until_parked();
    let inset = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_passive(editor, window);
            let (layout, gutter) = standalone_fold_lane(editor);
            assert_eq!(gutter.width, LINT_WIDTH);
            assert!(gutter.numbers_width.is_none() && gutter.folds.is_none());
            assert!(lint_paint(editor).is_empty());
            let inset = gutter.inset();
            assert_eq!(
                layout.bounds_at(0, false).unwrap().left(),
                unindented + inset
            );
            publish(editor, cx);
            inset
        })
    });
    cx.run_until_parked();
    let hits = cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert_passive(editor, window);
        let (layout, gutter) = standalone_fold_lane(editor);
        assert_eq!(gutter.inset(), inset);
        assert_eq!(
            lint_rows(editor),
            [
                (0, Severity::Error),
                (blank, Severity::Info),
                (end, Severity::Warning)
            ]
        );
        let expected_size = size(LINT_ICON_SIZE, LINT_ICON_SIZE);
        for (&start, paint) in lint_paint(editor) {
            let row = layout.rows.iter().find(|row| row.start == start).unwrap();
            assert_eq!(paint.icon.size, expected_size);
            assert_eq!(paint.hit, paint.icon);
            assert_eq!(
                paint.icon.center(),
                layout.origin + point(LINT_WIDTH / 2., row.y + row.height / 2.)
            );
            assert!(paint.hit.right() < layout.origin.x + inset);
            let caret = layout.bounds_at(start, false).unwrap();
            assert_eq!(caret.left(), unindented + inset);
            assert_eq!(layout.index_at(caret.origin + point(px(0.), px(1.))), start);
        }
        let wrapped = &layout.rows[1];
        assert!(layout.rows[0].soft_end && 0 < wrapped.start && wrapped.start < blank);
        [
            lint_paint(editor)[&0].hit.center(),
            layout.origin + point(LINT_WIDTH / 2., wrapped.y + wrapped.height / 2.),
            layout.bounds_at(hint, false).unwrap().center()
                - point(inset - LINT_WIDTH / 2., px(0.)),
        ]
    });
    for hit in hits {
        cx.simulate_mouse_move(hit, None, Modifiers::none());
        cx.simulate_click(hit, Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| assert_passive(editor.read(cx), window));
    }
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        let delta = editor.scroll.bounds().top() - hits[0].y;
        editor
            .scroll
            .set_offset(editor.scroll.offset() + point(px(0.), delta));
        window.refresh();
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert_passive(editor, window);
        let first = &lint_paint(editor)[&0];
        assert_eq!(first.hit, first.icon.intersect(&editor.scroll.bounds()));
        assert_eq!(first.hit.size.height, LINT_ICON_SIZE / 2.);
        editor
            .scroll
            .set_offset(editor.scroll.offset() - point(px(0.), LINT_ICON_SIZE));
        window.refresh();
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(
                lint_rows(editor),
                [(blank, Severity::Info), (end, Severity::Warning)]
            );
            configure(editor, slot.reconfigure(Extension::default()), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert_passive(editor, window);
        let layout = editor.layout.as_ref().unwrap();
        assert!(layout.standalone_gutter.is_none());
        assert_eq!(layout.bounds_at(0, false).unwrap().left(), unindented);
    });
}

#[gpui::test]
fn standalone_lint_gutter_does_not_aggregate_folds_or_paint_wrapped_suffixes(
    cx: &mut TestAppContext,
) {
    let source = "\r\n{\r\n  \"value\": \"猫\"\r\n}\r\n";
    let open = source.find('{').unwrap();
    let interior = source.find("  \"value\"").unwrap();
    let close = source.find('}').unwrap();
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            source.into(),
            Extension::group([
                json_extension(),
                line_numbers(Default::default()),
                fold_gutter(Default::default()),
                code_folding_with_placeholders(FoldPlaceholderConfig {
                    text: Some("W".repeat(30).into()),
                    ..Default::default()
                }),
                slot.of(Extension::default()),
            ]),
            window,
            cx,
        )
        .unwrap();
        editor.set_path(Some("sample.json".into()), None);
        editor
    });
    cx.simulate_resize(size(px(230.), px(350.)));
    cx.run_until_parked();
    let (fold_hit, columns) = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let hit = standalone_fold_hit(editor, open);
            let (_, gutter) = standalone_fold_lane(editor);
            let width = gutter.width;
            configure(
                editor,
                slot.reconfigure(lint_gutter(Default::default())),
                cx,
            );
            let diagnostics = [
                (open, Severity::Warning),
                (interior, Severity::Error),
                (close, Severity::Info),
            ]
            .into_iter()
            .map(|(at, severity)| Diagnostic::new(at..at, severity, "diagnostic"))
            .collect();
            configure(editor, set_diagnostics_effect().of(diagnostics), cx);
            (hit, width)
        })
    });
    cx.run_until_parked();
    let inert = cx.update(|_, cx| {
        let editor = editor.read(cx);
        let (layout, gutter) = standalone_fold_lane(editor);
        assert_eq!(gutter.width, columns + LINT_WIDTH);
        assert_eq!(standalone_fold_hit(editor, open), fold_hit);
        assert_eq!(
            lint_rows(editor),
            [
                (open, Severity::Warning),
                (interior, Severity::Error),
                (close, Severity::Info)
            ]
        );
        [
            lint_paint(editor)[&open].hit.center(),
            point(layout.origin.x + columns, fold_hit.y),
        ]
    });
    for hit in inert {
        cx.simulate_click(hit, Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            assert!(!editor.focus.is_focused(window));
            assert!(editor.source_fold_ranges().is_empty());
            assert_eq!(editor.cursor(), 0);
        });
    }
    cx.simulate_click(fold_hit, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_only_fold(editor, open + 1..close);
            let (layout, _) = standalone_fold_lane(editor);
            let suffix = layout
                .rows
                .iter()
                .position(|row| row.start == close)
                .unwrap();
            assert!(layout.rows[suffix - 1].soft_end);
            assert_eq!(lint_rows(editor), [(open, Severity::Warning)]);
            assert!(editor.replace(0..0, "\r\n", cx));
        });
    });
    cx.run_until_parked();
    let unfold = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_only_fold(editor, open + 3..close + 2);
        assert_eq!(lint_rows(editor), [(open + 2, Severity::Warning)]);
        standalone_fold_hit(editor, open + 2)
    });
    cx.simulate_click(unfold, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(editor.source_fold_ranges().is_empty());
        assert_eq!(
            lint_rows(editor),
            [
                (open + 2, Severity::Warning),
                (interior + 2, Severity::Error),
                (close + 2, Severity::Info)
            ]
        );
        assert_eq!(editor.source(), format!("\r\n{source}"));
        assert_eq!(editor.view.undo_depth(), 1);
    });
}

#[gpui::test]
fn standalone_lint_gutter_does_not_create_rows_at_lone_cr_or_in_markdown(cx: &mut TestAppContext) {
    let source = "猫\rsecond\r\nthird";
    let second = source.find("second").unwrap();
    let third = source.find("third").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            source.into(),
            lint_gutter(Default::default()),
            window,
            cx,
        )
        .unwrap();
        let setup = basic_setup::<()>(editor.view.state().clone(), KeyPlatform::Mac).unwrap();
        editor.view.set_state(setup.view().state().clone());
        editor.set_path(Some("sample.txt".into()), None);
        configure(
            &mut editor,
            set_diagnostics_effect().of(vec![
                Diagnostic::new(second..second, Severity::Error, "lone CR"),
                Diagnostic::new(third..third, Severity::Info, "CRLF"),
            ]),
            cx,
        );
        editor
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let (layout, gutter) = standalone_fold_lane(editor);
            assert_eq!(
                gutter
                    .lint
                    .as_ref()
                    .unwrap()
                    .markers
                    .iter(0)
                    .map(|mark| mark.from)
                    .collect::<Vec<_>>(),
                [second, third]
            );
            assert!(!layout.rows.iter().any(|row| row.start == second));
            assert_eq!(lint_rows(editor), [(third, Severity::Info)]);
            editor.set_path(Some("sample.md".into()), None);
            cx.notify();
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(lint_gutter_markers(editor.view.state()).is_some());
        assert!(editor.layout.as_ref().unwrap().standalone_gutter.is_none());
        assert_eq!(editor.source(), source);
    });
}

#[gpui::test]
fn standalone_lint_tooltip_uses_the_measured_marker_and_filtered_literal_text(
    cx: &mut TestAppContext,
) {
    use std::{sync::atomic::AtomicUsize, time::Duration};

    use gpui_editor_codemirror::{
        READ_ONLY,
        lint::{Action, LintConfig, LintGutterConfig, LintSource, diagnostic_count, linter},
    };

    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let source = STANDALONE.replace('\n', "\r\n");
    let literal = "<b>猫</b> [link](https://example.com)";
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            source.clone(),
            Extension::group([
                READ_ONLY.of(true),
                linter(
                    Some(LintSource::new(|_| panic!("read-only provider ran"))),
                    LintConfig {
                        delay: Some(Duration::from_millis(1)),
                        marker_filter: Some(Arc::new(|_, _| Vec::new())),
                        ..Default::default()
                    },
                ),
                lint_gutter(LintGutterConfig {
                    hover_time: Some(Duration::from_millis(45)),
                    tooltip_filter: Some(Arc::new(move |mut diagnostics, _| {
                        observed.fetch_add(1, Ordering::Relaxed);
                        diagnostics.retain(|d| d.severity < Severity::Error);
                        diagnostics.reverse();
                        diagnostics
                    })),
                    ..Default::default()
                }),
            ]),
            window,
            cx,
        )
        .unwrap();
        editor.set_path(Some("sample.txt".into()), None);
        editor
    });
    cx.simulate_resize(size(px(228.), px(360.)));
    let clock = cx.executor();
    cx.update(|window, cx| {
        window.activate_window();
        editor.update(cx, |editor, cx| {
            window.focus(&editor.focus);
            assert!(!editor.has_source_diagnostics());
            let mut warning = Diagnostic::new(7..8, Severity::Warning, literal);
            warning
                .actions
                .push(Action::new("Do not render or execute", |_, _| {
                    panic!("a hover must not run the action")
                }));
            let end = source.find("const end").unwrap();
            configure(
                editor,
                set_diagnostics_effect().of(vec![
                    Diagnostic::new(0..1, Severity::Error, "hidden"),
                    Diagnostic::new(3..4, Severity::Info, "犬".repeat(512)),
                    warning,
                    Diagnostic::new(end..end, Severity::Error, "empty after filtering"),
                ]),
                cx,
            );
        });
    });
    cx.run_until_parked();
    let (hit, misses, filtered, before) = cx.update(|_, cx| {
        let e = editor.read(cx);
        let (layout, gutter) = standalone_fold_lane(e);
        let icon = lint_paint(e)[&0].hit;
        let wrapped = &layout.rows[1];
        assert!(layout.rows[0].soft_end);
        let end = source.find("const end").unwrap();
        assert_eq!(lint_paint(e)[&0].severity, Severity::Error);
        assert_eq!(diagnostic_count(e.view.state()), 0);
        (
            icon.center(),
            [
                point(icon.left() - px(1.), icon.center().y),
                point(
                    icon.center().x,
                    layout.origin.y + wrapped.y + wrapped.height / 2.,
                ),
                point(layout.origin.x + gutter.inset() + px(2.), icon.center().y),
            ],
            lint_paint(e)[&end].hit.center(),
            (
                e.view.state().selection().clone(),
                e.scroll.offset(),
                gutter.inset(),
            ),
        )
    });
    for miss in misses {
        cx.simulate_mouse_move(miss, None, Modifiers::none());
        clock.advance_clock(Duration::from_millis(60));
        assert_lint_overlay(&editor, cx, None);
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    cx.simulate_mouse_move(filtered, None, Modifiers::none());
    clock.advance_clock(Duration::from_millis(60));
    assert_lint_overlay(&editor, cx, None);
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    cx.simulate_mouse_move(hit, None, Modifiers::none());
    clock.advance_clock(Duration::from_millis(44));
    assert_lint_overlay(&editor, cx, None);
    cx.simulate_mouse_move(hit + point(px(1.), px(0.)), None, Modifiers::none());
    clock.advance_clock(Duration::from_millis(44));
    assert_lint_overlay(&editor, cx, None);
    clock.advance_clock(Duration::from_millis(1));
    let expected = format!("{literal}\n{}", "犬".repeat((512 - literal.len() - 1) / 3));
    assert_lint_overlay(&editor, cx, Some(&expected));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    cx.update(|window, cx| {
        let e = editor.read(cx);
        assert_eq!(e.source(), source);
        assert!(e.focus.is_focused(window));
        assert_eq!(e.view.state().selection(), &before.0);
        assert_eq!(e.scroll.offset(), before.1);
        assert_eq!(standalone_fold_lane(e).1.inset(), before.2);
        assert_eq!(
            (e.view.revision(), e.view.undo_depth(), e.view.redo_depth()),
            (0, 0, 0)
        );
        assert!(!e.view.can_undo_selection() && !e.view.can_redo_selection());
    });
    cx.update(|_, cx| {
        editor.update(cx, |e, cx| {
            e.message = Some("existing overlay".into());
            cx.notify();
        });
    });
    assert_lint_overlay(&editor, cx, Some("existing overlay"));
    cx.update(|_, cx| {
        editor.update(cx, |e, cx| {
            e.message = None;
            cx.notify();
        });
    });
    assert_lint_overlay(&editor, cx, Some(&expected));
    cx.simulate_mouse_move(misses[0], None, Modifiers::none());
    assert_lint_overlay(&editor, cx, None);
}

#[gpui::test]
fn standalone_lint_provider_gutter_takes_priority_only_on_the_lint_lane(cx: &mut TestAppContext) {
    use std::time::Duration;

    use gpui_editor_codemirror::lint::{LintConfig, LintGutterConfig, LintSource, linter};

    let delay = Duration::from_millis(40);
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            "alpha\r\n字 beta\r\n".into(),
            Extension::group([
                linter(
                    Some(LintSource::new(|_| {
                        vec![Diagnostic::new(0..5, Severity::Error, "body")]
                    })),
                    LintConfig {
                        delay: Some(delay),
                        ..Default::default()
                    },
                ),
                lint_gutter(LintGutterConfig {
                    hover_time: Some(delay),
                    tooltip_filter: Some(Arc::new(|mut diagnostics, _| {
                        diagnostics[0].message = "gutter".into();
                        diagnostics
                    })),
                    ..Default::default()
                }),
            ]),
            window,
            cx,
        )
        .unwrap();
        editor.set_path(Some("sample.txt".into()), None);
        editor
    });
    let clock = cx.executor();
    cx.update(|window, cx| {
        crate::editor::bind_keys(cx);
        window.activate_window();
        editor.update(cx, |e, _| window.focus(&e.focus));
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert!(lint_paint(editor.read(cx)).is_empty()));
    clock.advance_clock(delay);
    cx.run_until_parked();
    let (icon, body) = cx.update(|_, cx| {
        let e = editor.read(cx);
        let layout = e.layout.as_ref().unwrap();
        assert_eq!(lint_rows(e), [(0, Severity::Error)]);
        let body = layout.bounds_at(1, false).unwrap().center();
        (lint_paint(e)[&0].hit.center(), body)
    });
    cx.simulate_mouse_move(body, None, Modifiers::none());
    assert_lint_overlay(&editor, cx, Some("body"));
    cx.simulate_keystrokes("f8");
    assert_lint_overlay(&editor, cx, Some("body"));
    cx.simulate_mouse_move(icon, None, Modifiers::none());
    clock.advance_clock(delay - Duration::from_millis(1));
    assert_lint_overlay(&editor, cx, None);
    clock.advance_clock(Duration::from_millis(1));
    assert_lint_overlay(&editor, cx, Some("gutter"));
    // The keyboard tooltip still shows when the pointer leaves the lane and
    // is no longer over the body diagnostic.
    cx.simulate_mouse_move(body + point(px(45.), px(0.)), None, Modifiers::none());
    assert_lint_overlay(&editor, cx, Some("body"));
}

#[gpui::test]
fn standalone_lint_tooltip_cancels_pending_and_open_hover(cx: &mut TestAppContext) {
    use std::time::Duration;

    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut e = Editor::with_native_extensions(
            "alpha\n".repeat(40),
            slot.of(lint_gutter(Default::default())),
            window,
            cx,
        )
        .unwrap();
        e.set_path(Some("sample.txt".into()), None);
        e
    });
    let clock = cx.executor();
    let publish =
        || set_diagnostics_effect().of(vec![Diagnostic::new(0..1, Severity::Info, "tip")]);
    cx.update(|window, cx| {
        crate::editor::bind_keys(cx);
        window.activate_window();
        editor.update(cx, |e, cx| {
            window.focus(&e.focus);
            configure(e, publish(), cx);
        });
    });
    for open in [false, true] {
        for event in [
            "leave",
            "window exit",
            "publish",
            "configure",
            "edit",
            "blur",
            "deactivate",
            "freeze",
            "IME",
            "escape",
            "offscreen",
        ] {
            cx.run_until_parked();
            let hit = cx.update(|_, cx| lint_paint(editor.read(cx))[&0].hit.center());
            cx.simulate_mouse_move(hit, None, Modifiers::none());
            clock.advance_clock(Duration::from_millis(299));
            assert_lint_overlay(&editor, cx, None);
            if open {
                clock.advance_clock(Duration::from_millis(1));
                assert_lint_overlay(&editor, cx, Some("tip"));
            }
            match event {
                "leave" => {
                    cx.simulate_mouse_move(hit - point(px(8.), px(0.)), None, Modifiers::none())
                }
                "window exit" => cx.simulate_event(gpui::MouseExitEvent::default()),
                "deactivate" => cx.deactivate_window(),
                "escape" => cx.simulate_keystrokes("escape"),
                "blur" => cx.update(|window, _| window.blur()),
                _ => cx.update(|window, cx| {
                    editor.update(cx, |e, cx| match event {
                        "publish" => configure(e, publish(), cx),
                        "configure" => {
                            let previous = lint_gutter_markers(e.view.state()).unwrap();
                            configure(e, slot.reconfigure(lint_gutter(Default::default())), cx);
                            assert!(Arc::ptr_eq(
                                &previous,
                                &lint_gutter_markers(e.view.state()).unwrap()
                            ));
                        }
                        "edit" => {
                            let end = e.source().find('\n').unwrap();
                            assert!(e.replace(end..end, "!", cx));
                        }
                        "freeze" => e.set_input_enabled(false, cx),
                        "IME" => {
                            e.replace_and_mark_text_in_range(Some(1..1), "字", None, window, cx);
                            assert!(e.is_composing());
                        }
                        "offscreen" => {
                            e.scroll.set_offset(point(px(0.), px(-200.)));
                            window.refresh();
                        }
                        _ => unreachable!(),
                    });
                }),
            }
            assert_lint_overlay(&editor, cx, None);
            cx.update(|window, cx| {
                editor.update(cx, |e, cx| match event {
                    "deactivate" => window.activate_window(),
                    "blur" => window.focus(&e.focus),
                    "freeze" => e.set_input_enabled(true, cx),
                    "IME" => e.unmark_text(window, cx),
                    "offscreen" => {
                        e.scroll.set_offset(point(px(0.), px(0.)));
                        window.refresh();
                    }
                    _ => {}
                });
            });
            clock.advance_clock(Duration::from_millis(300));
            assert_lint_overlay(&editor, cx, None);
            cx.update(|_, cx| editor.update(cx, |e, cx| configure(e, publish(), cx)));
        }
    }
}

fn assert_lint_overlay(
    editor: &gpui::Entity<Editor>,
    cx: &mut gpui::VisualTestContext,
    expected: Option<&str>,
) {
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(editor.read(cx).prepainted_overlay.as_deref(), expected));
}
