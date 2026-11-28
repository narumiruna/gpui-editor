use gpui::{
    Background, Focusable, Hsla, Rgba, TestAppContext, VisualTestContext, WindowAppearance, rgb,
};
use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Compartment, Extension,
    search::{SelectionMatchConfig, selection_match_extension},
    theme_one_dark::{one_dark_highlighting, one_dark_theme},
};

use super::{super::layout::Layout, *};

#[gpui::test]
fn one_dark_panel_inputs_reconfigure_in_the_find_frame(cx: &mut TestAppContext) {
    use gpui::{EntityInputHandler, Modifiers};

    use crate::editor::one_dark_tests::{configure, panel_input_colors};

    let slot = Compartment::new();
    let source = "raw 字🦀";
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source.into(), slot.of(Extension::default()), window, cx)
            .unwrap()
    });
    cx.simulate_resize(size(px(640.), px(420.)));
    cx.update(|window, cx| {
        assert_eq!(window.appearance(), WindowAppearance::Light);
        super::super::bind_keys(cx);
        window.focus(&editor.read(cx).focus);
    });
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input(" 字🦀 ");
    let replace_button = cx.debug_bounds("document-find-show-replacement").unwrap();
    cx.simulate_click(replace_button.center(), Modifiers::none());
    cx.simulate_input(" $& ");
    cx.simulate_keystrokes("cmd-f secondary-a");
    cx.run_until_parked();
    let ids = ["document-find-input", "document-replacement-input"];
    let bounds = ids.map(|id| cx.debug_bounds(id).unwrap());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_find_message(Some("Keep status".into()), cx)
        });
    });
    let original_selection = cx.update(|_, cx| editor.read(cx).view.state().selection().clone());
    for (extension, themed) in [
        (Extension::default(), false),
        (one_dark_theme(), true),
        (one_dark_highlighting(), false),
    ] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, slot.reconfigure(extension), cx)
            });
        });
        cx.run_until_parked();
        assert_eq!(ids.map(|id| cx.debug_bounds(id).unwrap()), bounds);
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            let find = editor.find.as_ref().unwrap();
            assert_eq!(find.message.as_deref(), Some("Keep status"));
            let (query, replacement) = (find.query.read(cx), find.replacement.read(cx));
            assert!(query.focus_handle(cx).is_focused(window));
            assert!(!replacement.focus_handle(cx).is_focused(window));
            assert_eq!(query.text(), " 字🦀 ");
            assert_eq!(replacement.text(), " $& ");
            assert_eq!(
                query.painted_colors(),
                panel_input_colors("find", themed, true)
            );
            assert_eq!(
                replacement.painted_colors(),
                panel_input_colors("replace", themed, false)
            );
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &original_selection);
            assert!(!editor.view.can_undo());
            find.query.clone().update(cx, |field, cx| {
                let selection = field.selected_text_range(false, window, cx).unwrap();
                assert_eq!(selection.range, 0..5);
                assert!(!selection.reversed);
            });
        });
    }
}

fn selection_match_bounds(
    layout: &Layout,
    ranges: impl Iterator<Item = Range<usize>> + Clone,
) -> (Vec<Bounds<Pixels>>, Vec<Bounds<Pixels>>) {
    let mut expected = Vec::new();
    for row in &layout.rows {
        for item in row.items.iter().filter(|item| item.is_text()) {
            for range in ranges.clone() {
                expected.extend(item.x_spans(range).map(|x| {
                    Bounds::new(
                        layout.origin + point(x.start, row.y),
                        size(x.end - x.start, row.height),
                    )
                }));
            }
        }
    }
    let color = rgba(0xaafe_661a).into();
    let actual = layout
        .painted_find_quads
        .iter()
        .filter(|quad| quad.background == color)
        .map(|quad| quad.bounds)
        .collect();
    (actual, expected)
}

fn submitted_quad(quad: &PaintQuad) -> (Bounds<Pixels>, Background, Edges<Pixels>, Hsla) {
    (
        quad.bounds,
        quad.background,
        quad.border_widths,
        quad.border_color,
    )
}

