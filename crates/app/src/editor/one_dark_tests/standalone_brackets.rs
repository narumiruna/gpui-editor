use std::sync::Arc;

use gpui::{Background, Edges, EntityInputHandler, Pixels, Rgba, TestAppContext, rgba};
use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Compartment, EditorSelection, Extension, SelectionRange,
    language::brackets::{BracketMatchRender, BracketMatchingOptions, bracket_matching},
    theme_one_dark::one_dark_theme,
    view::MarkDecoration,
};
use serde::Deserialize;

use super::{Bounds, Editor, ItemPaint, Layout, configure, point, px, size};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserFrame<'a> {
    path: &'a str,
    mode: &'a str,
    source: String,
    ranges: Vec<[usize; 2]>,
    focused: bool,
    marks: Vec<BrowserMark<'a>>,
    selection_color: &'a str,
    selection_below_content: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserMark<'a> {
    range: [usize; 2],
    text: &'a str,
    matched: bool,
    color: &'a str,
    glyph_width: bool,
    overlaps_selection: bool,
}

fn color(hex: &str) -> Rgba {
    rgba(u32::from_str_radix(hex, 16).unwrap())
}

fn range_bounds(layout: &Layout, [from, to]: [usize; 2]) -> Bounds<Pixels> {
    let start = layout.bounds_at(from, false).unwrap();
    let end = layout.bounds_at(to, false).unwrap();
    assert_eq!(start.top(), end.top());
    assert!(start.left() < end.left());
    let item = layout
        .rows
        .iter()
        .flat_map(|row| &row.items)
        .find(|item| item.source.start <= from && item.source.end >= to)
        .unwrap();
    let ItemPaint::Text(line) = &item.paint else {
        panic!("expected shaped text");
    };
    let x = |offset| item.left + line.x_for_index(offset - item.source.start);
    Bounds::new(start.origin, size(x(to) - x(from), start.size.height))
}

fn quads(quads: &[gpui::PaintQuad]) -> Vec<(Bounds<Pixels>, Background)> {
    quads
        .iter()
        .map(|quad| {
            assert_eq!(quad.border_widths, Edges::all(px(0.)));
            (quad.bounds, quad.background)
        })
        .collect()
}

fn close_only() -> BracketMatchingOptions {
    BracketMatchingOptions {
        render_match: Some(Arc::new(|hit, _| {
            if let Some(end) = &hit.end {
                assert!(hit.matched);
                vec![
                    MarkDecoration::new("cm-nonmatchingBracket")
                        .range(end.clone())
                        .unwrap(),
                ]
            } else {
                Vec::new()
            }
        })),
        ..Default::default()
    }
}

