use gpui::{Context, EntityInputHandler, Modifiers, TestAppContext, point, size};
use gpui_editor_codemirror::{
    Compartment, Extension, StateEffect, Transaction,
    language::{HighlightLayer, HighlightProperties, bidi_isolates},
    theme_one_dark::highlight_layer,
};

use super::*;
use crate::{
    document::{BlockPreview, Section, SectionKind},
    editor::bind_keys,
};

// The mixed-breaks fixture also drives the independently tested TextKit glyph proof.
const MIXED: &str = "(module (data \"ABC אב 🦀\") ;; عرب";

fn activate(editor: &mut Editor, cx: &mut Context<Editor>) -> Range<usize> {
    let fence = editor
        .markdown()
        .sections()
        .into_iter()
        .find(|s| matches!(s.preview, Some(BlockPreview::Code { .. })))
        .map_or(0..editor.source().len(), |s| s.source);
    editor.activate(
        Section::new(SectionKind::DraftCode, fence.clone(), fence),
        false,
        cx,
    );
    editor.source_fence().unwrap().body
}

fn configure(editor: &mut Editor, effect: StateEffect, cx: &mut Context<Editor>) {
    editor
        .view
        .update(
            Transaction::default().with_effects([effect]),
            editor.active.clone(),
            |_| (None, None),
        )
        .unwrap();
    cx.notify();
}

fn isolated(layout: &Layout) -> impl Iterator<Item = (&Row, &Item, &IsolatedRow)> {
    layout.rows.iter().flat_map(|row| {
        row.items.iter().filter_map(move |item| {
            let ItemPaint::Isolated(native) = &item.paint else {
                return None;
            };
            Some((row, item, native.as_ref()))
        })
    })
}

#[gpui::test]
fn live_wast_uses_native_pixels_hits_selection_and_undo_boundaries(cx: &mut TestAppContext) {
    let source = format!("Before\r\n\r\n```wast\r\n{MIXED}\r\n)\r\n```\r\n\r\nAfter");
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
    cx.simulate_resize(size(px(470.), px(400.)));
    let body = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            let body = activate(editor, cx);
            window.focus(&editor.focus);
            assert_eq!(
                isolated(&editor.shape_for_width(window, cx, px(320.))).count(),
                0
            );
            configure(editor, bidi_isolates(false).append(), cx);
            assert!(editor.source_isolates().is_some());
            body
        })
    });
    cx.run_until_parked();
    let (a, b, image) = cx.update(|window, cx| {
        let editor = editor.read(cx);
        let layout = editor.layout.as_ref().unwrap();
        let (row, item, native) = isolated(layout).next().expect("active native body");
        assert_eq!(item.source, body.start..body.start + MIXED.len());
        assert_eq!(native.layout.source(), &source[body.clone()]);
        assert!(
            native
                .layout
                .image
                .as_bytes(0)
                .unwrap()
                .chunks_exact(4)
                .any(|p| p[3] > 120)
        );
        let plain = TextKitLayout::shape(
            native.layout.source(),
            &[],
            &[TextRun {
                len: body.len(),
                font: window.text_style().font(),
                color: layout.theme.text.into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            &TextKitOptions {
                width: native.layout.size.width,
                margins: Edges::default(),
                font_size: px(16.),
                line_height: DEFAULT_LINE_HEIGHT,
                scale_factor: window.scale_factor(),
            },
        )
        .unwrap();
        assert_ne!(native.layout.image.as_bytes(0), plain.image.as_bytes(0));
        let punctuation: Vec<_> = item.x_spans(body.start + 31..body.start + 33).collect();
        assert_eq!(punctuation.len(), 1);
        let arabic = item
            .x_spans(body.start + 34..body.start + 40)
            .next()
            .unwrap();
        assert!(arabic.end < punctuation[0].start);
        assert_eq!(item.x_spans(body.start + 18..body.start + 21).count(), 2);
        let before = EditorSelection::cursor(body.start + 19, Some(Assoc::Before), None, None);
        let after = EditorSelection::cursor(body.start + 19, Some(Assoc::After), None, None);
        let a = layout.bounds_at_selection(before, false).unwrap().origin
            + point(px(-1.), row.height / 2.);
        let b = layout.bounds_at_selection(after, false).unwrap().origin
            + point(px(-1.), row.height / 2.);
        assert!(b.x - a.x > px(5.));
        assert_eq!(layout.bidi_hit_at(a).unwrap().assoc(), Some(Assoc::Before));
        assert_eq!(layout.bidi_hit_at(b).unwrap().assoc(), Some(Assoc::After));
        assert_eq!(layout.index_at(a), before.head());
        assert_eq!(layout.index_at(b), after.head());
        (a, b, native.layout.image.clone())
    });
    for (at, assoc) in [(a, Assoc::Before), (b, Assoc::After)] {
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.cursor(), body.start + 19);
            assert_eq!(editor.view.state().selection().main().assoc(), Some(assoc));
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.revision(), 0);
            assert!(!editor.view.can_undo());
            let (_, _, native) = isolated(editor.layout.as_ref().unwrap()).next().unwrap();
            assert!(Arc::ptr_eq(&image, &native.layout.image));
        });
    }
    cx.simulate_click(a, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.cursor(), body.start + 21);
    });
    let selected = body.start + 18..body.start + 21;
    cx.update(|_, cx| {
        editor.update(cx, |e, cx| {
            e.set_user_selection(selected.start, selected.end, "select.pointer", cx);
            cx.notify();
        })
    });
    cx.simulate_keystrokes("secondary-c");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            &source[selected.clone()]
        )
    });
    cx.update(|_, cx| {
        editor.update(cx, |e, cx| {
            e.replace(selected.clone(), "x", cx);
            assert_eq!(&e.source()[..body.start], &source[..body.start]);
            assert!(e.source().ends_with(&source[body.end..]));
            assert_eq!(e.view.undo_depth(), 1);
            e.replay_history(false, cx);
            assert_eq!(e.source(), source);
            assert_eq!(e.selection(), selected);
        })
    });
}

