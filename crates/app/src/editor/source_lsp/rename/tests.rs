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
    Change, Compartment, EditorSelection, Extension, READ_ONLY, lsp::transport::Program,
    transaction_filter,
};
use serde_json::Map;
use tempfile::TempDir;

use super::super::Status;
use super::*;

const URI: &str = "file:///rename-draft.md";
const SOURCE: &str = "Lead\r\n\r\n```rs\r\n// 😀foo + foo\r\n```\r\n\r\n```rs\r\nfoo\r\n```\r\n";
const WAIT: Duration = Duration::from_secs(5);

struct Peer(TempDir);

impl Peer {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/editor/source_lsp/rename/fixture.rs");
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
                    let rename = &editor.source_lsp.as_ref().unwrap().rename;
                    status == Status::Ready && rename.wanted.is_none() && rename.pending.is_none()
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
    let start = editor.source().find("```rs").unwrap();
    let end = start + editor.source()[start..].find("```\r\n\r\n").unwrap() + 3;
    editor.active = Some(Section::new(SectionKind::DraftCode, start..end, start..end));
    let head = word(editor) + 1;
    editor.set_selection(head, head);
}

fn request(editor: &mut Editor) -> bool {
    editor.request_source_lsp_rename(word(editor), "bar")
}

#[gpui::test]
fn rename_syncs_before_request_and_applies_once_with_undo_and_redo(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!request(editor));
            peer.attach(editor, "flow");
            assert!(!request(editor));
            assert!(editor.replace(0..4, "Changed", cx));
            enter_fence(editor);
            assert!(editor.request_source_lsp_rename(word(editor), "élan"));
            assert!(editor.take_source_lsp_rename().is_none());
        })
    });
    peer.drive(&editor, cx);
    let before = SOURCE.replacen("Lead", "Changed", 1);
    let after = before.replacen("foo + foo", "élan + élan", 1);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let first = word(editor);
            assert_eq!(editor.source(), before);
            assert_eq!(editor.cursor(), first + 1);
            assert_eq!(editor.view.undo_depth(), 1);
            let result = editor.take_source_lsp_rename().unwrap();
            assert_eq!(result.uri(), URI);
            assert_eq!(
                result
                    .changes()
                    .changes()
                    .map(|(range, text)| (range.clone(), text))
                    .collect::<Vec<_>>(),
                [(first..first + 3, "élan"), (first + 6..first + 9, "élan")]
            );
            assert!(editor.take_source_lsp_rename().is_none());
            assert_eq!(editor.source(), before);
            assert_eq!(editor.cursor(), first + 1);
            assert!(editor.apply_source_lsp_rename(result, cx));
            assert_eq!(editor.source(), after);
            assert_eq!(editor.view.undo_depth(), 2);
            assert!(editor.source_fence().is_some());
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            editor.replay_history(false, cx);
            assert_eq!(editor.source(), before);
            assert_eq!(editor.cursor(), first + 1);
            assert_eq!(editor.view.undo_depth(), 1);
            assert_eq!(editor.view.redo_depth(), 1);
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            editor.replay_history(true, cx);
            assert_eq!(editor.source(), after);
            assert_eq!(editor.view.undo_depth(), 2);
            assert_eq!(editor.view.redo_depth(), 0);
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
        })
    });
    peer.detach(&editor, cx);
}

#[gpui::test]
fn rename_rejects_the_entire_unsafe_batch_and_keeps_stable_edit_order(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let source = SOURCE.replace("foo + foo\r\n", "foo + foo\n");
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            peer.attach(editor, "bad-results");
            enter_fence(editor);
            for offset in [
                0,
                word(editor) - 1,
                editor.source().find("Lead\r\n").unwrap() + "Lead\r".len(),
                editor.source().rfind("foo").unwrap(),
            ] {
                assert!(
                    !editor.request_source_lsp_rename(offset, "bar"),
                    "offset {offset}"
                );
            }
            for name in ["", "  ", "a\nb", "\tbar", &"x".repeat(MAX_NAME_BYTES + 1)] {
                assert!(!editor.request_source_lsp_rename(word(editor), name));
            }
        })
    });
    for index in 0..20 {
        cx.update(|_, cx| editor.update(cx, |editor, _| assert!(request(editor))));
        peer.drive(&editor, cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert!(editor.take_source_lsp_rename().is_none(), "reply {index}");
                assert_eq!(editor.source(), source);
                assert_eq!(editor.cursor(), word(editor) + 1);
                assert!(!editor.view.can_undo());
            })
        });
    }
    cx.update(|_, cx| editor.update(cx, |editor, _| assert!(request(editor))));
    peer.drive(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let result = editor.take_source_lsp_rename().unwrap();
            assert!(editor.apply_source_lsp_rename(result, cx));
            assert_eq!(
                editor.source(),
                source.replacen("foo + foo", "abbar + bar", 1)
            );
            assert_eq!(editor.view.undo_depth(), 1);
        })
    });
    peer.detach(&editor, cx);
}

