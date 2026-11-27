use std::{
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use gpui::{Entity, TestAppContext, VisualTestContext};
use gpui_editor_codemirror::{
    Change, Compartment, Extension, READ_ONLY, lsp::transport::Program, transaction_filter,
};
use serde_json::Map;
use tempfile::TempDir;

use super::super::Status;
use super::*;
use crate::document::SectionKind;

const URI: &str = "file:///completion-draft.md";
const SOURCE: &str = "Lead\r\n\r\n```js\r\n// 😀foo + foo\n```\r\n\r\n```js\r\nfoo\r\n```\r\n";
const WAIT: Duration = Duration::from_secs(5);

struct Peer(TempDir);

impl Peer {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/editor/source_lsp/completion/fixture.rs");
        let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--edition=2024", "-Dwarnings"])
            .arg(fixture)
            .arg("-o")
            .arg(dir.path().join("peer"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self(dir)
    }

    fn error(&self) -> String {
        std::fs::read_to_string(self.0.path().join("panic")).unwrap_or_default()
    }

    fn attach(&self, editor: &mut Editor, mode: &str) {
        let program = Program::new(self.0.path().join("peer"))
            .arg(mode)
            .arg(self.0.path().join("panic"));
        let client = StdioClient::start(&program, Map::new(), WAIT).unwrap();
        editor.attach_source_lsp(client, URI, "markdown").unwrap();
    }

    fn detach(&self, editor: &Entity<Editor>, cx: &mut VisualTestContext) {
        let client =
            cx.update(|_, cx| editor.update(cx, |editor, _| editor.detach_source_lsp().unwrap()));
        assert!(
            client.shutdown(WAIT).is_ok_and(|status| status.success()),
            "{}",
            self.error()
        );
    }

    fn drive(&self, editor: &Entity<Editor>, cx: &mut VisualTestContext) {
        let deadline = Instant::now() + WAIT;
        loop {
            if cx.update(|_, cx| {
                editor.update(cx, |editor, _| {
                    let status = editor.poll_source_lsp();
                    assert_ne!(status, Status::Disconnected, "{}", self.error());
                    let completion = &editor.source_lsp.as_ref().unwrap().completion;
                    status == Status::Ready
                        && completion.wanted.is_none()
                        && completion.pending.is_none()
                })
            }) {
                return;
            }
            assert!(Instant::now() < deadline, "{}", self.error());
            thread::sleep(Duration::from_millis(1));
        }
    }
}

fn word(editor: &Editor) -> usize {
    editor.source().find("foo").unwrap()
}

fn enter_fence(editor: &mut Editor) {
    let start = editor.source().find("```js").unwrap();
    let end = start + editor.source()[start..].find("```\r\n\r\n").unwrap() + 3;
    editor.active = Some(Section::new(SectionKind::DraftCode, start..end, start..end));
    let head = word(editor) + 2;
    editor.set_selection(head, head);
}

fn request(editor: &mut Editor) -> bool {
    editor.request_source_lsp_completion(editor.cursor())
}

#[gpui::test]
fn completion_syncs_explicitly_and_applies_the_chosen_item_in_one_undo_step(
    cx: &mut TestAppContext,
) {
    let peer = Peer::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!request(editor));
            peer.attach(editor, "flow");
            assert!(!request(editor));
            assert!(editor.replace(0..4, "Changed", cx));
            enter_fence(editor);
            assert!(editor.request_source_completion(true, cx));
            assert!(request(editor));
            assert!(editor.take_source_lsp_completion().is_none());
        })
    });
    peer.drive(&editor, cx);
    let before = SOURCE.replacen("Lead", "Changed", 1);
    let after = before.replacen("foo + foo", "félan + tail", 1);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let start = word(editor);
            let head = start + 2;
            assert_eq!(editor.source(), before);
            assert_eq!(editor.cursor(), head);
            assert_eq!(editor.view.undo_depth(), 1);
            let choices = editor.take_source_lsp_completion().unwrap();
            assert_eq!(
                choices
                    .items()
                    .iter()
                    .map(SourceLspCompletionItem::label)
                    .collect::<Vec<_>>(),
                ["first", "[literal]"]
            );
            assert_eq!(
                choices.items()[0].changes().changes().collect::<Vec<_>>(),
                [(&(start..head), "alpha")]
            );
            assert_eq!(
                choices.items()[1].changes().changes().collect::<Vec<_>>(),
                [
                    (&(start + 1..start + 3), "élan"),
                    (&(start + 6..start + 9), "tail")
                ]
            );
            assert!(editor.take_source_lsp_completion().is_none());
            assert!(
                editor.completion.is_some(),
                "LSP polling or taking closed the native popup"
            );
            assert_eq!(editor.source(), before);
            assert_eq!(editor.cursor(), head);
            assert!(editor.apply_source_lsp_completion(choices, 1, cx));
            assert_eq!(editor.source(), after);
            assert_eq!(editor.cursor(), start + "félan".len());
            assert_eq!(editor.view.undo_depth(), 2);
            assert!(editor.source_fence().is_some());
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            editor.replay_history(false, cx);
            assert_eq!(editor.source(), before);
            assert_eq!(editor.cursor(), head);
            assert_eq!(editor.view.undo_depth(), 1);
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            editor.replay_history(true, cx);
            assert_eq!(editor.source(), after);
            assert_eq!(editor.cursor(), start + "félan".len());
            assert_eq!(editor.view.undo_depth(), 2);
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
        })
    });
    peer.detach(&editor, cx);
}