#[gpui::test]
fn java_json_and_sql_fences_apply_opt_in_live_isolation(cx: &mut TestAppContext) {
    for (language, code, expected) in [
        (
            "java",
            r#"class C{ String s="אב"; } // جد"#,
            &["\"אב\"", "// جد"][..],
        ),
        (
            "sql",
            r#"SELECT 'אב', s."جد"; -- גד!"#,
            &["'אב'", "\"جد\"", "-- גד!"][..],
        ),
        (
            "json",
            r#"{"אב": "جد", "key": "🙂"}"#,
            &["\"אב\"", "\"جد\"", "\"key\"", "\"🙂\""][..],
        ),
    ] {
        let source = format!("Before\r\n\r\n```{language}\r\n{code}\r\n```\r\n\r\nAfter");
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                let body = activate(editor, cx);
                assert!(editor.source_isolates().is_none());
                assert_eq!(
                    isolated(&editor.shape_for_width(window, cx, px(440.))).count(),
                    0
                );
                let slot = Compartment::new();
                configure(editor, slot.of(bidi_isolates(false)).append(), cx);
                let marks: Vec<_> = editor
                    .source_isolates()
                    .unwrap()
                    .into_iter()
                    .map(|mark| &code[mark.from..mark.to])
                    .collect();
                assert_eq!(marks, expected, "{language}");
                let layout = editor.shape_for_width(window, cx, px(440.));
                let (_, item, native) = isolated(&layout).next().expect("live syntax isolation");
                assert_eq!(item.source, body);
                assert_eq!(native.layout.source(), code);
                assert!(
                    native
                        .layout
                        .image
                        .as_bytes(0)
                        .unwrap()
                        .iter()
                        .any(|&byte| byte != 0)
                );
                let start = body.start + code.find("אב").unwrap();
                assert_eq!(item.x_spans(start..start + "אב".len()).count(), 1);
                configure(editor, slot.reconfigure(Extension::default()), cx);
                assert!(editor.source_isolates().is_none());
                assert_eq!(editor.source(), source);
                assert!(!editor.view.can_undo());
            });
        });
    }
}

