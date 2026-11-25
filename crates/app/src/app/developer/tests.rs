use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    thread,
    time::Instant,
};

use gpui::{
    EntityInputHandler, Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase,
    VisualTestContext, point, size,
};

use serde_json::{Value, json};

use super::*;
use crate::workspace::Workspace;

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing {selector}"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn enable(cx: &mut VisualTestContext, shell: &Entity<Shell>) {
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| shell.developer_preferences(window, cx));
    });
    click(cx, "Developer Mode: Off");
    click(cx, "Done");
}

fn request_pair(shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
    shell.developer_event(
        Event::Pair {
            id: 1,
            project: "Test project".into(),
            session: "Test session".into(),
            code: "123456".into(),
        },
        window,
        cx,
    );
}

// Exercise the UI with controlled connection events. Bridge tests cover the actual socket.
fn paired(cx: &mut VisualTestContext, shell: &Entity<Shell>) {
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| request_pair(shell, window, cx));
    });
    click(cx, "Pair");
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.developer_event(Event::Paired { id: 1 }, window, cx)
        });
    });
}

#[gpui::test]
fn developer_mode_requires_approval_and_can_be_disabled(cx: &mut TestAppContext) {
    let (shell, cx) = cx.add_window_view(Shell::new);
    assert!(cx.debug_bounds("developer-picker").is_none());
    enable(cx, &shell);
    click(cx, "developer-picker");
    cx.update(|_, cx| assert!(!shell.read(cx).developer.picking));
    cx.update(|window, cx| shell.update(cx, |shell, cx| request_pair(shell, window, cx)));
    assert!(cx.debug_bounds("Pair with Pi?").is_some());
    click(cx, "Decline");
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            // A reply that arrives after the request closes cannot re-enable the icon.
            shell.developer_event(Event::Paired { id: 1 }, window, cx);
            assert!(!shell.developer.connected());
        });
    });
    paired(cx, &shell);
    click(cx, "developer-picker");
    cx.update(|_, cx| assert!(shell.read(cx).developer.picking));
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| shell.update(cx, |shell, cx| shell.developer_preferences(window, cx)));
    click(cx, "Developer Mode: On");
    click(cx, "Done");
    cx.update(|_, cx| {
        shell.update(cx, |shell, cx| {
            assert!(shell.developer.bridge.is_none());
            assert!(!shell.developer.connected());
            assert!(shell.developer_icon(true, cx).is_none());
        })
    });
}

#[gpui::test]
fn picker_selects_wrapped_controls_and_scrolled_rows_without_activating_them(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(directory.path()).unwrap();
    for index in 0..40 {
        fs::write(
            workspace.root().join(format!("file-{index:02}.md")),
            "original",
        )
        .unwrap();
    }
    let file = workspace
        .read_document(&workspace.root().join("file-00.md"))
        .unwrap();
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.files = workspace.markdown_files().unwrap();
            shell.workspace = Some(workspace.clone());
            shell.open_editor(Some(file), window, cx);
            let editor = &shell.current.as_ref().unwrap().editor;
            editor.update(cx, |editor, cx| {
                editor.replace_text_in_range(Some(0..8), "draft", window, cx)
            });
            cx.focus_view(editor, window);
            cx.notify();
        })
    });
    cx.simulate_resize(size(px(560.), px(360.)));
    enable(cx, &shell);
    paired(cx, &shell);
    let open = cx.debug_bounds("Open Folder…").unwrap();
    let picker = cx.debug_bounds("developer-picker").unwrap();
    assert_eq!(picker.top(), open.top());
    assert!(picker.size.width >= px(100.));
    assert!(picker.right() <= px(560.));
    let file = workspace.root().join("file-00.md");
    for (button, target) in [("Save", "Save"), ("More tools", "More tools")] {
        click(cx, "developer-picker");
        click(cx, button);
        cx.update(|_, cx| {
            let shell = shell.read(cx);
            assert_eq!(shell.developer.draft.as_ref().unwrap().target.id, target);
            assert!(!shell.show_more_tools);
            assert!(shell.current.as_ref().unwrap().dirty(cx));
        });
        click(cx, "Cancel");
    }
    click(cx, "developer-picker");
    let position = point(px(45.), px(285.));
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.), px(-260.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    cx.simulate_click(position, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert_eq!(
            shell.developer.draft.as_ref().unwrap().target.id,
            "sidebar.file"
        );
        assert_eq!(
            shell
                .current
                .as_ref()
                .unwrap()
                .file
                .as_ref()
                .unwrap()
                .path(),
            file
        );
    });
    assert_eq!(fs::read_to_string(&file).unwrap(), "original");
}

#[gpui::test]
fn a_disconnected_draft_survives_until_explicitly_canceled(cx: &mut TestAppContext) {
    let (shell, cx) = cx.add_window_view(Shell::new);
    enable(cx, &shell);
    paired(cx, &shell);
    click(cx, "developer-picker");
    click(cx, "New note");
    cx.simulate_input("keep this draft\n文字");
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.developer_event(Event::Disconnected { id: 1 }, window, cx);
            assert!(shell.current.is_none());
            assert_eq!(
                shell.developer.draft.as_ref().unwrap().text.read(cx).text(),
                "keep this draft\n文字"
            );
        })
    });
    click(cx, "Send to Pi");
    cx.update(|_, cx| assert!(shell.read(cx).developer.draft.is_some()));
    click(cx, "Cancel");
    cx.update(|_, cx| assert!(shell.read(cx).developer.draft.is_none()));
}

