use std::{
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use gpui::{Context, Entity, Modifiers, TestAppContext, VisualTestContext};
use gpui_editor_codemirror::{
    lint::{DEFAULT_LINT_DELAY, Severity},
    lsp::transport::Program,
};
use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::{
    document::{BlockPreview, Section, SectionKind},
    editor::source_lsp::Status,
};

const URI: &str = "file:///explicit.md";
const SOURCE: &str =
    "Outside 😀\r\n\r\n```json\r\n[\"😀x\",]\r\n```\r\n\r\n```rust\r\nsecond\r\n```\r\n";
const WAIT: Duration = Duration::from_secs(5);
const LITERAL: &str = "[link](file:///do-not-open) <b>😀</b>�";

struct Peer(TempDir);

impl Peer {
    fn start(mode: &str) -> (Self, StdioClient) {
        let peer = Self(tempfile::tempdir().unwrap());
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/editor/source_lsp/diagnostics/fixture.rs");
        let executable = peer.0.path().join("fixture");
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let output = Command::new(rustc)
            .arg("--edition=2024")
            .arg(source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let program = Program::new(executable)
            .arg(mode)
            .arg(peer.0.path().join("error"));
        let init =
            json!({"capabilities":{"textDocument":{"publishDiagnostics":{"versionSupport":true}}}});
        let client = StdioClient::start(&program, init.as_object().unwrap().clone(), WAIT).unwrap();
        (peer, client)
    }

    fn error(&self) -> String {
        std::fs::read_to_string(self.0.path().join("error")).unwrap_or_default()
    }

    fn drive(
        &self,
        editor: &Entity<Editor>,
        cx: &mut VisualTestContext,
        done: impl Fn(&Editor, Status) -> bool,
    ) {
        let deadline = Instant::now() + WAIT;
        loop {
            if cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    let status = editor.poll_source_lsp();
                    cx.notify();
                    done(editor, status)
                })
            }) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "peer did not reach the expected state: {}",
                self.error()
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn ping(&self, editor: &Entity<Editor>, cx: &mut VisualTestContext, label: &str) {
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert!(editor.request_source_lsp_hover(0));
            })
        });
        self.drive(editor, cx, |editor, status| {
            assert_ne!(status, Status::Disconnected, "{}", self.error());
            editor
                .source_lsp_hover()
                .is_some_and(|result| result.text == label)
        });
    }
}

