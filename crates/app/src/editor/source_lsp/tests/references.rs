use gpui_editor_codemirror::{
    Change, ChangeSet, Transaction,
    lsp::references::get_reference_panel,
    transaction_filter,
    view::panels::{Panel, PanelId, PanelPlacement, active_panels},
};

use super::*;
use crate::document::{BlockPreview, Section};

fn ready(editor: &Editor, status: Status, fixture: &Fixture) -> bool {
    assert_ne!(status, Status::Disconnected, "{}", fixture.peer_panic());
    status == Status::Ready && editor.source_lsp.as_ref().unwrap().references.is_idle()
}

fn complete(editor: &Entity<Editor>, cx: &mut VisualTestContext, fixture: &Fixture) {
    drive(editor, cx, |editor, status| ready(editor, status, fixture));
}

fn queue(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        assert!(editor.update(cx, |editor, _| editor.request_source_lsp_references()));
    });
}

fn detach(editor: &Entity<Editor>, cx: &mut VisualTestContext, fixture: &Fixture) {
    let client = cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            let client = editor.detach_source_lsp().unwrap();
            assert!(editor.source_lsp_references().is_none());
            assert!(get_reference_panel(editor.view.state()).is_none());
            assert!(client.with_workspace(|workspace| workspace.files().is_empty()));
            client
        })
    });
    assert!(
        client.shutdown(WAIT).unwrap().success(),
        "{}",
        fixture.peer_panic()
    );
}

#[gpui::test]
fn reference_panel_matches_single_draft_oracle_cases_on_real_stdio(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    cx.simulate_resize(size(px(660.), px(220.)));
    focus(&editor, cx);
    let full_body = cx.update(|_, cx| editor.read(cx).scroll.bounds());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.request_source_lsp_references());
            editor
                .attach_source_lsp(fixture.start("references"), URI, "markdown")
                .unwrap();
            assert!(editor.replace(WORD..WORD + 2, "name", cx));
            editor.set_selection(WORD + 4, WORD + 1);
            // As in the upstream oracle, use the directed selection's head.
            assert!(editor.request_source_lsp_references());
        });
    });
    complete(&editor, cx, &fixture);

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let panel = editor.source_lsp_references().unwrap();
            assert_eq!(panel.selected, 0);
            assert_eq!(
                *active_panels(editor.view.state()),
                [Panel::Custom {
                    id: panel.id,
                    placement: PanelPlacement::Bottom,
                }]
            );
            assert_eq!(panel.rows.len(), 3);
            assert_eq!(panel.rows[0], panel.rows[2]);
            assert_eq!(panel.rows[0].line_number, 3);
            assert_eq!(panel.rows[0].matched, "Tail");
            assert_eq!(panel.rows[0].after, ". [site](https://example.invalid)");
            assert_eq!(panel.rows[1].before, "😀 ");
            assert_eq!(panel.rows[1].matched, "name");
            assert_eq!(editor.cursor(), WORD + 1);
            assert!(!editor.source_lsp_reference_key(panel.id, "Tab", cx));
            assert!(!editor.source_lsp_reference_key(panel.id, " ", cx));
        })
    });
    cx.run_until_parked();
    let body_with_panel = cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(!editor.focus.is_focused(window));
        let body = editor.scroll.bounds();
        assert_eq!(body.origin, full_body.origin);
        assert!(body.bottom() < full_body.bottom());
        body
    });
    for (key, index) in [
        ("up", 2),
        ("pagedown", 0),
        ("down", 1),
        ("pageup", 0),
        ("end", 2),
        ("end", 2),
        ("home", 0),
        ("down", 1),
        ("space", 1),
    ] {
        cx.simulate_keystrokes(key);
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            assert!(!editor.focus.is_focused(window), "{key}");
            assert_eq!(
                editor.source_lsp_references().unwrap().selected,
                index,
                "{key}"
            );
            assert_eq!(editor.cursor(), WORD + 1);
            assert_eq!(editor.view.undo_depth(), 1);
            assert_eq!(editor.source(), SOURCE.replacen("id", "name", 1));
        });
    }
    let selected = cx.update(|_, cx| editor.read(cx).source_lsp_references().unwrap());

    // Null, empty, all unavailable, invalid UTF-16, and too many rows must
    // not replace the current panel or disturb its selection.
    for _ in 0..5 {
        queue(&editor, cx);
        complete(&editor, cx, &fixture);
        cx.update(|_, cx| {
            assert_eq!(
                editor.read(cx).source_lsp_references().as_ref(),
                Some(&selected)
            )
        });
    }

    queue(&editor, cx);
    complete(&editor, cx, &fixture);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let replacement = editor.source_lsp_references().unwrap();
            assert_ne!(replacement.id, selected.id);
            assert_eq!(replacement.selected, 0);
            assert_eq!(replacement.rows, selected.rows);
            assert!(!editor.source_lsp_reference_key(selected.id, "Escape", cx));

            // Request at the old head and supersede it before its reply.
            assert!(editor.request_source_lsp_references());
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            let tail = editor.source().find("Tail").unwrap();
            editor.set_selection(tail, tail);
            assert!(editor.request_source_lsp_references());
        });
    });
    complete(&editor, cx, &fixture);
    let (id, undo_depth, had_selection_undo) = cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            let panel = editor.source_lsp_references().unwrap();
            assert_eq!(panel.rows, [selected.rows.into_iter().next().unwrap()]);
            assert_eq!(panel.selected, 0);
            editor.set_selection(WORD, WORD);
            (
                panel.id,
                editor.view.undo_depth(),
                editor.view.can_undo_selection(),
            )
        })
    });
    cx.simulate_keystrokes("down enter");
    cx.update(|window, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.focus.is_focused(window));
            assert_eq!(editor.cursor(), editor.source().find("Tail").unwrap());
            assert_eq!(editor.source_lsp_references().unwrap().id, id);
            assert_eq!(editor.view.undo_depth(), undo_depth);
            assert_eq!(editor.view.can_undo_selection(), had_selection_undo);
            assert_eq!(editor.source(), SOURCE.replacen("id", "name", 1));
            assert!(editor.request_source_lsp_references());
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
        });
    });
    let close = cx.debug_bounds("source-lsp-references-close").unwrap();
    assert!(close.top() >= body_with_panel.bottom());
    cx.simulate_click(close.center(), Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(editor.focus.is_focused(window));
        assert_eq!(editor.scroll.bounds(), full_body);
        assert!(get_reference_panel(editor.view.state()).is_none());
    });
    complete(&editor, cx, &fixture);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.source_lsp_references().is_none());
            assert!(!editor.dismiss_source_lsp_references());
            assert!(editor.request_source_lsp_references());
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            editor.replay_history(false, cx);
            assert_eq!(editor.source(), SOURCE);
            assert_eq!(editor.view.redo_depth(), 1);
        });
    });
    complete(&editor, cx, &fixture);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.source_lsp_references().is_none());
            let tail = editor.source().find("Tail").unwrap();
            editor.set_selection(tail, tail);
            assert!(editor.request_source_lsp_references());
        });
    });
    complete(&editor, cx, &fixture);
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).source_lsp_references().unwrap().rows[0].matched,
            "Tail"
        )
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(editor.focus.is_focused(window));
        assert_eq!(editor.scroll.bounds(), full_body);
        assert!(editor.source_lsp_references().is_none());
    });
    detach(&editor, cx, &fixture);
}

