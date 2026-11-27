use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use gpui::{Bounds, Hsla, Modifiers, Pixels, Point, TestAppContext, TextLayout, rgb, rgba, size};
use gpui_editor_codemirror::{
    Compartment, Extension, StateEffect,
    language::{
        Color, HighlightLayer, HighlightProperties, HighlightRule, HighlightSelector,
        HighlightStyle, TextDecoration,
        markdown::{
            MarkdownCodeLanguages, MarkdownConfig, MarkdownParserConfig, markdown_with_parser,
        },
    },
    language_data::NativeLanguage,
    theme_one_dark::{COLOR, highlight_layer, one_dark, one_dark_highlighting, one_dark_theme},
};

use super::*;
use crate::editor::bind_keys;

type PaintedSides = [BTreeMap<usize, PaintedText>; 2];

thread_local! {
    static PAINTED: RefCell<PaintedSides> = const { RefCell::new([BTreeMap::new(), BTreeMap::new()]) };
}

#[derive(Debug, PartialEq)]
struct TextGeometry {
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    font_size: Pixels,
    width: Pixels,
    glyphs: Vec<(usize, Point<Pixels>)>,
}

struct PaintedText {
    text: String,
    geometry: TextGeometry,
    colors: Vec<(usize, Hsla)>,
}

impl PaintedText {
    fn colors_text(&self, token: &str, color: Hsla) {
        let start = self.text.find(token).unwrap();
        let range = start..start + token.len();
        let painted: Vec<_> = self
            .colors
            .iter()
            .filter(|(byte, _)| range.contains(byte))
            .copied()
            .collect();
        assert_eq!(
            painted,
            token
                .char_indices()
                .map(|(i, _)| (start + i, color))
                .collect::<Vec<_>>(),
            "{token:?} in {:?}",
            self.text
        );
    }
}

pub(super) fn record_paint(
    side: Side,
    number: usize,
    layout: &TextLayout,
    color_at: impl Fn(usize) -> Hsla,
) {
    let line = layout.line_layout_for_index(0).unwrap();
    assert!(line.wrap_boundaries().is_empty());
    let glyphs: Vec<_> = line
        .runs()
        .iter()
        .flat_map(|run| &run.glyphs)
        .map(|glyph| (glyph.index, glyph.position))
        .collect();
    let colors = glyphs
        .iter()
        .map(|&(byte, _)| (byte, color_at(byte)))
        .collect();
    PAINTED.with(|painted| {
        painted.borrow_mut()[usize::from(side == Side::B)].insert(
            number,
            PaintedText {
                text: layout.text(),
                geometry: TextGeometry {
                    bounds: layout.bounds(),
                    line_height: layout.line_height(),
                    width: line.width(),
                    font_size: line.font_size(),
                    glyphs,
                },
                colors,
            },
        );
    });
}

fn take_painted() -> PaintedSides {
    PAINTED.with(RefCell::take)
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

#[gpui::test]
fn review_reconfigures_syntax_and_view_without_moving_text_or_rows(cx: &mut TestAppContext) {
    let saved = "Text `const`\r\n\r\n~~~js opts\r\nconst café = \"字🐱\"; /* first\r\nsecond */\r\n\r\n~~~\r\n";
    let draft = saved.replace("café", "茶");
    let initial = draft.clone();
    let config = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(initial, config.of(Extension::default()), window, cx)
            .unwrap()
    });
    cx.simulate_resize(size(px(450.), px(380.)));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| editor.review_changes(saved, window, cx))
    });
    cx.run_until_parked();
    let baseline = take_painted();
    let cases = [
        (one_dark_theme(), true, false),
        (one_dark(), true, true),
        (one_dark_highlighting(), false, true),
        (Extension::default(), false, false),
    ];
    for (extension, dark, syntax) in cases {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, config.reconfigure(extension), cx)
            })
        });
        cx.run_until_parked();
        let sides = take_painted();
        let base = rgb(if dark { 0xabb2bf } else { 0x172336 }).into();
        let color = |value: Color| {
            if syntax {
                rgba(value.rgba32()).into()
            } else {
                base
            }
        };
        for (index, source) in [saved, &draft].into_iter().enumerate() {
            let painted = &sides[index];
            for (line, raw) in source.lines().enumerate() {
                let number = line + 1;
                assert_eq!(painted[&number].text, raw);
                assert_eq!(painted[&number].geometry, baseline[index][&number].geometry);
                assert_eq!(
                    painted[&number].geometry.bounds.top(),
                    sides[1 - index][&number].geometry.bounds.top()
                );
                if number > 1 {
                    assert_eq!(
                        painted[&number].geometry.bounds.top()
                            - painted[&(number - 1)].geometry.bounds.top(),
                        px(24.)
                    );
                }
            }
            painted[&1].colors_text("const", base);
            painted[&3].colors_text("~~~js opts", base);
            painted[&4].colors_text("const", color(COLOR.violet));
            painted[&4].colors_text("\"字🐱\"", color(COLOR.sage));
            painted[&4].colors_text("/* first", color(COLOR.stone));
            painted[&5].colors_text("second */", color(COLOR.stone));
            painted[&7].colors_text("~~~", base);
        }
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let review = editor.merge_review.as_ref().unwrap();
            assert_eq!(review.saved.as_str(), saved);
            assert_eq!(review.draft.as_str(), draft);
            assert_eq!(editor.source(), draft);
            assert_eq!(editor.view.revision(), 0);
            assert!(!editor.view.can_undo());
        });
    }
}

