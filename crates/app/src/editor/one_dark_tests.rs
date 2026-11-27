use std::cell::RefCell;

use gpui::{Hsla, Rgba, ShapedLine, TestAppContext, WindowAppearance};
use gpui_editor_codemirror::{
    Compartment, Precedence, StateEffect,
    language::{
        Color, HighlightLayer, HighlightProperties, HighlightResets, HighlightRule,
        HighlightSelector, HighlightStyle, HighlightTheme, TextDecoration,
    },
    language_data::NativeLanguage,
    theme_one_dark::{
        EditorSurfaceColors, highlight_layer, one_dark, one_dark_highlighting, one_dark_theme,
        override_editor_surface,
    },
};

use super::*;

pub(super) mod panel_menu;
mod stock_paint;
pub(super) use stock_paint::record_fonts;

const CODE: &str = "{\"字\": \"🐱\"}";

pub(super) fn panel_input_colors(
    field: &str,
    dark: bool,
    focused: bool,
) -> crate::app::developer::FieldColors {
    use crate::app::developer::FieldColors;

    let (surface, text, border) = if dark {
        let fixture =
            include_str!("../../../codemirror/tests/fixtures/one_dark_panel_input_browser.jsonl");
        let row = fixture
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .find(|row| row["mode"] == "theme" && row["field"] == field)
            .unwrap();
        assert_eq!(row["width"], "1px");
        let get = |key: &str| u32::from_str_radix(row[key].as_str().unwrap(), 16).unwrap();
        (get("surface"), get("text"), get("border"))
    } else {
        (0xffffff, 0x172336, 0xd3dce9)
    };
    let accent = rgb(if dark { 0x528bff } else { 0x174fc3 });
    FieldColors {
        surface: rgb(surface),
        text: rgb(text),
        border: if focused { accent } else { rgb(border) },
        accent,
        selection: rgba(if dark { 0x3e4451ff } else { 0x4a86ed50 }),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OverlayText {
    Anchored,
    SelectedCompletion,
    Completion,
}

thread_local! {
    static OVERLAY_TEXT: RefCell<Option<Vec<(OverlayText, Hsla)>>> = const { RefCell::new(None) };
}

pub(super) fn capture_overlay_text() {
    OVERLAY_TEXT.replace(Some(Vec::new()));
}

pub(super) fn captured_overlay_text() -> Vec<(OverlayText, Hsla)> {
    OVERLAY_TEXT.take().unwrap()
}

pub(super) fn overlay_text_probe(role: OverlayText) -> Option<impl IntoElement> {
    OVERLAY_TEXT
        .with_borrow(|capture| capture.is_some())
        .then(|| {
            canvas(
                |_, _, _| (),
                move |_, (), window, _| {
                    OVERLAY_TEXT.with_borrow_mut(|capture| {
                        if let Some(colors) = capture {
                            colors.push((role, window.text_style().color));
                        }
                    });
                },
            )
            .absolute()
            .w_0()
            .h_0()
        })
}

pub(super) fn configure(editor: &mut Editor, effect: StateEffect, cx: &mut Context<Editor>) {
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

pub(super) fn activate_code(editor: &mut Editor, body: &str, cx: &mut Context<Editor>) -> usize {
    let fence = editor.markdown().sections()[0].source.clone();
    let at = editor.source().find(body).unwrap();
    assert!(fence.contains(&at));
    editor.activate(
        Section::new(SectionKind::DraftCode, fence.clone(), fence),
        false,
        cx,
    );
    at
}

fn painted_text_contains_color(line: &ShapedLine, color: Rgba) -> bool {
    // GPUI exposes the decoration runs consumed by ShapedLine::paint through Debug.
    let shaped = format!("{line:?}");
    let (_, runs) = shaped.split_once("decoration_runs:").unwrap();
    runs.contains(&format!("color: {:?}", Hsla::from(color)))
}

#[gpui::test]
fn configured_markdown_parsers_apply_to_previews_and_editable_fences(cx: &mut TestAppContext) {
    use std::sync::Arc;

    use gpui_editor_codemirror::language::markdown::{
        MarkdownCodeLanguages, MarkdownConfig, MarkdownParserConfig, markdown_with_parser,
    };

    let source = "```json info\r\nname = \"字\"\r\n```\r\n\r\n```\r\n{\"名\": \"🐱\"}\r\n```\r\n\r\nOutside.";
    let extension = markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Resolve(Arc::new(|name| {
                (name == "json").then_some(NativeLanguage::Python)
            }))),
            default_code_language: Some(NativeLanguage::Json),
            ..Default::default()
        },
    );
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.into(),
            Extension::group([extension, one_dark()]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(500.), px(400.)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            let layout = editor.layout.as_ref().unwrap();
            let previews: Vec<_> = layout
                .rows
                .iter()
                .flat_map(|row| &row.items)
                .filter_map(|item| match &item.paint {
                    ItemPaint::Block(block) => block.code_lines().next(),
                    _ => None,
                })
                .collect();
            let palette = gpui_editor_codemirror::theme_one_dark::COLOR;
            assert_eq!(previews.len(), 2);
            assert_eq!(previews[0].text.as_ref(), "name = \"字\"");
            assert_eq!(previews[1].text.as_ref(), "{\"名\": \"🐱\"}");
            assert!(painted_text_contains_color(
                previews[0],
                rgba(palette.sage.rgba32())
            ));
            assert!(painted_text_contains_color(
                previews[1],
                rgba(palette.coral.rgba32())
            ));
            let at = activate_code(editor, "name =", cx);
            editor.move_to(at, false, cx);
            window.focus(&editor.focus);
            assert_eq!(
                editor.source_fence().unwrap().language(),
                Some(NativeLanguage::Python)
            );
        });
    });
    cx.simulate_keystrokes("cmd-/");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source.replace("name =", "# name ="));
        assert_eq!(editor.view.undo_depth(), 1);
    });
    cx.simulate_keystrokes("cmd-z");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), source);
            let fence = editor
                .markdown()
                .sections()
                .into_iter()
                .find(|section| section.source.contains(&source.find('名').unwrap()))
                .unwrap()
                .source;
            editor.activate(
                Section::new(SectionKind::DraftCode, fence.clone(), fence),
                false,
                cx,
            );
            let fence = editor.source_fence().unwrap();
            assert_eq!(fence.language(), Some(NativeLanguage::Json));
            assert_eq!(&editor.source()[fence.body], "{\"名\": \"🐱\"}");
        });
    });
}

