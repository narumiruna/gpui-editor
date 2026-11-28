use std::{
    future::{Future, poll_fn},
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
    task::{Context as TaskContext, Poll, Waker},
};

use gpui::{Bounds, Pixels};
use gpui_editor_codemirror::{
    StateEffectType, TransactionExtension,
    language::markdown::{MarkdownCodeLoad, MarkdownCodeSelection},
    transaction_extender,
};

use super::*;

const ORIGINAL: &str = "[\"猫\"]";
const EDITED: &str = "[\"🐱\"]";
const PREVIEW: &str = "    \"値\"";
const FALLBACK: &str = "const x = \"steady\";";
const JSON_COLOR: u32 = 0x187ba3;
const JS_COLOR: u32 = 0x93283b;

static LOAD_DESCRIPTION: LanguageDescription = LanguageDescription {
    name: "Deferred fixture",
    ..LANGUAGES[0]
};

fn string_color(language: NativeLanguage, rules: &'static [HighlightRule<'static>]) -> Extension {
    highlight_layer(HighlightLayer::Active(
        HighlightStyle::define(rules).with_native_scope(language),
    ))
}

const fn string_rule(color: u32) -> HighlightRule<'static> {
    const STRING: &[HighlightSelector<'_>] = &[HighlightSelector::new("string")];
    HighlightRule::new(
        STRING,
        HighlightProperties {
            foreground: Some(Color::from_rgba32(color << 8 | 0xff)),
            bold: false,
            italic: false,
            text_decoration: None,
        },
    )
}

