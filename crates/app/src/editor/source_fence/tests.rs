use gpui::{TestAppContext, px, rgb, size};
use gpui_editor_codemirror::{
    Compartment, Extension,
    language::{
        Color, HighlightLayer, HighlightProperties, HighlightRule, HighlightSelector,
        HighlightStyle,
        markdown::{
            MarkdownCodeLanguages, MarkdownConfig, MarkdownParserConfig, markdown_snapshot,
            markdown_with_parser,
        },
        wast::NodeProp,
    },
    language_data::{NativeSyntax, WastLanguage, wast_language},
    theme_one_dark::highlight_layer,
};

use super::*;
use crate::{
    document::Section,
    editor::{
        bind_keys,
        layout::ItemPaint,
        one_dark_tests::{activate_code, configure, painted_runs},
    },
};

mod deferred;
mod indent;
mod lr;

fn markdown_parser(named: WastLanguage, fallback: Option<WastLanguage>) -> Extension {
    markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Configured {
                resolve: Arc::new(move |name| {
                    (name == "wat").then(|| MarkdownCodeParser::Wast(named.clone()))
                }),
                fallback: fallback.map(MarkdownCodeParser::Wast),
            }),
            ..Default::default()
        },
    )
}

const fn keyword_rule(color: u32) -> HighlightRule<'static> {
    const KEYWORD: &[HighlightSelector<'_>] = &[HighlightSelector::new("keyword")];
    HighlightRule::new(
        KEYWORD,
        HighlightProperties {
            foreground: Some(Color::from_rgba32((color << 8) | 0xff)),
            bold: false,
            italic: false,
            text_decoration: None,
        },
    )
}

fn configured_root(rules: &'static [HighlightRule<'static>]) -> (WastLanguage, Extension) {
    let property = NodeProp::new();
    let parser =
        wast_language().with_node_props(&[property.add(|row| row.is_top().then_some(true))]);
    let root = parser.node_types().find(|row| row.is_top()).unwrap();
    let layer = highlight_layer(HighlightLayer::Active(
        HighlightStyle::define(rules).with_wast_node_type_scope(root),
    ));
    (parser, layer)
}

#[gpui::test]
fn configured_fences_repaint_draft_and_fallback_preview_on_same_text_swap(cx: &mut TestAppContext) {
    const BODY: &str = "(func $a)";
    const FALLBACK: &str = "(func $b)";
    const FIRST: u32 = 0xa12843;
    const NEXT: u32 = 0x1068a4;
    const DEFAULT: u32 = 0x3a8457;
    let source =
        format!("```wat\r\n{BODY}\r\n```\r\n\r\n```\r\n{FALLBACK}\r\n```\r\n\r\nProse (func)\r\n");
    const FIRST_RULE: &[HighlightRule<'_>] = &[keyword_rule(FIRST)];
    const NEXT_RULE: &[HighlightRule<'_>] = &[keyword_rule(NEXT)];
    const DEFAULT_RULE: &[HighlightRule<'_>] = &[keyword_rule(DEFAULT)];
    let (first, first_layer) = configured_root(FIRST_RULE);
    let (next, next_layer) = configured_root(NEXT_RULE);
    let (fallback, fallback_layer) = configured_root(DEFAULT_RULE);
    assert_eq!(first.name(), next.name());
    let config = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.clone(),
            Extension::group([
                config.of(markdown_parser(first.clone(), Some(fallback.clone()))),
                first_layer,
                next_layer,
                fallback_layer,
            ]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(750.), px(660.)));
    let (at, selection, before) = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let at = activate_code(editor, BODY, cx);
            let state = editor.view.state();
            (
                at,
                state.selection().clone(),
                markdown_snapshot(state).unwrap(),
            )
        })
    });
    let mut old_context = None;
    for (parser, color, absent) in [(first, FIRST, NEXT), (next, NEXT, FIRST)] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let old_fence = editor.source_fence().unwrap();
                let new = MarkdownCodeParser::Wast(parser.clone());
                let settings = StreamSettings::default();
                if old_fence.parser.as_ref() != Some(&new) {
                    configure(
                        editor,
                        config.reconfigure(markdown_parser(parser.clone(), Some(fallback.clone()))),
                        cx,
                    );
                    assert!(
                        editor
                            .current_source_document(&old_fence.body, &new, settings)
                            .is_none()
                    );
                }
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let fence = editor.source_fence().unwrap();
            let selected = fence.parser.unwrap();
            assert!(selected == MarkdownCodeParser::Wast(parser.clone()));
            let document = editor
                .current_source_document(&fence.body, &selected, StreamSettings::default())
                .unwrap();
            let NativeSyntax::Wast(wast) = document.syntax() else {
                panic!("expected WAST body syntax")
            };
            assert!(std::ptr::eq(
                wast.root().node_type(),
                parser.node_types().find(|row| row.is_top()).unwrap()
            ));
            assert_eq!(document.text().as_str(), BODY);
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
                    ItemPaint::Block(block) => block
                        .code_lines()
                        .find(|line| line.text.as_ref() == FALLBACK),
                    _ => None,
                })
                .unwrap();
            assert_eq!(painted_runs(draft, rgb(color)), [(1..5, false)]);
            assert!(painted_runs(draft, rgb(absent)).is_empty());
            assert_eq!(painted_runs(preview, rgb(DEFAULT)), [(1..5, false)]);
            let current = markdown_snapshot(editor.view.state()).unwrap();
            let context = current.highlight_context(&(at + 1..at + 5)).unwrap();
            if let Some(old) = old_context {
                assert_ne!(old, context);
                assert_eq!(before.highlight_context(&(at + 1..at + 5)), Some(old));
            } else {
                assert!(Arc::ptr_eq(&before, &current));
                old_context = Some(context);
            }
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.revision(), 0);
            assert_eq!(editor.view.undo_depth(), 0);
        });
    }
}