#[gpui::test]
fn find_hides_only_the_overlapping_part_of_selection_match_paint(cx: &mut TestAppContext) {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Case {
        source: String,
        query: String,
        selection: [usize; 2],
        selected_text_matches: Vec<(usize, usize, String)>,
    }
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../codemirror/tests/fixtures/search_selection_overlap.jsonl"
    ));
    for (line, installed) in fixture
        .lines()
        .flat_map(|line| [(line, false), (line, true)])
    {
        let case: Case = serde_json::from_str(line).unwrap();
        let selection_match =
            installed.then(|| selection_match_extension(SelectionMatchConfig::default()));
        let extensions = Extension::group([one_dark_theme()].into_iter().chain(selection_match));
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(case.source.clone(), extensions, window, cx).unwrap()
        });
        cx.simulate_resize(size(px(560.), px(360.)));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_selection(case.selection[0], case.selection[1]);
                editor.open_find(window, cx);
            });
        });
        cx.run_until_parked();
        cx.simulate_input(&case.query);
        cx.update(|window, cx| window.focus(&editor.read(cx).focus));
        for width in [560., 184.] {
            cx.simulate_resize(size(px(width), px(360.)));
            cx.run_until_parked();
            cx.update(|_, cx| {
                let editor = editor.read(cx);
                let layout = editor.layout.as_ref().unwrap();
                assert_eq!(layout.rows.len() > 1, width < 200.);
                let ranges = case
                    .selected_text_matches
                    .iter()
                    .filter(|(_, _, color)| color != "rgba(0, 0, 0, 0)")
                    .map(|(from, to, _)| *from..*to);
                let (actual, expected) = selection_match_bounds(layout, ranges);
                assert!(!expected.is_empty());
                assert_eq!(
                    actual, expected,
                    "{} at width {width}, installed: {installed}",
                    case.query
                );
                assert_eq!(editor.source(), case.source);
                assert_eq!([editor.anchor(), editor.cursor()], case.selection);
                assert!(!editor.view.can_undo());
            });
        }
    }
}

#[gpui::test]
fn configured_selection_matches_follow_the_pinned_view_in_actual_paint(cx: &mut TestAppContext) {
    use crate::document::{BlockPreview, Section, SectionKind};

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Config {
        highlight_word_around_cursor: Option<bool>,
        min_selection_length: Option<usize>,
        max_matches: Option<usize>,
        whole_words: Option<bool>,
    }

    type Marks = Vec<(usize, usize, String)>;
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PaintCase {
        name: String,
        source: String,
        config: Vec<Config>,
        selection: [usize; 2],
        before: Marks,
        unfocused: Marks,
        stock: Marks,
        word_scope: Option<(usize, usize, String)>,
    }

    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../codemirror/tests/fixtures/search_selection_config.jsonl"
    ));
    let mut failures = Vec::new();
    let mut measured = 0;
    for line in fixture.lines() {
        let case: serde_json::Value = serde_json::from_str(line).unwrap();
        if case["name"].is_null() {
            continue;
        }
        let PaintCase {
            name,
            source,
            config,
            selection,
            before,
            unfocused,
            stock,
            word_scope,
        } = serde_json::from_value(case).unwrap();
        measured += 1;
        let slot = Compartment::new();
        let configured = Extension::group(config.into_iter().map(|config| {
            selection_match_extension(SelectionMatchConfig {
                highlight_word_around_cursor: config.highlight_word_around_cursor,
                min_selection_length: config.min_selection_length,
                max_matches: config.max_matches,
                whole_words: config.whole_words,
            })
        }));
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                Extension::group([one_dark_theme(), slot.of(configured)]),
                window,
                cx,
            )
            .unwrap()
        });
        cx.simulate_resize(size(px(560.), px(360.)));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                if word_scope.is_some() {
                    let fence = editor
                        .markdown()
                        .sections()
                        .into_iter()
                        .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                        .unwrap()
                        .source;
                    editor.activate(
                        Section::new(SectionKind::DraftCode, fence.clone(), fence),
                        false,
                        cx,
                    );
                }
                editor.set_selection(selection[0], selection[1]);
                if let Some((from, to, _)) = word_scope {
                    assert_eq!(editor.word_scope().unwrap().0, from..to);
                }
                window.focus(&editor.focus);
            });
        });
        let mut check =
            |cx: &mut VisualTestContext, phase, focused, marks: &[(usize, usize, String)]| {
                cx.run_until_parked();
                cx.update(|window, cx| {
                    let editor = editor.read(cx);
                    assert_eq!(editor.focus.is_focused(window), focused, "{name}: {phase}");
                    assert_eq!(editor.source(), source);
                    assert_eq!([editor.anchor(), editor.cursor()], selection);
                    assert!(!editor.view.can_undo());
                    let layout = editor.layout.as_ref().unwrap();
                    let ranges = marks.iter().map(|(from, to, _)| *from..*to);
                    let (actual, expected) = selection_match_bounds(layout, ranges);
                    assert_eq!(expected.len(), marks.len(), "{name}: test geometry");
                    if actual != expected {
                        failures.push((name.clone(), phase, actual.len(), expected.len()));
                    }
                });
            };
        let reconfigure = |cx: &mut VisualTestContext, extension| {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    let active = editor.active.clone();
                    editor
                        .view
                        .update(
                            Transaction::default().with_effects([slot.reconfigure(extension)]),
                            active.clone(),
                            |_| (None, active),
                        )
                        .unwrap();
                    cx.notify();
                });
            });
        };
        let input_enabled = |cx: &mut VisualTestContext, enabled| {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| editor.set_input_enabled(enabled, cx))
            });
        };
        check(cx, "focused", true, &before);
        input_enabled(cx, false);
        check(cx, "input frozen", true, &[]);
        input_enabled(cx, true);
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| editor.open_find(window, cx));
        });
        check(cx, "unfocused", false, &unfocused);
        reconfigure(
            cx,
            selection_match_extension(SelectionMatchConfig::default()),
        );
        check(cx, "reconfigured while unfocused", false, &stock);
        reconfigure(cx, Extension::default());
        check(cx, "removed while unfocused", false, &[]);
        cx.update(|window, cx| window.focus(&editor.read(cx).focus));
        check(cx, "legacy focused", true, &stock);
    }
    assert_eq!(measured, 5);
    assert!(
        failures.is_empty(),
        "(case, phase, actual, expected): {failures:?}"
    );
}

