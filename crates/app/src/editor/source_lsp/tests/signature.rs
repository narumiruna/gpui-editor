use gpui::BackgroundExecutor;
use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, Extension, READ_ONLY, Transaction,
    language::fold::{fold_effect, unfold_effect},
};

use crate::document::{BlockPreview, Section};

use super::*;

fn idle(editor: &Editor) -> bool {
    editor.source_lsp.as_ref().unwrap().signature.is_idle()
}

fn drain(editor: &Entity<Editor>, cx: &mut VisualTestContext, fixture: &Fixture) {
    let deadline = Instant::now() + WAIT;
    loop {
        if cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let status = editor.poll_source_lsp();
                assert_ne!(status, Status::Disconnected, "{}", fixture.peer_panic());
                cx.notify();
                status == Status::Ready && idle(editor)
            })
        }) {
            return;
        }
        assert!(Instant::now() < deadline, "{}", fixture.peer_panic());
        // Yield to the local stdio peer. The debounce uses only the GPUI clock.
        thread::yield_now();
    }
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
            enter_fence(editor, cx);
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

fn enter_fence(editor: &mut Editor, cx: &mut gpui::Context<Editor>) {
    let range = editor
        .markdown()
        .sections()
        .into_iter()
        .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
        .unwrap()
        .source;
    editor.activate(
        Section::new(SectionKind::DraftCode, range.clone(), range),
        false,
        cx,
    );
    let head = editor.source_fence().unwrap().body.start + 1;
    editor.set_selection(head, head);
}

fn advance(clock: &BackgroundExecutor, cx: &mut VisualTestContext, millis: u64) {
    clock.advance_clock(Duration::from_millis(millis));
    cx.run_until_parked();
}

fn assert_idle(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| assert!(idle(editor.read(cx))));
}

fn insert_without_selection(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let body = editor.source_fence().unwrap().body;
            let changes = ChangeSet::single(Change::new(body.start..body.start, text)).unwrap();
            assert!(editor.apply_source_transaction(
                Transaction::new(changes).with_user_event("input.type"),
                body,
                cx,
            ));
        });
    });
    cx.run_until_parked();
}

const FIRST_SIGNATURE: &str = "1/2  f(x, y)\nParameter: x";
const SECOND_SIGNATURE: &str = "2/2  f(a, b)\nParameter: a";
const REFRESHED_SIGNATURE: &str = "2/2  f(a, b)\nParameter: b";

#[gpui::test]
fn signature_content_change_debounces_explicit_selection_including_typing(cx: &mut TestAppContext) {
    let clock = cx.executor();
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    focus(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor
                .attach_source_lsp(fixture.start("signature-debounce"), URI, "markdown")
                .unwrap();
            enter_fence(editor, cx);
        });
    });
    advance(&clock, cx, 250);
    assert_idle(&editor, cx);
    cx.simulate_keystrokes("cmd-shift-space");
    drain(&editor, cx, &fixture);
    assert_overlay(&editor, cx, Some(FIRST_SIGNATURE));
    cx.simulate_keystrokes("cmd-shift-down");
    assert_overlay(&editor, cx, Some(SECOND_SIGNATURE));

    insert_without_selection(&editor, cx, "0");
    advance(&clock, cx, 250);
    assert_idle(&editor, cx);
    drain(&editor, cx, &fixture);
    assert_overlay(&editor, cx, Some(SECOND_SIGNATURE));

    cx.simulate_keystrokes("right");
    advance(&clock, cx, 249);
    assert_idle(&editor, cx);
    cx.simulate_input("z"); // Accepted typing with an explicit selection, but no trigger match.
    advance(&clock, cx, 1);
    assert_idle(&editor, cx); // The old deadline did not fire.
    insert_without_selection(&editor, cx, "1"); // Move the caret without resetting the deadline.
    advance(&clock, cx, 248);
    assert_idle(&editor, cx);
    advance(&clock, cx, 1);
    cx.update(|_, cx| {
        assert!(
            editor
                .read(cx)
                .source_lsp
                .as_ref()
                .unwrap()
                .signature
                .can_send()
        )
    });
    drain(&editor, cx, &fixture);
    assert_overlay(&editor, cx, Some(REFRESHED_SIGNATURE));
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        let layout = editor.layout.as_ref().unwrap();
        let tooltip = editor
            .source_lsp_tooltip(layout, editor.scroll.bounds(), window, None)
            .unwrap();
        let anchor = editor.source_fence().unwrap().body.start + 3;
        assert_eq!(tooltip.anchor, layout.bounds_at(anchor, false));
        assert_ne!(anchor, editor.cursor());
    });

    // An explicit selection set to the same value still schedules a request.
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            let head = editor.cursor();
            editor.set_selection(head, head);
        })
    });
    advance(&clock, cx, 250);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!idle(editor));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            let head = editor.cursor();
            editor.set_selection(head + 1, head + 1);
            editor.set_selection(head, head);
            cx.notify();
        })
    });
    drain(&editor, cx, &fixture);
    assert_overlay(&editor, cx, Some(REFRESHED_SIGNATURE)); // The first reply is stale.
    advance(&clock, cx, 250);
    drain(&editor, cx, &fixture);
    assert_overlay(&editor, cx, None); // A current null reply closes the tooltip.
    shutdown(&editor, cx, &fixture);
}