#[gpui::test]
fn completion_rejects_unsafe_coordinates_bounds_and_protected_markdown(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            peer.attach(editor, "rejected");
            enter_fence(editor);
            let head = editor.cursor();
            for offset in [
                0,
                word(editor) - 1,
                head + 1,
                editor.source().rfind("foo").unwrap(),
            ] {
                assert!(!editor.request_source_lsp_completion(offset));
            }
            editor.set_selection(head - 1, head);
            assert!(!request(editor));
            editor.set_selection(head, head);
        })
    });
    for index in 0..18 {
        cx.update(|_, cx| editor.update(cx, |editor, _| assert!(request(editor))));
        peer.drive(&editor, cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let result = editor.take_source_lsp_completion();
                if index < 16 {
                    assert!(result.is_none(), "reply {index}");
                } else {
                    let result = result
                        .unwrap_or_else(|| panic!("reply {index} must pass coordinate checks"));
                    assert!(
                        !editor.apply_source_lsp_completion(result, 0, cx),
                        "reply {index}"
                    );
                }
                assert_eq!(editor.source(), SOURCE);
                assert_eq!(editor.cursor(), word(editor) + 2);
                assert!(!editor.view.can_undo());
            })
        });
    }
    peer.detach(&editor, cx);
}

#[gpui::test]
fn completion_requires_a_capability_and_sync_and_retires_an_inflight_request(
    cx: &mut TestAppContext,
) {
    let peer = Peer::new();
    for mode in ["unsupported", "no-sync", "stale"] {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                peer.attach(editor, mode);
                enter_fence(editor);
                if mode == "unsupported" {
                    assert!(request(editor)); // Initialization has not advertised capabilities yet.
                }
            })
        });
        peer.drive(&editor, cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                if mode == "stale" {
                    assert!(request(editor));
                    assert_eq!(editor.poll_source_lsp(), Status::Ready);
                    assert!(
                        editor
                            .source_lsp
                            .as_ref()
                            .unwrap()
                            .completion
                            .pending
                            .is_some()
                    );
                }
                editor.active = None;
                assert!(editor.replace(0..4, "Changed", cx));
                enter_fence(editor);
                assert_eq!(request(editor), mode != "unsupported");
            })
        });
        peer.drive(&editor, cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                let result = editor.take_source_lsp_completion();
                if mode == "stale" {
                    assert_eq!(result.unwrap().items()[0].label(), "fresh");
                } else {
                    assert!(result.is_none(), "{mode}");
                }
                assert!(editor.source().starts_with("Changed"));
                assert_eq!(editor.view.undo_depth(), 1);
            })
        });
        peer.detach(&editor, cx);
    }
}

