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
    Change, ChangeSet, Compartment, Extension, READ_ONLY, language::fold::fold_effect,
    lsp::transport::Program, transaction_filter, view::scroll_handler,
};
use serde_json::Map;
use tempfile::TempDir;

use super::super::Status;
use super::*;
use crate::document::SectionKind;

const URI: &str = "file:///navigation-fixture.md";
const SOURCE: &str = "Lead\r\n\r\n```rs\r\nfn demo() {\r\n    // 😀 target one\r\n    first();\r\n}\r\n```\r\n\r\n```rs\r\nother();\r\n```\r\n";
const WAIT: Duration = Duration::from_secs(5);

struct Peer(TempDir);

impl Peer {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/editor/source_lsp/navigation/fixture.rs");
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
                    let navigation = &editor.source_lsp.as_ref().unwrap().navigation;
                    status == Status::Ready
                        && navigation.wanted.is_none()
                        && navigation.pending.is_none()
                })
            }) {
                return;
            }
            assert!(Instant::now() < deadline, "{}", self.error());
            thread::sleep(Duration::from_millis(1));
        }
    }
}

fn in_fence(editor: &mut Editor) {
    let source = editor.source();
    let start = source.find("```rs").unwrap();
    let end = start + source[start..].find("```\r\n\r\n").unwrap() + 3;
    let anchor = source.find('😀').unwrap();
    let head = source.find("target").unwrap();
    editor.active = Some(Section::new(SectionKind::DraftCode, start..end, start..end));
    editor.set_selection(anchor, head);
}

fn fold_around_target(editor: &mut Editor) -> Range<usize> {
    let start = editor.source().find("    first").unwrap();
    let range = start..start + "    first();".len();
    let active = editor.active.clone();
    editor
        .view
        .update(
            Transaction::default()
                .with_effects([fold_effect().of(range.clone())])
                .with_add_to_history(false),
            active.clone(),
            |_| (None, active),
        )
        .unwrap();
    assert_eq!(editor.source_fold_ranges(), std::slice::from_ref(&range));
    range
}

#[gpui::test]
fn navigation_uses_all_four_methods_and_applies_first_target_without_history(
    cx: &mut TestAppContext,
) {
    let peer = Peer::new();
    let revealed = Arc::new(AtomicUsize::new(0));
    let scroll = revealed.clone();
    let expected = SOURCE.replacen("Lead", "New lead", 1);
    let target = expected.find("first").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            SOURCE.into(),
            scroll_handler(move |_, at| {
                assert_eq!(at.range.head(), target);
                scroll.fetch_add(1, Ordering::Relaxed);
                true
            }),
            window,
            cx,
        )
        .unwrap()
    });
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.request_source_lsp_navigation(NavigationKind::Definition));
            peer.attach(editor, "flow");
            assert!(!editor.request_source_lsp_navigation(NavigationKind::Definition));
            assert!(editor.replace(0..4, "New lead", cx));
            in_fence(editor);
        })
    });
    for kind in [
        NavigationKind::Definition,
        NavigationKind::Declaration,
        NavigationKind::TypeDefinition,
        NavigationKind::Implementation,
    ] {
        let before = cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                in_fence(editor);
                fold_around_target(editor);
                assert!(editor.request_source_lsp_navigation(kind));
                assert!(editor.take_source_lsp_navigation().is_none());
                editor.view.state().selection().clone()
            })
        });
        peer.drive(&editor, cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                assert_eq!(editor.source(), expected);
                assert_eq!(editor.view.state().selection(), &before);
                assert_eq!(editor.view.undo_depth(), 1);
                assert_eq!(editor.source_fold_ranges().len(), 1);
                let result = editor.take_source_lsp_navigation().unwrap();
                assert_eq!(result.kind(), kind);
                assert_eq!(result.range(), &(target..target + 5));
                assert!(editor.take_source_lsp_navigation().is_none());
                assert_eq!(editor.view.state().selection(), &before);
                assert!(editor.apply_source_lsp_navigation(result, cx));
                assert!(editor.source_fold_ranges().is_empty());
                assert_eq!(editor.selection(), target..target);
                assert_eq!(editor.reveal_source.unwrap().offset, target);
                assert!(editor.source_fence().is_some());
                assert_eq!(editor.source(), expected);
                assert_eq!(editor.view.undo_depth(), 1);
                assert_eq!(editor.view.redo_depth(), 0);
                assert!(!editor.view.can_undo_selection());
                assert_eq!(editor.poll_source_lsp(), Status::Ready);
            })
        });
        cx.run_until_parked();
    }
    assert_eq!(revealed.load(Ordering::Relaxed), 4);
    peer.detach(&editor, cx);
}

#[gpui::test]
fn navigation_rejects_foreign_malformed_and_protected_results(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            peer.attach(editor, "bad-results");
            in_fence(editor);
            let line = editor.source().find("// 😀").unwrap();
            let head = editor.cursor();
            let crlf_interior = line + editor.source()[line..].find('\n').unwrap();
            editor.set_selection(crlf_interior, crlf_interior);
            assert!(!editor.request_source_lsp_navigation(NavigationKind::Definition));
            let header = editor.active.as_ref().unwrap().source.start;
            editor.set_selection(header, header);
            assert!(!editor.request_source_lsp_navigation(NavigationKind::Definition));
            editor.set_selection(head, head);
        })
    });
    for i in 0..15 {
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert!(editor.request_source_lsp_navigation(NavigationKind::Definition));
            })
        });
        peer.drive(&editor, cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert!(
                    editor.take_source_lsp_navigation().is_none(),
                    "response {i}"
                );
                assert_eq!(editor.source(), SOURCE);
                assert_eq!(editor.cursor(), SOURCE.find("target").unwrap());
                assert!(editor.reveal_source.is_none());
                assert!(!editor.view.can_undo_selection());
            })
        });
    }
    peer.detach(&editor, cx);
}