#[gpui::test]
fn configured_lr_isolation_repaints_the_same_editable_body(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::{
        language::{
            lezer::{ISOLATE, Isolation},
            markdown::{
                MarkdownCodeLanguages, MarkdownCodeParser, MarkdownConfig, MarkdownParserConfig,
                markdown_with_parser,
            },
        },
        language_data::{json_language, lezer_language},
        view::bidi::Direction,
    };

    let json = json_language();
    let lezer = lezer_language();
    let json_next = json.with_node_props(&[ISOLATE.add(|row| match row.name() {
        "String" => Some(Some(Isolation::Rtl)),
        "PropertyName" => Some(None),
        _ => None,
    })]);
    let lezer_next = lezer
        .with_node_props(&[ISOLATE.add(|row| row.is("Literal").then_some(Some(Isolation::Ltr)))]);
    for (body, stock, configured, stock_marks, configured_marks) in [
        (
            r#"{"😀": "אב"}"#,
            MarkdownCodeParser::Json(json.clone()),
            MarkdownCodeParser::Json(json_next),
            &[("\"😀\"", None), ("\"אב\"", None)][..],
            &[("\"אב\"", Some(Direction::Rtl))][..],
        ),
        (
            r#"@top Main { "אב" }"#,
            MarkdownCodeParser::Lezer(lezer.clone()),
            MarkdownCodeParser::Lezer(lezer_next),
            &[][..],
            &[("\"אב\"", Some(Direction::Ltr))][..],
        ),
    ] {
        let source = format!("Before\r\n\r\n```example\r\n{body}\r\n```\r\n\r\nAfter");
        let install = |parser: MarkdownCodeParser| {
            markdown_with_parser(
                MarkdownConfig::default(),
                MarkdownParserConfig {
                    code_languages: Some(MarkdownCodeLanguages::Configured {
                        resolve: Arc::new(move |info| (info == "example").then(|| parser.clone())),
                        fallback: None,
                    }),
                    ..Default::default()
                },
            )
        };
        let slot = Compartment::new();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                Extension::group([slot.of(install(stock.clone())), bidi_isolates(true)]),
                window,
                cx,
            )
            .unwrap()
        });
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                let scope = activate(editor, cx);
                let selection = editor.view.state().selection().clone();
                for (parser, expected) in [
                    (stock.clone(), stock_marks),
                    (configured, configured_marks),
                    (stock, stock_marks),
                ] {
                    configure(editor, slot.reconfigure(install(parser)), cx);
                    let marks: Vec<_> = editor
                        .source_isolates()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|mark| (&body[mark.from..mark.to], mark.direction))
                        .collect();
                    assert_eq!(marks, expected);
                    let layout = editor.shape_for_width(window, cx, px(440.));
                    if expected.is_empty() {
                        assert_eq!(isolated(&layout).count(), 0);
                    } else {
                        let (_, item, native) = isolated(&layout).next().unwrap();
                        assert_eq!(item.source, scope);
                        assert_eq!(native.layout.source(), body);
                    }
                    assert_eq!(editor.source(), source);
                    assert_eq!(editor.view.state().selection(), &selection);
                    assert_eq!((editor.view.revision(), editor.view.undo_depth()), (0, 0));
                }
            })
        });
    }
}