#[gpui::test]
fn completion_rechecks_selection_input_generation_and_attachment_at_apply(cx: &mut TestAppContext) {
    let peer = Peer::new();
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
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            peer.attach(editor, "repeat");
            enter_fence(editor);
        })
    });
    for guard in [
        "selection",
        "read-only",
        "toggled read-only",
        "composition",
        "disabled",
        "superseded",
        "new attachment",
        "index",
    ] {
        cx.update(|_, cx| editor.update(cx, |editor, _| assert!(request(editor))));
        peer.drive(&editor, cx);
        let result = cx.update(|_, cx| {
            editor.update(cx, |editor, _| editor.take_source_lsp_completion().unwrap())
        });
        if guard == "new attachment" {
            peer.detach(&editor, cx);
        }
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let head = editor.cursor();
                let active = editor.active.clone();
                let set_read_only = |editor: &mut Editor, enabled| {
                    editor
                        .view
                        .update(
                            Transaction::default()
                                .with_effects([read_only.reconfigure(READ_ONLY.of(enabled))])
                                .with_add_to_history(false),
                            active.clone(),
                            |_| (None, active.clone()),
                        )
                        .unwrap();
                };
                match guard {
                    "selection" => {
                        editor.set_selection(head + 1, head + 1);
                        editor.set_selection(head, head);
                    }
                    "read-only" | "toggled read-only" => {
                        set_read_only(editor, true);
                        assert!(!request(editor));
                        if guard == "toggled read-only" {
                            set_read_only(editor, false);
                        }
                    }
                    "composition" => {
                        editor.marked = Some(head..head);
                        assert!(!request(editor));
                    }
                    "disabled" => {
                        editor.set_input_enabled(false, cx);
                        assert!(!request(editor));
                    }
                    "superseded" => assert!(request(editor)),
                    "new attachment" => peer.attach(editor, "repeat"),
                    "index" => {}
                    _ => unreachable!(),
                }
                assert!(
                    !editor.apply_source_lsp_completion(result, usize::from(guard == "index"), cx),
                    "{guard}"
                );
                assert_eq!(editor.source(), SOURCE);
                assert_eq!(editor.cursor(), head);
                assert!(!editor.view.can_undo());
                match guard {
                    "read-only" => set_read_only(editor, false),
                    "composition" => editor.marked = None,
                    "disabled" => editor.set_input_enabled(true, cx),
                    _ => {}
                }
            })
        });
    }
    peer.detach(&editor, cx);
}

#[gpui::test]
fn completion_rejects_a_rewritten_transaction_atomically(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let mode = Arc::new(AtomicUsize::new(0));
    let filter = mode.clone();
    let extensions = transaction_filter(move |_, tx| {
        if tx.user_event() != Some("input.complete") {
            return Ok(tx);
        }
        Ok(match filter.load(Ordering::Relaxed) {
            0 => Transaction::new(ChangeSet::single(Change::new(0..4, "Other")).unwrap()),
            1 => tx.with_selection(EditorSelection::single(0, 0)),
            2 => tx.with_effects([Extension::default().append()]),
            3 => tx.with_add_to_history(false),
            4 => Transaction::new(tx.changes().clone())
                .with_selection(tx.selection().unwrap().clone())
                .with_user_event("input.complete")
                .isolate_history(HistoryIsolation::Before),
            _ => unreachable!(),
        })
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(SOURCE.into(), extensions, window, cx).unwrap()
    });
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            peer.attach(editor, "repeat");
            enter_fence(editor);
        })
    });
    for index in 0..5 {
        mode.store(index, Ordering::Relaxed);
        cx.update(|_, cx| editor.update(cx, |editor, _| assert!(request(editor))));
        peer.drive(&editor, cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let result = editor.take_source_lsp_completion().unwrap();
                assert!(
                    !editor.apply_source_lsp_completion(result, 0, cx),
                    "filter {index}"
                );
                assert_eq!(editor.source(), SOURCE);
                assert_eq!(editor.cursor(), word(editor) + 2);
                assert!(!editor.view.can_undo());
            })
        });
    }
    peer.detach(&editor, cx);
}
