use std::cell::RefCell;

use gpui::{Edges, FontStyle, FontWeight, Rgba, TestAppContext, TextRun, rgba};
use gpui_editor_codemirror::{
    Compartment, Extension,
    theme_one_dark::{one_dark, one_dark_highlighting, one_dark_theme},
};
use serde_json::Value;

use super::{Editor, ItemPaint, activate_code, configure, painted_runs, px, size};

struct FontRequest {
    preview: bool,
    text: String,
    runs: Vec<(usize, FontWeight, FontStyle)>,
}

thread_local! {
    static FONT_REQUESTS: RefCell<Option<Vec<FontRequest>>> = const { RefCell::new(None) };
}

// The GPUI test platform uses one font ID. Record the requests passed to shaping.
pub(in crate::editor) fn record_fonts(preview: bool, text: &str, runs: &[TextRun]) {
    FONT_REQUESTS.with_borrow_mut(|requests| {
        if let Some(requests) = requests {
            requests.push(FontRequest {
                preview,
                text: text.into(),
                runs: runs
                    .iter()
                    .map(|run| (run.len, run.font.weight, run.font.style))
                    .collect(),
            });
        }
    });
}

fn fixture(name: &str) -> Value {
    include_str!("../../../tests/fixtures/codemirror/one_dark_stock_paint_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|row| row["name"] == name)
        .unwrap()
}

fn rgba_value(value: &Value) -> Rgba {
    rgba(u32::from_str_radix(value.as_str().unwrap(), 16).unwrap())
}

#[gpui::test]
fn stock_selection_and_caret_paint_follow_focus_and_theme_removal(cx: &mut TestAppContext) {
    let theme = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions("abc xyz".into(), theme.of(one_dark_theme()), window, cx)
            .unwrap()
    });
    cx.update(|window, cx| {
        window.activate_window();
        editor.update(cx, |editor, cx| {
            editor.set_selection(0, 3);
            window.focus(&editor.focus);
            cx.notify();
        });
    });
    cx.run_until_parked();
    let mut selected_bounds = None;
    for name in [
        "selection-focused",
        "selection-blurred",
        "selection-focused",
        "ordinary-caret",
    ] {
        let row = fixture(name);
        if name == "ordinary-caret" {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    editor.set_selection(3, 3);
                    cx.notify();
                })
            });
        } else {
            cx.update(|window, cx| {
                if row["focused"].as_bool().unwrap() {
                    window.focus(&editor.read(cx).focus);
                } else {
                    window.blur();
                }
            });
        }
        cx.run_until_parked();
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            if name == "ordinary-caret" {
                assert_eq!(layout.painted_carets.len(), 1);
                assert_eq!(
                    layout.painted_carets[0].background,
                    rgba_value(&row["color"]).into()
                );
                assert_eq!(layout.theme.selection, rgba_value(&row["selection"]));
            } else {
                assert_eq!(
                    editor.focus.is_focused(window),
                    row["focused"].as_bool().unwrap()
                );
                assert!(layout.painted_carets.is_empty());
                let [mark] = layout.painted_find_quads.as_slice() else {
                    panic!("expected an ordinary selection quad")
                };
                assert_eq!(mark.background, rgba_value(&row["selection"]).into());
                assert_eq!(mark.border_widths, Edges::all(px(0.)));
                assert_eq!(*selected_bounds.get_or_insert(mark.bounds), mark.bounds);
            }
            assert_eq!(editor.source(), "abc xyz");
            assert_eq!(editor.view.undo_depth(), 0);
        });
    }
    let before =
        cx.update(|_, cx| editor.read(cx).layout.as_ref().unwrap().painted_carets[0].clone());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            configure(editor, theme.reconfigure(Extension::default()), cx)
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(editor.focus.is_focused(window));
        let layout = editor.layout.as_ref().unwrap();
        let [caret] = layout.painted_carets.as_slice() else {
            panic!("expected a caret after removing the theme")
        };
        assert_ne!(caret.background, before.background);
        assert_eq!(caret.background, layout.theme.accent.into());
        assert_eq!(caret.bounds, before.bounds);
        assert_eq!(editor.cursor(), 3);
        assert_eq!(editor.source(), "abc xyz");
        assert_eq!(editor.view.undo_depth(), 0);
    });
}