#[gpui::test]
fn rename_requires_capability_and_current_sync(cx: &mut TestAppContext) {
    let peer = Peer::new();
    for mode in ["unsupported", "no-sync"] {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                peer.attach(editor, mode);
                enter_fence(editor);
                assert!(request(editor));
            })
        });
        peer.drive(&editor, cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                assert_eq!(editor.take_source_lsp_rename().is_some(), mode == "no-sync");
                if mode == "unsupported" {
                    assert!(!request(editor));
                } else {
                    editor.active = None;
                    assert!(editor.replace(0..4, "Changed", cx));
                    enter_fence(editor);
                    assert!(request(editor));
                }
            })
        });
        peer.drive(&editor, cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert!(editor.take_source_lsp_rename().is_none())
            })
        });
        peer.detach(&editor, cx);
    }
}

#[gpui::test]
fn rename_rejects_stale_requests_and_taken_results(cx: &mut TestAppContext) {
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
    peer.drive(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(request(editor));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            assert!(editor.source_lsp.as_ref().unwrap().rename.pending.is_some());
            let offset = word(editor);
            assert!(editor.replace(offset..offset + 3, "qux", cx));
            editor.replay_history(false, cx);
            assert_eq!(editor.source(), SOURCE);
        })
    });
    peer.drive(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.take_source_lsp_rename().is_none());
            assert!(request(editor));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            let head = editor.cursor();
            editor.set_selection(head + 1, head + 1);
            editor.set_selection(head, head);
        })
    });
    peer.drive(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.take_source_lsp_rename().is_none())
        })
    });
    for guard in [
        "scope",
        "new request",
        "composition",
        "read-only",
        "disabled",
        "attachment",
    ] {
        cx.update(|_, cx| editor.update(cx, |editor, _| assert!(request(editor))));
        peer.drive(&editor, cx);
        let result = cx.update(|_, cx| {
            editor.update(cx, |editor, _| editor.take_source_lsp_rename().unwrap())
        });
        if guard == "attachment" {
            peer.detach(&editor, cx);
        }
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let active = editor.active.clone();
                let head = editor.cursor();
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
                    "scope" => editor.active = None,
                    "new request" => assert!(request(editor)),
                    "composition" => editor.marked = Some(head..head),
                    "read-only" => set_read_only(editor, true),
                    "disabled" => editor.set_input_enabled(false, cx),
                    "attachment" => peer.attach(editor, "repeat"),
                    _ => unreachable!(),
                }
                if matches!(guard, "scope" | "composition" | "read-only" | "disabled") {
                    assert!(!request(editor));
                }
                assert!(!editor.apply_source_lsp_rename(result, cx), "{guard}");
                assert_eq!(editor.source(), SOURCE);
                assert_eq!(editor.cursor(), head);
                assert!(!editor.view.can_undo());
                match guard {
                    "scope" => editor.active = active,
                    "composition" => editor.marked = None,
                    "read-only" => set_read_only(editor, false),
                    "disabled" => editor.set_input_enabled(true, cx),
                    _ => {}
                }
            })
        });
    }
    peer.drive(&editor, cx);
    peer.detach(&editor, cx);
}

#[gpui::test]
fn rename_rejects_extension_rewrites_without_applying_any_part(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let mode = Arc::new(AtomicUsize::new(0));
    let filter = mode.clone();
    let extensions = transaction_filter(move |_, tx| {
        if tx.user_event() != Some("rename") {
            return Ok(tx);
        }
        Ok(match filter.load(Ordering::Relaxed) {
            0 => Transaction::new(ChangeSet::single(Change::new(0..4, "Other")).unwrap()),
            1 => tx.with_selection(EditorSelection::single(0, 0)),
            2 => tx.with_effects([Extension::default().append()]),
            3 => tx.with_add_to_history(false),
            4 => Transaction::new(tx.changes().clone())
                .with_user_event("rename")
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
                let result = editor.take_source_lsp_rename().unwrap();
                let head = editor.cursor();
                assert!(
                    !editor.apply_source_lsp_rename(result, cx),
                    "rewrite {index}"
                );
                assert_eq!(editor.source(), SOURCE);
                assert_eq!(editor.cursor(), head);
                assert!(!editor.view.can_undo());
                assert!(editor.source_fence().is_some());
            })
        });
    }
    peer.detach(&editor, cx);
}
