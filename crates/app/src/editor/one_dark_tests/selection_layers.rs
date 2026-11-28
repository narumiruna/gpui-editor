use gpui::{Edges, Rgba, TestAppContext, rgba};
use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Compartment, EditorSelection, Extension, SelectionRange,
    theme_one_dark::one_dark_theme,
    view::{ViewHighlight, gutters::highlight_active_line_gutter},
};
use serde::Deserialize;

use super::{Bounds, Editor, ItemPaint, activate_code, configure, point, px, size};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserLayers<'a> {
    source: String,
    caret: usize,
    range: [usize; 2],
    focused: bool,
    scroller_z: &'a str,
    selection_layer_z: &'a str,
    content_z: &'a str,
    scroller_siblings: bool,
    brackets_in_active_line: bool,
    active_line_overlaps_selection: bool,
    selection_crosses_close_only: bool,
    selection: &'a str,
    active_line: &'a str,
    matching_bracket: &'a str,
    active_gutter: &'a str,
}

fn color(hex: &str) -> Rgba {
    rgba(u32::from_str_radix(hex, 16).unwrap())
}

#[gpui::test]
fn selection_paints_below_themed_active_line_and_brackets(cx: &mut TestAppContext) {
    let browser: BrowserLayers =
        serde_json::from_str(include_str!("selection_layers_browser.jsonl")).unwrap();
    assert!(browser.focused && browser.scroller_siblings && browser.brackets_in_active_line);
    assert!(browser.active_line_overlaps_selection && browser.selection_crosses_close_only);
    assert_eq!(browser.scroller_z, "0");
    assert_eq!(browser.selection_layer_z, "-2");
    assert_eq!(browser.content_z, "auto");

    let source = format!("```json\n{}\n```\n", browser.source);
    let theme = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.clone(),
            Extension::group([
                theme.of(one_dark_theme()),
                ALLOW_MULTIPLE_SELECTIONS.of(true),
                ViewHighlight::ActiveLine.extension(),
                highlight_active_line_gutter(),
            ]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(640.), px(420.)));
    let (at, selection, revision) = cx.update(|window, cx| {
        window.activate_window();
        editor.update(cx, |editor, cx| {
            let at = activate_code(editor, &browser.source, cx);
            let caret = at + browser.caret;
            let selection = EditorSelection::new(
                vec![
                    SelectionRange::new(caret, caret),
                    SelectionRange::new(at + browser.range[0], at + browser.range[1]),
                ],
                1,
            )
            .unwrap();
            editor.view.set_selection(selection.clone()).unwrap();
            window.focus(&editor.focus);
            cx.notify();
            (at, selection, editor.view.revision())
        })
    });
    let first_line = browser.source.lines().next().unwrap();
    let open = first_line.find('[').unwrap();
    let close = first_line.find(']').unwrap();
    let mut geometry = None;
    for themed in [true, false] {
        if !themed {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    configure(editor, theme.reconfigure(Extension::default()), cx);
                });
            });
        }
        cx.run_until_parked();
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            assert!(editor.focus.is_focused(window));
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.revision(), revision);
            assert!(!editor.view.can_undo());

            let layout = editor.layout.as_ref().unwrap();
            let frame = layout.source_frame.as_ref().unwrap();
            assert_eq!(layout.theme.view.is_some(), themed);
            let (row, item) = layout
                .rows
                .iter()
                .flat_map(|row| row.items.iter().map(move |item| (row, item)))
                .find(|(_, item)| item.source == (at..at + first_line.len()))
                .unwrap();
            let ItemPaint::Text(line) = &item.paint else {
                panic!("expected shaped draft text");
            };
            assert_eq!(line.text.as_ref(), first_line);
            let left = layout.origin.x + item.left;
            let top = layout.origin.y + row.y;
            let x = |offset| item.left + line.x_for_index(offset);
            let bounds = |from, to| {
                Bounds::new(
                    point(layout.origin.x + x(from), top),
                    size(x(to) - x(from), row.height),
                )
            };
            let [start, end] = browser.range;
            let selected_bounds = bounds(start, end);
            let open_bounds = bounds(open, open + 1);
            let close_bounds = bounds(close, close + 1);
            let active_bounds = Bounds::new(
                point(left, top),
                size(
                    layout.origin.x + frame.bounds.right() - px(1.) - left,
                    row.height,
                ),
            );
            assert!(open_bounds.right() < selected_bounds.left());
            assert!(selected_bounds.left() < close_bounds.left());
            assert!(close_bounds.right() < selected_bounds.right());
            assert!(active_bounds.left() < selected_bounds.left());
            assert!(active_bounds.right() > selected_bounds.right());

            let [gutter] = layout.painted_active_lines.gutter.as_slice() else {
                panic!("expected the first draft line's active gutter");
            };
            assert_eq!(gutter.bounds.top(), top);
            assert_eq!(gutter.bounds.bottom(), active_bounds.bottom());
            assert_eq!(gutter.bounds.right(), active_bounds.left());
            let [select, active, bracket, gutter_color] = if themed {
                [
                    browser.selection,
                    browser.active_line,
                    browser.matching_bracket,
                    browser.active_gutter,
                ]
            } else {
                ["4a86ed50", "cceeff44", "328c8252", "e2f2ffff"]
            }
            .map(color);
            assert_eq!(gutter.background, gutter_color.into());
            let selection_fill = (selected_bounds, select.into());
            let mut expected = vec![
                (active_bounds, active.into()),
                (open_bounds, bracket.into()),
                (close_bounds, bracket.into()),
            ];
            if themed {
                expected.insert(0, selection_fill);
            } else {
                expected.push(selection_fill);
            }
            // These are canvas quad submissions, not pixels read from a scene.
            let submitted = layout
                .painted_layer_order
                .quads
                .iter()
                .map(|quad| {
                    assert_eq!(quad.border_widths, Edges::all(px(0.)));
                    (quad.bounds, quad.background)
                })
                .collect::<Vec<_>>();
            assert_eq!(submitted, expected);

            let hit_geometry = (0..=first_line.len())
                .map(|offset| {
                    let caret = layout.bounds_at(at + offset, false).unwrap();
                    assert_eq!(caret.left(), layout.origin.x + x(offset));
                    assert_eq!(caret.top(), top);
                    assert_eq!(
                        layout.index_at(point(caret.left(), caret.center().y)),
                        at + offset
                    );
                    caret
                })
                .collect::<Vec<_>>();
            let snapshot = (
                selected_bounds,
                open_bounds,
                close_bounds,
                active_bounds,
                gutter.bounds,
                hit_geometry,
            );
            assert_eq!(&snapshot, geometry.get_or_insert_with(|| snapshot.clone()));
        });
    }
}
