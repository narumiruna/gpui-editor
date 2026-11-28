use std::{ops::Range, sync::Arc};

use gpui::{Entity, EntityInputHandler, TestAppContext, VisualTestContext, point, px};
use gpui_editor_codemirror::{
    Change, EditorSelection, Extension, HistoryIsolation, READ_ONLY, StateEffectType, Transaction,
    TransactionExtension, transaction_extender, transaction_filter,
};

use super::*;

const BASE: usize = 19;
const PREFIX: &str = "# 🧭 raw\r\n\r\n[link](https://example.invalid) `literal`\r\n\r\n~~~rs  \r\n";
const BODY: &str = "éz\rraw";

fn open<'a>(
    cx: &'a mut TestAppContext,
    id: &str,
    source: &str,
    extra: Extension,
) -> (Entity<Editor>, &'a mut VisualTestContext) {
    cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.to_owned(),
            Extension::group([collab(CollabConfig::new(id, BASE)), extra]),
            window,
            cx,
        )
        .unwrap()
    })
}

fn edit(id: &str, len: usize, range: Range<usize>, insert: &str) -> Update {
    Update::new(
        id,
        ChangeSet::single(Change::new(range, insert)).unwrap(),
        len,
    )
    .unwrap()
}

struct Authority {
    text: Text,
    accepted: Vec<Update>,
}

impl Authority {
    fn version(&self) -> usize {
        BASE + self.accepted.len()
    }

    fn submit(&mut self, queue: &SourceCollabQueue) -> Result<(), CollabError> {
        if queue.version != self.version() {
            return Err(CollabError::UnexpectedVersion {
                expected: self.version(),
                received: queue.version,
            });
        }
        let mut text = self.text.clone();
        let mut accepted = Vec::new();
        for update in &queue.updates {
            assert!(update.effects().is_empty());
            // The authority keeps edits, not references to either editor's state.
            let copy = Update::new(update.client_id(), update.changes().clone(), text.len())?;
            update.apply_to(&mut text)?;
            accepted.push(copy);
        }
        self.text = text;
        self.accepted.extend(accepted);
        Ok(())
    }

    fn deliver(&self, editor: &Entity<Editor>, cx: &mut VisualTestContext) {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let version = editor.source_collab_queue().unwrap().version;
                editor
                    .receive_source_collab_updates(version, &self.accepted[version - BASE..], cx)
                    .unwrap();
            });
        });
    }
}

fn queue(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> SourceCollabQueue {
    cx.update(|_, cx| editor.read(cx).source_collab_queue().unwrap())
}

fn activate_at_end(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let draft = editor.markdown().sections().pop().unwrap();
            assert_eq!(draft.kind, SectionKind::DraftCode);
            editor.activate(draft, false, cx);
            editor.set_selection(editor.source().len(), editor.source().len());
            window.focus(&editor.focus);
        });
    });
    cx.run_until_parked();
}

fn assert_converged(
    authority: &Authority,
    a: &Entity<Editor>,
    b: &Entity<Editor>,
    cx: &mut VisualTestContext,
) {
    cx.update(|_, cx| {
        for peer in [a, b] {
            let peer = peer.read(cx);
            assert_eq!(peer.source().as_bytes(), authority.text.as_str().as_bytes());
            assert_eq!(&peer.source().as_bytes()[..PREFIX.len()], PREFIX.as_bytes());
            assert_eq!(
                peer.source_collab_queue().unwrap(),
                SourceCollabQueue {
                    version: authority.version(),
                    updates: vec![]
                }
            );
        }
    });
}