fn send(client: &mut BufReader<UnixStream>, value: Value) {
    writeln!(client.get_mut(), "{value}").unwrap();
}

fn receive(client: &mut BufReader<UnixStream>) -> Value {
    let mut line = String::new();
    client.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn apply_event(shell: &Entity<Shell>, cx: &mut VisualTestContext) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                let event = shell.developer.bridge.as_ref().unwrap().try_event();
                if let Some(event) = event {
                    shell.developer_event(event, window, cx);
                    true
                } else {
                    false
                }
            })
        }) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out waiting for editor bridge event"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[gpui::test]
fn only_explicit_send_delivers_feedback_to_the_approved_socket(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir_in("/tmp").unwrap();
    let bridge = Bridge::listen(directory.path()).unwrap();
    let instance = fs::read_dir(directory.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let socket = UnixStream::connect(instance.join("s")).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut client = BufReader::new(socket);
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.update(|_, cx| {
        shell.update(cx, |shell, cx| {
            shell.developer.bridge = Some(bridge);
            cx.notify();
        })
    });
    send(
        &mut client,
        json!({"type":"pair", "version":1, "project":"test", "session":"test", "code":"012345"}),
    );
    apply_event(&shell, cx);
    click(cx, "Pair");
    assert_eq!(receive(&mut client)["type"], "paired");
    apply_event(&shell, cx);
    click(cx, "developer-picker");
    click(cx, "New note");
    cx.simulate_input("do not send");
    click(cx, "Cancel");
    let workspace = Workspace::open(directory.path()).unwrap();
    let preview_file = workspace.root().join("diagnostic-private-name.md");
    let confidential = "diagnostic-private-body";
    fs::write(&preview_file, format!("```mermaid\n{confidential}\n```\n")).unwrap();
    let file = workspace.read_document(&preview_file).unwrap();
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_editor(Some(file), window, cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    send(&mut client, json!({"type":"logs", "id":1, "limit":10}));
    let logs = receive(&mut client);
    assert_eq!(logs["type"], "logs");
    let logs = logs["entries"].to_string();
    assert!(logs.contains("Mermaid preview failed"));
    assert!(!logs.contains("diagnostic-private"));
    assert!(!logs.contains(workspace.root().to_str().unwrap()));
    click(cx, "developer-picker");
    click(cx, "New note");
    cx.simulate_input("  test feedback\n文字  ");
    click(cx, "Capture Window");
    cx.update(|_, cx| {
        let draft = shell.read(cx).developer.draft.as_ref().unwrap();
        assert!(draft.notice.is_some());
        assert!(!draft.loading);
    });
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(1, 1, image::Rgba([15, 30, 45, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let image = gpui::Image::from_bytes(gpui::ImageFormat::Png, png.into_inner());
    cx.cx
        .write_to_clipboard(gpui::ClipboardItem::new_image(&image));
    click(cx, "Paste Image");
    click(cx, "Remove Image");
    cx.update(|_, cx| {
        assert!(
            shell
                .read(cx)
                .developer
                .draft
                .as_ref()
                .unwrap()
                .picture
                .is_none()
        )
    });
    click(cx, "Paste Image");
    click(cx, "Send to Pi");
    let feedback = receive(&mut client);
    cx.simulate_input("not in the sent snapshot");
    cx.simulate_keystrokes("cmd-a backspace cmd-v");
    cx.update(|window, cx| {
        let text = shell
            .read(cx)
            .developer
            .draft
            .as_ref()
            .unwrap()
            .text
            .clone();
        text.update(cx, |text, cx| {
            // A platform input callback can arrive before the view redraws.
            text.replace_text_in_range(Some(0..1), "late input", window, cx);
            text.replace_and_mark_text_in_range(None, "未確認", None, window, cx);
            assert_eq!(text.text(), "  test feedback\n文字  ");
            assert!(!text.is_composing());
        });
    });
    assert_eq!(feedback["type"], "feedback");
    assert_eq!(feedback["target"]["id"], "New note");
    assert_eq!(feedback["text"], "  test feedback\n文字  ");
    assert_eq!(feedback["image"]["mimeType"], "image/png");
    assert!(
        feedback["image"]["data"]
            .as_str()
            .unwrap()
            .starts_with("iVBORw0K")
    );
    send(
        &mut client,
        json!({"type":"receipt", "id":feedback["id"], "accepted":false}),
    );
    apply_event(&shell, cx);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("edited after rejection");
    click(cx, "Send to Pi");
    let feedback = receive(&mut client);
    assert_eq!(feedback["text"], "edited after rejection");
    send(
        &mut client,
        json!({"type":"receipt", "id":feedback["id"], "accepted":true}),
    );
    apply_event(&shell, cx);
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert_eq!(
            shell
                .current
                .as_ref()
                .unwrap()
                .file
                .as_ref()
                .unwrap()
                .path(),
            preview_file
        );
        assert!(shell.developer.draft.is_none());
    });
    click(cx, "developer-picker");
    click(cx, "New note");
    cx.simulate_input("draft before disconnect");
    click(cx, "Send to Pi");
    assert_eq!(receive(&mut client)["text"], "draft before disconnect");
    cx.update(|window, cx| shell.update(cx, |shell, cx| shell.unpair(window, cx)));
    assert_eq!(client.read_line(&mut String::new()).unwrap(), 0);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("edited after disconnect");
    cx.update(|_, cx| {
        let draft = shell.read(cx).developer.draft.as_ref().unwrap();
        assert_eq!(draft.text.read(cx).text(), "edited after disconnect");
        assert!(draft.delivery.is_none());
    });
}