#[gpui::test]
fn review_uses_reconfigured_fence_parsers_and_native_scope_without_styling_prefixes(
    cx: &mut TestAppContext,
) {
    const RULES: &[HighlightRule<'_>] = &[HighlightRule::new(
        &[HighlightSelector::new("string")],
        HighlightProperties {
            foreground: Some(Color::from_rgba32(0xff2266ff)),
            bold: true,
            italic: true,
            text_decoration: Some(TextDecoration::Underline),
        },
    )];
    let layers = highlight_layer(HighlightLayer::Active(
        HighlightStyle::define(RULES).with_native_scope(NativeLanguage::Python),
    ));
    let parser = Compartment::new();
    let saved = "---\n```py\nname = \"front matter\"\n```\n---\n\n> ~~~personal opts\n> name = \"字\"\n> ~~~\n\n~~~unrecognized\nname = \"unknown\"\n~~~\n\n    name = \"indented\"\n";
    let draft = saved.replace('字', "🐱");
    let initial = draft.clone();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            initial,
            Extension::group([layers, parser.of(Extension::default())]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(650.), px(560.)));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.review_changes(saved, window, cx);
            let projection = &mut editor.merge_review.as_mut().unwrap().projection;
            let collapsed: Vec<_> = projection
                .rows(Side::A)
                .iter()
                .filter_map(|row| match row {
                    ProjectionRow::Collapsed { range, .. } => Some(range.start),
                    _ => None,
                })
                .collect();
            for start in collapsed {
                assert!(projection.expand(Side::A, start));
            }
        })
    });
    cx.run_until_parked();
    let before = take_painted();
    let mut configured = false;
    let resolutions = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&resolutions);
    for extension in [
        markdown_with_parser(
            MarkdownConfig::default(),
            MarkdownParserConfig {
                code_languages: Some(MarkdownCodeLanguages::Resolve(Arc::new(move |name| {
                    counter.fetch_add(1, Ordering::Relaxed);
                    (name == "personal").then_some(NativeLanguage::Python)
                }))),
                default_code_language: Some(NativeLanguage::Json),
                ..Default::default()
            },
        ),
        Extension::default(),
    ] {
        configured = !configured;
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, parser.reconfigure(extension), cx)
            })
        });
        cx.run_until_parked();
        let base = rgb(0x172336).into();
        let accent = rgb(0xff2266).into();
        let sides = take_painted();
        for (side, value) in ["\"字\"", "\"🐱\""].into_iter().enumerate() {
            let painted = &sides[side];
            let line = &painted[&8];
            assert_eq!(line.text, format!("> name = {value}"));
            assert_eq!(line.geometry, before[side][&8].geometry);
            line.colors_text("> name = ", base);
            line.colors_text(value, if configured { accent } else { base });
            painted[&3].colors_text("\"front matter\"", base);
            painted[&7].colors_text("~~~personal opts", base);
            painted[&9].colors_text("> ~~~", base);
            painted[&12].colors_text("\"unknown\"", base);
            painted[&15].colors_text("\"indented\"", base);
        }
        if configured {
            let resolved = resolutions.load(Ordering::Relaxed);
            assert!(resolved > 0);
            cx.update(|_, cx| editor.update(cx, |_, cx| cx.notify()));
            cx.run_until_parked();
            assert!(take_painted().iter().all(|side| side.contains_key(&8)));
            assert_eq!(resolutions.load(Ordering::Relaxed), resolved);
        }
    }
}

