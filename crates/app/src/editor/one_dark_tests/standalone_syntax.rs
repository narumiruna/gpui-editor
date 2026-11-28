use std::ops::Range;

use gpui::{Rgba, TestAppContext, WindowAppearance, point, px, rgb, rgba, size};
use gpui_editor_codemirror::{
    Compartment, Extension,
    language::{
        Color, HighlightLayer, HighlightProperties, HighlightRule, HighlightSelector,
        HighlightStyle, HighlightTheme, TextDecoration,
    },
    language_data::NativeLanguage,
    theme_one_dark::{highlight_layer, one_dark, one_dark_highlighting, one_dark_theme},
};
use serde::Deserialize;

use super::{Editor, ItemPaint, Layout, activate_code, configure, painted_runs};

#[derive(Deserialize)]
struct BrowserFrame {
    mode: String,
    source: String,
    foreground: String,
    surface: String,
    samples: Vec<BrowserSample>,
}

#[derive(Deserialize)]
struct BrowserSample {
    role: String,
    text: String,
    range: [usize; 2],
    color: String,
    styled: bool,
}

fn color(hex: &str) -> Rgba {
    rgba(u32::from_str_radix(hex, 16).unwrap())
}

fn sample_has_style(layout: &Layout, range: Range<usize>, color: Rgba, underline: bool) -> bool {
    let mut covered = 0;
    for item in layout.rows.iter().flat_map(|row| &row.items) {
        let ItemPaint::Text(line) = &item.paint else {
            continue;
        };
        let from = range.start.max(item.source.start);
        let to = range.end.min(item.source.end);
        if from < to {
            let local = from - item.source.start..to - item.source.start;
            for (run, painted_underline) in painted_runs(line, color) {
                if painted_underline == underline {
                    covered += run
                        .end
                        .min(local.end)
                        .saturating_sub(run.start.max(local.start));
                }
            }
        }
    }
    covered == range.len()
}

#[gpui::test]
fn chromium_javascript_syntax_layers_reconfigure_in_standalone_and_fence(cx: &mut TestAppContext) {
    let cases: Vec<BrowserFrame> = include_str!("standalone_syntax_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        cases
            .iter()
            .map(|frame| frame.mode.as_str())
            .collect::<Vec<_>>(),
        [
            "initial",
            "view-only",
            "highlight-only",
            "combined",
            "removed"
        ]
    );
    let source = &cases[0].source;
    for standalone in [true, false] {
        let draft = source.replacen("42", "43", 1);
        let initial = if standalone {
            draft.clone()
        } else {
            format!("```js\n{draft}```\n")
        };
        let config = Compartment::new();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            let mut editor = Editor::with_native_extensions(
                initial,
                config.of(Extension::default()),
                window,
                cx,
            )
            .unwrap();
            if standalone {
                editor.set_path(Some("project/hello.js".into()), None);
            }
            editor
        });
        cx.simulate_resize(size(px(260.), px(480.)));
        let (body, doc, selection, revision, undo_depth) = cx.update(|window, cx| {
            assert_eq!(window.appearance(), WindowAppearance::Light);
            editor.update(cx, |editor, cx| {
                let body = if standalone {
                    0
                } else {
                    activate_code(editor, &draft, cx)
                };
                let number = body + draft.find("43").unwrap();
                assert!(editor.replace(number..number + 2, "42", cx));
                editor.set_selection(number, number + 2);
                let undo_depth = editor.view.undo_depth();
                assert!(undo_depth > 0);
                (
                    body,
                    editor.source().to_owned(),
                    editor.view.state().selection().clone(),
                    editor.view.revision(),
                    undo_depth,
                )
            })
        });

        let mut geometry = None;
        for frame in &cases {
            let (dark_view, syntax, extension) = match frame.mode.as_str() {
                "initial" | "removed" => (false, false, Extension::default()),
                "view-only" => (true, false, one_dark_theme()),
                "highlight-only" => (false, true, one_dark_highlighting()),
                "combined" => (true, true, one_dark()),
                _ => unreachable!(),
            };
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    configure(editor, config.reconfigure(extension), cx)
                });
            });
            cx.run_until_parked();
            cx.update(|_, cx| {
                let editor = editor.read(cx);
                assert_eq!(editor.source(), doc);
                assert_eq!(editor.view.state().selection(), &selection);
                assert_eq!(editor.view.revision(), revision);
                assert_eq!(editor.view.undo_depth(), undo_depth);
                assert_eq!(editor.view.redo_depth(), 0);
                assert_eq!(&doc[body..body + source.len()], frame.source);
                let layout = editor.layout.as_ref().unwrap();
                assert_eq!(layout.source_frame.is_none(), standalone);
                let theme = layout.theme;
                assert_eq!(theme.view.is_some(), dark_view);
                assert_eq!(
                    theme.text,
                    if dark_view {
                        color(&frame.foreground)
                    } else {
                        rgb(0x172336)
                    }
                );
                assert_eq!(
                    theme.surface,
                    if dark_view {
                        color(&frame.surface)
                    } else {
                        rgb(0xffffff)
                    }
                );
                let mut hits = Vec::new();
                for sample in &frame.samples {
                    let [start, end] = sample.range.map(|offset| offset + body);
                    assert_eq!(doc.get(start..end), Some(sample.text.as_str()));
                    assert_eq!(sample.styled, syntax && sample.role != "untagged");
                    let expected = if sample.styled {
                        color(&sample.color)
                    } else if !syntax && standalone {
                        // CodeMirror has no fallback here. The app keeps its Syntect palette.
                        match sample.role.as_str() {
                            "keyword" => theme.syntax_keyword,
                            "string" => theme.syntax_string,
                            "number" => theme.syntax_constant,
                            "comment" => theme.muted,
                            "function" | "untagged" => theme.text,
                            _ => unreachable!(),
                        }
                    } else {
                        theme.text
                    };
                    let fallback_name = !syntax && standalone && sample.role == "function";
                    assert!(
                        sample_has_style(layout, start..end, expected, false)
                            || (fallback_name
                                && sample_has_style(layout, start..end, theme.accent, false)),
                        "{} {} (standalone: {standalone}): expected {expected:?}",
                        frame.mode,
                        sample.role,
                    );
                    let bounds = layout.bounds_at(start, false).unwrap();
                    hits.push((
                        bounds,
                        layout.position_at(point(bounds.left(), bounds.top() + px(1.))),
                    ));
                }
                // Compare the retained GPUI shapes and hit geometry, not screen pixels.
                let rows: Vec<_> = layout
                    .rows
                    .iter()
                    .map(|row| {
                        let items: Vec<_> = row
                            .items
                            .iter()
                            .filter_map(|item| {
                                let ItemPaint::Text(line) = &item.paint else {
                                    return None;
                                };
                                assert_eq!(line.text.as_ref(), &doc[item.source.clone()]);
                                Some((item.source.clone(), item.left, item.width))
                            })
                            .collect();
                        (row.y, row.height, row.width, items)
                    })
                    .collect();
                let current = (rows, hits);
                if let Some(baseline) = &geometry {
                    assert_eq!(
                        &current, baseline,
                        "{} (standalone: {standalone})",
                        frame.mode
                    );
                } else {
                    geometry = Some(current);
                }
            });
        }
    }
}

