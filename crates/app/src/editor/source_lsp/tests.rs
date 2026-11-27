use std::{path::Path, process::Command, thread, time::Instant};

use gpui::{Entity, EntityInputHandler, Modifiers, TestAppContext, VisualTestContext};
use gpui_editor_codemirror::lsp::transport::Program;
use serde_json::{Map, json};
use tempfile::TempDir;

use super::*;

mod references;
mod signature;

const SOURCE: &str =
    "😀 id\r\n\r\nTail. [site](https://example.invalid)\r\n\r\n```rs\r\nlet x = 1;\r\n```\r\n";
const URI: &str = "file:///explicit.md";
const WAIT: Duration = Duration::from_secs(5);
const WORD: usize = "😀 ".len();
const LITERAL: &str = "[link](https://example.invalid) <script>ignored</script>\nlet x = 1;";
const POINT_OVERLAY: &str = "hover point";

struct Fixture(TempDir);

impl Fixture {
    fn new() -> Self {
        let fixture = Self(tempfile::tempdir().unwrap());
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/editor/source_lsp/fixture.rs");
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let output = Command::new(rustc)
            .arg("--edition=2024")
            .arg(source)
            .arg("-o")
            .arg(fixture.0.path().join("fixture"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        fixture
    }

    fn peer_panic(&self) -> String {
        std::fs::read_to_string(self.0.path().join("peer-panic")).unwrap_or_default()
    }

    fn start(&self, mode: &str) -> StdioClient {
        let program = Program::new(self.0.path().join("fixture"))
            .arg(mode)
            .arg(self.0.path().join("peer-panic"));
        let params = Map::from_iter([
            ("processId".into(), Value::Null),
            ("rootUri".into(), Value::Null),
            (
                "capabilities".into(),
                json!({"textDocument": {
                    "hover": {"contentFormat": ["plaintext"]},
                    "signatureHelp": {"contextSupport": true, "signatureInformation": {
                        "documentationFormat": ["plaintext"],
                        "parameterInformation": {"labelOffsetSupport": true},
                        "activeParameterSupport": true,
                    }},
                }}),
            ),
        ]);
        StdioClient::start(&program, params, WAIT).unwrap()
    }
}

fn drive(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
    mut done: impl FnMut(&Editor, Status) -> bool,
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
            "client did not reach the expected state"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn pending(editor: &Editor) -> bool {
    editor.source_lsp.as_ref().unwrap().pending.is_some()
}

fn focus(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        crate::editor::bind_keys(cx);
        window.activate_window();
        window.focus(&editor.read(cx).focus);
    });
    cx.run_until_parked();
}

fn assert_overlay(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: Option<&str>) {
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(editor.read(cx).prepainted_overlay.as_deref(), text));
}

#[gpui::test]
fn editor_sends_accepted_changes_before_hover_and_detaches_in_wire_order(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    focus(&editor, cx);
    let client = fixture.start("flow");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.poll_source_lsp(), Status::Detached);
            assert!(!editor.request_source_lsp_hover(WORD));
            editor.attach_source_lsp(client, URI, "markdown").unwrap();
            let preview = SOURCE.find("let x").unwrap();
            assert!(!editor.replace(preview..preview + 3, "var", cx));
            assert!(editor.replace(WORD..WORD + 2, "name", cx));
            assert!(!editor.request_source_lsp_hover(1));
            assert!(!editor.request_source_lsp_hover(editor.source().find('\n').unwrap()));
            assert!(!editor.request_source_lsp_hover(usize::MAX));
            assert!(editor.request_source_lsp_hover(WORD));
        })
    });
    drive(&editor, cx, |editor, status| {
        assert_ne!(status, Status::Disconnected, "{}", fixture.peer_panic());
        editor.source_lsp_hover().is_some()
    });
    assert_overlay(&editor, cx, Some(LITERAL));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(
                editor.source_lsp_hover(),
                Some(&HoverText {
                    range: WORD..WORD + 4,
                    text: LITERAL.into(),
                })
            );
            assert_eq!(editor.source(), SOURCE.replacen("id", "name", 1));
            assert_eq!(editor.view.undo_depth(), 1);
            assert!(editor.message.is_none());

            // A new position on the same revision supersedes the in-flight one.
            assert!(editor.request_source_lsp_hover(WORD + 1));
            assert!(editor.source_lsp_hover().is_none());
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            assert!(pending(editor));
            assert!(editor.request_source_lsp_hover(editor.source().find("Tail").unwrap()));
            cx.notify();
        })
    });
    assert_overlay(&editor, cx, None);
    drive(&editor, cx, |editor, status| {
        assert_eq!(status, Status::Ready);
        assert!(editor.source_lsp_hover().is_none());
        !pending(editor) && editor.source_lsp.as_ref().unwrap().wanted.is_none()
    });
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.request_source_lsp_hover(WORD));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            assert!(pending(editor));
            editor.replay_history(false, cx);
            assert_eq!(editor.source(), SOURCE);
        })
    });
    // Undo is an accepted change too. The fixture waits for it before sending
    // the older request's response. Do not publish that old revision.
    drive(&editor, cx, |editor, status| {
        assert_eq!(status, Status::Ready);
        assert!(editor.source_lsp_hover().is_none());
        !pending(editor)
    });
    let client = cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.request_source_lsp_hover(WORD));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            assert!(pending(editor));
            let client = editor.detach_source_lsp().unwrap();
            assert!(client.with_workspace(|workspace| workspace.files().is_empty()));
            assert_eq!(editor.poll_source_lsp(), Status::Detached);
            assert!(editor.source_lsp_hover().is_none());
            assert_eq!(editor.view.redo_depth(), 1);
            client
        })
    });
    assert!(client.shutdown(WAIT).unwrap().success());
}