#[gpui::test]
fn independent_exact_base_authority_rebases_unicode_and_maps_native_undo_redo(
    cx: &mut TestAppContext,
) {
    #[derive(serde::Deserialize)]
    struct Trace {
        source: String,
        base: usize,
        stages: [Stage; 4],
    }
    #[derive(serde::Deserialize)]
    struct Stage {
        name: String,
        draft: String,
        version: usize,
        caret: usize,
        depths: (usize, usize),
    }
    let trace: Trace = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/codemirror/collab_host_trace.json"
    )))
    .unwrap();
    let source = format!("{PREFIX}{BODY}");
    assert_eq!(trace.source, source);
    assert_eq!(trace.base, BASE);
    let [synced, undo, redo, next_input] = trace.stages;
    let mut second = cx.clone();
    let (a, a_cx) = open(cx, "a", &source, Extension::default());
    let (b, b_cx) = open(&mut second, "b", &source, Extension::default());
    let check_stage = |stage: &Stage, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let peer = a.read(cx);
            assert_eq!(
                peer.source(),
                format!("{PREFIX}{}", stage.draft),
                "{}",
                stage.name
            );
            assert_eq!(
                peer.source_collab_queue().unwrap().version,
                stage.version,
                "{}",
                stage.name
            );
            assert_eq!(
                (peer.view.undo_depth(), peer.view.redo_depth()),
                stage.depths,
                "{}",
                stage.name
            );
            assert_eq!(peer.cursor(), stage.caret, "{}", stage.name);
        });
    };
    a_cx.update(|_, cx| super::super::bind_keys(cx));
    activate_at_end(&a, a_cx);
    a_cx.simulate_input("🐈");
    activate_at_end(&b, b_cx);
    b_cx.simulate_input("🦊");
    let a_first = queue(&a, a_cx);
    let b_first = queue(&b, b_cx);
    assert_eq!(
        a_first.updates,
        [edit("a", source.len(), source.len()..source.len(), "🐈")]
    );
    assert_eq!(
        b_first.updates,
        [edit("b", source.len(), source.len()..source.len(), "🦊")]
    );
    assert_eq!(queue(&a, a_cx), a_first);
    let mut authority = Authority {
        text: Text::new(&source),
        accepted: Vec::new(),
    };
    authority.submit(&b_first).unwrap();
    assert_eq!(authority.text.as_str(), format!("{source}🦊"));
    assert_eq!(
        authority.submit(&a_first),
        Err(CollabError::UnexpectedVersion {
            expected: BASE + 1,
            received: BASE,
        })
    );
    assert_eq!(authority.version(), BASE + 1);

    // Echo with no active frame. A remains active; its caret and bounds must map.
    b_cx.update(|_, cx| b.update(cx, |editor, _| editor.finish_active()));
    authority.deliver(&b, b_cx);
    authority.deliver(&a, a_cx);
    let rebased = queue(&a, a_cx);
    let end = authority.text.len();
    assert_eq!(rebased.version, BASE + 1);
    assert_eq!(rebased.updates, [edit("a", end, end..end, "🐈")]);
    assert!(std::ptr::eq(
        rebased.updates[0].origin_context().unwrap(),
        a_first.updates[0].origin_context().unwrap(),
    ));
    authority.submit(&rebased).unwrap();
    authority.deliver(&b, b_cx);
    authority.deliver(&a, a_cx);
    assert_converged(&authority, &a, &b, a_cx);
    check_stage(&synced, a_cx);

    for (key, stage) in [("secondary-z", &undo), ("secondary-shift-z", &redo)] {
        a_cx.simulate_keystrokes(key);
        a_cx.update(|_, cx| {
            let editor = a.read(cx);
            assert_eq!(
                editor.source_fence().unwrap().body,
                PREFIX.len()..editor.source().len()
            );
        });
        authority.submit(&queue(&a, a_cx)).unwrap();
        authority.deliver(&a, a_cx);
        authority.deliver(&b, b_cx);
        assert_converged(&authority, &a, &b, a_cx);
        check_stage(stage, a_cx);
    }
    // The fixture inserts at the end of the source, not at the redo caret.
    a_cx.update(|_, cx| {
        a.update(cx, |editor, _| {
            let end = editor.source().len();
            editor.set_selection(end, end);
        });
    });
    a_cx.simulate_input("λ");
    authority.submit(&queue(&a, a_cx)).unwrap();
    authority.deliver(&a, a_cx);
    authority.deliver(&b, b_cx);
    assert_converged(&authority, &a, &b, a_cx);
    check_stage(&next_input, a_cx);
    a_cx.update(|_, cx| {
        let editor = b.read(cx);
        assert!(editor.active.is_none());
        assert_eq!(
            editor.view.undo_depth(),
            1,
            "no peer edit becomes a local step"
        );
    });
}

