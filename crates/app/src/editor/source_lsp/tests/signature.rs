use crate::document::{BlockPreview, Section};

use super::*;

fn idle(editor: &Editor) -> bool {
    editor.source_lsp.as_ref().unwrap().signature.is_idle()
}

fn drain(editor: &Entity<Editor>, cx: &mut VisualTestContext, fixture: &Fixture) {
    drive(editor, cx, |editor, status| {
        assert_ne!(status, Status::Disconnected, "{}", fixture.peer_panic());
        status == Status::Ready && idle(editor)
    });
}

fn shutdown(editor: &Entity<Editor>, cx: &mut VisualTestContext, fixture: &Fixture) {
    let client = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let client = editor.detach_source_lsp().unwrap();
            cx.notify();
            client
        })
    });
    assert!(
        client.shutdown(WAIT).unwrap().success(),
        "{}",
        fixture.peer_panic()
    );
    assert_overlay(editor, cx, None);
}

#[gpui::test]
fn signature_request_is_literal_scoped_and_selection_guarded(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    cx.simulate_resize(size(px(660.), px(380.)));
    focus(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.request_source_lsp_signature_help());
            editor
                .attach_source_lsp(fixture.start("signature-request"), URI, "markdown")
                .unwrap();
            for word in ["site", "let x", "```rs"] {
                let offset = SOURCE.find(word).unwrap();
                editor.set_selection(offset, offset);
                assert!(!editor.request_source_lsp_signature_help(), "{word}");
            }
            editor.set_selection(WORD, WORD);
            cx.notify();
        });
    });
    cx.simulate_keystrokes("cmd-shift-space");
    drain(&editor, cx, &fixture);
    assert_overlay(
        &editor,
        cx,
        Some(
            "1/2  f(😀, x)\nParameter: 😀\n[site](https://example.invalid) <script>literal</script>�",
        ),
    );
    cx.simulate_keystrokes("cmd-shift-down cmd-shift-down");
    assert_overlay(&editor, cx, Some("2/2  f(value)"));
    cx.simulate_keystrokes("cmd-shift-up");
    assert_overlay(
        &editor,
        cx,
        Some(
            "1/2  f(😀, x)\nParameter: 😀\n[site](https://example.invalid) <script>literal</script>�",
        ),
    );
    cx.simulate_keystrokes("escape");
    assert_overlay(&editor, cx, None);

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), SOURCE);
            assert_eq!(editor.cursor(), WORD);
            assert_eq!(editor.view.undo_depth(), 0);
            assert!(editor.request_source_lsp_signature_help());
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            assert!(!idle(editor));
            // Even if the caret returns to the same byte and the text did not
            // change, a result from before the selection updates is obsolete.
            editor.set_selection(WORD + 1, WORD + 1);
            editor.set_selection(WORD, WORD);
            cx.notify();
        });
    });
    drain(&editor, cx, &fixture);
    assert_overlay(&editor, cx, None);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.request_source_lsp_signature_help());
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            assert!(!idle(editor));
            let tail = SOURCE.find("Tail.").unwrap();
            assert!(editor.replace(tail..tail + 5, "Changed.", cx));
        });
    });
    drain(&editor, cx, &fixture);
    assert_overlay(&editor, cx, None);

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let block = editor
                .markdown()
                .sections()
                .into_iter()
                .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                .unwrap();
            let range = block.source;
            editor.activate(
                Section::new(SectionKind::DraftCode, range.clone(), range),
                false,
                cx,
            );
            let marker = editor.source().find("```rs").unwrap() + 1;
            editor.set_selection(marker, marker);
            assert!(!editor.request_source_lsp_signature_help());
            let body = editor.source().find("let x").unwrap();
            editor.set_selection(body, body);
            assert!(editor.request_source_lsp_signature_help());
            cx.notify();
        });
    });
    drain(&editor, cx, &fixture);
    assert_overlay(&editor, cx, Some("in code"));
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        let layout = editor.layout.as_ref().unwrap();
        let outside = layout.bounds_at(WORD, false).unwrap();
        assert!(
            editor
                .source_lsp_tooltip(layout, outside, window, None)
                .is_none()
        );
    });
    cx.deactivate_window();
    assert_overlay(&editor, cx, None);
    focus(&editor, cx);
    assert_overlay(&editor, cx, None);
    shutdown(&editor, cx, &fixture);
}

#[gpui::test]
fn signature_characters_follow_accepted_typing_and_sync_order(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    focus(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor
                .attach_source_lsp(fixture.start("signature-trigger"), URI, "markdown")
                .unwrap();
            editor.set_selection(WORD, WORD);
            cx.notify();
        });
    });
    drain(&editor, cx, &fixture);
    // A retrigger character alone is not a trigger.
    cx.simulate_input(",");
    drain(&editor, cx, &fixture);
    assert_overlay(&editor, cx, None);
    cx.simulate_input("(");
    drain(&editor, cx, &fixture);
    assert_overlay(&editor, cx, Some("f(x)"));
    cx.simulate_input(",");
    drain(&editor, cx, &fixture);
    assert_overlay(&editor, cx, None);

    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), SOURCE.replacen(" id", " ,(,id", 1));
        assert_eq!(editor.cursor(), WORD + 3);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("(".into()));
    });
    cx.simulate_keystrokes("secondary-v");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(editor.source().contains(",(,(id"));
        assert!(idle(editor));
    });
    cx.simulate_keystrokes("secondary-z secondary-shift-z");
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.source().contains(",(,(id"));
            assert!(idle(editor));
            editor.replace_and_mark_text_in_range(None, "(", None, window, cx);
            assert!(editor.is_composing());
            assert!(idle(editor));
            assert!(!editor.request_source_lsp_signature_help());
        });
    });
    shutdown(&editor, cx, &fixture);
}