fn host_case(name: &str) -> serde_json::Value {
    include_str!("../../../../codemirror/tests/fixtures/wast_host_commands.jsonl")
        .lines()
        .map(|row| serde_json::from_str::<serde_json::Value>(row).unwrap())
        .find(|row| row["name"] == name)
        .unwrap()
}

fn fixture_range(value: &serde_json::Value) -> Range<usize> {
    let bound = |i: usize| usize::try_from(value[i].as_u64().unwrap()).unwrap();
    bound(0)..bound(1)
}

fn activate_host_fence(editor: &mut Editor, body: &str, cx: &mut Context<Editor>) {
    let at = editor.source().find(body).unwrap();
    let fence = editor
        .markdown()
        .sections()
        .into_iter()
        .find(|section| section.source.contains(&at))
        .unwrap()
        .source;
    editor.activate(
        Section::new(SectionKind::DraftCode, fence.clone(), fence),
        false,
        cx,
    );
}

#[gpui::test]
fn configured_wast_fold_uses_host_coordinates_for_gutter_and_command(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::language::{
        fold::{FoldGutterConfig, FoldGutterState, fold_gutter},
        wast::{FOLD, WastFoldCallback},
    };
    use std::sync::Mutex;
    let pinned = host_case("fold");
    let disabled = host_case("disabled fold");
    let source = pinned["source"].as_str().unwrap();
    let body = pinned["body"].as_str().unwrap();
    let head = usize::try_from(pinned["selection"].as_u64().unwrap()).unwrap();
    let line = fixture_range(&pinned["line"]);
    let expected = fixture_range(&pinned["range"]);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let original = source.to_owned();
    let callback: WastFoldCallback = Arc::new(move |node, state| {
        assert_eq!(state.text().as_str(), original);
        let range = node.range();
        assert_eq!(state.text().slice(range.start..range.start + 1), Some("("));
        assert_eq!(node.children().next().unwrap().range().start, range.start);
        seen.lock()
            .unwrap()
            .push([range.start, range.end, state.selection().main().head()]);
        Some(range.start + 2..range.end - 1)
    });
    let configured = wast_language()
        .with_node_props(&[FOLD.add(move |row| row.is("App").then(|| Some(callback.clone())))]);
    let no_fold = wast_language().with_node_props(&[FOLD.add(|row| row.is("App").then_some(None))]);
    let parser = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.into(),
            Extension::group([
                parser.of(markdown_parser(configured, None)),
                fold_gutter(FoldGutterConfig::default()),
            ]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            activate_host_fence(editor, body, cx);
            editor.set_selection(head, head);
            let scope = editor.source_fence().unwrap().body;
            let gutter = FoldGutterConfig::configured(editor.view.state()).unwrap();
            let marker = editor
                .source_fold_query(&scope)
                .unwrap()
                .marker(&gutter, line.clone())
                .unwrap();
            assert_eq!(marker.state, FoldGutterState::Open);
            assert_eq!(
                serde_json::json!(std::mem::take(&mut *calls.lock().unwrap())),
                pinned["calls"]
            );
            editor.click_source_fold_gutter(&scope, line.clone(), &gutter, cx);
            assert_eq!(editor.source_fold_ranges(), std::slice::from_ref(&expected));
            assert_eq!(editor.cursor(), head);
            assert!(editor.toggle_source_fold(cx));
            assert!(editor.source_fold_ranges().is_empty());
            assert!(editor.toggle_source_fold(cx));
            assert_eq!(editor.source_fold_ranges(), [expected]);
            assert!(editor.toggle_source_fold(cx));
            configure(
                editor,
                parser.reconfigure(markdown_parser(no_fold, None)),
                cx,
            );
            assert!(disabled["range"].is_null());
            assert!(
                editor
                    .source_fold_query(&scope)
                    .unwrap()
                    .marker(&gutter, line)
                    .is_none()
            );
            assert!(!editor.toggle_source_fold(cx));
            assert_eq!(editor.cursor(), head);
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.undo_depth(), 0);
        })
    });
}