#[gpui::test]
fn rejected_batches_keep_the_view_queue_and_history_unchanged(cx: &mut TestAppContext) {
    let source = format!("{PREFIX}{BODY}");
    let len = source.len();
    let (editor, cx) = open(cx, "a", &source, Extension::default());
    activate_at_end(&editor, cx);
    cx.simulate_input("X");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let effect = StateEffectType::new();
            let valid = edit("b", len, len..len, "字");
            let outside = edit("b", len, 0..1, "!");
            let ending = edit("b", len, len..len, "\r\n~~~\r\n");
            let before = collab_state(editor.view.state()).unwrap().unwrap();
            let selection = editor.view.state().selection().clone();
            let frame = editor.active.clone();
            let revision = editor.view.revision();
            for (version, batch, error) in [
                (
                    BASE - 1,
                    vec![valid.clone()],
                    CollabError::UnexpectedVersion {
                        expected: BASE,
                        received: BASE - 1,
                    }
                    .into(),
                ),
                (
                    BASE,
                    vec![edit("a", len, len..len, "?")],
                    CollabError::MismatchedAcknowledgement.into(),
                ),
                (
                    BASE,
                    vec![valid.clone().with_effects([effect.of(())])],
                    SourceCollabError::EffectsUnsupported,
                ),
                (
                    BASE,
                    vec![
                        valid.clone(),
                        edit("b", len + "字".len(), len + 1..len + 2, "_"),
                    ],
                    EditError::InvalidRange.into(),
                ),
                (
                    BASE,
                    vec![
                        valid.clone(),
                        edit("b", len + "字".len(), PREFIX.len() - 1..PREFIX.len(), "!"),
                    ],
                    SourceCollabError::OutsideDraft,
                ),
                (
                    BASE,
                    vec![outside, edit("b", len, 0..1, "#")],
                    SourceCollabError::OutsideDraft,
                ),
                (BASE, vec![ending], SourceCollabError::ChangedTopology),
            ] {
                assert_eq!(
                    editor.receive_source_collab_updates(version, &batch, cx),
                    Err(error)
                );
                assert_eq!(editor.source(), format!("{source}X"));
                assert_eq!(editor.active, frame);
                assert_eq!(editor.view.state().selection(), &selection);
                assert_eq!(editor.view.revision(), revision);
                assert_eq!((editor.view.undo_depth(), editor.view.redo_depth()), (1, 0));
                assert!(Arc::ptr_eq(
                    &before,
                    &collab_state(editor.view.state()).unwrap().unwrap()
                ));
                assert_eq!(
                    editor.source_collab.as_ref().unwrap().authority.as_str(),
                    source
                );
            }
            editor
                .receive_source_collab_updates(BASE, &[valid], cx)
                .unwrap();
            assert_eq!(editor.source(), format!("{source}字X"));
        });
    });
}