fn enter(editor: &mut Editor, index: usize, cx: &mut Context<Editor>) {
    let range = editor
        .markdown()
        .sections()
        .into_iter()
        .filter(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
        .nth(index)
        .unwrap()
        .source;
    editor.activate(
        Section::new(SectionKind::DraftCode, range.clone(), range),
        false,
        cx,
    );
}

#[gpui::test]
fn push_is_explicit_ordered_scoped_and_composes_with_local_marks(cx: &mut TestAppContext) {
    let (peer, client) = Peer::start("flow");
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    let executor = cx.update(|window, cx| {
        window.activate_window();
        window.focus(&editor.read(cx).focus);
        editor.update(cx, |editor, cx| {
            assert!(!editor.set_source_lsp_diagnostics(true));
            enter(editor, 0, cx);
            editor.attach_source_lsp(client, URI, "markdown").unwrap();
        });
        cx.background_executor().clone()
    });
    cx.run_until_parked();
    executor.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    cx.update(|_, cx| {
        let lsp = editor.read(cx).source_lsp.as_ref().unwrap();
        assert!(matches!(lsp.client.state(), LspState::Initializing(_)));
    });
    peer.ping(&editor, cx, "disabled");
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.source_lsp_diagnostics().is_empty());
            assert_eq!(editor.source_diagnostic_paint_marks().len(), 1);
            assert!(editor.set_source_lsp_diagnostics(true));
        })
    });
    peer.ping(&editor, cx, "accepted");
    peer.ping(&editor, cx, "invalid");
    cx.run_until_parked();
    let emoji = SOURCE.find("😀x").unwrap();
    let pointer = cx.update(|window, cx| {
        let editor = editor.read(cx);
        let batch = editor.source_lsp_diagnostics();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].range, emoji..emoji + "😀".len());
        assert_eq!(batch[0].message, LITERAL);
        assert_eq!(batch[0].severity, Severity::Warning);
        let point = emoji + "😀x".len();
        assert_eq!(batch[1].range, point..point);
        assert!(batch.iter().all(|diagnostic| diagnostic.actions.is_empty()));
        assert_eq!(editor.source_diagnostic_marks().len(), 1);
        assert_eq!(editor.source_diagnostic_paint_marks().len(), 3);
        assert_eq!(editor.source(), SOURCE);
        assert!(!editor.view.can_undo());
        assert!(editor.focus.is_focused(window));
        let bounds = editor
            .layout
            .as_ref()
            .unwrap()
            .bounds_at(emoji, false)
            .unwrap();
        gpui::point(bounds.left(), bounds.bottom() - gpui::px(3.))
    });
    cx.simulate_mouse_move(pointer, None, Modifiers::none());
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert_eq!(
            editor
                .source_diagnostic_tooltip(
                    editor.layout.as_ref().unwrap(),
                    editor.scroll.bounds(),
                    window
                )
                .unwrap()
                .text,
            LITERAL
        );
    });
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(
            editor
                .source_diagnostic_tooltip(
                    editor.layout.as_ref().unwrap(),
                    editor.scroll.bounds(),
                    window
                )
                .is_none()
        );
        assert_eq!(editor.source_diagnostic_paint_marks().len(), 3);
    });
    cx.update(|window, cx| {
        window.focus(&editor.read(cx).focus);
        editor.update(cx, |editor, cx| enter(editor, 1, cx));
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert!(editor.read(cx).source_lsp_diagnostics().is_empty()));
    peer.ping(&editor, cx, "second fence");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let batch = editor.source_lsp_diagnostics();
            assert_eq!(batch.len(), 1);
            assert_eq!(batch[0].message, "active fence");
            assert!(editor.source_diagnostic_marks().is_empty());
            assert_eq!(editor.source_diagnostic_paint_marks().len(), 1);
            enter(editor, 0, cx);
            assert!(editor.source_lsp_diagnostics().is_empty());
            editor.marked = Some(emoji..emoji);
            cx.notify();
        })
    });
    peer.ping(&editor, cx, "composing");
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.source_lsp_diagnostics().is_empty());
            editor.finish_composition();
            assert!(editor.source_lsp_diagnostics().is_empty());
        })
    });
    peer.ping(&editor, cx, "before edit");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source_lsp_diagnostics()[0].message, "before edit");
            assert!(editor.request_source_lsp_hover(0));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            let x = emoji + "😀".len();
            assert!(editor.replace(x..x + 1, "élan", cx));
            assert!(editor.source_lsp_diagnostics().is_empty());
            assert!(editor.request_source_lsp_hover(0));
        })
    });
    peer.drive(&editor, cx, |editor, status| {
        assert_ne!(status, Status::Disconnected, "{}", peer.error());
        assert!(
            editor
                .source_lsp_diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.message == "next revision")
        );
        editor
            .source_lsp_hover()
            .is_some_and(|result| result.text == "updated")
    });
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let batch = editor.source_lsp_diagnostics();
        assert_eq!(batch.len(), 1);
        let range = batch[0].range.clone();
        assert_eq!(&editor.source()[range], "élan");
        assert_eq!(batch[0].message, "next revision");
        assert_eq!(editor.view.undo_depth(), 1);
        assert_eq!(editor.source(), SOURCE.replacen('x', "élan", 1));
    });
    peer.ping(&editor, cx, "clear");
    cx.update(|_, cx| assert!(editor.read(cx).source_lsp_diagnostics().is_empty()));
    peer.ping(&editor, cx, "disable");
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert_eq!(editor.source_lsp_diagnostics()[0].message, "before disable");
            assert!(editor.set_source_lsp_diagnostics(false));
            assert!(editor.source_lsp_diagnostics().is_empty());
            assert!(editor.set_source_lsp_diagnostics(true));
            assert!(editor.source_lsp_diagnostics().is_empty());
        })
    });
    peer.ping(&editor, cx, "detach");
    let client = cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert_eq!(editor.source_lsp_diagnostics()[0].message, "before detach");
            let client = editor.detach_source_lsp().unwrap();
            assert!(editor.source_lsp_diagnostics().is_empty());
            assert!(client.with_workspace(|workspace| workspace.files().is_empty()));
            client
        })
    });
    assert!(client.shutdown(WAIT).unwrap().success(), "{}", peer.error());
}

#[gpui::test]
fn a_server_without_change_sync_cannot_republish_after_edit_and_eof_retires_the_attachment(
    cx: &mut TestAppContext,
) {
    let (peer, client) = Peer::start("no-sync");
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            enter(editor, 0, cx);
            editor.attach_source_lsp(client, URI, "markdown").unwrap();
            assert!(editor.set_source_lsp_diagnostics(true));
        })
    });
    peer.ping(&editor, cx, "accepted");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(
                editor.source_lsp_diagnostics()[0].message,
                "opening revision"
            );
            assert!(editor.request_source_lsp_hover(0));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            let x = SOURCE.find('x').unwrap();
            assert!(editor.replace(x..x + 1, "élan", cx));
            assert!(editor.source_lsp_diagnostics().is_empty());
        })
    });
    peer.drive(&editor, cx, |editor, status| {
        assert!(editor.source_lsp_diagnostics().is_empty());
        status == Status::Disconnected
    });
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert_eq!(editor.poll_source_lsp(), Status::Detached);
            assert!(!editor.set_source_lsp_diagnostics(true));
        })
    });
    assert!(peer.error().is_empty(), "{}", peer.error());
}
