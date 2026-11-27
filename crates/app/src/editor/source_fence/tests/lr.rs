use gpui_editor_codemirror::{
    language::{
        HighlightTag, TextDecoration,
        brackets::BracketMatch,
        lezer::{CLOSED_BY, LANGUAGE_DATA, LanguageFacet, style_tags},
    },
    language_data::{json_language, lezer_language},
};

use super::*;

fn install(parser: MarkdownCodeParser, named: bool) -> Extension {
    let fallback = (!named).then(|| parser.clone());
    markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Configured {
                resolve: Arc::new(move |label| {
                    (named && label == "example").then(|| parser.clone())
                }),
                fallback,
            }),
            ..Default::default()
        },
    )
}

#[gpui::test]
fn configured_lr_draft_rebuilds_on_a_same_text_swap_and_rejects_invalid_edits(
    cx: &mut TestAppContext,
) {
    for (body, loose, strict, named) in [
        (
            "{\"😀\":\"old\"}",
            MarkdownCodeParser::Json(json_language().configure(Some(false), Some("custom"))),
            MarkdownCodeParser::Json(json_language().configure(Some(true), Some("custom"))),
            true,
        ),
        (
            "@top Main { \"😀old\" }",
            MarkdownCodeParser::Lezer(lezer_language().configure(Some(false), Some("custom"))),
            MarkdownCodeParser::Lezer(lezer_language().configure(Some(true), Some("custom"))),
            false,
        ),
    ] {
        let label = if named { "example opts" } else { "" };
        let source = format!("```{label}\r\n{body}\r\n```\r\n");
        let edited = source.replace("old", "new");
        let config = Compartment::new();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                config.of(install(loose.clone(), named)),
                window,
                cx,
            )
            .unwrap()
        });
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_code(editor, body, cx);
                window.focus(&editor.focus);
                let change = source.find("old").unwrap();
                editor.set_selection(change, change);
                let selection = editor.view.state().selection().clone();
                let old = markdown_snapshot(editor.view.state()).unwrap();
                let fence = editor.source_fence().unwrap();
                let scope = fence.body;
                let settings = StreamSettings::default();
                assert!(fence.parser.as_ref() == Some(&loose));
                assert_eq!(
                    editor
                        .source_document(&scope, &loose, settings)
                        .unwrap()
                        .text()
                        .as_str(),
                    body
                );

                configure(
                    editor,
                    config.reconfigure(install(strict.clone(), named)),
                    cx,
                );
                assert!(editor.source_fence().unwrap().parser.as_ref() == Some(&strict));
                assert!(
                    editor
                        .current_source_document(&scope, &strict, settings)
                        .is_none()
                );
                assert_eq!(
                    editor
                        .source_document(&scope, &strict, settings)
                        .unwrap()
                        .text()
                        .as_str(),
                    body
                );
                let current = markdown_snapshot(editor.view.state()).unwrap();
                assert!(!Arc::ptr_eq(&old, &current));
                let close = source.rfind('}').unwrap();
                assert!(!editor.replace(close..close + 1, "", cx));
                assert!(Arc::ptr_eq(
                    &current,
                    &markdown_snapshot(editor.view.state()).unwrap()
                ));
                assert_eq!(editor.source(), source);
                assert_eq!(editor.view.state().selection(), &selection);
                assert_eq!((editor.view.revision(), editor.view.undo_depth()), (0, 0));
                assert!(editor.replace(change..change + 3, "new", cx));
                assert_eq!(editor.source(), edited);
                assert_eq!(editor.view.undo_depth(), 1);
            });
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert_eq!((editor.view.undo_depth(), editor.view.redo_depth()), (0, 1));
        });
        cx.simulate_keystrokes("secondary-shift-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), edited);
            assert_eq!((editor.view.undo_depth(), editor.view.redo_depth()), (1, 0));
        });
    }
}