#[gpui::test]
fn one_dark_view_and_syntax_reconfigure_independently_in_the_painted_frame(
    cx: &mut TestAppContext,
) {
    let source = format!("```json\r\n{CODE}\r\n```\r\n\r\n```json\r\n{CODE}\r\n```\r\n");
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source, window, cx));
    cx.simulate_resize(size(px(480.), px(320.)));
    let config = Compartment::new();
    let (source, selection, history, code_start) = cx.update(|window, cx| {
        assert_eq!(window.appearance(), WindowAppearance::Light);
        editor.update(cx, |editor, cx| {
            editor.replace(0..0, "Note\r\n\r\n", cx);
            let at = activate_code(editor, CODE, cx);
            editor.set_selection(at, at + CODE.len());
            configure(editor, config.of(Extension::default()).append(), cx);
            let history = editor.view.undo_depth();
            assert!(history > 0);
            (
                editor.source().to_owned(),
                editor.view.state().selection().clone(),
                history,
                at,
            )
        })
    });

    let host = override_editor_surface(EditorSurfaceColors {
        foreground: Some(Color::from_rgba32(0x3040_50ff)),
        background: Some(Color::from_rgba32(0x1020_30ff)),
    });
    let cases = [
        (None, false, false, 0x172336, 0xffffff),
        (Some(one_dark_theme()), true, false, 0xabb2bf, 0x282c34),
        (Some(one_dark()), true, true, 0xabb2bf, 0x282c34),
        (
            Some(Extension::group([host.clone(), one_dark()])),
            true,
            true,
            0x304050,
            0x102030,
        ),
        (
            Some(Extension::group([one_dark(), host.clone()])),
            true,
            true,
            0xabb2bf,
            0x282c34,
        ),
        (Some(host), false, false, 0x304050, 0x102030),
        (
            Some(one_dark_highlighting()),
            false,
            true,
            0x172336,
            0xffffff,
        ),
        (Some(Extension::default()), false, false, 0x172336, 0xffffff),
    ];
    for (extension, dark_view, highlighted, foreground, surface) in cases {
        if let Some(extension) = extension {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    configure(editor, config.reconfigure(extension), cx);
                })
            });
        }
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            // This layout was retained by the canvas paint callback, not a direct call to shape.
            let layout = editor.layout.as_ref().unwrap();
            let foreground = rgb(foreground);
            assert_eq!(layout.theme.surface, rgb(surface));
            assert_eq!(
                layout.theme.panel,
                rgb(if dark_view { 0x21252b } else { 0xf0f3f9 })
            );
            assert_eq!(
                layout.theme.selection,
                rgba(if dark_view { 0x3e4451ff } else { 0x4a86ed50 })
            );
            let items: Vec<_> = layout.rows.iter().flat_map(|row| &row.items).collect();
            let draft = items
                .iter()
                .find_map(|item| match &item.paint {
                    ItemPaint::Text(line) if item.source.start == code_start => Some(line),
                    _ => None,
                })
                .unwrap();
            let preview = items
                .iter()
                .find_map(|item| match &item.paint {
                    ItemPaint::Block(block) => block.code_lines().next(),
                    _ => None,
                })
                .unwrap();
            assert_eq!(draft.text.as_ref(), CODE);
            assert_eq!(preview.text.as_ref(), CODE);
            assert!(painted_text_contains_color(draft, foreground));
            assert_eq!(
                painted_text_contains_color(draft, rgb(0x98c379)),
                highlighted
            );
            let preview_string = if highlighted {
                rgb(0x98c379)
            } else if dark_view {
                foreground
            } else {
                rgb(0x14683f)
            };
            assert!(painted_text_contains_color(preview, preview_string));
            assert_eq!(
                painted_text_contains_color(preview, rgb(0x98c379)),
                highlighted
            );
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.undo_depth(), history);
            assert_eq!(editor.view.redo_depth(), 0);
        });
    }
}