#[gpui::test]
fn one_dark_bottom_references_panel_roles_follow_live_reconfiguration(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::{Compartment, Extension, theme_one_dark::one_dark_theme};

    use crate::editor::one_dark_tests::{
        configure,
        panel_menu::{Role, assert_panel, sample},
    };

    let fixture = Fixture::new();
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(SOURCE.into(), slot.of(Extension::default()), window, cx)
            .unwrap()
    });
    cx.simulate_resize(size(px(660.), px(460.)));
    focus(&editor, cx);
    let full = cx.update(|_, cx| editor.read(cx).scroll.bounds());
    let selection = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor
                .attach_source_lsp(fixture.start("references-pointer"), URI, "markdown")
                .unwrap();
            editor.set_selection(WORD + 2, WORD);
            assert!(editor.request_source_lsp_references());
            cx.notify();
            editor.view.state().selection().clone()
        })
    });
    complete(&editor, cx, &fixture);
    cx.run_until_parked();
    let results = cx.update(|_, cx| editor.read(cx).source_lsp_references().unwrap());
    for (themed, extension) in [
        (false, None),
        (true, Some(one_dark_theme())),
        (false, Some(Extension::default())),
    ] {
        if let Some(extension) = extension {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    configure(editor, slot.reconfigure(extension), cx)
                })
            });
        }
        assert_panel(&sample(&editor, cx), Role::References, themed);
        let panel = cx.debug_bounds("source-lsp-references-panel").unwrap();
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            let body = editor.scroll.bounds();
            assert_eq!(body.origin, full.origin);
            assert_eq!(body.bottom(), panel.top());
            assert_eq!(panel.bottom(), full.bottom());
            assert!(!editor.focus.is_focused(window));
            assert_eq!(editor.source_lsp_references().as_ref(), Some(&results));
            assert_eq!(editor.source(), SOURCE);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.undo_depth(), 0);
        });
    }
    cx.simulate_keystrokes("escape");
    assert!(sample(&editor, cx).is_empty());
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(editor.focus.is_focused(window));
        assert_eq!(editor.scroll.bounds(), full);
        assert_eq!(editor.view.state().selection(), &selection);
    });
    detach(&editor, cx, &fixture);
}