#[gpui::test]
fn configured_lr_roles_refresh_bracket_marks_in_editable_fences(cx: &mut TestAppContext) {
    let json = json_language();
    let lezer = lezer_language();
    let marks = |from, to: &str| {
        let to = to.to_owned();
        CLOSED_BY.add(move |row| row.is(from).then(|| vec![to.clone()]))
    };
    for (body, start, end, stock, configured, named) in [
        (
            "{\"😀\": [ 12, \"字\" ]}",
            "12",
            "\"字\"",
            MarkdownCodeParser::Json(json.clone()),
            MarkdownCodeParser::Json(json.with_node_props(&[marks("Number", "String")])),
            true,
        ),
        (
            "@top Main { word \"😀\" }",
            "word",
            "\"😀\"",
            MarkdownCodeParser::Lezer(lezer.clone()),
            MarkdownCodeParser::Lezer(lezer.with_node_props(&[marks("RuleName", "Literal")])),
            false,
        ),
    ] {
        let label = if named { "example" } else { "" };
        let source = format!("前 😀\r\n\r\n```{label}\r\n{body}\r\n```\r\n");
        let start_at = source.find(start).unwrap();
        let end_at = source.find(end).unwrap();
        let expected = BracketMatch {
            start: start_at..start_at + start.len(),
            end: Some(end_at..end_at + end.len()),
            matched: true,
        };
        let config = Compartment::new();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                config.of(install(stock.clone(), named)),
                window,
                cx,
            )
            .unwrap()
        });
        let (scope, selection) = cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                activate_code(editor, body, cx);
                window.focus(&editor.focus);
                editor.set_selection(start_at + 1, start_at + 1);
                assert!(editor.source_bracket_match().is_none());
                (
                    editor.source_fence_with_unlabeled(true).unwrap().body,
                    editor.view.state().selection().clone(),
                )
            })
        });
        for (parser, hit, paint_count) in [(configured, Some(expected), 2), (stock, None, 0)] {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    configure(editor, config.reconfigure(install(parser, named)), cx);
                    assert_eq!(editor.source_bracket_match(), hit);
                });
            });
            cx.run_until_parked();
            cx.update(|_, cx| {
                let editor = editor.read(cx);
                let painted = &editor.layout.as_ref().unwrap().painted_bracket_quads;
                assert_eq!(painted.len(), paint_count);
                assert_eq!(
                    editor.source_fence_with_unlabeled(true).unwrap().body,
                    scope
                );
                assert_eq!(editor.source(), source);
                assert_eq!(editor.view.state().selection(), &selection);
                assert_eq!((editor.view.revision(), editor.view.undo_depth()), (0, 0));
            });
        }
    }
}

#[gpui::test]
fn configured_lr_fold_rules_drive_the_gutter_and_keyboard(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::language::{
        fold::{FoldGutterConfig, FoldGutterState, fold_gutter},
        lezer::{FOLD, LrFoldCallback},
    };

    for (body, row) in [
        ("[ \r\n  \"😀\",\r 1\r\n]", "Array"),
        ("@top Main { \r\n  \"😀\"\r  \"x\"\r\n}", "Body"),
    ] {
        let source = format!("前 😀\r\n\r\n```example\r\n{body}\r\n```\r\n");
        let origin = source.find(body).unwrap();
        let line = origin..origin + body.find('\r').unwrap();
        let opener = origin + body.find(['[', '{']).unwrap();
        let end = origin + body.len() - 1;
        let expected = opener + 2..end;
        let callback: LrFoldCallback = Arc::new({
            let source = source.clone();
            move |node, state| {
                assert_eq!(state.text().as_str(), source);
                assert_eq!(state.selection().main().head(), opener);
                assert_eq!(node.range(), opener..end + 1);
                assert_eq!(node.children().next().unwrap().range(), opener..opener + 1);
                Some(opener + 2..end)
            }
        });
        let rule = FOLD.add(move |node| node.is(row).then(|| Some(callback.clone())));
        let disable = FOLD.add(move |node| node.is(row).then_some(None));
        let (parser, disabled) = if row == "Array" {
            let stock = json_language();
            (
                MarkdownCodeParser::Json(stock.with_node_props(&[rule])),
                MarkdownCodeParser::Json(stock.with_node_props(&[disable])),
            )
        } else {
            let stock = lezer_language();
            (
                MarkdownCodeParser::Lezer(stock.with_node_props(&[rule])),
                MarkdownCodeParser::Lezer(stock.with_node_props(&[disable])),
            )
        };
        let slot = Compartment::new();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                Extension::group([
                    slot.of(install(parser, true)),
                    fold_gutter(FoldGutterConfig::default()),
                ]),
                window,
                cx,
            )
            .unwrap()
        });
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                activate_code(editor, body, cx);
                editor.set_selection(opener, opener);
                let body = editor.source_fence().unwrap().body;
                let gutter = FoldGutterConfig::configured(editor.view.state()).unwrap();
                let marker = editor
                    .source_fold_query(&body)
                    .unwrap()
                    .marker(&gutter, line.clone());
                assert_eq!(marker.unwrap().state, FoldGutterState::Open);
                editor.click_source_fold_gutter(&body, line.clone(), &gutter, cx);
                assert_eq!(editor.source_fold_ranges(), std::slice::from_ref(&expected));
                assert!(editor.toggle_source_fold(cx));
                assert!(editor.source_fold_ranges().is_empty());
                assert!(editor.toggle_source_fold(cx));
                assert_eq!(editor.source_fold_ranges(), [expected]);
                assert!(editor.toggle_source_fold(cx));

                configure(editor, slot.reconfigure(install(disabled, true)), cx);
                assert!(
                    editor
                        .source_fold_query(&body)
                        .unwrap()
                        .marker(&gutter, line)
                        .is_none()
                );
                assert!(!editor.toggle_source_fold(cx));
                assert_eq!(editor.source(), source);
                assert_eq!(editor.cursor(), opener);
                assert_eq!((editor.view.revision(), editor.view.undo_depth()), (0, 0));
            });
        });
    }
}