#[gpui::test]
fn highlighting_a_wrapped_draft_keeps_its_source_geometry(cx: &mut TestAppContext) {
    let value = format!("\"{}\"", "字🐱".repeat(20));
    let source = format!("```json\r\n{value}\r\n```\r\n");
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source, window, cx));
    cx.simulate_resize(size(px(200.), px(480.)));
    let at = cx.update(|_, cx| editor.update(cx, |editor, cx| activate_code(editor, &value, cx)));
    let end = at + value.len();
    let rendered_value = |editor: &Editor, highlighted| {
        let layout = editor.layout.as_ref().unwrap();
        layout
            .rows
            .iter()
            .flat_map(|row| &row.items)
            .filter_map(|item| {
                if let ItemPaint::Text(line) = &item.paint
                    && (at..end).contains(&item.source.start)
                {
                    assert_eq!(line.text.as_ref(), &editor.source()[item.source.clone()]);
                    assert_eq!(
                        painted_text_contains_color(line, rgb(0x98c379)),
                        highlighted
                    );
                    Some((item.source.clone(), item.left, item.width))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    cx.run_until_parked();
    let old = cx.update(|_, cx| {
        let old = rendered_value(editor.read(cx), false);
        editor.update(cx, |editor, cx| {
            configure(editor, one_dark_highlighting().append(), cx)
        });
        old
    });
    assert!(old.len() > 1);
    assert_eq!(old.first().unwrap().0.start, at);
    assert_eq!(old.last().unwrap().0.end, end);
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(rendered_value(editor.read(cx), true), old));
}

pub(super) fn painted_runs(line: &ShapedLine, color: Rgba) -> Vec<(Range<usize>, bool)> {
    // GPUI keeps these byte ranges private. Inspect the runs on the retained frame.
    let shaped = format!("{line:?}");
    let (_, runs) = shaped.split_once("decoration_runs:").unwrap();
    let color = format!("color: {:?},", Hsla::from(color));
    let mut offset = 0;
    let result = runs
        .split("DecorationRun { len: ")
        .skip(1)
        .filter_map(|run| {
            let (len, style) = run.split_once(", ").unwrap();
            let end = offset + len.parse::<usize>().unwrap();
            let range = offset..end;
            offset = end;
            style
                .starts_with(&color)
                .then(|| (range, style.contains("underline: Some")))
        })
        .collect();
    assert_eq!(offset, line.text.len());
    result
}

#[gpui::test]
fn scoped_all_covers_json_gaps_in_draft_and_preview(cx: &mut TestAppContext) {
    const BODY: &str = "{\"字\": 1, \"🦀\": 2}";
    const LIGHT: u32 = 0x172336;
    const IVORY: u32 = 0xabb2bf;
    const RED: u32 = 0xff0000;
    const AUX: HighlightStyle<'static> = HighlightStyle::define(&[HighlightRule::new(
        &[HighlightSelector::new("punctuation")],
        HighlightProperties {
            foreground: Some(Color::from_rgba32(RED << 8 | 0xff)),
            bold: false,
            italic: true,
            text_decoration: None,
        },
    )])
    .with_all(HighlightProperties {
        foreground: None,
        bold: false,
        italic: false,
        text_decoration: Some(TextDecoration::Underline),
    })
    .with_native_scope(NativeLanguage::Json)
    .with_theme(HighlightTheme::Dark);
    const SEPARATOR_RESET: HighlightStyle<'static> = HighlightStyle::define(&[HighlightRule::new(
        &[HighlightSelector::new("separator")],
        HighlightProperties {
            foreground: None,
            bold: false,
            italic: false,
            text_decoration: None,
        },
    )
    .with_resets(HighlightResets {
        no_decoration: true,
        ..HighlightResets::NONE
    })])
    .with_native_scope(NativeLanguage::Json);

    let source = format!("```json\r\n{BODY}\r\n```\r\n\r\n```json\r\n{BODY}\r\n```\r\n");
    let config = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source, config.of(Extension::default()), window, cx).unwrap()
    });
    cx.simulate_resize(size(px(720.), px(480.)));
    let (at, source, selection, revision, history) = cx.update(|window, cx| {
        assert_eq!(window.appearance(), WindowAppearance::Light);
        editor.update(cx, |editor, cx| {
            editor.replace(0..0, "Note here.\r\n\r\n", cx);
            let at = activate_code(editor, BODY, cx);
            editor.set_selection(at + 1, at + BODY.len() - 1);
            window.focus(&editor.focus);
            let history = editor.view.undo_depth();
            assert!(history > 0);
            (
                at,
                editor.source().to_owned(),
                editor.view.state().selection().clone(),
                editor.view.revision(),
                history,
            )
        })
    });
    let auxiliary = highlight_layer(HighlightLayer::Active(AUX));
    let high = auxiliary.clone().with_precedence(Precedence::High);
    let mut gap_mismatches = Vec::new();
    for (case, extensions, foreground, separator, underline) in [
        ("uninstalled", Extension::default(), LIGHT, LIGHT, false),
        (
            "One Dark first",
            Extension::group([one_dark(), auxiliary]),
            IVORY,
            IVORY,
            true,
        ),
        (
            "auxiliary first",
            Extension::group([one_dark(), high.clone()]),
            IVORY,
            RED,
            true,
        ),
        (
            "separator reset",
            Extension::group([
                one_dark(),
                high.clone(),
                highlight_layer(HighlightLayer::Active(SEPARATOR_RESET))
                    .with_precedence(Precedence::Highest),
            ]),
            IVORY,
            RED,
            true,
        ),
        (
            "scope miss",
            Extension::group([
                one_dark(),
                highlight_layer(HighlightLayer::Active(
                    AUX.with_native_scope(NativeLanguage::Css),
                )),
            ]),
            IVORY,
            IVORY,
            false,
        ),
        (
            "theme removed",
            Extension::group([one_dark_highlighting(), high]),
            LIGHT,
            IVORY,
            false,
        ),
        (
            "highlighters removed",
            one_dark_theme(),
            IVORY,
            IVORY,
            false,
        ),
        ("all removed", Extension::default(), LIGHT, LIGHT, false),
    ] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, config.reconfigure(extensions), cx)
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.revision(), revision);
            assert_eq!(editor.view.undo_depth(), history);
            assert_eq!(editor.view.redo_depth(), 0);
            let layout = editor.layout.as_ref().unwrap();
            assert_eq!(layout.theme.text, rgb(foreground));
            let items = || layout.rows.iter().flat_map(|row| &row.items);
            let draft = items()
                .find_map(|item| match &item.paint {
                    ItemPaint::Text(line) if item.source.start == at => Some(line.as_ref()),
                    _ => None,
                })
                .unwrap();
            let preview = items()
                .find_map(|item| match &item.paint {
                    ItemPaint::Block(block) => block.code_lines().next(),
                    _ => None,
                })
                .unwrap();
            for (kind, line) in [("draft", draft), ("preview", preview)] {
                assert_eq!(line.text.as_ref(), BODY);
                let separators = painted_runs(line, rgb(separator));
                let separator_underline = underline && case != "separator reset";
                for (at, _) in BODY.match_indices([':', ',']) {
                    assert!(
                        separators
                            .iter()
                            .any(|(r, under)| r.contains(&at) && *under == separator_underline),
                        "{case}: {kind} separator at byte {at}"
                    );
                }
                if underline {
                    let braces = painted_runs(line, rgb(RED));
                    assert!(braces.iter().any(|(r, under)| r.contains(&0) && *under));
                }
                let runs = painted_runs(line, rgb(foreground));
                for (at, _) in BODY.match_indices(' ') {
                    let actual = runs.iter().find(|(r, _)| r.contains(&at)).unwrap().1;
                    if actual != underline {
                        gap_mismatches.push((case, kind, at, actual));
                    }
                }
            }
            if matches!(case, "uninstalled" | "all removed") {
                let number = BODY.find('1').unwrap();
                assert!(
                    painted_runs(preview, rgb(0x673ab7)).contains(&(number..number + 1, false))
                );
            }
        });
    }
    assert!(
        gap_mismatches.is_empty(),
        "unexpected gap underlines: {gap_mismatches:?}"
    );
}