#[gpui::test]
fn editor_does_not_query_an_outdated_snapshot_without_change_sync(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    focus(&editor, cx);
    let client = fixture.start("no-sync");
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor.attach_source_lsp(client, URI, "markdown").unwrap();
            assert!(editor.request_source_lsp_hover(WORD));
        })
    });
    drive(&editor, cx, |editor, status| {
        assert_ne!(status, Status::Disconnected, "{}", fixture.peer_panic());
        editor.source_lsp_hover().is_some()
    });
    assert_overlay(&editor, cx, Some("opening snapshot"));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.replace(WORD..WORD + 2, "new", cx));
            assert!(editor.source_lsp_hover().is_none());
        });
    });
    assert_overlay(&editor, cx, None);
    let client = cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.request_source_lsp_hover(WORD));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            assert!(!pending(editor));
            editor.detach_source_lsp().unwrap()
        })
    });
    // The fixture expects close immediately after the first hover, not a
    // didChange or a hover against its older document.
    assert!(client.shutdown(WAIT).unwrap().success());
}

#[gpui::test]
fn editor_removes_failed_and_disconnected_attachments(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    for mode in ["invalid-initialize", "oversized-peer-id", "disconnect"] {
        let client = fixture.start(mode);
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                editor.attach_source_lsp(client, URI, "markdown").unwrap();
                assert!(editor.request_source_lsp_hover(WORD));
            })
        });
        drive(&editor, cx, |editor, status| {
            assert!(editor.source_lsp_hover().is_none());
            status == Status::Disconnected
        });
        assert!(
            fixture.peer_panic().is_empty(),
            "{mode}: {}",
            fixture.peer_panic()
        );
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert_eq!(editor.poll_source_lsp(), Status::Detached);
                assert!(editor.detach_source_lsp().is_none());
                assert!(!editor.request_source_lsp_hover(WORD));
                assert_eq!(editor.source(), SOURCE);
                assert!(!editor.view.can_undo());
            })
        });
    }
}

fn request_and_receive(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
    fixture: &Fixture,
    offset: usize,
) {
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.request_source_lsp_hover(offset));
            cx.notify();
        });
    });
    drive(editor, cx, |editor, status| {
        assert_ne!(status, Status::Disconnected, "{}", fixture.peer_panic());
        editor.source_lsp_hover().is_some()
    });
}