#[gpui::test]
fn configured_lr_root_scopes_repaint_drafts_and_previews(cx: &mut TestAppContext) {
    const BEFORE: u32 = 0x89332f;
    const EXACT: u32 = 0x4267a3;
    const FACET: u32 = 0x047950;
    const GAP: u32 = 0x8e5093;
    const fn rule(color: u32) -> HighlightRule<'static> {
        const TAGS: &[HighlightSelector<'_>] = &[HighlightSelector::new("string")];
        HighlightRule::new(
            TAGS,
            HighlightProperties {
                foreground: Some(Color::from_rgba32((color << 8) | 0xff)),
                bold: false,
                italic: false,
                text_decoration: None,
            },
        )
    }
    const STOCK_RULE: &[HighlightRule<'static>] = &[rule(BEFORE)];
    const EXACT_RULE: &[HighlightRule<'static>] = &[rule(EXACT)];
    const FACET_RULE: &[HighlightRule<'static>] = &[rule(FACET)];
    let facet = LanguageFacet::new(None);
    let prop = NodeProp::new();
    let sources = [prop.add(|row| row.is_top().then_some(true))];
    let data = [LANGUAGE_DATA.add({
        let facet = facet.clone();
        move |row| row.is_top().then(|| Some(facet.clone()))
    })];
    let json = json_language();
    let lezer = lezer_language();
    let json_next = json.with_node_props(&sources);
    let lezer_next = lezer.with_node_props(&sources);
    for (body, stock, next, replaced, stock_row, next_row) in [
        (
            "{\"😀\":  \"字\"}",
            MarkdownCodeParser::Json(json.clone()),
            MarkdownCodeParser::Json(json_next.clone()),
            MarkdownCodeParser::Json(json.with_node_props(&data)),
            json.node_types().find(|row| row.is_top()).unwrap(),
            json_next.node_types().find(|row| row.is_top()).unwrap(),
        ),
        (
            "@top Main {  \"字\" }",
            MarkdownCodeParser::Lezer(lezer.clone()),
            MarkdownCodeParser::Lezer(lezer_next.clone()),
            MarkdownCodeParser::Lezer(lezer.with_node_props(&data)),
            lezer.node_types().find(|row| row.is_top()).unwrap(),
            lezer_next.node_types().find(|row| row.is_top()).unwrap(),
        ),
    ] {
        let source = format!(
            "```example\r\n{body}\r\n```\r\n\r\n```example\r\n{body}\r\n```\r\n\r\nProse\r\n"
        );
        let config = Compartment::new();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                Extension::group([
                    config.of(install(stock, true)),
                    highlight_layer(HighlightLayer::Active(
                        HighlightStyle::define(STOCK_RULE).with_node_type_scope(stock_row),
                    )),
                    highlight_layer(HighlightLayer::Active(
                        HighlightStyle::define(EXACT_RULE).with_node_type_scope(next_row),
                    )),
                    highlight_layer(HighlightLayer::Active(
                        HighlightStyle::define(FACET_RULE).with_data_facet_scope(&facet),
                    )),
                    highlight_layer(HighlightLayer::Active(
                        HighlightStyle::define(&[])
                            .with_node_type_scope(next_row)
                            .with_all(HighlightProperties {
                                foreground: Some(Color::from_rgba32((GAP << 8) | 0xff)),
                                text_decoration: Some(TextDecoration::Underline),
                                ..Default::default()
                            }),
                    )),
                ]),
                window,
                cx,
            )
            .unwrap()
        });
        cx.simulate_resize(size(px(750.), px(660.)));
        let (at, selection) = cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let at = activate_code(editor, body, cx);
                (at, editor.view.state().selection().clone())
            })
        });
        for (color, parser) in [(BEFORE, None), (EXACT, Some(next)), (FACET, Some(replaced))] {
            if let Some(parser) = parser {
                cx.update(|_, cx| {
                    editor.update(cx, |editor, cx| {
                        configure(editor, config.reconfigure(install(parser, true)), cx)
                    })
                });
            }
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
                let literal = body.find("\"字\"").unwrap();
                for line in [draft, preview] {
                    assert_eq!(line.text.as_ref(), body);
                    assert_eq!(
                        painted_runs(line, rgb(color)),
                        [(literal..literal + "\"字\"".len(), color == EXACT)]
                    );
                    let gap = painted_runs(line, rgb(GAP));
                    assert_eq!(
                        gap.iter()
                            .any(|(range, underlined)| range.start <= literal - 2
                                && range.end >= literal
                                && *underlined),
                        color == EXACT
                    );
                    for other in [BEFORE, EXACT, FACET]
                        .into_iter()
                        .filter(|&other| other != color)
                    {
                        assert!(painted_runs(line, rgb(other)).is_empty());
                    }
                }
                let prose = items()
                    .find_map(|item| match &item.paint {
                        ItemPaint::Text(line) if line.text.starts_with("Prose") => Some(line),
                        _ => None,
                    })
                    .unwrap();
                for color in [BEFORE, EXACT, FACET, GAP] {
                    assert!(painted_runs(prose, rgb(color)).is_empty());
                }
                assert_eq!(editor.source(), source);
                assert_eq!(editor.view.state().selection(), &selection);
                assert_eq!((editor.view.revision(), editor.view.undo_depth()), (0, 0));
            });
        }
    }
}