#[gpui::test]
fn replays_pinned_standalone_bracket_paint(cx: &mut TestAppContext) {
    let frames: Vec<BrowserFrame> = include_str!("standalone_brackets_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for path in ["sample.js", "sample.txt"] {
        let cases: Vec<_> = frames.iter().filter(|frame| frame.path == path).collect();
        assert_eq!(
            cases.iter().map(|frame| frame.mode).collect::<Vec<_>>(),
            [
                "theme-only",
                "default",
                "blurred",
                "custom-pairs",
                "limited",
                "rendered",
                "removed"
            ]
        );
        let default = cases[1];
        let source = &default.source;
        let ranges = &default.ranges;
        let selected = ranges[1];
        let close = if path.ends_with(".js") { 31 } else { 25 };
        assert_eq!(
            default
                .marks
                .iter()
                .map(|mark| (mark.range, mark.matched))
                .collect::<Vec<_>>(),
            [
                ([18, 19], true),
                ([close, close + 1], true),
                ([70, 71], false)
            ]
        );
        assert_eq!(cases[5].marks[0].range, [close, close + 1]);
        assert!(!cases[5].marks[0].matched);
        let theme = Compartment::new();
        let brackets = Compartment::new();
        let config = Extension::group([
            theme.of(one_dark_theme()),
            brackets.of(Extension::default()),
            ALLOW_MULTIPLE_SELECTIONS.of(true),
        ]);
        let (editor, cx) = cx.add_window_view(|window, cx| {
            let mut editor = Editor::with_native_extensions(
                source.replacen("42", "43", 1),
                config.clone(),
                window,
                cx,
            )
            .unwrap();
            editor.set_path(Some(path.into()), None);
            editor
        });
        cx.simulate_resize(size(px(640.), px(420.)));
        let selection = EditorSelection::new(
            ranges
                .iter()
                .map(|&[anchor, head]| SelectionRange::new(anchor, head))
                .collect(),
            1,
        )
        .unwrap();
        let history = cx.update(|window, cx| {
            window.activate_window();
            editor.update(cx, |editor, cx| {
                // The app installs stock bracket matching. Use the browser's root
                // configuration here to exercise a genuinely absent extension.
                configure(editor, config.reconfigure(), cx);
                let number = source.find("42").unwrap();
                assert!(editor.replace(number..number + 2, "42", cx));
                editor.view.set_selection(selection.clone()).unwrap();
                window.focus(&editor.focus);
                let history = (editor.view.revision(), editor.view.undo_depth());
                assert!(history.1 > 0);
                history
            })
        });
        let mut geometry = None;
        for (mode, frame) in cases.into_iter().map(|frame| (frame.mode, frame)).chain([
            ("input-disabled", default),
            ("input-restored", default),
            ("unthemed", default),
        ]) {
            assert_eq!(frame.source, *source);
            assert_eq!(frame.ranges, *ranges);
            assert!(frame.selection_below_content);
            let themed = mode != "unthemed";
            let marks = if themed && mode != "input-disabled" {
                frame.marks.as_slice()
            } else {
                &[]
            };
            cx.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    let extension = match mode {
                        "default" | "input-disabled" => bracket_matching(Default::default()),
                        "blurred" => return window.blur(),
                        "custom-pairs" | "limited" => {
                            window.focus(&editor.focus);
                            bracket_matching(BracketMatchingOptions {
                                brackets: Some("«»".into()),
                                after_cursor: Some(false),
                                max_scan_distance: (mode == "limited").then_some(1),
                                ..Default::default()
                            })
                        }
                        "rendered" => bracket_matching(close_only()),
                        "removed" => Extension::default(),
                        "input-restored" => return editor.set_input_enabled(true, cx),
                        "unthemed" => {
                            return configure(editor, theme.reconfigure(Extension::default()), cx);
                        }
                        _ => return,
                    };
                    configure(editor, brackets.reconfigure(extension), cx);
                    if mode == "input-disabled" {
                        editor.set_input_enabled(false, cx);
                    }
                });
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                let editor = editor.read(cx);
                assert_eq!(editor.focus.is_focused(window), frame.focused);
                assert_eq!(editor.source(), source);
                assert_eq!(editor.view.state().selection(), &selection);
                assert_eq!((editor.view.revision(), editor.view.undo_depth()), history);
                assert_eq!(editor.view.redo_depth(), 0);

                let layout = editor.layout.as_ref().unwrap();
                assert!(layout.source_frame.is_none());
                assert_eq!(layout.theme.view.is_some(), themed);
                assert!(layout.painted_active_lines.content.is_empty());
                assert!(layout.painted_active_lines.gutter.is_empty());
                assert!(layout.painted_active_lines.ordinary_gutter.is_empty());
                assert!(layout.painted_active_lines.gutter_labels.is_empty());
                let selected_bounds = range_bounds(layout, selected);
                let selection_color = if themed {
                    color(frame.selection_color)
                } else {
                    layout.theme.selection
                };
                let expected = marks
                    .iter()
                    .map(|mark| {
                        let [from, to] = mark.range;
                        assert_eq!(&source[from..to], mark.text);
                        assert!(mark.glyph_width);
                        let bounds = range_bounds(layout, mark.range);
                        assert_eq!(
                            bounds.left() < selected_bounds.right()
                                && selected_bounds.left() < bounds.right()
                                && bounds.top() < selected_bounds.bottom()
                                && selected_bounds.top() < bounds.bottom(),
                            mark.overlaps_selection
                        );
                        (bounds, color(mark.color).into())
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    quads(&layout.painted_bracket_quads),
                    expected,
                    "{path} {mode}: bracket paint"
                );
                let mut layers = vec![(selected_bounds, selection_color.into())];
                layers.extend(expected);
                assert_eq!(
                    quads(&layout.painted_layer_order.quads),
                    layers,
                    "{path} {mode}: selection below brackets"
                );
                let text_order = layout.rows.iter().flat_map(|row| {
                    let count = layers
                        .iter()
                        .take_while(|(bounds, _)| bounds.top() <= layout.origin.y + row.y)
                        .count();
                    row.items.iter().map(move |_| count)
                });
                assert_eq!(
                    layout.painted_layer_order.standalone_text_after_quads,
                    text_order.collect::<Vec<_>>()
                );
                let rows = layout
                    .rows
                    .iter()
                    .map(|row| {
                        let items = row
                            .items
                            .iter()
                            .map(|item| {
                                let ItemPaint::Text(line) = &item.paint else {
                                    panic!("expected text")
                                };
                                assert_eq!(line.text.as_ref(), &source[item.source.clone()]);
                                (item.source.clone(), item.left, item.width)
                            })
                            .collect::<Vec<_>>();
                        (row.y, row.height, row.width, items)
                    })
                    .collect::<Vec<_>>();
                let hit_geometry = [18, 19, 25, 26, 31, 32, 37, 39, 53, 55, 70, 71].map(|offset| {
                    let bounds = layout.bounds_at(offset, false).unwrap();
                    (
                        bounds,
                        layout.index_at(point(bounds.left(), bounds.center().y)),
                    )
                });
                let snapshot = (rows, hit_geometry, editor.scroll.bounds());
                assert_eq!(
                    &snapshot,
                    geometry.get_or_insert_with(|| snapshot.clone()),
                    "{path} {mode}: geometry"
                );
            });
        }
    }
}