#[gpui::test]
fn gap_decorations_stay_separate_from_folds_atoms_and_composition(cx: &mut TestAppContext) {
    const BODY: &str = "{ \"字\": 1,\r\n  \"🦀\": 2\r\n} ";
    let source = format!(
        "```json\r\n{BODY}\r\n```\r\n\r\nNote here.\r\n\r\n- list item\r\n\r\n```unrecognized\r\nplain text\r\n```"
    );
    let config = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let style = HighlightStyle::define(&[])
            .with_native_scope(NativeLanguage::Json)
            .with_all(HighlightProperties {
                text_decoration: Some(TextDecoration::Underline),
                ..Default::default()
            });
        let extension = config.of(highlight_layer(HighlightLayer::Active(style)));
        Editor::with_native_extensions(source.clone(), extension, window, cx).unwrap()
    });
    cx.simulate_resize(size(px(720.), px(480.)));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let at = activate_code(editor, BODY, cx);
            editor.set_selection(at + 1, at + 1);
            window.focus(&editor.focus);
            assert!(editor.toggle_source_fold(cx));
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let layout = editor.read(cx).layout.as_ref().unwrap();
        let at = source.find(BODY).unwrap();
        let body = at..at + BODY.len();
        let mut plain = Vec::new();
        let mut styled_tail = false;
        for item in layout.rows.iter().flat_map(|row| &row.items) {
            let line = match &item.paint {
                ItemPaint::Text(line) if body.contains(&item.source.start) => {
                    if line.text.as_ref() == "} " {
                        assert_eq!(painted_runs(line, layout.theme.text), [(0..2, true)]);
                        styled_tail = true;
                    }
                    continue;
                }
                ItemPaint::Text(line) | ItemPaint::Fold(line) => line,
                ItemPaint::Block(block) => block.code_lines().next().unwrap(),
                _ => continue,
            };
            assert!(!format!("{line:?}").contains("underline: Some"));
            plain.push(line.text.as_ref());
        }
        assert!(styled_tail);
        for text in ["```json", " … ", "```", "Note here.", "•  ", "plain text"] {
            assert!(plain.contains(&text), "missing unstyled item {text:?}");
        }
    });
    let indent = source.find("  \"🦀\"").unwrap();
    let mark = indent + 1;
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.toggle_source_fold(cx));
            editor.set_selection(mark, mark);
            editor.replace_and_mark_text_in_range(None, " ", None, window, cx);
            assert_eq!(editor.marked, Some(mark..mark + 1));
        });
    });
    for installed in [true, false] {
        if !installed {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    configure(editor, config.reconfigure(Extension::default()), cx)
                });
            });
        }
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.marked, Some(mark..mark + 1));
            let layout = editor.layout.as_ref().unwrap();
            let line = layout
                .rows
                .iter()
                .flat_map(|row| &row.items)
                .find_map(|item| match &item.paint {
                    ItemPaint::Text(line) if item.source.start == indent => Some(line),
                    _ => None,
                })
                .unwrap();
            assert_eq!(line.text.as_ref(), "   \"🦀\": 2");
            assert_eq!(
                painted_runs(line, layout.theme.text),
                [
                    (0..1, installed),
                    (1..2, true),
                    (2..line.text.len(), installed),
                ]
            );
        });
    }
}

