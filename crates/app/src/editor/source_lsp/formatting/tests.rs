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

use gpui::{Entity, EntityInputHandler, TestAppContext, VisualTestContext};
use gpui_editor_codemirror::{Change, Extension, lsp::transport::Program, transaction_filter};
use serde_json::Map;
use tempfile::TempDir;

use super::super::Status;
use super::*;

const URI: &str = "file:///format-fixture.md";
const SOURCE: &str =
    "😀 id\r\n\r\nTail. [site](https://example.invalid)\r\n\r\n```rs\r\nlet x = 1;\r\n```\r\n";
const WORD: usize = "😀 ".len();
const WAIT: Duration = Duration::from_secs(5);
const SPACES: FormattingOptions = FormattingOptions {
    tab_size: 2,
    insert_spaces: true,
};

struct Peer(TempDir);

impl Peer {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/editor/source_lsp/formatting/fixture.rs");
        let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--edition=2024", "-Dwarnings"])
            .arg(source)
            .arg("-o")
            .arg(dir.path().join("peer"))
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        Self(dir)
    }

    fn error(&self) -> String {
        std::fs::read_to_string(self.0.path().join("panic")).unwrap_or_default()
    }

    fn start(&self, mode: &str) -> StdioClient {
        let program = Program::new(self.0.path().join("peer"))
            .arg(mode)
            .arg(self.0.path().join("panic"));
        StdioClient::start(&program, Map::new(), WAIT).unwrap()
    }

    fn shutdown(&self, client: StdioClient) {
        assert!(
            client.shutdown(WAIT).is_ok_and(|status| status.success()),
            "{}",
            self.error()
        );
    }
}

fn drive(editor: &Entity<Editor>, cx: &mut VisualTestContext, peer: &Peer) {
    let deadline = Instant::now() + WAIT;
    loop {
        let done = cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                let status = editor.poll_source_lsp();
                assert_ne!(status, Status::Disconnected, "{}", peer.error());
                let formatting = &editor.source_lsp.as_ref().unwrap().formatting;
                status == Status::Ready
                    && formatting.wanted.is_none()
                    && formatting.pending.is_none()
            })
        });
        if done {
            return;
        }
        assert!(Instant::now() < deadline, "{}", peer.error());
        thread::sleep(Duration::from_millis(1));
    }
}

fn detach(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> StdioClient {
    cx.update(|_, cx| editor.update(cx, |editor, _| editor.detach_source_lsp().unwrap()))
}

#[gpui::test]
fn whole_draft_flushes_before_format_and_applies_once_with_raw_endings(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let seen = Arc::new(AtomicUsize::new(0));
    let track = seen.clone();
    let extensions = transaction_filter(move |_, tx| {
        if tx.is_user_event("format") {
            assert_eq!(tx.history_isolation(), Some(HistoryIsolation::Full));
            track.fetch_add(1, Ordering::Relaxed);
        }
        Ok(tx)
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(SOURCE.into(), extensions, window, cx).unwrap()
    });
    let tabs = FormattingOptions {
        tab_size: 8,
        insert_spaces: false,
    };
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.request_source_lsp_formatting(tabs));
            editor
                .attach_source_lsp(peer.start("flow"), URI, "markdown")
                .unwrap();
            assert!(editor.replace(WORD..WORD + 2, "name", cx));
            assert!(editor.request_source_lsp_formatting(tabs));
            assert!(editor.take_source_lsp_formatting().is_none());
        })
    });
    drive(&editor, cx, &peer);
    let synced = SOURCE.replacen("id", "name", 1);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), synced);
            assert_eq!(editor.view.undo_depth(), 1);
            let result = editor.take_source_lsp_formatting().unwrap();
            assert_eq!(result.uri(), URI);
            let tail = synced.find("Tail").unwrap();
            assert_eq!(
                result
                    .changes()
                    .changes()
                    .map(|(range, text)| (range.clone(), text))
                    .collect::<Vec<_>>(),
                [(WORD..WORD + 4, "élan"), (tail..tail + 4, "Lead\r\nNext")]
            );
            assert!(editor.take_source_lsp_formatting().is_none());
            assert_eq!(editor.cursor(), WORD + "name".len());
            assert!(editor.apply_source_lsp_formatting(result, cx));
            assert_eq!(editor.cursor(), WORD + "élan".len());
            assert_eq!(
                editor.source(),
                synced
                    .replacen("name", "élan", 1)
                    .replacen("Tail", "Lead\r\nNext", 1)
            );
            assert_eq!(editor.view.undo_depth(), 2);
            assert_eq!(seen.load(Ordering::Relaxed), 1);
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            editor.replay_history(false, cx);
            assert_eq!(editor.source(), synced);
            assert_eq!(editor.view.undo_depth(), 1);
            assert_eq!(editor.view.redo_depth(), 1);
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
        })
    });
    peer.shutdown(detach(&editor, cx));
}