#[gpui::test]
fn queue_and_matching_echo_reject_protected_edits_that_cancel_in_the_final_text(
    cx: &mut TestAppContext,
) {
    let source = format!("plain\r\n\r\n```rs\r\n{BODY}");
    let (editor, cx) = open(cx, "a", &source, Extension::default());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.replace(0..1, "P", cx));
            assert!(editor.replace(0..1, "p", cx));
            assert_eq!(editor.source(), source);
            let session = collab_state(editor.view.state()).unwrap().unwrap();
            let pending = session.sendable_updates();
            assert_eq!(pending.len(), 2);
            let revision = editor.view.revision();
            let history = editor.view.undo_depth();
            let selection = editor.view.state().selection().clone();
            assert_eq!(
                editor.source_collab_queue(),
                Err(SourceCollabError::OutsideDraft)
            );
            assert_eq!(
                editor.receive_source_collab_updates(BASE, pending, cx),
                Err(SourceCollabError::OutsideDraft)
            );
            assert!(Arc::ptr_eq(
                &session,
                &collab_state(editor.view.state()).unwrap().unwrap()
            ));
            assert_eq!(
                editor.source_collab.as_ref().unwrap().authority.as_str(),
                source
            );
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(
                (editor.view.revision(), editor.view.undo_depth()),
                (revision, history)
            );
        })
    });
}

#[gpui::test]
fn queue_does_not_disclose_or_strip_unsupported_shared_effects(cx: &mut TestAppContext) {
    let source = format!("{PREFIX}{BODY}");
    let marker = StateEffectType::new();
    let expose = {
        let marker = marker.clone();
        collab(
            CollabConfig::default().with_shared_effects(move |transaction| {
                transaction
                    .effects()
                    .iter()
                    .filter(|effect| effect.get(&marker).is_some())
                    .cloned()
                    .collect()
            }),
        )
    };
    let (editor, cx) = open(cx, "a", &source, expose);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            let receipt = marker.of("not a text update");
            editor
                .view
                .update(
                    Transaction::default().with_effects([receipt.clone()]),
                    None,
                    |_| (None, None),
                )
                .unwrap();
            let session = collab_state(editor.view.state()).unwrap().unwrap();
            assert_eq!(session.sendable_updates().len(), 1);
            assert_eq!(
                session.sendable_updates()[0].effects()[0].get(&marker),
                Some(&"not a text update")
            );
            assert_eq!(
                editor.source_collab_queue(),
                Err(SourceCollabError::EffectsUnsupported)
            );
            assert!(Arc::ptr_eq(
                &session,
                &collab_state(editor.view.state()).unwrap().unwrap()
            ));
            assert_eq!(editor.source(), source);
            assert_eq!((editor.view.revision(), editor.view.undo_depth()), (0, 0));
        })
    });
}

#[gpui::test]
fn authority_and_optimistic_topology_are_both_checked(cx: &mut TestAppContext) {
    // Only reconciling both edits can form a closing marker: the local edit
    // supplies a tilde before the peer deletes the trailing non-marker text.
    let source = format!("{PREFIX}~~z");
    let (editor, cx) = open(cx, "a", &source, Extension::default());
    activate_at_end(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor.set_selection(source.len() - 1, source.len() - 1)
        })
    });
    cx.simulate_input("~");
    let before = queue(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let remote = edit("b", source.len(), source.len() - 1..source.len(), "");
            assert_eq!(
                editor.receive_source_collab_updates(BASE, &[remote], cx),
                Err(SourceCollabError::ChangedTopology)
            );
            assert_eq!(editor.source(), format!("{PREFIX}~~~z"));
            assert_eq!(editor.source_collab_queue().unwrap(), before);
        })
    });
}