#[gpui::test]
fn wrapped_wat_rows_keep_native_selection_and_crlf_ranges(cx: &mut TestAppContext) {
    let source = format!("```wat\r\n{MIXED}\r\n\r\n)\r\n```\r\n");
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let body = activate(editor, cx);
            configure(editor, bidi_isolates(false).append(), cx);
            let layout = editor.shape_for_width(window, cx, px(101.));
            let native: Vec<_> = isolated(&layout).collect();
            assert!(
                native.len() >= 6,
                "expected wrapped body: {} native rows",
                native.len()
            );
            for (row, item, native) in &native {
                assert_eq!(row.start..row.end, item.source);
                assert!(item.source.clone().all(|b| b < body.end));
                for stop in native.logical_stops() {
                    assert!(source.is_char_boundary(stop));
                    assert_ne!(
                        source.as_bytes().get(stop.wrapping_sub(1)..stop + 1),
                        Some(&b"\r\n"[..])
                    );
                }
            }
            let (upper, a, _) = native[0];
            let (lower, b, _) = native[1];
            assert!(upper.soft_end);
            assert_eq!(a.source.end, b.source.start);
            let boundary = a.source.end;
            let up = layout.bounds_at(boundary, true).unwrap();
            let down = layout.bounds_at(boundary, false).unwrap();
            assert_eq!(up.top(), upper.y);
            assert_eq!(down.top(), lower.y);
            assert_eq!(
                layout.position_at(up.origin + point(px(0.), px(8.))),
                (boundary, true)
            );
            assert_eq!(
                layout.position_at(down.origin + point(px(0.), px(8.))),
                (boundary, false)
            );
            assert_eq!(
                layout.row_edge(boundary, true, false),
                Some((a.source.start, false))
            );
            assert_eq!(
                layout.row_edge(boundary, false, true),
                Some((b.source.end, lower.soft_end))
            );
            let terminator = body.start + MIXED.len();
            let (_, item, _) = native
                .iter()
                .find(|(_, item, _)| item.selection_source().contains(&terminator))
                .unwrap();
            assert_eq!(item.source.end, terminator);
            assert_eq!(item.selection_source().end, terminator + 2);
            assert!(item.x_spans(terminator..terminator + 2).count() > 0);
            assert_eq!(
                layout.bounds_at(terminator + 1, false),
                layout.bounds_at(terminator, false)
            );
            let selected = body.start + 23..body.start + 28;
            assert_eq!(
                native
                    .iter()
                    .filter(|(_, item, _)| item.x_spans(selected.clone()).count() > 0)
                    .count(),
                2
            );
            let all_rows = native[0].2.layout.selection_rects(0..body.len()).unwrap();
            let gutter = layout
                .source_frame
                .as_ref()
                .unwrap()
                .gutter
                .as_ref()
                .unwrap();
            let mut numbers = Vec::new();
            for (row, item, native) in native {
                let bounds = native.bounds();
                let expected: Vec<_> = all_rows
                    .iter()
                    .filter(|rect| rect.top() < bounds.bottom() && bounds.top() < rect.bottom())
                    .filter_map(|rect| {
                        let span = rect.left().max(bounds.left())..rect.right().min(bounds.right());
                        (!span.is_empty()).then(|| item.left + span.start..item.left + span.end)
                    })
                    .collect();
                assert_eq!(item.x_spans(body.clone()).collect::<Vec<_>>(), expected);
                let (line, first) = gutter.line_at_row(row).unwrap();
                if first {
                    numbers.push(line);
                }
            }
            assert_eq!(numbers, [1, 2, 3]);
            assert_eq!(editor.source(), source);
        })
    });
    cx.simulate_resize(size(px(159.), px(500.)));
    cx.run_until_parked();
    let (boundary, upstream, downstream) = cx.update(|_, cx| {
        let layout = editor.read(cx).layout.as_ref().unwrap();
        let mut rows = isolated(layout);
        let (a, item, _) = rows.next().unwrap();
        let (b, _, _) = rows.next().unwrap();
        assert!(a.soft_end);
        let boundary = item.source.end;
        let up = layout.bounds_at(boundary, true).unwrap();
        let down = layout.bounds_at(boundary, false).unwrap();
        assert_eq!(down.top(), layout.origin.y + b.y);
        (boundary, up.center(), down.center())
    });
    for (point, assoc) in [(upstream, Assoc::Before), (downstream, Assoc::After)] {
        cx.simulate_click(point, Modifiers::none());
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let caret = editor.view.state().selection().main();
            assert_eq!((caret.head(), caret.assoc()), (boundary, Some(assoc)));
            let bounds = editor
                .layout
                .as_ref()
                .unwrap()
                .bounds_at_selection(caret, false)
                .unwrap();
            assert_eq!(bounds.center().y, point.y);
        });
    }
    // Selection history resets the editor's transient wrap flag. Commands must
    // still use the upstream row when the restored selection has its affinity.
    cx.simulate_click(upstream, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |e, cx| {
            let caret = e.view.state().selection().main();
            let (expected, _) = e
                .layout
                .as_ref()
                .unwrap()
                .vertical_caret(caret, None, None, true)
                .unwrap();
            e.upstream = false;
            assert!(e.move_in_source_rows(true, false, cx));
            assert_eq!(e.cursor(), expected.head());
            assert_eq!(e.view.state().selection().main().assoc(), expected.assoc());
        });
    });
}