#[gpui::test]
fn clicking_a_reference_selects_and_reveals_the_row_without_editing(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    cx.simulate_resize(size(px(660.), px(460.)));
    focus(&editor, cx);
    let undo_selection = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor
                .attach_source_lsp(fixture.start("references-pointer"), URI, "markdown")
                .unwrap();
            editor.set_selection(WORD, WORD);
            assert!(editor.request_source_lsp_references());
            cx.notify();
            editor.view.can_undo_selection()
        })
    });
    complete(&editor, cx, &fixture);
    let id = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.focus.is_focused(window));
            let panel = editor.source_lsp_references().unwrap();
            assert_eq!(panel.selected, 0);
            assert_eq!(panel.rows[1].matched, "Tail");
            assert!(!editor.activate_source_lsp_reference_row(panel.id, 2, cx));
            assert!(!editor.activate_source_lsp_reference_row(PanelId::new(), 1, cx));
            assert_eq!(editor.cursor(), WORD);
            panel.id
        })
    });
    let row = cx.debug_bounds("source-lsp-reference-row-1").unwrap();
    cx.simulate_click(row.center(), Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(editor.focus.is_focused(window));
        assert_eq!(editor.cursor(), SOURCE.find("Tail").unwrap());
        let panel = editor.source_lsp_references().unwrap();
        assert_eq!(panel.id, id);
        assert_eq!(panel.selected, 1);
        assert_eq!(editor.source(), SOURCE);
        assert_eq!(editor.view.revision(), 0);
        assert!(!editor.view.can_undo());
        assert_eq!(editor.view.can_undo_selection(), undo_selection);
    });
    detach(&editor, cx, &fixture);
}

#[gpui::test]
fn references_require_live_markdown_scope_and_reject_filtered_edits(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    let body = SOURCE.find("let x").unwrap();
    focus(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor
                .attach_source_lsp(fixture.start("references-code"), URI, "markdown")
                .unwrap();
            editor.set_selection(body, body);
            assert!(!editor.request_source_lsp_references());
            let block = editor
                .markdown()
                .sections()
                .into_iter()
                .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                .unwrap();
            editor.activate(
                Section::new(SectionKind::DraftCode, block.source.clone(), block.source),
                false,
                cx,
            );
            let marker = SOURCE.find("```rs").unwrap();
            editor.set_selection(marker, marker);
            assert!(!editor.request_source_lsp_references());
            editor.set_selection(body, body);
            assert!(editor.request_source_lsp_references());
        });
    });
    complete(&editor, cx, &fixture);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let panel = editor.source_lsp_references().unwrap();
            assert_eq!(panel.rows.len(), 1);
            assert_eq!(panel.rows[0].matched, "let");
            let start = body + 4;
            editor.set_selection(start, start);
            assert!(editor.source_lsp_reference_key(panel.id, "Enter", cx));
            assert_eq!(editor.cursor(), body);
            let active = editor.active.clone().unwrap();
            let filter = transaction_filter(|_, tx| {
                if tx.scroll_into_view() && tx.selection().is_some() {
                    Ok(Transaction::new(
                        ChangeSet::single(Change::new(0..0, "bad")).unwrap(),
                    ))
                } else {
                    Ok(tx)
                }
            });
            editor
                .view
                .update(
                    Transaction::default().with_effects([filter.append()]),
                    Some(active.clone()),
                    |_| (None, Some(active)),
                )
                .unwrap();
            editor.set_selection(start, start);
        });
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let panel = editor.source_lsp_references().unwrap();
            assert!(!editor.focus.is_focused(window));
            assert_eq!(editor.cursor(), body + 4);
            assert_eq!(editor.source(), SOURCE);
            assert!(!editor.view.can_undo());
            editor.set_input_enabled(false, cx);
            assert!(editor.source_lsp_references().is_none());
            assert!(!editor.source_lsp_reference_key(panel.id, "Enter", cx));
            assert!(!editor.request_source_lsp_references());
            editor.set_input_enabled(true, cx);
            editor.finish_active();
            assert!(!editor.source_lsp_reference_key(panel.id, "Enter", cx));
        });
    });
    detach(&editor, cx, &fixture);
}