#[gpui::test]
fn configured_wast_comments_follow_live_mounted_tokens_and_undo(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::{commands::CommentTokens, language::LanguageData};
    let pinned = host_case("comment override");
    let disabled = host_case("disabled comment");
    let source = pinned["source"].as_str().unwrap();
    let body = pinned["body"].as_str().unwrap();
    let head = usize::try_from(pinned["selection"].as_u64().unwrap()).unwrap();
    let language = wast_language().clone();
    let tokens = |line| {
        language.data(
            LanguageData::default().with("commentTokens", CommentTokens { line, block: None }),
        )
    };
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.into(),
            Extension::group([
                markdown_parser(language.clone(), None),
                slot.of(tokens(Some(";; host"))),
            ]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_host_fence(editor, body, cx);
            editor.set_selection(head, head);
            window.focus(&editor.focus);
            assert!(editor.toggle_source_comment(cx));
            assert_eq!(editor.source(), pinned["text"].as_str().unwrap());
            assert_eq!(editor.view.undo_depth(), 1);
        });
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), source);
            assert_eq!(editor.cursor(), head);
            configure(editor, slot.reconfigure(tokens(None)), cx);
            assert!(disabled["text"].is_null());
            assert!(!editor.toggle_source_comment(cx));
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.undo_depth(), 0);
            assert!(editor.view.can_redo());
        })
    });
}

#[gpui::test]
fn strict_configured_fence_rejects_invalid_edit_without_changes_or_history(
    cx: &mut TestAppContext,
) {
    const BODY: &str = "(module ;; 字\r  (func $a))";
    let source = format!("```wat\r\n{BODY}\r\n```\r\n\r\nProse.");
    let strict = wast_language().configure(Some(true), None);
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source.clone(), markdown_parser(strict, None), window, cx)
            .unwrap()
    });
    let name = source.find("$a").unwrap() + 1;
    let closing = source.find("))").unwrap();
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, BODY, cx);
            editor.set_selection(name, name);
            window.focus(&editor.focus);
            let before = markdown_snapshot(editor.view.state()).unwrap();
            let selection = editor.view.state().selection().clone();
            assert!(!editor.replace(closing..closing + 1, "", cx));
            assert!(Arc::ptr_eq(
                &before,
                &markdown_snapshot(editor.view.state()).unwrap()
            ));
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.revision(), 0);
            assert_eq!(editor.view.undo_depth(), 0);
            assert!(editor.replace(name..name + 1, "b", cx));
            assert_eq!(editor.source(), source.replace("$a", "$b"));
            assert_eq!(editor.view.undo_depth(), 1);
        });
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.view.undo_depth(), 0);
        assert!(editor.view.can_redo());
    });
    cx.simulate_keystrokes("secondary-shift-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source.replace("$a", "$b"));
        assert_eq!(editor.view.undo_depth(), 1);
    });
}

#[gpui::test]
fn strict_wast_rejects_undo_without_consuming_the_entry(cx: &mut TestAppContext) {
    let body = "(func $a";
    let source = format!("```wat\r\n{body}\r\n```\r\n");
    let permissive = wast_language().clone();
    let strict = permissive.configure(Some(true), None);
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.clone(),
            slot.of(markdown_parser(permissive.clone(), None)),
            window,
            cx,
        )
        .unwrap()
    });
    let (snapshot, selection, revision) = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            let start = activate_code(editor, body, cx);
            let end = start + body.len();
            window.focus(&editor.focus);
            editor.set_selection(end, end);
            assert!(editor.replace(end..end, ")", cx));
            configure(editor, slot.reconfigure(markdown_parser(strict, None)), cx);
            assert_eq!(editor.view.undo_depth(), 1);
            (
                markdown_snapshot(editor.view.state()).unwrap(),
                editor.view.state().selection().clone(),
                editor.view.revision(),
            )
        })
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), source.replace(body, "(func $a)"));
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.revision(), revision);
            assert!(Arc::ptr_eq(
                &snapshot,
                &markdown_snapshot(editor.view.state()).unwrap()
            ));
            assert_eq!(editor.view.undo_depth(), 1);
            assert!(!editor.view.can_redo());
            configure(
                editor,
                slot.reconfigure(markdown_parser(permissive, None)),
                cx,
            );
        })
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.view.undo_depth(), 0);
        assert!(editor.view.can_redo());
    });
}