#[gpui::test]
fn stock_gutter_and_fold_paint_follow_view_theme(cx: &mut TestAppContext) {
    const BODY: &str = "{\r\n  \"word\": \"字\"\r\n}";
    let source = format!("```json\r\n{BODY}\r\n```\r\n");
    let theme = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source.clone(), theme.of(one_dark_theme()), window, cx)
            .unwrap()
    });
    cx.simulate_resize(size(px(740.), px(580.)));
    let (selection, folds) = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let at = activate_code(editor, BODY, cx);
            editor.set_selection(at + 1, at + 1);
            assert!(editor.toggle_source_fold(cx));
            window.blur();
            (
                editor.view.state().selection().clone(),
                editor.source_fold_ranges(),
            )
        })
    });
    assert_eq!(folds.len(), 1);
    let row = fixture("fold-and-gutter");
    let mut fold_bounds = None;
    for extension in [one_dark_theme(), Extension::default()] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, theme.reconfigure(extension), cx)
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            let painted = &layout.painted_active_lines;
            assert!(!painted.gutter_labels.is_empty());
            let (line, geometry) = layout
                .rows
                .iter()
                .find_map(|row| {
                    row.items.iter().find_map(|item| match &item.paint {
                        ItemPaint::Fold(line) => {
                            Some((line, (item.left, item.width, row.y, row.height)))
                        }
                        _ => None,
                    })
                })
                .unwrap();
            assert_eq!(line.text.as_ref(), " … ");
            assert_eq!(*fold_bounds.get_or_insert(geometry), geometry);
            if layout.theme.view.is_some() {
                let expect_gutter = &row["gutter"];
                let expect_fold = &row["fold"];
                assert!(!painted.ordinary_gutter.is_empty());
                for fill in &painted.ordinary_gutter {
                    assert_eq!(
                        fill.background,
                        rgba_value(&expect_gutter["background"]).into()
                    );
                    assert_eq!(
                        fill.border_widths,
                        Edges::all(px(expect_gutter["borderWidth"].as_f64().unwrap() as f32))
                    );
                }
                for label in &painted.gutter_labels {
                    assert_eq!(
                        painted_runs(label, rgba_value(&expect_gutter["color"])),
                        [(0..label.len(), false)]
                    );
                }
                assert_eq!(
                    painted_runs(line, rgba_value(&expect_fold["color"])),
                    [(0..line.len(), false)]
                );
                assert_eq!(expect_fold["background"], "00000000");
                assert_eq!(expect_fold["borderWidth"], 0);
                let shaped = format!("{line:?}");
                let (_, decorations) = shaped.split_once("decoration_runs:").unwrap();
                assert!(decorations.contains("background_color: None"));
            } else {
                assert!(painted.ordinary_gutter.is_empty());
                for label in &painted.gutter_labels {
                    assert_eq!(
                        painted_runs(label, layout.theme.muted),
                        [(0..label.len(), false)]
                    );
                }
                assert!(painted_runs(line, rgba_value(&row["fold"]["color"])).is_empty());
            }
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.source_fold_ranges(), folds);
            assert_eq!(editor.view.undo_depth(), 0);
        });
    }
}

#[gpui::test]
fn stock_markdown_syntax_styles_reach_draft_and_text_preview(cx: &mut TestAppContext) {
    const HEADING: &str = "# Heading";
    const INLINE: &str = "**Bold** *Italic* [Link](https://example.invalid)";
    let source =
        format!("```markdown\n{HEADING}\n{INLINE}\n```\n\n```markdown\n{HEADING}\n{INLINE}\n```\n");
    let theme = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source.clone(), theme.of(one_dark_theme()), window, cx)
            .unwrap()
    });
    cx.simulate_resize(size(px(870.), px(720.)));
    let (at, selection, history) = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let at = activate_code(editor, HEADING, cx);
            (
                at,
                editor.view.state().selection().clone(),
                editor.view.undo_depth(),
            )
        })
    });
    for (name, extension) in [
        ("view-only", one_dark_theme()),
        ("highlight-only", one_dark_highlighting()),
        ("combined", one_dark()),
        ("view-only", one_dark_theme()),
    ] {
        let row = fixture(name);
        FONT_REQUESTS.with_borrow_mut(|requests| *requests = Some(Vec::new()));
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, theme.reconfigure(extension), cx)
            })
        });
        cx.run_until_parked();
        let font_requests = FONT_REQUESTS.with_borrow_mut(Option::take).unwrap();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            let items = || layout.rows.iter().flat_map(|row| &row.items);
            let draft_line = |offset| {
                items()
                    .find_map(|item| match &item.paint {
                        ItemPaint::Text(line) if item.source.start == at + offset => {
                            Some(line.as_ref())
                        }
                        _ => None,
                    })
                    .unwrap()
            };
            let preview = items()
                .find_map(|item| match &item.paint {
                    ItemPaint::Block(block) => Some(block.code_lines().collect::<Vec<_>>()),
                    _ => None,
                })
                .unwrap();
            assert_eq!(preview.len(), 2);
            for (is_preview, lines) in [
                (false, [draft_line(0), draft_line(HEADING.len() + 1)]),
                (true, [preview[0], preview[1]]),
            ] {
                assert_eq!(lines[0].text.as_ref(), HEADING);
                assert_eq!(lines[1].text.as_ref(), INLINE);
                for (token, index) in [("Heading", 0), ("Bold", 1), ("Italic", 1), ("Link", 1)] {
                    let line = lines[index];
                    let start = line.text.find(token).unwrap();
                    let end = start + token.len();
                    let expected = &row["tokens"][token];
                    let color = if expected["color"] == "000000ff" {
                        layout.theme.text
                    } else {
                        rgba_value(&expected["color"])
                    };
                    let underline = expected["underline"].as_bool().unwrap();
                    assert!(
                        painted_runs(line, color)
                            .iter()
                            .any(|(range, actual)| range.start <= start
                                && end <= range.end
                                && *actual == underline),
                        "{name}: {token}"
                    );
                    let request = font_requests
                        .iter()
                        .rev()
                        .find(|request| {
                            request.preview == is_preview && request.text == line.text.as_ref()
                        })
                        .unwrap();
                    let mut from = 0;
                    let font = request
                        .runs
                        .iter()
                        .find_map(|&(len, weight, style)| {
                            let to = from + len;
                            let contains = from <= start && end <= to;
                            from = to;
                            contains.then_some((weight, style))
                        })
                        .unwrap();
                    let weight = if expected["weight"] == 700 {
                        FontWeight::BOLD
                    } else {
                        FontWeight::NORMAL
                    };
                    let style = if expected["italic"] == true {
                        FontStyle::Italic
                    } else {
                        FontStyle::Normal
                    };
                    assert_eq!(font, (weight, style), "{name}: {token}");
                }
            }
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.undo_depth(), history);
        });
    }
}