#[gpui::test]
fn wast_root_scopes_style_stock_draft_and_preview(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::{language::wast::NodeProp, language_data::wast_language};

    const BODY: &str = "(func  $f)";
    const GAP: u32 = 0x3a8457;
    const KEYWORD: u32 = 0xa12843;
    const WRONG: u32 = 0x1068a4;
    const RULES: &[HighlightRule<'_>] = &[HighlightRule::new(
        &[HighlightSelector::new("keyword")],
        HighlightProperties {
            foreground: Some(Color::from_rgba32(KEYWORD << 8 | 0xff)),
            bold: false,
            italic: false,
            text_decoration: None,
        },
    )];
    let stock = wast_language();
    let root = stock.node_types().find(|row| row.is_top()).unwrap();
    let prop = NodeProp::new();
    let other = stock.with_node_props(&[prop.add(|row| row.is_top().then_some(true))]);
    let wrong_root = other.node_types().find(|row| row.is_top()).unwrap();
    let fill = |color: u32| HighlightProperties {
        foreground: Some(Color::from_rgba32(color << 8 | 0xff)),
        text_decoration: Some(TextDecoration::Underline),
        ..Default::default()
    };
    let root_layer = highlight_layer(HighlightLayer::Active(
        HighlightStyle::define(&[])
            .with_all(fill(GAP))
            .with_wast_node_type_scope(root),
    ));
    let miss_layer = highlight_layer(HighlightLayer::Active(
        HighlightStyle::define(&[])
            .with_all(fill(WRONG))
            .with_wast_node_type_scope(wrong_root),
    ));
    let facet_layer = highlight_layer(HighlightLayer::Active(
        HighlightStyle::define(RULES).with_wast_facet_scope(other.data_facet()),
    ));
    let source =
        format!("```wat\r\n{BODY}\r\n```\r\n\r\n```wast\r\n{BODY}\r\n```\r\n\r\nProse {BODY}\r\n");
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
    cx.simulate_resize(size(px(750.), px(660.)));
    let config = Compartment::new();
    let (at, selection, history) = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let at = activate_code(editor, BODY, cx);
            configure(editor, config.of(Extension::default()).append(), cx);
            (
                at,
                editor.view.state().selection().clone(),
                editor.view.undo_depth(),
            )
        })
    });

    for (extension, facet_matches, root_matches) in [
        (
            Extension::group([miss_layer.clone(), facet_layer.clone(), root_layer]),
            true,
            true,
        ),
        (Extension::group([miss_layer, facet_layer]), true, false),
        (Extension::default(), false, false),
    ] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, config.reconfigure(extension), cx)
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            let items = || layout.rows.iter().flat_map(|row| &row.items);
            let draft = items()
                .find_map(|item| match &item.paint {
                    ItemPaint::Text(line) if item.source.start == at => Some(line),
                    _ => None,
                })
                .unwrap();
            let preview = items()
                .find_map(|item| match &item.paint {
                    ItemPaint::Block(block) => block.code_lines().next(),
                    _ => None,
                })
                .unwrap();
            for line in [draft, preview] {
                assert_eq!(line.text.as_ref(), BODY);
                let keyword = painted_runs(line, rgb(KEYWORD));
                if facet_matches {
                    assert_eq!(keyword, [(1..5, root_matches)]);
                } else {
                    assert!(keyword.is_empty());
                }
                assert_eq!(
                    painted_runs(line, rgb(GAP))
                        .iter()
                        .any(|(range, underlined)| {
                            range.start <= 5 && range.end >= 7 && *underlined
                        }),
                    root_matches
                );
                assert!(painted_runs(line, rgb(WRONG)).is_empty());
            }
            let prose = items()
                .find_map(|item| match &item.paint {
                    ItemPaint::Text(line) if line.text.starts_with("Prose") => Some(line),
                    _ => None,
                })
                .unwrap();
            for color in [GAP, KEYWORD, WRONG] {
                assert!(painted_runs(prose, rgb(color)).is_empty());
            }
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.undo_depth(), history);
        });
    }
}