#[gpui::test]
fn receipts_do_not_run_filters_or_take_focus_scroll_or_selection(cx: &mut TestAppContext) {
    let source = format!("{PREFIX}{BODY}\r\n");
    let protected = PREFIX.len();
    let (editor, cx) = open(
        cx,
        "a",
        &source,
        transaction_filter(|_, tx| {
            if tx.user_event().is_none() {
                return Err(EditError::InvalidRange);
            }
            Ok(tx)
        }),
    );
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let unfocused = cx.focus_handle();
            window.focus(&unfocused);
            assert!(editor.active.is_none());
            let selection = EditorSelection::new(
                vec![
                    EditorSelection::cursor(0, None, None, None),
                    EditorSelection::range(protected + 3, protected + 2, Some(4.0), Some(1), None),
                ],
                0,
            )
            .unwrap();
            editor.view.set_selection(selection.clone()).unwrap();
            editor.scroll.set_offset(point(px(0.), px(-30.)));
            let offset = editor.scroll.offset();
            let remote = edit("b", source.len(), protected..protected, "😀\r\n");
            let expected = selection.map_desc(remote.change_desc()).unwrap();
            editor
                .receive_source_collab_updates(BASE, &[remote], cx)
                .unwrap();
            assert_eq!(editor.view.state().selection(), &expected);
            assert_eq!(editor.view.state().selection().main_index(), 0);
            assert!(editor.active.is_none());
            assert!(unfocused.is_focused(window));
            assert!(!editor.view.focused());
            assert_eq!(editor.scroll.offset(), offset);
            assert!(!editor.reveal_caret);
            assert!(editor.reveal_source.is_none());
            assert!(editor.view.take_scroll_request().is_none());
            assert_eq!(editor.view.undo_depth(), 0);
            editor
                .receive_source_collab_updates(BASE + 1, &[], cx)
                .unwrap();
        })
    });
}

#[gpui::test]
fn extenders_cannot_apply_effects_scroll_or_history_policy(cx: &mut TestAppContext) {
    let source = format!("{PREFIX}{BODY}");
    let effect = StateEffectType::new();
    let extensions = [
        TransactionExtension {
            effects: vec![effect.of(())],
            ..Default::default()
        },
        TransactionExtension {
            scroll_into_view: true,
            ..Default::default()
        },
        TransactionExtension {
            annotations: vec![
                Transaction::history_isolation_annotation().of(HistoryIsolation::Full),
            ],
            ..Default::default()
        },
    ];
    for extra in extensions {
        let extension = transaction_extender(move |_, _| TransactionExtension {
            effects: extra.effects.clone(),
            annotations: extra.annotations.clone(),
            scroll_into_view: extra.scroll_into_view,
        });
        let (editor, cx) = open(cx, "a", &source, extension);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let remote = edit("b", source.len(), source.len()..source.len(), "!");
                for batch in [vec![], vec![remote]] {
                    assert_eq!(
                        editor.receive_source_collab_updates(BASE, &batch, cx),
                        Err(EditError::Configuration(ConfigurationError::InvalidFieldData).into())
                    );
                    assert_eq!(editor.source(), source);
                    assert_eq!(editor.view.revision(), 0);
                    assert_eq!(editor.source_collab_queue().unwrap().version, BASE);
                    assert!(editor.view.take_scroll_request().is_none());
                }
            })
        });
    }
}

#[gpui::test]
fn empty_receipts_work_without_a_body_but_remote_edits_do_not(cx: &mut TestAppContext) {
    for source in [
        "just prose",
        "```rs",
        "```rs\r\nx\r\n```\r\n",
        "> ```rs\n> x",
        "- ```rs\n  x",
    ] {
        let (editor, cx) = open(cx, "a", source, Extension::default());
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.receive_source_collab_updates(BASE, &[], cx).unwrap();
                let remote = edit("b", source.len(), source.len()..source.len(), "!");
                assert_eq!(
                    editor.receive_source_collab_updates(BASE, &[remote], cx),
                    Err(SourceCollabError::OutsideDraft),
                    "{source:?}"
                );
                assert_eq!(editor.view.revision(), 0);
            })
        });
    }
}