#[gpui::test]
fn style_fold_and_composition_replace_the_whole_native_body(cx: &mut TestAppContext) {
    let source = format!("```wast\r\n{MIXED}\r\n  (func)\r\n)\r\n```\r\n");
    let style = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
    let body = cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            let body = activate(e, cx);
            window.focus(&e.focus);
            configure(e, bidi_isolates(false).append(), cx);
            configure(e, style.of(Extension::default()).append(), cx);
            body
        })
    });
    cx.run_until_parked();
    let native_count =
        cx.update(|_, cx| isolated(editor.read(cx).layout.as_ref().unwrap()).count());
    assert_eq!(native_count, 3);
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            let layout = e.shape_for_width(window, cx, px(9000.));
            assert_eq!(isolated(&layout).count(), 0, "TextKit width limit");
            let first = layout.row_index(body.start, false).unwrap();
            assert!(matches!(
                layout.rows[first].items[0].paint,
                ItemPaint::Text(_)
            ));
        });
    });

    // Foreground color is supported. A non-regular run switches paint and geometry together.
    for (bold, expected) in [(false, native_count), (true, 0)] {
        cx.update(|_, cx| {
            editor.update(cx, |e, cx| {
                let color = gpui_editor_codemirror::language::Color::from_rgba32(0xff0000ff);
                let layer = HighlightLayer::Resolved(HighlightProperties {
                    bold,
                    foreground: Some(color),
                    ..Default::default()
                });
                configure(e, style.reconfigure(highlight_layer(layer)), cx);
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let e = editor.read(cx);
            let layout = e.layout.as_ref().unwrap();
            assert_eq!(isolated(layout).count(), expected);
            if !bold {
                let (_, _, native) = isolated(layout).next().unwrap();
                assert!(
                    native
                        .layout
                        .image
                        .as_bytes(0)
                        .unwrap()
                        .chunks_exact(4)
                        .any(|p| p[2] > 230 && p[1] < 25 && p[0] < 25 && p[3] > 120)
                );
            } else {
                let first = layout.row_index(body.start, false).unwrap();
                assert!(matches!(
                    layout.rows[first].items[0].paint,
                    ItemPaint::Text(_)
                ));
            }
            assert_eq!(e.source(), source);
            assert!(!e.view.can_undo());
        });
    }
    cx.update(|_, cx| {
        editor.update(cx, |e, cx| {
            configure(e, style.reconfigure(Extension::default()), cx);
            e.set_selection(body.start, body.start);
            assert!(e.toggle_source_fold(cx));
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let e = editor.read(cx);
        let layout = e.layout.as_ref().unwrap();
        assert_eq!(isolated(layout).count(), 0);
        let hidden = source.find("(func)").unwrap();
        assert!(layout.fold_hides(hidden));
        assert_eq!(e.source(), source);
        assert!(!e.view.can_undo());
    });
    cx.update(|_, cx| editor.update(cx, |e, cx| assert!(e.toggle_source_fold(cx))));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| isolated(editor.read(cx).layout.as_ref().unwrap()).count()),
        native_count
    );
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            e.set_selection(body.start + 16, body.start + 16);
            e.replace_and_mark_text_in_range(None, "字", None, window, cx);
            assert!(e.is_composing());
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            assert_eq!(isolated(e.layout.as_ref().unwrap()).count(), 0);
            e.unmark_text(window, cx);
            e.replay_history(false, cx);
            assert_eq!(e.source(), source);
        })
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| isolated(editor.read(cx).layout.as_ref().unwrap()).count()),
        native_count
    );
}

#[gpui::test]
fn isolation_stays_opt_in_and_scoped(cx: &mut TestAppContext) {
    for (label, body, close, optimized, always) in [
        ("wat", "(data \"ABC\")", "\n```", false, true),
        ("json", "{\"s\": \"אב\"}", "\n```", true, true),
        ("json", "{\"s\": \"abc\"}", "\n```", false, true),
        ("json", "{\r\"s\": \"אב\"}", "\n```", false, false),
        ("wat", MIXED, "", false, false),
        ("wat", ";; \tعرب", "\n```", false, false),
        ("wat", ";; \u{2067}عرب\u{2069}", "\n```", false, false),
        ("wat", ";; عرب\r;; عرب", "\n```", false, false),
    ] {
        let source = format!("```{label}\n{body}{close}");
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                let slot = Compartment::new();
                configure(e, slot.of(bidi_isolates(true)).append(), cx);
                assert_eq!(
                    isolated(&e.shape_for_width(window, cx, px(320.))).count(),
                    0
                );
                activate(e, cx);
                for (extension, expected) in [
                    (bidi_isolates(true), always),
                    (bidi_isolates(false), optimized),
                    (Extension::default(), false),
                ] {
                    configure(e, slot.reconfigure(extension), cx);
                    let layout = e.shape_for_width(window, cx, px(320.));
                    assert_eq!(isolated(&layout).next().is_some(), expected, "{source:?}");
                    assert_eq!(e.source(), source);
                }
            })
        });
    }
}