fn assert_brackets(editor: &Editor, expected: &[[usize; 2]]) {
    let layout = editor.layout.as_ref().unwrap();
    let expected: Vec<_> = expected
        .iter()
        .map(|&range| (range_bounds(layout, range), rgba(0xbad0_f847).into()))
        .collect();
    assert_eq!(quads(&layout.painted_bracket_quads), expected);
}

#[gpui::test]
fn standalone_bracket_paint_hides_during_marked_text_and_rejects_bad_renderer_ranges(
    cx: &mut TestAppContext,
) {
    let brackets = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            "(é)".into(),
            Extension::group([
                one_dark_theme(),
                brackets.of(bracket_matching(Default::default())),
            ]),
            window,
            cx,
        )
        .unwrap();
        editor.set_path(Some("sample.txt".into()), None);
        editor
    });
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            window.focus(&editor.focus);
            editor.set_selection(1, 1);
            cx.notify();
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_brackets(editor, &[[0, 1], [3, 4]]);
            editor.replace_and_mark_text_in_range(None, "仮", Some(0..0), window, cx);
        });
    });
    cx.run_until_parked();
    let bounds = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.marked, Some(1..4));
            assert_eq!(editor.selection(), 1..1);
            assert_brackets(editor, &[]);
            let bounds = editor.layout.as_ref().unwrap().bounds_at(1, false);
            editor.unmark_text(window, cx);
            bounds
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(!editor.is_composing());
        assert_eq!(editor.source(), "(仮é)");
        assert_eq!(editor.layout.as_ref().unwrap().bounds_at(1, false), bounds);
        assert_brackets(editor, &[[0, 1], [6, 7]]);
    });
    for invalid in [2..3, 7..8] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let render_match: BracketMatchRender = Arc::new(move |hit, _| {
                    let mark = MarkDecoration::new("cm-matchingBracket");
                    vec![
                        mark.clone().range(hit.start.clone()).unwrap(),
                        mark.range(invalid.clone()).unwrap(),
                    ]
                });
                configure(
                    editor,
                    brackets.reconfigure(bracket_matching(BracketMatchingOptions {
                        render_match: Some(render_match),
                        ..Default::default()
                    })),
                    cx,
                );
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| assert_brackets(editor.read(cx), &[]));
    }
}

#[gpui::test]
fn standalone_bracket_paint_tracks_current_text_and_path(cx: &mut TestAppContext) {
    const SOURCE: &str = "run(\"🐱 )\", 42);";
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            SOURCE.into(),
            Extension::group([one_dark_theme(), bracket_matching(Default::default())]),
            window,
            cx,
        )
        .unwrap();
        editor.set_path(Some("sample.js".into()), None);
        editor
    });
    let open = SOURCE.find('(').unwrap();
    let literal = SOURCE.find(')').unwrap();
    let close = SOURCE.rfind(')').unwrap();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            window.focus(&editor.focus);
            editor.set_selection(open, open);
            cx.notify();
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_brackets(editor, &[[open, open + 1], [close, close + 1]]);
            assert!(editor.replace(close..close + 1, " ", cx));
            editor.set_selection(open, open);
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_brackets(editor, &[[open, open + 1]]);
            editor.set_path(Some("sample.txt".into()), None);
            cx.notify();
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), SOURCE.replacen("42)", "42 ", 1));
        assert_brackets(editor, &[[open, open + 1], [literal, literal + 1]]);
    });
}