#[gpui::test]
fn committed_ime_edit_converges_with_a_waiting_peer_update_and_one_undo_step(
    cx: &mut TestAppContext,
) {
    let trace: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/codemirror/collab_ime_host_trace.json"
    )))
    .unwrap();
    let source = format!("{PREFIX}{BODY}");
    assert_eq!(trace["source"], source);
    assert_eq!(trace["base"], BASE);
    let mut second = cx.clone();
    let (a, a_cx) = open(cx, "a", &source, Extension::default());
    let (b, b_cx) = open(&mut second, "b", &source, Extension::default());
    a_cx.update(|_, cx| super::super::bind_keys(cx));
    activate_at_end(&a, a_cx);
    a_cx.simulate_input("!");
    let prefix = queue(&a, a_cx);
    activate_at_end(&b, b_cx);
    b_cx.simulate_input("🦊");
    let mut authority = Authority {
        text: Text::new(&source),
        accepted: Vec::new(),
    };
    authority.submit(&queue(&b, b_cx)).unwrap();
    a_cx.update(|window, cx| {
        a.update(cx, |editor, cx| {
            for interim in ["ni", "你🦀"] {
                editor.replace_and_mark_text_in_range(None, interim, None, window, cx);
                assert_eq!(editor.source(), format!("{source}!{interim}"));
                assert_eq!(
                    editor.source_collab_queue(),
                    Err(SourceCollabError::InputBlocked)
                );
                assert_eq!(
                    editor.receive_source_collab_updates(BASE, &authority.accepted, cx),
                    Err(SourceCollabError::InputBlocked)
                );
            }
            editor.replace_text_in_range(None, "你好", window, cx);
            assert!(!editor.is_composing());
            assert_eq!(editor.source(), format!("{source}!你好"));
        })
    });
    let committed = queue(&a, a_cx);
    let at = source.len() + 1;
    assert_eq!(
        committed.updates,
        [prefix.updates[0].clone(), edit("a", at, at..at, "你好")]
    );
    let origin = committed.updates[1].origin_context().unwrap();
    assert_eq!(origin.start_text().as_str(), format!("{source}!"));
    assert_eq!(origin.new_text().as_str(), format!("{source}!你好"));
    authority.deliver(&b, b_cx);
    authority.deliver(&a, a_cx);
    for (index, key) in [None, Some("secondary-z"), Some("secondary-shift-z")]
        .into_iter()
        .enumerate()
    {
        if let Some(key) = key {
            a_cx.simulate_keystrokes(key);
        }
        let outgoing = queue(&a, a_cx);
        assert_eq!(outgoing.updates.len(), if index == 0 { 2 } else { 1 });
        authority.submit(&outgoing).unwrap();
        authority.deliver(&b, b_cx);
        authority.deliver(&a, a_cx);
        assert_converged(&authority, &a, &b, a_cx);
        a_cx.update(|_, cx| {
            let editor = a.read(cx);
            let expected = &trace["stages"][index];
            assert_eq!(&editor.source()[PREFIX.len()..], expected["draft"]);
            assert_eq!(
                editor.source_collab_queue().unwrap().version,
                expected["version"].as_u64().unwrap() as usize
            );
            assert_eq!(
                editor.cursor(),
                expected["caret"].as_u64().unwrap() as usize
            );
            assert_eq!(
                serde_json::json!([editor.view.undo_depth(), editor.view.redo_depth()]),
                expected["depths"]
            );
        });
    }
}

#[gpui::test]
fn canceling_ime_does_not_publish_its_marked_revisions(cx: &mut TestAppContext) {
    let source = format!("{PREFIX}{BODY}");
    let (editor, cx) = open(cx, "a", &source, Extension::default());
    activate_at_end(&editor, cx);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.replace_and_mark_text_in_range(None, "你🧭", None, window, cx);
            editor.replace_and_mark_text_in_range(None, "", None, window, cx);
            assert!(!editor.is_composing());
            assert_eq!(editor.source(), source);
            assert_eq!(
                editor.source_collab_queue().unwrap(),
                SourceCollabQueue {
                    version: BASE,
                    updates: Vec::new()
                }
            );
            let remote = edit("b", source.len(), source.len()..source.len(), "!");
            editor
                .receive_source_collab_updates(BASE, &[remote], cx)
                .unwrap();
            assert_eq!(editor.source(), format!("{source}!"));
            assert_eq!(editor.source_collab_queue().unwrap().version, BASE + 1);
        })
    });
}