#[gpui::test]
fn configured_lr_style_tags_repaint_drafts_and_previews(cx: &mut TestAppContext) {
    const COLOR: u32 = 0xb22458;
    const RULES: &[HighlightRule<'static>] = &[
        HighlightRule::new(
            &[HighlightSelector::new("parent-decoration")],
            HighlightProperties {
                foreground: None,
                bold: false,
                italic: false,
                text_decoration: Some(TextDecoration::Underline),
            },
        ),
        HighlightRule::new(
            &[HighlightSelector::new("literal-color")],
            HighlightProperties {
                foreground: Some(Color::from_rgba32((COLOR << 8) | 0xff)),
                bold: false,
                italic: false,
                text_decoration: None,
            },
        ),
    ];
    let paths = style_tags([
        (
            "JsonText/... Grammar/...",
            vec![HighlightTag::new("parent-decoration")],
        ),
        ("String Literal", vec![HighlightTag::new("literal-color")]),
    ])
    .unwrap();
    let paths = [paths];
    for (body, stock, configured) in [
        (
            r#"{"😀":  "字"}"#,
            MarkdownCodeParser::Json(json_language().clone()),
            MarkdownCodeParser::Json(json_language().with_node_props(&paths)),
        ),
        (
            r#"@top Main {  "字" }"#,
            MarkdownCodeParser::Lezer(lezer_language().clone()),
            MarkdownCodeParser::Lezer(lezer_language().with_node_props(&paths)),
        ),
    ] {
        let source = format!("```example\r\n{body}\r\n```\r\n\r\n```example\r\n{body}\r\n```\r\n");
        let slot = Compartment::new();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                Extension::group([
                    slot.of(install(stock.clone(), true)),
                    highlight_layer(HighlightLayer::Active(HighlightStyle::define(RULES))),
                ]),
                window,
                cx,
            )
            .unwrap()
        });
        cx.simulate_resize(size(px(750.), px(660.)));
        let (at, selection) = cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let at = activate_code(editor, body, cx);
                (at, editor.view.state().selection().clone())
            })
        });
        for (parser, expects_color) in [(stock.clone(), false), (configured, true), (stock, false)]
        {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    configure(editor, slot.reconfigure(install(parser, true)), cx);
                })
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
                let literal = body.find("\"字\"").unwrap();
                for line in [draft, preview] {
                    assert_eq!(line.text.as_ref(), body);
                    let painted = painted_runs(line, rgb(COLOR));
                    if expects_color {
                        assert_eq!(painted, [(literal..literal + "\"字\"".len(), true)]);
                    } else {
                        assert!(painted.is_empty());
                    }
                }
                assert_eq!(editor.source(), source);
                assert_eq!(editor.view.state().selection(), &selection);
                assert_eq!((editor.view.revision(), editor.view.undo_depth()), (0, 0));
            });
        }
    }
}

#[gpui::test]
fn configured_json_fence_uses_syntax_for_quote_input(cx: &mut TestAppContext) {
    let body = "{\r\n  \"😀\": \"item\"\r}";
    let source = format!("```example\r\n{body}\r\n```\r\n");
    let at = source.find("\"item\"").unwrap();
    let parser = MarkdownCodeParser::Json(json_language().configure(None, Some("custom")));
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source.clone(), install(parser.clone(), true), window, cx)
            .unwrap()
    });
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, body, cx);
            editor.set_selection(at, at);
            window.focus(&editor.focus);
            assert!(editor.source_fence().unwrap().parser.as_ref() == Some(&parser));
        });
    });
    cx.simulate_input("\"");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(
            editor.source(),
            format!("{}\"\"{}", &source[..at], &source[at..])
        );
        assert_eq!(editor.selection(), at + 1..at + 1);
        assert_eq!(editor.view.undo_depth(), 1);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
}
