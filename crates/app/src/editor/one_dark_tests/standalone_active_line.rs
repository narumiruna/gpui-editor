use gpui::{EntityInputHandler, Rgba, TestAppContext, rgba};
use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Compartment, EditorSelection, Extension, SelectionRange,
    theme_one_dark::one_dark,
    view::{ViewHighlight, gutters::highlight_active_line_gutter},
};
use serde::Deserialize;

use super::{Bounds, Editor, HORIZONTAL_PADDING, ItemPaint, configure, point, px, size};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserFrame<'a> {
    path: &'a str,
    mode: &'a str,
    source: String,
    range: [usize; 2],
    caret: usize,
    focused: bool,
    marked: Vec<BrowserLine<'a>>,
    selection_color: &'a str,
    selection_below_content: bool,
    selection_overlaps_first_line: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserLine<'a> {
    range: [usize; 2],
    color: &'a str,
    full_width: bool,
    measured_height: bool,
    wrapped: bool,
    blank: bool,
}

fn color(hex: &str) -> Rgba {
    rgba(u32::from_str_radix(hex, 16).unwrap())
}

#[gpui::test]
fn standalone_active_line_matches_pinned_body_paint(cx: &mut TestAppContext) {
    let frames: Vec<BrowserFrame> = include_str!("standalone_active_line_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for path in ["sample.txt", "sample.js"] {
        let cases: Vec<_> = frames.iter().filter(|frame| frame.path == path).collect();
        assert_eq!(
            cases.iter().map(|frame| frame.mode).collect::<Vec<_>>(),
            ["theme-only", "active-focused", "active-blurred", "removed"]
        );
        let active_frame = cases[2];
        let source = &active_frame.source;
        let [anchor, head] = active_frame.range;
        let caret = active_frame.caret;
        assert_eq!(cases[0].marked.len(), 0);
        assert_eq!(cases[1].marked.len(), 2);
        assert_eq!(active_frame.marked.len(), 2);
        assert_eq!(cases[3].marked.len(), 0);
        // Reuse the blurred browser frame for native-only controls.
        let cases = cases.into_iter().map(|frame| (frame.mode, frame)).chain([
            ("input-disabled", active_frame),
            ("input-restored", active_frame),
            ("unthemed-with-opt-in", active_frame),
        ]);
        let theme = Compartment::new();
        let active = Compartment::new();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            let mut editor = Editor::with_native_extensions(
                source.replacen("42", "43", 1),
                Extension::group([
                    theme.of(one_dark()),
                    active.of(Extension::default()),
                    ALLOW_MULTIPLE_SELECTIONS.of(true),
                    highlight_active_line_gutter(),
                ]),
                window,
                cx,
            )
            .unwrap();
            editor.set_path(Some(path.into()), None);
            editor
        });
        cx.simulate_resize(size(px(228.), px(420.)));
        let selection = EditorSelection::new(
            vec![
                SelectionRange::new(anchor, head),
                SelectionRange::new(caret, caret),
            ],
            0,
        )
        .unwrap();
        let history = cx.update(|window, cx| {
            window.activate_window();
            editor.update(cx, |editor, cx| {
                let offset = source.find("42").unwrap();
                assert!(editor.replace(offset..offset + 2, "42", cx));
                editor.view.set_selection(selection.clone()).unwrap();
                window.focus(&editor.focus);
                let history = (editor.view.revision(), editor.view.undo_depth());
                assert!(history.1 > 0);
                history
            })
        });
        let mut geometry = None;
        for (mode, frame) in cases {
            let themed = mode != "unthemed-with-opt-in";
            let marks = if themed && mode != "input-disabled" {
                frame.marked.as_slice()
            } else {
                &[]
            };
            assert_eq!(frame.source, *source);
            assert_eq!(frame.range, [anchor, head]);
            assert_eq!(frame.caret, caret);
            assert!(frame.selection_below_content && frame.selection_overlaps_first_line);
            assert_eq!(
                frame.focused,
                matches!(frame.mode, "theme-only" | "active-focused")
            );
            cx.update(|window, cx| {
                editor.update(cx, |editor, cx| match mode {
                    "active-focused" | "input-disabled" => {
                        configure(
                            editor,
                            active.reconfigure(ViewHighlight::ActiveLine.extension()),
                            cx,
                        );
                        if mode == "input-disabled" {
                            editor.set_input_enabled(false, cx);
                        }
                    }
                    "active-blurred" => window.blur(),
                    "removed" => configure(editor, active.reconfigure(Extension::default()), cx),
                    "input-restored" => editor.set_input_enabled(true, cx),
                    "unthemed-with-opt-in" => {
                        configure(editor, theme.reconfigure(Extension::default()), cx)
                    }
                    _ => (),
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
                assert!(layout.painted_active_lines.gutter.is_empty());
                assert!(layout.painted_active_lines.ordinary_gutter.is_empty());
                assert!(layout.painted_active_lines.gutter_labels.is_empty());
                assert!(layout.painted_bracket_quads.is_empty());

                let left = editor.scroll.bounds().left() + px(HORIZONTAL_PADDING);
                let width = editor.scroll.bounds().size.width - px(2. * HORIZONTAL_PADDING);
                assert_eq!(layout.origin.x, left);
                let mut expected_content = Vec::new();
                for line in marks {
                    assert!(line.full_width && line.measured_height);
                    let [from, to] = line.range;
                    assert_eq!(layout.line_block(from), Some(from..to));
                    let first = layout.bounds_at(from, false).unwrap();
                    let last = layout.bounds_at(to, false).unwrap();
                    let row_tops = first.top()..=last.top();
                    let count = expected_content.len();
                    expected_content.extend(layout.rows.iter().filter_map(|row| {
                        let top = layout.origin.y + row.y;
                        row_tops.contains(&top).then_some((
                            Bounds::new(point(left, top), size(width, row.height)),
                            color(line.color).into(),
                        ))
                    }));
                    assert_eq!(expected_content.len() - count > 1, line.wrapped);
                    assert_eq!(from == to, line.blank);
                }
                let paints = |quads: &[gpui::PaintQuad]| {
                    quads
                        .iter()
                        .map(|quad| (quad.bounds, quad.background))
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    paints(&layout.painted_active_lines.content),
                    expected_content,
                    "{path} {mode}: body fill"
                );
                let start = layout.bounds_at(anchor, false).unwrap();
                let end = layout.bounds_at(head, false).unwrap();
                assert_eq!(start.top(), end.top());
                let selected = Bounds::new(
                    start.origin,
                    size(end.left() - start.left(), start.size.height),
                );
                let selected_color = if themed {
                    color(frame.selection_color)
                } else {
                    layout.theme.selection
                };
                let mut expected_order = vec![(selected, selected_color.into())];
                expected_order.extend(expected_content);
                assert_eq!(
                    paints(&layout.painted_layer_order.quads),
                    expected_order,
                    "{path} {mode}: selection and active-line paint order"
                );
                let text_after_quads = layout.rows.iter().flat_map(|row| {
                    let top = layout.origin.y + row.y;
                    let count = expected_order
                        .iter()
                        .take_while(|(bounds, _)| bounds.top() <= top)
                        .count();
                    row.items.iter().map(move |_| count)
                });
                assert_eq!(
                    layout.painted_layer_order.standalone_text_after_quads,
                    text_after_quads.collect::<Vec<_>>(),
                    "{path} {mode}: text paint order"
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
                                    panic!("expected plain source text");
                                };
                                assert_eq!(line.text.as_ref(), &source[item.source.clone()]);
                                (item.source.clone(), item.left, item.width)
                            })
                            .collect::<Vec<_>>();
                        (row.y, row.height, row.width, items)
                    })
                    .collect::<Vec<_>>();
                let hits = [anchor, head, caret].map(|offset| {
                    let bounds = layout.bounds_at(offset, false).unwrap();
                    assert_eq!(
                        layout.index_at(bounds.origin + point(px(0.), px(1.))),
                        offset
                    );
                    bounds
                });
                let snapshot = (rows, hits, editor.scroll.bounds());
                assert_eq!(
                    &snapshot,
                    geometry.get_or_insert_with(|| snapshot.clone()),
                    "{path} {mode}: geometry"
                );
            });
        }
    }
}

#[gpui::test]
fn standalone_active_line_respects_marked_text(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            "alpha\n\nomega".into(),
            Extension::group([one_dark(), ViewHighlight::ActiveLine.extension()]),
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
            editor.replace_and_mark_text_in_range(Some(6..6), "仮", None, window, cx);
        });
    });
    cx.run_until_parked();
    let geometry = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.marked, Some(6..9));
        assert_eq!(editor.source(), "alpha\n仮\nomega");
        let layout = editor.layout.as_ref().unwrap();
        assert!(layout.painted_active_lines.content.is_empty());
        layout.bounds_at(editor.cursor(), false).unwrap()
    });
    cx.update(|window, cx| editor.update(cx, |editor, cx| editor.unmark_text(window, cx)));
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(!editor.is_composing());
        assert_eq!(editor.source(), "alpha\n仮\nomega");
        assert_eq!(editor.view.undo_depth(), 1);
        let layout = editor.layout.as_ref().unwrap();
        assert_eq!(layout.bounds_at(editor.cursor(), false), Some(geometry));
        assert_eq!(layout.painted_active_lines.content.len(), 1);
    });
}