#[gpui::test]
fn malformed_empty_and_protected_format_results_are_discarded_as_batches(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor
                .attach_source_lsp(peer.start("bad-results"), URI, "markdown")
                .unwrap();
        })
    });
    for i in 0..11 {
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert!(editor.request_source_lsp_formatting(SPACES));
            })
        });
        drive(&editor, cx, &peer);
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert!(
                    editor.take_source_lsp_formatting().is_none(),
                    "response {i}"
                );
                assert_eq!(editor.source(), SOURCE);
                assert!(!editor.view.can_undo());
            })
        });
    }
    peer.shutdown(detach(&editor, cx));
}

#[gpui::test]
fn format_only_edits_the_body_of_the_active_code_fence(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    let options = FormattingOptions {
        tab_size: 4,
        ..SPACES
    };
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            let start = SOURCE.find("```rs").unwrap();
            let end = SOURCE.rfind("```").unwrap() + 3;
            editor.active = Some(Section::new(SectionKind::DraftCode, start..end, start..end));
            let head = SOURCE.find("let x").unwrap();
            editor.set_selection(head, head);
            editor
                .attach_source_lsp(peer.start("code"), URI, "markdown")
                .unwrap();
        })
    });
    for i in 0..5 {
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert!(editor.request_source_lsp_formatting(options))
            })
        });
        drive(&editor, cx, &peer);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                assert_eq!(editor.source(), SOURCE);
                let result = editor.take_source_lsp_formatting();
                if i < 4 {
                    assert!(result.is_none(), "response {i}");
                    assert!(!editor.view.can_undo());
                    return;
                }
                assert!(editor.apply_source_lsp_formatting(result.unwrap(), cx));
                assert_eq!(
                    editor.source(),
                    SOURCE.replace("let x = 1;", "\tlet x = 1;\r\nlet y = \"😀\";")
                );
                assert_eq!(editor.view.undo_depth(), 1);
                assert!(editor.source_fence().is_some());
                editor.replay_history(false, cx);
                assert_eq!(editor.source(), SOURCE);
            })
        });
    }
    peer.shutdown(detach(&editor, cx));
}

#[gpui::test]
fn formatting_requires_capability_and_a_synced_draft(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    for mode in ["unsupported", "missing", "no-sync"] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                editor
                    .attach_source_lsp(peer.start(mode), URI, "markdown")
                    .unwrap();
                assert!(!editor.request_source_lsp_formatting(FormattingOptions {
                    tab_size: 0,
                    ..SPACES
                }));
                assert!(!editor.request_source_lsp_formatting(FormattingOptions {
                    tab_size: u32::MAX,
                    ..SPACES
                }));
                assert!(editor.request_source_lsp_formatting(SPACES));
            })
        });
        drive(&editor, cx, &peer);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                if mode == "no-sync" {
                    assert!(editor.take_source_lsp_formatting().is_some());
                    assert!(editor.replace(WORD..WORD + 2, "new", cx));
                    assert!(editor.request_source_lsp_formatting(SPACES));
                    assert_eq!(editor.poll_source_lsp(), Status::Ready);
                    assert!(!editor.source_lsp.as_ref().unwrap().formatting.can_send());
                } else {
                    assert!(!editor.request_source_lsp_formatting(SPACES));
                }
                assert!(editor.take_source_lsp_formatting().is_none());
            })
        });
        peer.shutdown(detach(&editor, cx));
    }
}