#[gpui::test]
fn review_uses_configured_wast_without_recovering_an_invalid_saved_fence(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::{
        language::{markdown::MarkdownCodeParser, wast::NodeProp},
        language_data::wast_language,
    };
    const PREFIX: &str = "```wat opts\r\n";
    const SAVED: &str = "(module (func $a)";
    const DRAFT: &str = "(module (func $b))";
    const ACCENT: u32 = 0xff2266;
    const RULES: &[HighlightRule<'_>] = &[HighlightRule::new(
        &[HighlightSelector::new("keyword")],
        HighlightProperties {
            foreground: Some(Color::from_rgba32((ACCENT << 8) | 0xff)),
            bold: true,
            italic: true,
            text_decoration: Some(TextDecoration::Underline),
        },
    )];
    let prop = NodeProp::new();
    let parser = wast_language()
        .with_node_props(&[prop.add(|row| row.is_top().then_some(true))])
        .configure(Some(true), None);
    let layer = highlight_layer(HighlightLayer::Active(
        HighlightStyle::define(RULES)
            .with_wast_node_type_scope(parser.node_types().find(|row| row.is_top()).unwrap()),
    ));
    let source_parser = markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Configured {
                resolve: Arc::new(move |name| {
                    (name == "wat").then(|| MarkdownCodeParser::Wast(parser.clone()))
                }),
                fallback: None,
            }),
            ..Default::default()
        },
    );
    let saved = format!("{PREFIX}{SAVED}\r\n```\r\n");
    let draft = format!("{PREFIX}{DRAFT}\r\n```\r\n");
    let theme = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            draft.clone(),
            Extension::group([source_parser, theme.of(Extension::default())]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(650.), px(420.)));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| editor.review_changes(&saved, window, cx))
    });
    cx.run_until_parked();
    let before = take_painted();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            configure(editor, theme.reconfigure(layer), cx)
        })
    });
    cx.run_until_parked();
    let after = take_painted();
    let line = |side: usize| {
        after[side]
            .values()
            .find(|line| line.text.starts_with("(module"))
            .unwrap()
    };
    let left = line(0);
    let right = line(1);
    left.colors_text("func", rgb(0x172336).into());
    right.colors_text("func", rgb(ACCENT).into());
    assert_eq!(left.text, SAVED);
    assert_eq!(right.text, DRAFT);
    for (side, after) in [left, right].into_iter().enumerate() {
        let before = before[side]
            .values()
            .find(|line| line.text == after.text)
            .unwrap();
        assert_eq!(before.geometry, after.geometry);
    }
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), draft);
        assert_eq!(editor.view.undo_depth(), 0);
        assert_eq!(editor.view.revision(), 0);
    });
}

#[gpui::test]
fn review_restores_a_single_chunk_and_undo_reopens_its_difference(cx: &mut TestAppContext) {
    let shared = "same\r\n".repeat(24);
    let saved = format!("before\r\n{shared}last\r\n");
    let draft = format!("字\r\n{shared}new last\r\n");
    let initial = draft.clone();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(initial, window, cx));
    cx.simulate_resize(size(px(850.), px(520.)));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| editor.review_changes(&saved, window, cx));
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("merge-review").is_some());
    let collapsed = cx.update(|_, cx| {
        let editor = editor.read(cx);
        let review = editor.merge_review.as_ref().unwrap();
        assert_eq!(review.chunks.len(), 2);
        assert!(!editor.view.can_undo());
        assert_eq!(editor.source(), draft);
        review
            .projection
            .rows(Side::A)
            .iter()
            .position(|row| matches!(row, ProjectionRow::Collapsed { .. }))
            .unwrap()
    });
    assert_eq!(collapsed, 4);
    let expander = cx.debug_bounds("merge-expand-A-4").unwrap().center();
    cx.simulate_click(expander, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let review = editor.merge_review.as_ref().unwrap();
        for side in [Side::A, Side::B] {
            assert!(
                review
                    .projection
                    .rows(side)
                    .iter()
                    .all(|row| !matches!(row, ProjectionRow::Collapsed { .. }))
            );
        }
        assert_eq!(editor.source(), draft);
        assert!(!editor.view.can_undo());
    });
    let next = cx.debug_bounds("merge-next").unwrap().center();
    cx.simulate_click(next, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(editor.read(cx).merge_review.as_ref().unwrap().current, 1));
    let restore = cx.debug_bounds("merge-restore").unwrap().center();
    cx.simulate_click(restore, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), format!("字\r\n{shared}last\r\n"));
        assert_eq!(editor.merge_review.as_ref().unwrap().chunks.len(), 1);
        assert!(editor.view.can_undo());
    });
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), draft);
        assert_eq!(editor.merge_review.as_ref().unwrap().chunks.len(), 2);
        assert!(!editor.view.can_undo());
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(!editor.is_reviewing_changes());
        assert!(editor.focus.is_focused(window));
    });
}

#[gpui::test]
fn review_rejects_stale_and_read_only_restores(cx: &mut TestAppContext) {
    let saved = "```js\r\nconst saved = 1;\r\n```\r\n";
    let draft = "```js\r\nconst draft = 2;\r\n```\r\n";
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.marked = Some(0..0);
            editor.review_changes(saved, window, cx);
            assert!(!editor.is_reviewing_changes());
            editor.finish_composition();
            editor.review_changes(saved, window, cx);
            let revision = editor.view.revision();
            editor.restore_review_chunk(revision, 0, cx);
            assert_eq!(editor.source(), draft);
            assert_eq!(
                editor.merge_review.as_ref().unwrap().message,
                Some("This change touches a read-only section.")
            );
            assert!(!editor.view.can_undo());
            let end = draft.len();
            assert!(editor.replace(end..end, "\r\nLater", cx));
            editor.restore_review_chunk(revision, 0, cx);
            assert_eq!(editor.source(), format!("{draft}\r\nLater"));
        });
    });
}