#[gpui::test]
fn navigation_discards_unsupported_and_unsynchronized_requests(cx: &mut TestAppContext) {
    let peer = Peer::new();
    for mode in ["unsupported", "no-sync"] {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                peer.attach(editor, mode);
                in_fence(editor);
                assert!(editor.request_source_lsp_navigation(NavigationKind::Definition));
            })
        });
        peer.drive(&editor, cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                assert_eq!(
                    editor.take_source_lsp_navigation().is_some(),
                    mode == "no-sync"
                );
                if mode == "no-sync" {
                    editor.active = None;
                    assert!(editor.replace(0..4, "New lead", cx));
                    in_fence(editor);
                    assert!(editor.request_source_lsp_navigation(NavigationKind::Definition));
                }
            })
        });
        peer.drive(&editor, cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert!(editor.take_source_lsp_navigation().is_none());
            })
        });
        peer.detach(&editor, cx);
    }
}

#[gpui::test]
fn navigation_drops_stale_wire_replies_and_taken_results(cx: &mut TestAppContext) {
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
            in_fence(editor);
        })
    });
    peer.drive(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.request_source_lsp_navigation(NavigationKind::Definition));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            assert!(
                editor
                    .source_lsp
                    .as_ref()
                    .unwrap()
                    .navigation
                    .pending
                    .is_some()
            );
            let first = editor.source().find("first").unwrap();
            assert!(editor.replace(first..first + 5, "second", cx));
            editor.replay_history(false, cx);
            assert_eq!(editor.source(), SOURCE);
            in_fence(editor);
        })
    });
    peer.drive(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.take_source_lsp_navigation().is_none());
            assert!(editor.request_source_lsp_navigation(NavigationKind::Definition));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            let head = editor.cursor();
            editor.set_selection(head, head);
            in_fence(editor);
        })
    });
    peer.drive(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.take_source_lsp_navigation().is_none());
        })
    });
    for guard in [
        "scope",
        "new query",
        "IME",
        "read-only",
        "disabled",
        "attachment",
    ] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert!(editor.request_source_lsp_navigation(NavigationKind::Definition));
            })
        });
        peer.drive(&editor, cx);
        let result = cx.update(|_, cx| {
            editor.update(cx, |editor, _| editor.take_source_lsp_navigation().unwrap())
        });
        if guard == "attachment" {
            peer.detach(&editor, cx);
        }
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let active = editor.active.clone();
                let head = editor.cursor();
                let before = editor.view.state().selection().clone();
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
                    "new query" => {
                        assert!(editor.request_source_lsp_navigation(NavigationKind::Definition))
                    }
                    "IME" => editor.marked = Some(head..head),
                    "read-only" => set_read_only(editor, true),
                    "disabled" => editor.set_input_enabled(false, cx),
                    "attachment" => peer.attach(editor, "repeat"),
                    _ => unreachable!(),
                }
                if matches!(guard, "scope" | "IME" | "read-only" | "disabled") {
                    assert!(!editor.request_source_lsp_navigation(NavigationKind::Definition));
                }
                assert!(!editor.apply_source_lsp_navigation(result, cx), "{guard}");
                assert_eq!(editor.view.state().selection(), &before);
                assert_eq!(editor.source(), SOURCE);
                match guard {
                    "scope" => editor.active = active,
                    "IME" => editor.marked = None,
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
fn navigation_rejects_extension_rewrites_without_opening_folds(cx: &mut TestAppContext) {
    let peer = Peer::new();
    let mode = Arc::new(AtomicUsize::new(0));
    let filter = mode.clone();
    let extensions = transaction_filter(move |_, tx| {
        if !tx.is_user_event("select.definition") {
            return Ok(tx);
        }
        Ok(match filter.load(Ordering::Relaxed) {
            0 => tx.with_selection(EditorSelection::single(0, 0)),
            1 => Transaction::new(ChangeSet::single(Change::new(0..4, "Other")).unwrap())
                .with_add_to_history(false),
            2 => Transaction::default().with_selection(tx.selection().unwrap().clone()),
            3 => tx.with_effects([Extension::default().append()]),
            4 => tx.with_scroll_into_view(),
            _ => unreachable!(),
        })
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(SOURCE.into(), extensions, window, cx).unwrap()
    });
    let folded = cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            peer.attach(editor, "repeat");
            in_fence(editor);
            fold_around_target(editor)
        })
    });
    for i in 0..5 {
        mode.store(i, Ordering::Relaxed);
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert!(editor.request_source_lsp_navigation(NavigationKind::Definition));
            })
        });
        peer.drive(&editor, cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let before = editor.view.state().selection().clone();
                let result = editor.take_source_lsp_navigation().unwrap();
                assert!(
                    !editor.apply_source_lsp_navigation(result, cx),
                    "rewrite {i}"
                );
                assert_eq!(editor.source(), SOURCE);
                assert_eq!(editor.view.state().selection(), &before);
                assert!(!editor.view.can_undo_selection());
                assert_eq!(editor.source_fold_ranges(), std::slice::from_ref(&folded));
                assert!(editor.reveal_source.is_none());
            })
        });
    }
    peer.detach(&editor, cx);
}