#[gpui::test]
fn native_scopes_color_html_and_mounted_css_in_drafts_and_previews(cx: &mut TestAppContext) {
    const STRING: &str = "\"字🐱\"";
    const CSS: &str = "a { content: \"字🐱\"; }";
    const HTML: &str = "<p title=\"字🐱\">note</p>";
    const FOREGROUND: u32 = 0x172336;
    const CSS_COLOR: u32 = 0xa12843;
    const HTML_COLOR: u32 = 0x1068a4;
    const UNSCOPED: u32 = 0x3a8457;
    const WRONG: u32 = 0xe53c09;
    const fn rule(color: u32) -> HighlightRule<'static> {
        const SELECTORS: &[HighlightSelector<'_>] = &[HighlightSelector::new("string")];
        HighlightRule::new(
            SELECTORS,
            HighlightProperties {
                foreground: Some(Color::from_rgba32(color << 8 | 0xff)),
                bold: false,
                italic: false,
                text_decoration: None,
            },
        )
    }
    const CSS_RULE: &[HighlightRule<'_>] = &[rule(CSS_COLOR)];
    const HTML_RULE: &[HighlightRule<'_>] = &[rule(HTML_COLOR)];
    const UNSCOPED_RULE: &[HighlightRule<'_>] = &[rule(UNSCOPED)];
    const WRONG_RULE: &[HighlightRule<'_>] = &[rule(WRONG)];

    let body = format!("<style>\r\n{CSS}\r\n</style>\r\n{HTML}");
    let draft = body.replacen('🐱', "X", 1);
    let source = format!(
        "```html\r\n{draft}\r\n```\r\n\r\n```html\r\n{body}\r\n```\r\n\r\nProse {STRING}\r\n"
    );
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source, window, cx));
    cx.simulate_resize(size(px(800.), px(700.)));
    let config = Compartment::new();
    let (source, body_start, marked, selection, history) = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let start = activate_code(editor, &draft, cx);
            let mark = editor.source().find('X').unwrap();
            editor.set_selection(mark, mark + 1);
            editor.replace_and_mark_text_in_range(None, "🐱", None, window, cx);
            assert_eq!(editor.marked, Some(mark..mark + '🐱'.len_utf8()));
            configure(editor, config.of(Extension::default()).append(), cx);
            (
                editor.source().to_owned(),
                start,
                editor.marked.clone().unwrap(),
                editor.view.state().selection().clone(),
                editor.view.undo_depth(),
            )
        })
    });
    assert_eq!(
        source,
        format!(
            "```html\r\n{body}\r\n```\r\n\r\n```html\r\n{body}\r\n```\r\n\r\nProse {STRING}\r\n"
        )
    );

    let supported = NativeLanguage::Html
        .support()
        .parse(gpui_editor_codemirror::Text::new(&body))
        .unwrap();
    let css = body.find(CSS).unwrap();
    let html = body.find(HTML).unwrap();
    let css_token = CSS.find(STRING).unwrap();
    let html_token = HTML.find(STRING).unwrap();
    let spans = supported.highlights(0..body.len()).unwrap();
    for (offset, language) in [
        (css + css_token, NativeLanguage::Css),
        (html + html_token, NativeLanguage::Html),
    ] {
        let range = offset..offset + STRING.len();
        assert!(spans.iter().any(|span| span.range == range));
        assert_eq!(supported.highlight_scope(&range), Some(language));
    }
    assert_eq!(supported.highlight_scope(&(0..body.len())), None);
    assert_eq!(
        supported.highlight_scope(&(marked.start - body_start + 1..marked.end - body_start)),
        None
    );

    let css_layer = highlight_layer(HighlightLayer::Active(
        HighlightStyle::define(CSS_RULE).with_native_scope(NativeLanguage::Css),
    ));
    let html_layer = highlight_layer(HighlightLayer::Active(
        HighlightStyle::define(HTML_RULE).with_native_scope(NativeLanguage::Html),
    ));
    let unscoped = highlight_layer(HighlightLayer::Active(HighlightStyle::define(
        UNSCOPED_RULE,
    )));
    let wrong = highlight_layer(HighlightLayer::Active(
        HighlightStyle::define(WRONG_RULE).with_native_scope(NativeLanguage::Json),
    ));

    for (extension, css_color, html_color) in [
        (
            Extension::group([wrong, css_layer.clone(), html_layer, unscoped.clone()]),
            CSS_COLOR,
            HTML_COLOR,
        ),
        (
            Extension::group([css_layer, unscoped.clone()]),
            CSS_COLOR,
            UNSCOPED,
        ),
        (unscoped, UNSCOPED, UNSCOPED),
        (Extension::default(), FOREGROUND, FOREGROUND),
    ] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, config.reconfigure(extension), cx)
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            let mut items = layout.rows.iter().flat_map(|row| &row.items);
            let mut draft_line = |offset, text: &str| {
                let start = body_start + offset;
                items
                    .find_map(|item| match &item.paint {
                        ItemPaint::Text(line) if item.source.start == start => {
                            assert_eq!(item.source, start..start + text.len());
                            assert_eq!(line.text.as_ref(), text);
                            Some(line)
                        }
                        _ => None,
                    })
                    .unwrap()
            };
            let draft_css = draft_line(css, CSS);
            let draft_html = draft_line(html, HTML);
            let preview = items
                .find_map(|item| match &item.paint {
                    ItemPaint::Block(block) => Some(block.code_lines().collect::<Vec<_>>()),
                    _ => None,
                })
                .unwrap();
            assert_eq!(preview[1].text.as_ref(), CSS);
            assert_eq!(preview[3].text.as_ref(), HTML);
            let marked_local = marked.start - body_start - css..marked.end - body_start - css;
            let css_runs = painted_runs(draft_css, rgb(css_color));
            assert!(css_runs.contains(&(marked_local.clone(), true)));
            if css_color != FOREGROUND {
                assert_eq!(
                    css_runs,
                    [
                        (css_token..marked_local.start, false),
                        (marked_local.clone(), true),
                        (marked_local.end..css_token + STRING.len(), false),
                    ]
                );
                assert_eq!(
                    painted_runs(preview[1], rgb(css_color)),
                    [(css_token..css_token + STRING.len(), false)]
                );
                for line in [draft_html, preview[3]] {
                    assert_eq!(
                        painted_runs(line, rgb(html_color)),
                        [(html_token..html_token + STRING.len(), false)]
                    );
                }
            } else {
                assert_eq!(
                    painted_runs(preview[1], rgb(0x14683f)),
                    [(css_token..css_token + STRING.len(), false)]
                );
                for line in [draft_css, draft_html, preview[1], preview[3]] {
                    for color in [CSS_COLOR, HTML_COLOR, UNSCOPED] {
                        assert!(painted_runs(line, rgb(color)).is_empty());
                    }
                }
            }
            for line in [draft_css, draft_html, preview[1], preview[3]] {
                assert!(painted_runs(line, rgb(WRONG)).is_empty());
            }
            assert_eq!(editor.source(), source);
            assert_eq!(editor.marked.as_ref(), Some(&marked));
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.undo_depth(), history);
        });
    }
}