#[gpui::test]
fn installed_one_dark_paints_find_above_exact_and_partial_selections(cx: &mut TestAppContext) {
    use serde_json::Value;

    const SOURCE: &str = "Find hit, hit, hit, hits.";
    let hits: Vec<_> = SOURCE
        .match_indices("hit")
        .map(|(at, _)| at..at + 3)
        .collect();
    let partial = hits[3].start + 1..hits[3].end + 1;
    let selection = EditorSelection::new(
        vec![
            SelectionRange::new(hits[0].start, hits[0].start),
            SelectionRange::new(hits[0].start, hits[0].end),
            SelectionRange::new(hits[1].end, hits[1].start),
            SelectionRange::new(partial.start, partial.end),
        ],
        2,
    )
    .unwrap();
    let theme = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            SOURCE.into(),
            Extension::group([
                ALLOW_MULTIPLE_SELECTIONS.of(true),
                theme.of(Extension::default()),
            ]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(560.), px(360.)));
    cx.update(|window, cx| {
        assert_eq!(window.appearance(), WindowAppearance::Light);
        editor.update(cx, |editor, cx| {
            editor.view.set_selection(selection.clone()).unwrap();
            editor.open_find(window, cx);
        });
    });
    cx.run_until_parked();
    cx.simulate_input("hit");

    let browser = include_str!("../one_dark_tests/find_cascade_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap());
    let [focused, blurred] = <[Value; 2]>::try_from(browser.collect::<Vec<_>>()).unwrap();
    assert_eq!(focused["focused"], true);
    assert_eq!(blurred["focused"], false);

    let mut body_geometry = None;
    for (extension, browser) in [
        (None, None),
        (Some(one_dark_theme()), Some(&focused)),
        (None, Some(&blurred)),
        (Some(one_dark_highlighting()), None),
    ] {
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                if let Some(extension) = extension {
                    crate::editor::one_dark_tests::configure(
                        editor,
                        theme.reconfigure(extension),
                        cx,
                    );
                }
                if let Some(browser) = browser {
                    if browser["focused"] == true {
                        window.focus(&editor.focus);
                    } else {
                        editor.find.as_ref().unwrap().query.read(cx).focus(window);
                    }
                }
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            let query = editor.find.as_ref().unwrap().query.read(cx);
            let focused = browser.is_some_and(|browser| browser["focused"] == true);
            assert_eq!(editor.focus.is_focused(window), focused);
            assert_eq!(query.focus_handle(cx).is_focused(window), !focused);
            assert_eq!(query.text(), "hit");
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.source(), SOURCE);
            assert!(!editor.view.can_undo());
            let layout = editor.layout.as_ref().unwrap();
            assert_eq!(layout.theme.view.is_some(), browser.is_some());
            let (row, item) = layout
                .rows
                .iter()
                .flat_map(|row| row.items.iter().map(move |item| (row, item)))
                .find(|(_, item)| item.source == (0..SOURCE.len()))
                .unwrap();
            let ItemPaint::Text(line) = &item.paint else {
                panic!("Find fixture must shape one text item");
            };
            let hit_bounds = |range: &Range<usize>| {
                let left = item.left + line.x_for_index(range.start);
                let right = item.left + line.x_for_index(range.end);
                Bounds::new(point(left, row.y), size(right - left, row.height))
            };
            let body = hits.iter().map(hit_bounds).collect::<Vec<_>>();
            assert_eq!(&body, body_geometry.get_or_insert_with(|| body.clone()));
            let in_window =
                |bounds: Bounds<Pixels>| Bounds::new(layout.origin + bounds.origin, bounds.size);
            let no_border = Edges::all(px(0.));
            let clear = transparent_black();
            let ordinary = |range: &Range<usize>, color: Rgba| {
                (in_window(hit_bounds(range)), color.into(), no_border, clear)
            };
            let mut expected = Vec::new();
            if let Some(browser) = browser {
                assert_eq!(browser["selectionLayerZ"], "-2");
                assert_eq!(browser["contentZ"], "auto");
                assert_eq!(browser["scrollerZ"], "0");
                assert_eq!(browser["partialOverlap"], true);
                let color =
                    |key| rgba(u32::from_str_radix(browser[key].as_str().unwrap(), 16).unwrap());
                let width = px(browser["outlineWidth"].as_f64().unwrap() as f32);
                let [secondary, primary] =
                    browser["exactSelections"].as_array().unwrap().as_slice()
                else {
                    panic!("expected one primary and one secondary exact selection");
                };
                // The primary selection is submitted first. Both exact selections
                // keep their ordinary fill below the translucent Find fill.
                for (index, exact) in [(1, primary), (0, secondary)] {
                    assert_eq!(exact["aligned"], true);
                    assert_eq!(exact["background"], browser["selection"]);
                    expected.push(ordinary(&hits[index], color("selection")));
                }
                expected.push(ordinary(&partial, color("selection")));
                for (index, bounds) in body.into_iter().map(in_window).enumerate() {
                    let fill_color = color(if browser["selectedHits"][index] == true {
                        "selectedSearchMatch"
                    } else {
                        "searchMatch"
                    });
                    expected.push((bounds, fill_color.into(), no_border, clear));
                    expected.push((
                        bounds.dilate(width),
                        clear.into(),
                        Edges::all(width),
                        color("outline").into(),
                    ));
                }
            } else {
                let selection_color = rgba(0x4a86ed50);
                for (index, bounds) in body.into_iter().map(in_window).enumerate() {
                    let (fill_color, stroke) = if index == 1 {
                        (selection_color, rgb(0x174fc3))
                    } else {
                        (rgba(0xffce5a66), rgba(0x956000ff))
                    };
                    expected.push((bounds, fill_color.into(), Edges::all(px(1.)), stroke.into()));
                }
                // The light fallback still paints ordinary selections over Find marks.
                expected.push(ordinary(&hits[0], selection_color));
                expected.push(ordinary(&partial, selection_color));
            }
            assert_eq!(
                layout
                    .painted_find_quads
                    .iter()
                    .map(submitted_quad)
                    .collect::<Vec<_>>(),
                expected
            );
        });
    }
}