static JSON_RULE: [HighlightRule<'static>; 1] = [string_rule(JSON_COLOR)];
static JS_RULE: [HighlightRule<'static>; 1] = [string_rule(JS_COLOR)];

fn assert_pane_frame(editor: &Editor, draft_text: &str, published: bool) -> Bounds<Pixels> {
    let layout = editor.layout.as_ref().unwrap();
    let items = || layout.rows.iter().flat_map(|row| &row.items);
    let draft = items()
        .find_map(|item| match &item.paint {
            ItemPaint::Text(line) if line.text.as_ref() == draft_text => Some(line.as_ref()),
            _ => None,
        })
        .unwrap();
    let preview_line = |text: &str| {
        items()
            .filter_map(|item| match &item.paint {
                ItemPaint::Block(block) => Some(block.code_lines()),
                _ => None,
            })
            .flatten()
            .find(|line| line.text.as_ref() == text)
            .unwrap()
    };
    let preview = preview_line(PREVIEW);
    assert_eq!(
        painted_runs(preview_line(FALLBACK), rgb(JS_COLOR)),
        [(FALLBACK.find('"').unwrap()..FALLBACK.len() - 1, false)],
    );
    for (line, token) in [
        (draft, 1..draft_text.len() - 1),
        (preview, 4..PREVIEW.len()),
    ] {
        assert!(painted_runs(line, rgb(JS_COLOR)).is_empty());
        if published {
            assert_eq!(painted_runs(line, rgb(JSON_COLOR)), [(token, false)]);
        } else {
            assert!(painted_runs(line, rgb(JSON_COLOR)).is_empty());
            assert_eq!(
                painted_runs(line, layout.theme.text),
                [(0..line.text.len(), false)]
            );
        }
    }
    layout.source_frame.as_ref().unwrap().bounds
}

#[gpui::test]
fn deferred_fences_paint_only_the_published_snapshot(cx: &mut TestAppContext) {
    let source = format!(
        "```lazy\r\n{ORIGINAL}\r\n```\r\n\r\n\
         ```lazy\r\n[\r\n\t\"値\"\r\n]\r\n```\r\n\r\n\
         ```\r\n{FALLBACK}\r\n```\r\n"
    );
    let edited = source.replace(ORIGINAL, EDITED);
    let factories = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(AtomicUsize::new(0));
    let resolutions = Arc::new(AtomicUsize::new(0));
    let load = Arc::new(MarkdownCodeLoad::new(&LOAD_DESCRIPTION, {
        let (factories, polls) = (factories.clone(), polls.clone());
        move |_| {
            factories.fetch_add(1, Relaxed);
            let polls = polls.clone();
            poll_fn(move |_| {
                polls.fetch_add(1, Relaxed);
                Poll::Ready(Ok(NativeLanguage::Json.support()))
            })
        }
    }));
    let javascript = fence_language("js").unwrap();
    let config = |code_languages| {
        markdown_with_parser(
            MarkdownConfig::default(),
            MarkdownParserConfig {
                code_languages: Some(code_languages),
                default_code_language: Some(javascript),
                ..Default::default()
            },
        )
    };
    let deferred = config(MarkdownCodeLanguages::Deferred {
        resolve: Arc::new({
            let (load, resolutions) = (load.clone(), resolutions.clone());
            move |name| {
                resolutions.fetch_add(1, Relaxed);
                (name == "lazy").then(|| MarkdownCodeSelection::Load(load.clone()))
            }
        }),
        fallback: None,
    });
    let synchronous = config(MarkdownCodeLanguages::Resolve(Arc::new(|name| {
        (name == "lazy").then_some(NativeLanguage::Json)
    })));
    let block_refresh = Arc::new(AtomicUsize::new(0));
    let extender = transaction_extender({
        let block_refresh = block_refresh.clone();
        let extra = StateEffectType::<()>::new();
        move |_, transaction| {
            if transaction.add_to_history() || transaction.effects().is_empty() {
                return TransactionExtension::default();
            }
            match block_refresh.swap(0, Relaxed) {
                1 => TransactionExtension {
                    effects: vec![extra.of(())],
                    ..Default::default()
                },
                2 => TransactionExtension {
                    scroll_into_view: true,
                    ..Default::default()
                },
                _ => TransactionExtension::default(),
            }
        }
    });
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.clone(),
            Extension::group([
                slot.of(synchronous),
                string_color(NativeLanguage::Json, &JSON_RULE),
                string_color(javascript, &JS_RULE),
                extender,
            ]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(750.), px(660.)));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, ORIGINAL, cx);
            window.focus(&editor.focus);
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            // Warm the cache with the same parser and old text before the pending edit.
            assert_pane_frame(editor, ORIGINAL, true);
            assert_eq!(factories.load(Relaxed), 0);
            configure(editor, slot.reconfigure(deferred), cx);
            editor.request_markdown_refresh(cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_pane_frame(editor, ORIGINAL, false);
            assert_eq!(factories.load(Relaxed), 1);
            assert_eq!(polls.load(Relaxed), 0);
            assert_eq!(resolutions.load(Relaxed), 2);
            let start = editor.source().find(ORIGINAL).unwrap();
            assert!(editor.replace(start..start + ORIGINAL.len(), EDITED, cx));
        });
    });
    cx.run_until_parked();
    let (pending, selection, active, revision, scroll) = cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert_pane_frame(editor, EDITED, false);
        assert_eq!(editor.source(), edited);
        assert_eq!(editor.view.undo_depth(), 1);
        assert_eq!(editor.view.redo_depth(), 0);
        assert!(editor.focus.is_focused(window));
        (
            markdown_snapshot(editor.view.state()).unwrap(),
            editor.view.state().selection().clone(),
            editor.active.clone(),
            editor.view.revision(),
            editor.scroll.offset(),
        )
    });
    let after_edit_resolutions = resolutions.load(Relaxed);
    assert!(matches!(
        Pin::new(&mut load.load()).poll(&mut TaskContext::from_waker(Waker::noop())),
        Poll::Ready(Ok(_))
    ));
    cx.update(|_, cx| editor.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    let pending_bounds = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(Arc::ptr_eq(
            &pending,
            &markdown_snapshot(editor.view.state()).unwrap()
        ));
        assert!(editor.source_fence().unwrap().parser.is_none());
        assert_pane_frame(editor, EDITED, false)
    });

    for mode in [1, 2] {
        block_refresh.store(mode, Relaxed);
        cx.update(|_, cx| editor.update(cx, |editor, cx| editor.request_markdown_refresh(cx)));
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(Arc::ptr_eq(
                &pending,
                &markdown_snapshot(editor.view.state()).unwrap()
            ));
            assert_pane_frame(editor, EDITED, false);
        });
        assert_eq!(block_refresh.load(Relaxed), 0);
    }

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.request_markdown_refresh(cx);
            editor.request_markdown_refresh(cx);
            // Even a ready mount is not published during the host callback.
            assert!(Arc::ptr_eq(
                &pending,
                &markdown_snapshot(editor.view.state()).unwrap()
            ));
        });
    });
    cx.run_until_parked();
    let (published, cached) = cx.update(|window, cx| {
        let editor = editor.read(cx);
        let snapshot = markdown_snapshot(editor.view.state()).unwrap();
        assert!(!Arc::ptr_eq(&pending, &snapshot));
        assert_eq!(pending.text().as_str(), edited);
        let block = &active.as_ref().unwrap().source;
        let body = editor.source_fence().unwrap().body;
        assert!(pending.published_markdown_code_parser(block).is_none());
        assert!(pending.highlights(body.clone()).unwrap().is_empty());
        assert!(snapshot.published_markdown_code_parser(&body).is_none());
        assert!(!snapshot.highlights(body).unwrap().is_empty());
        assert_eq!(assert_pane_frame(editor, EDITED, true), pending_bounds);
        assert_eq!(editor.source(), edited);
        assert_eq!(editor.view.state().selection(), &selection);
        assert_eq!(editor.view.revision(), revision);
        assert_eq!(editor.active, active);
        assert_eq!(editor.view.undo_depth(), 1);
        assert_eq!(editor.view.redo_depth(), 0);
        assert_eq!(editor.scroll.offset(), scroll);
        assert!(editor.focus.is_focused(window));
        let cached = cached_body(editor).clone();
        assert_eq!(cached.as_str(), EDITED);
        (snapshot, cached)
    });
    assert_eq!(factories.load(Relaxed), 1);
    assert_eq!(polls.load(Relaxed), 1);
    assert_eq!(resolutions.load(Relaxed), after_edit_resolutions);

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.request_markdown_refresh(cx);
            cx.notify();
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(Arc::ptr_eq(
            &published,
            &markdown_snapshot(editor.view.state()).unwrap()
        ));
        assert_eq!(
            cached_body(editor).as_str().as_ptr(),
            cached.as_str().as_ptr()
        );
    });
    assert_eq!(resolutions.load(Relaxed), after_edit_resolutions);

    for (keys, text, body, undo, redo) in [
        ("secondary-z", &source, ORIGINAL, 0, 1),
        ("secondary-shift-z", &edited, EDITED, 1, 0),
    ] {
        cx.simulate_keystrokes(keys);
        cx.run_until_parked();
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), text);
            assert_eq!(editor.view.undo_depth(), undo);
            assert_eq!(editor.view.redo_depth(), redo);
            assert!(editor.focus.is_focused(window));
            assert_pane_frame(editor, body, true);
            assert_eq!(cached_body(editor).as_str(), body);
        });
    }
}

fn cached_body(editor: &Editor) -> &Text {
    let fence = editor.source_fence().unwrap();
    let parser = MarkdownCodeParser::Native(NativeLanguage::Json);
    assert!(fence.parser.as_ref() == Some(&parser));
    let settings = StreamSettings {
        tab_size: editor.view.state().tab_size(),
        ..Default::default()
    };
    editor
        .current_source_document(&fence.body, &parser, settings)
        .unwrap()
        .text()
}