#[gpui::test]
fn references_are_capability_gated_and_require_a_synchronized_revision(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    focus(&editor, cx);
    let full_body = cx.update(|_, cx| editor.read(cx).scroll.bounds());
    for mode in ["references-unsupported", "references-no-sync"] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                editor
                    .attach_source_lsp(fixture.start(mode), URI, "markdown")
                    .unwrap();
                editor.set_selection(WORD, WORD);
                assert!(editor.request_source_lsp_references());
            });
        });
        complete(&editor, cx, &fixture);
        if mode == "references-no-sync" {
            cx.run_until_parked();
            cx.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    assert!(!editor.focus.is_focused(window));
                    assert!(editor.scroll.bounds().bottom() < full_body.bottom());
                    let panel = editor.source_lsp_references().unwrap();
                    assert_eq!(panel.rows[0].matched, "id");
                    assert!(editor.replace(WORD..WORD + 2, "new", cx));
                    assert!(editor.source_lsp_references().is_none());
                    assert!(!editor.source_lsp_reference_key(panel.id, "Enter", cx));
                    editor.set_selection(WORD, WORD);
                    assert!(editor.request_source_lsp_references());
                });
            });
            // The draft revision alone removes the UI, without another host poll.
            cx.run_until_parked();
            cx.update(|window, cx| {
                let editor = editor.read(cx);
                assert!(get_reference_panel(editor.view.state()).is_some());
                assert_eq!(editor.scroll.bounds(), full_body);
                assert!(editor.focus.is_focused(window));
                assert_eq!(editor.source(), SOURCE.replacen("id", "new", 1));
                assert_eq!(editor.view.undo_depth(), 1);
            });
            complete(&editor, cx, &fixture);
            cx.update(|_, cx| assert!(get_reference_panel(editor.read(cx).view.state()).is_none()));
        } else {
            cx.update(|_, cx| assert!(editor.read(cx).source_lsp_references().is_none()));
        }
        // The peer only accepts close, not a reference query for an unsupported
        // capability or a newer revision when change sync is absent.
        detach(&editor, cx, &fixture);
    }
}

#[gpui::test]
fn references_do_not_take_focus_from_find(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    focus(&editor, cx);
    let find_focus = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor
                .attach_source_lsp(fixture.start("references-no-sync"), URI, "markdown")
                .unwrap();
            editor.set_selection(WORD, WORD);
            assert!(editor.request_source_lsp_references());
            editor.open_find(window, cx);
        });
        window.focused(cx).unwrap()
    });
    complete(&editor, cx, &fixture);
    cx.run_until_parked();
    let close = cx.debug_bounds("source-lsp-references-close").unwrap();
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(find_focus.is_focused(window));
        assert!(editor.source_lsp_references().is_some());
        assert!(close.top() >= editor.scroll.bounds().bottom());
    });
    cx.simulate_click(close.center(), Modifiers::none());
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(find_focus.is_focused(window));
        assert!(editor.source_lsp_references().is_none());
        assert_eq!(editor.source(), SOURCE);
        assert!(!editor.view.can_undo());
    });
    detach(&editor, cx, &fixture);
}

#[gpui::test]
fn references_reply_during_composition_does_not_mount_or_take_focus(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    focus(&editor, cx);
    let full_body = cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor
                .attach_source_lsp(fixture.start("references-no-sync"), URI, "markdown")
                .unwrap();
            editor.set_selection(WORD, WORD);
            editor.scroll.bounds()
        })
    });
    complete(&editor, cx, &fixture);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.request_source_lsp_references());
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            assert!(!editor.source_lsp.as_ref().unwrap().references.is_idle());
            editor.replace_and_mark_text_in_range(None, "字", None, window, cx);
            assert!(editor.is_composing());
            assert!(!editor.request_source_lsp_references());
        });
    });
    complete(&editor, cx, &fixture);
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.focus.is_focused(window));
            assert!(editor.is_composing());
            assert!(get_reference_panel(editor.view.state()).is_none());
            assert_eq!(editor.scroll.bounds(), full_body);
            assert_eq!(editor.source(), SOURCE.replacen("id", "字id", 1));
            assert_eq!(editor.view.undo_depth(), 1);
            editor.finish_composition();
            editor.replay_history(false, cx);
            assert_eq!(editor.source(), SOURCE);
            assert_eq!(editor.view.redo_depth(), 1);
        });
    });
    detach(&editor, cx, &fixture);
}

#[gpui::test]
fn references_close_when_the_attachment_disconnects(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor
                .attach_source_lsp(fixture.start("references-disconnect"), URI, "markdown")
                .unwrap();
            editor.set_selection(WORD, WORD);
            assert!(editor.request_source_lsp_references());
        });
    });
    complete(&editor, cx, &fixture);
    cx.update(|_, cx| assert!(editor.read(cx).source_lsp_references().is_some()));
    queue(&editor, cx);
    drive(&editor, cx, |editor, status| {
        if status != Status::Disconnected {
            return false;
        }
        assert!(editor.source_lsp_references().is_none());
        assert!(get_reference_panel(editor.view.state()).is_none());
        assert_eq!(editor.source(), SOURCE);
        assert!(!editor.view.can_undo());
        true
    });
    assert!(fixture.peer_panic().is_empty(), "{}", fixture.peer_panic());
}