#[gpui::test]
fn signature_content_change_cancels_on_immediate_requests_and_editor_guards(
    cx: &mut TestAppContext,
) {
    let clock = cx.executor();
    let fixture = Fixture::new();
    let read_only = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            SOURCE.into(),
            read_only.of(Extension::default()),
            window,
            cx,
        )
        .unwrap()
    });
    focus(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor
                .attach_source_lsp(fixture.start("signature-cancel"), URI, "markdown")
                .unwrap();
            enter_fence(editor, cx);
            assert!(editor.request_source_lsp_signature_help());
        })
    });
    drain(&editor, cx, &fixture);
    assert_overlay(&editor, cx, Some(FIRST_SIGNATURE));
    cx.simulate_keystrokes("right");
    advance(&clock, cx, 100);
    cx.simulate_input(",");
    cx.update(|_, cx| assert!(!idle(editor.read(cx)))); // Character retrigger is immediate.
    drain(&editor, cx, &fixture);
    advance(&clock, cx, 250);
    assert_idle(&editor, cx);
    cx.simulate_keystrokes("right cmd-shift-space");
    cx.update(|_, cx| assert!(!idle(editor.read(cx)))); // Manual invocation is immediate too.
    drain(&editor, cx, &fixture);
    advance(&clock, cx, 250);
    assert_idle(&editor, cx);
    cx.simulate_keystrokes("escape");

    for guard in [
        "dismiss",
        "focus",
        "window",
        "read-only",
        "outside",
        "fold",
        "range",
        "disabled",
        "IME",
    ] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                enter_fence(editor, cx);
                assert!(editor.request_source_lsp_signature_help());
            })
        });
        drain(&editor, cx, &fixture);
        assert_overlay(&editor, cx, Some(FIRST_SIGNATURE));
        cx.simulate_keystrokes("right");
        advance(&clock, cx, 100);
        match guard {
            "dismiss" => cx.simulate_keystrokes("escape"),
            "focus" => {
                let other = cx.update(|window, cx| {
                    let other = cx.focus_handle();
                    window.focus(&other);
                    assert!(!editor.read(cx).focus.is_focused(window));
                    other
                });
                advance(&clock, cx, 250);
                cx.update(|window, _| assert!(other.is_focused(window)));
                focus(&editor, cx);
            }
            "window" => {
                cx.deactivate_window();
                cx.run_until_parked();
                focus(&editor, cx);
            }
            "fold" => {
                // The timer has queued the request, but the old anchor becomes
                // ineligible before the host polls to send it.
                advance(&clock, cx, 150);
                cx.update(|_, cx| {
                    editor.update(cx, |editor, cx| {
                        assert!(!idle(editor));
                        let start = editor.source_fence().unwrap().body.start;
                        let range = start..editor.cursor();
                        let active = editor.active.clone();
                        for effect in [fold_effect().of(range.clone()), unfold_effect().of(range)] {
                            editor
                                .view
                                .update(
                                    Transaction::default().with_effects([effect]),
                                    active.clone(),
                                    |_| (None, active.clone()),
                                )
                                .unwrap();
                            assert_eq!(editor.poll_source_lsp(), Status::Ready);
                            assert!(idle(editor));
                        }
                        cx.notify();
                    })
                });
            }
            _ => cx.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    let head = editor.cursor();
                    match guard {
                        "read-only" => {
                            let active = editor.active.clone();
                            for enabled in [true, false] {
                                editor
                                    .view
                                    .update(
                                        Transaction::default().with_effects([
                                            read_only.reconfigure(READ_ONLY.of(enabled))
                                        ]),
                                        active.clone(),
                                        |_| (None, active.clone()),
                                    )
                                    .unwrap();
                            }
                        }
                        "outside" => {
                            editor.set_selection(WORD, WORD);
                            editor.set_selection(head, head);
                        }
                        "range" => {
                            editor.set_selection(head, head + 1);
                            editor.set_selection(head, head);
                        }
                        "disabled" => {
                            editor.set_input_enabled(false, cx);
                            editor.set_input_enabled(true, cx);
                        }
                        "IME" => {
                            editor.replace_and_mark_text_in_range(None, "文", None, window, cx);
                            assert!(editor.is_composing());
                            editor.unmark_text(window, cx);
                        }
                        _ => unreachable!(),
                    }
                    cx.notify();
                })
            }),
        }
        advance(&clock, cx, 250);
        assert_idle(&editor, cx);
        assert_overlay(&editor, cx, None);
        drain(&editor, cx, &fixture);
    }

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            enter_fence(editor, cx);
            assert!(editor.request_source_lsp_signature_help());
        })
    });
    drain(&editor, cx, &fixture);
    cx.simulate_keystrokes("right");
    shutdown(&editor, cx, &fixture);
    advance(&clock, cx, 250);
    cx.update(|window, cx| assert!(editor.read(cx).focus.is_focused(window)));
}