#[gpui::test]
fn hover_presentation_respects_editor_geometry_and_dismissal(cx: &mut TestAppContext) {
    use crate::document::{BlockPreview, Section};

    let fixture = Fixture::new();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SOURCE.into(), window, cx));
    cx.simulate_resize(size(px(660.), px(380.)));
    focus(&editor, cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor
                .attach_source_lsp(fixture.start("presentation"), URI, "markdown")
                .unwrap();
        });
    });

    request_and_receive(&editor, cx, &fixture, WORD);
    assert_overlay(&editor, cx, Some(POINT_OVERLAY));
    let (link_hit, outside) = cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source_lsp_hover().unwrap().range, WORD..WORD);
        let layout = editor.layout.as_ref().unwrap();
        let viewport = editor.scroll.bounds();
        assert!(editor.overlay(layout, viewport, window, true).1.is_none());
        assert!(
            editor
                .source_lsp_tooltip(layout, viewport, window, None)
                .is_some()
        );
        let other_row = layout
            .bounds_at(SOURCE.find("Tail").unwrap(), false)
            .unwrap();
        assert!(
            editor
                .source_lsp_tooltip(layout, other_row, window, None)
                .is_none()
        );
        let row = layout.row_at(other_row.center());
        let item = row.items.iter().find(|item| item.target.is_some()).unwrap();
        (
            layout.origin + point(item.left + item.width / 2., row.y + row.height / 2.),
            viewport.bottom_right(),
        )
    });
    cx.simulate_mouse_move(link_hit, None, Modifiers::none());
    assert_overlay(&editor, cx, Some("https://example.invalid/"));
    cx.simulate_mouse_move(outside, None, Modifiers::none());
    assert_overlay(&editor, cx, Some(POINT_OVERLAY));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.message = Some("Editor notice".into());
            cx.notify();
        });
    });
    assert_overlay(&editor, cx, Some("Editor notice"));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.message = None;
            cx.notify();
        });
    });
    assert_overlay(&editor, cx, Some(POINT_OVERLAY));
    cx.simulate_keystrokes("escape");
    assert_overlay(&editor, cx, None);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.source_lsp_hover().is_none());
            assert!(editor.request_source_lsp_hover(WORD));
            assert_eq!(editor.poll_source_lsp(), Status::Ready);
            assert!(pending(editor));
            cx.notify();
        });
    });
    cx.simulate_keystrokes("escape");
    drive(&editor, cx, |editor, status| {
        assert_eq!(status, Status::Ready);
        assert!(editor.source_lsp_hover().is_none());
        !pending(editor)
    });
    assert_overlay(&editor, cx, None);

    let marker = SOURCE.find("```rs").unwrap() + 1;
    let body = SOURCE.find("let x").unwrap();
    request_and_receive(&editor, cx, &fixture, body);
    assert_overlay(&editor, cx, None);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let result = editor.current_source_lsp_hover().unwrap();
            assert_eq!(result.hover.range, body..body);
            assert!(!result.displayable, "active: {:?}", editor.active);
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
        });
    });
    assert_overlay(&editor, cx, None);
    request_and_receive(&editor, cx, &fixture, marker);
    assert_overlay(&editor, cx, None);
    request_and_receive(&editor, cx, &fixture, body);
    assert_overlay(&editor, cx, Some(POINT_OVERLAY));
    cx.deactivate_window();
    assert_overlay(&editor, cx, None);
    focus(&editor, cx);
    assert_overlay(&editor, cx, None);
    cx.update(|_, cx| assert!(editor.read(cx).source_lsp_hover().is_none()));

    request_and_receive(&editor, cx, &fixture, WORD);
    assert_overlay(&editor, cx, None); // Response range crosses rows.
    request_and_receive(&editor, cx, &fixture, WORD);
    assert_overlay(&editor, cx, Some(POINT_OVERLAY));
    let client = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_selection(body, body);
            editor.replace_and_mark_text_in_range(None, "語", None, window, cx);
            assert!(editor.is_composing());
            assert!(editor.source_lsp_hover().is_none());
            editor.detach_source_lsp().unwrap()
        })
    });
    assert_overlay(&editor, cx, None);
    assert!(client.shutdown(WAIT).unwrap().success());
}

#[test]
fn hover_preview_bounds_literal_text_and_neutralizes_controls() {
    assert_eq!(display_text(LITERAL).as_deref(), Some(LITERAL));
    assert_eq!(display_text(" \t\n"), None);
    assert_eq!(
        display_text("a\u{202e}b\u{1b}c\t d\n").as_deref(),
        Some("a�b�c  d\n")
    );
    let long = display_text(&"😀".repeat(MAX_DISPLAY_CHARS + 1)).unwrap();
    assert_eq!(long.chars().count(), MAX_DISPLAY_CHARS + 1);
    assert!(long.ends_with('…'));
    let lines = display_text(&"line\n".repeat(MAX_DISPLAY_LINES + 1)).unwrap();
    assert_eq!(lines.lines().count(), MAX_DISPLAY_LINES);
    assert!(lines.ends_with('…'));
}

#[test]
fn hover_limits_apply_before_decoding_server_text() {
    assert!(hover_fits(&json!({"contents":"a".repeat(MAX_HOVER_BYTES)})));
    assert!(!hover_fits(
        &json!({"contents":["a".repeat(MAX_HOVER_BYTES), ""]})
    ));
    assert!(!hover_fits(
        &json!({"contents":vec![""; MAX_HOVER_ITEMS + 1]})
    ));
    assert!(!hover_fits(
        &json!({"contents":{"language":"a".repeat(MAX_HOVER_BYTES),"value":"x"}})
    ));
    assert!(!hover_fits(&Value::Null));
}