#[gpui::test]
fn stale_wire_responses_and_taken_results_never_apply(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor
                .attach_source_lsp(peer.start("repeat"), URI, "markdown")
                .unwrap();
        })
    });
    drive(&editor, cx, &peer);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.request_source_lsp_formatting(SPACES));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            assert!(
                editor
                    .source_lsp
                    .as_ref()
                    .unwrap()
                    .formatting
                    .pending
                    .is_some()
            );
            assert!(editor.replace(
                SOURCE.find("Tail").unwrap()..SOURCE.find("Tail").unwrap() + 4,
                "New",
                cx
            ));
            editor.replay_history(false, cx);
            assert_eq!(editor.source(), SOURCE);
        })
    });
    drive(&editor, cx, &peer);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.take_source_lsp_formatting().is_none());
            assert!(editor.request_source_lsp_formatting(SPACES));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            let head = editor.cursor();
            editor.set_selection(WORD, WORD);
            editor.set_selection(head, head);
        })
    });
    drive(&editor, cx, &peer);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.take_source_lsp_formatting().is_none());
            assert!(editor.request_source_lsp_formatting(SPACES));
        })
    });
    drive(&editor, cx, &peer);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let result = editor.take_source_lsp_formatting().unwrap();
            let start = SOURCE.find("```rs").unwrap();
            let end = SOURCE.rfind("```").unwrap() + 3;
            editor.active = Some(Section::new(SectionKind::DraftCode, start..end, start..end));
            assert!(!editor.apply_source_lsp_formatting(result, cx));
            editor.active = None;
            assert!(editor.request_source_lsp_formatting(SPACES));
        })
    });
    drive(&editor, cx, &peer);
    let result = cx.update(|_, cx| {
        editor.update(cx, |editor, _| editor.take_source_lsp_formatting().unwrap())
    });
    peer.shutdown(detach(&editor, cx));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor
                .attach_source_lsp(peer.start("repeat"), URI, "markdown")
                .unwrap();
            assert!(editor.request_source_lsp_formatting(SPACES));
            assert!(!editor.apply_source_lsp_formatting(result, cx));
            assert_eq!(editor.source(), SOURCE);
            assert!(!editor.view.can_undo());
        })
    });
    drive(&editor, cx, &peer);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let result = editor.take_source_lsp_formatting().unwrap();
            editor.replace_and_mark_text_in_range(None, "中", None, window, cx);
            let composed = editor.source().to_owned();
            assert!(editor.is_composing());
            assert!(!editor.request_source_lsp_formatting(SPACES));
            assert!(!editor.apply_source_lsp_formatting(result, cx));
            assert!(editor.is_composing());
            assert_eq!(editor.source(), composed);
        })
    });
    peer.shutdown(detach(&editor, cx));
}

#[gpui::test]
fn native_filter_cannot_redirect_a_format_edit(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let extensions: Extension = transaction_filter(|_, tx| {
        if tx.is_user_event("format") {
            let at = SOURCE.find("```rs").unwrap();
            return Ok(Transaction::new(
                ChangeSet::single(Change::new(at..at + 3, "bad")).unwrap(),
            ));
        }
        Ok(tx)
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(SOURCE.into(), extensions, window, cx).unwrap()
    });
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor
                .attach_source_lsp(peer.start("repeat"), URI, "markdown")
                .unwrap();
            assert!(editor.request_source_lsp_formatting(SPACES));
        })
    });
    drive(&editor, cx, &peer);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let result = editor.take_source_lsp_formatting().unwrap();
            assert!(!editor.apply_source_lsp_formatting(result, cx));
            assert_eq!(editor.source(), SOURCE);
            assert!(!editor.view.can_undo());
        })
    });
    peer.shutdown(detach(&editor, cx));
}