#[gpui::test]
fn ime_outside_draft_or_invalid_queue_receipt_is_never_exported(cx: &mut TestAppContext) {
    for (source, extra) in [
        (
            String::from("Prose with no code fence"),
            Extension::default(),
        ),
        (
            format!("{PREFIX}{BODY}"),
            transaction_extender(|_, transaction| TransactionExtension {
                scroll_into_view: transaction.changes().is_empty()
                    && transaction.selection().is_none(),
                ..Default::default()
            }),
        ),
    ] {
        let (editor, cx) = open(cx, "a", &source, extra);
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_selection(source.len(), source.len());
                editor.replace_and_mark_text_in_range(None, "漢", None, window, cx);
                assert!(editor.is_composing());
                let session = collab_state(editor.view.state()).unwrap().unwrap();
                let selection = editor.view.state().selection().clone();
                let revision = editor.view.revision();
                editor.unmark_text(window, cx);
                assert_eq!(
                    editor.source_collab_queue(),
                    Err(SourceCollabError::ImeUnsupported)
                );
                assert_eq!(
                    editor.receive_source_collab_updates(BASE, &[], cx),
                    Err(SourceCollabError::ImeUnsupported)
                );
                assert!(Arc::ptr_eq(
                    &session,
                    &collab_state(editor.view.state()).unwrap().unwrap()
                ));
                assert_eq!(editor.source(), format!("{source}漢"));
                assert_eq!(editor.view.state().selection(), &selection);
                assert_eq!(
                    (editor.view.revision(), editor.view.undo_depth()),
                    (revision, 1)
                );
            })
        });
    }
}

#[gpui::test]
fn read_only_disabled_unconfigured_and_marked_text_are_not_host_operations(
    cx: &mut TestAppContext,
) {
    let source = format!("{PREFIX}{BODY}");
    let (read_only, read_only_cx) = open(cx, "r", &source, READ_ONLY.of(true));
    read_only_cx.update(|_, cx| {
        read_only.update(cx, |editor, cx| {
            assert_eq!(
                editor.receive_source_collab_updates(BASE, &[], cx),
                Err(SourceCollabError::InputBlocked)
            );
        })
    });
    let (plain, plain_cx) =
        cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
    plain_cx.update(|_, cx| {
        plain.update(cx, |editor, cx| {
            assert_eq!(
                editor.source_collab_queue(),
                Err(CollabError::NotConfigured.into())
            );
            assert_eq!(
                editor.receive_source_collab_updates(BASE, &[], cx),
                Err(CollabError::NotConfigured.into())
            );
        })
    });
    let (editor, cx) = open(cx, "a", &source, Extension::default());
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_input_enabled(false, cx);
            assert_eq!(
                editor.receive_source_collab_updates(BASE, &[], cx),
                Err(SourceCollabError::InputBlocked)
            );
            assert_eq!(editor.source(), source);
            editor.set_input_enabled(true, cx);
            editor.set_selection(source.len(), source.len());
            editor.replace_and_mark_text_in_range(None, "漢", None, window, cx);
            assert!(editor.is_composing());
            let remote = edit("b", source.len(), source.len()..source.len(), "!");
            assert_eq!(
                editor.receive_source_collab_updates(BASE, std::slice::from_ref(&remote), cx),
                Err(SourceCollabError::InputBlocked)
            );
            assert_eq!(
                editor.source_collab_queue(),
                Err(SourceCollabError::InputBlocked)
            );
            editor.unmark_text(window, cx);
            assert_eq!(editor.source(), format!("{source}漢"));
            let pending = editor.source_collab_queue().unwrap();
            assert_eq!(pending.version, BASE);
            assert_eq!(
                pending.updates,
                [edit("a", source.len(), source.len()..source.len(), "漢")]
            );
            editor
                .receive_source_collab_updates(BASE, &[remote], cx)
                .unwrap();
            assert_eq!(editor.source(), format!("{source}!漢"));
            assert_eq!(editor.source_collab_queue().unwrap().version, BASE + 1);
            assert_eq!(editor.view.undo_depth(), 1);
        })
    });
}