#[gpui::test]
fn standalone_layers_use_verified_json_scope_and_keep_syntect_fallback(cx: &mut TestAppContext) {
    const SOURCE: &str = "{ \"字\": 42 }\r\n";
    const SCOPED: HighlightStyle<'static> = HighlightStyle::define(&[HighlightRule::new(
        &[HighlightSelector::new("number")],
        HighlightProperties {
            foreground: Some(Color::from_rgba32(0x304050ff)),
            bold: false,
            italic: false,
            text_decoration: None,
        },
    )])
    .with_all(HighlightProperties {
        foreground: None,
        bold: false,
        italic: false,
        text_decoration: Some(TextDecoration::Underline),
    });
    let layers = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            SOURCE.into(),
            layers.of(Extension::default()),
            window,
            cx,
        )
        .unwrap();
        editor.set_path(Some("data.json".into()), None);
        editor
    });
    let number = SOURCE.find("42").unwrap();
    let gap = 1..2;
    let json = SCOPED.with_native_scope(NativeLanguage::Json);
    for (path, style, number_color, underlined_gap) in [
        (
            "data.json",
            Some(SCOPED.with_native_scope(NativeLanguage::Rust)),
            0x9a3412,
            false,
        ),
        (
            "data.json",
            Some(json.with_theme(HighlightTheme::Dark)),
            0x9a3412,
            false,
        ),
        ("data.json", Some(json), 0x304050, true),
        ("data.JSON", Some(json), 0x9a3412, false),
        ("data.unknown", Some(json), 0x172336, false),
        ("data.json", None, 0x9a3412, false),
    ] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_path(Some(path.into()), None);
                let extension = style
                    .map(|style| highlight_layer(HighlightLayer::Active(style)))
                    .unwrap_or_default();
                configure(editor, layers.reconfigure(extension), cx);
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), SOURCE);
            let layout = editor.layout.as_ref().unwrap();
            assert!(
                sample_has_style(
                    layout,
                    number..number + 2,
                    rgb(number_color),
                    underlined_gap
                ),
                "{path} {style:?}"
            );
            assert!(
                sample_has_style(layout, gap.clone(), layout.theme.text, underlined_gap),
                "{path} {style:?}"
            );
        });
    }
}
