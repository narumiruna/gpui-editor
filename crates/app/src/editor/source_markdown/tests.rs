use gpui::{ClipboardItem, TestAppContext};
use gpui_editor_codemirror::{EditorSelection, SelectionRange};

use super::*;
use crate::editor::bind_keys;

#[gpui::test]
fn pasting_a_destination_keeps_the_selected_text_and_undo(cx: &mut TestAppContext) {
    let source = "Before 字 guide\r\n\r\nAfter.";
    let from = source.find('字').unwrap();
    let to = from + "字 guide".len();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        cx.write_to_clipboard(ClipboardItem::new_string("www.example.test/path".into()));
        editor.update(cx, |editor, _| {
            editor.set_selection(to, from);
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("secondary-v");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(
            editor.source(),
            "Before [字 guide](https://www.example.test/path)\r\n\r\nAfter."
        );
        assert_eq!(editor.cursor(), from + 1);
        assert_eq!(editor.anchor(), to + 1);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.cursor(), from);
        assert_eq!(editor.anchor(), to);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn link_paste_wraps_only_the_primary_selection(cx: &mut TestAppContext) {
    let source = "猫 β😀 end\r\n";
    let start = source.find('β').unwrap();
    let end = start + "β😀".len();
    let tail = source.find("end").unwrap();
    let selection = |start, end, tail| {
        EditorSelection::new(
            vec![
                SelectionRange::new(0, '猫'.len_utf8()),
                SelectionRange::new(end, start),
                SelectionRange::new(tail, tail),
            ],
            1,
        )
        .unwrap()
    };
    let before = selection(start, end, tail);
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        cx.write_to_clipboard(ClipboardItem::new_string("https://example.test/".into()));
        editor.update(cx, |editor, _| {
            editor.view.set_selection(before.clone()).unwrap();
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("secondary-v");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let expected = "猫 [β😀](https://example.test/) end\r\n";
        assert_eq!(editor.source(), expected);
        assert_eq!(
            editor.view.state().selection(),
            &selection(start + 1, end + 1, expected.find("end").unwrap())
        );
        assert_eq!(editor.view.undo_depth(), 1);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.view.state().selection(), &before);
    });
}

#[gpui::test]
fn an_empty_primary_range_uses_normal_multi_range_paste(cx: &mut TestAppContext) {
    let source = "one two";
    let before = EditorSelection::new(
        vec![SelectionRange::new(0, 3), SelectionRange::new(4, 4)],
        1,
    )
    .unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        cx.write_to_clipboard(ClipboardItem::new_string("www.example.test".into()));
        editor.update(cx, |editor, _| {
            editor.view.set_selection(before.clone()).unwrap();
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("secondary-v");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), "www.example.test www.example.testtwo");
        assert_eq!(editor.view.undo_depth(), 1);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.view.state().selection(), &before);
    });
}

#[gpui::test]
fn markdown_link_paste_uses_the_filtered_destination(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::{Transaction, view::clipboard_input_filter};

    let source = "a guide\r\n";
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        cx.write_to_clipboard(ClipboardItem::new_string("shortcut".into()));
        editor.update(cx, |editor, _| {
            let extension = clipboard_input_filter(|text, _| {
                assert_eq!(text, "shortcut");
                "https://example.test".to_owned()
            });
            editor
                .view
                .update(
                    Transaction::default().with_effects([extension.append()]),
                    None,
                    |_| (None, None),
                )
                .unwrap();
            editor.set_selection(2, 7);
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("secondary-v");
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).source(),
            "a [guide](https://example.test)\r\n"
        )
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
}

#[gpui::test]
fn pasting_plain_text_or_into_protected_syntax_does_not_make_a_link(cx: &mut TestAppContext) {
    let source = "Before guide\n\n```js\nlet value\n```\n";
    let from = source.find("guide").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        cx.write_to_clipboard(ClipboardItem::new_string("plain".into()));
        editor.update(cx, |editor, _| {
            editor.set_selection(from, from + 5);
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("secondary-v");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source.replace("guide", "plain")));
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string("https://example.test".into()));
        editor.update(cx, |editor, _| {
            let start = source.find("value").unwrap();
            editor.set_selection(start, start + 5);
        });
    });
    cx.simulate_keystrokes("secondary-v");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn enter_continues_a_bullet_without_normalizing_line_endings(cx: &mut TestAppContext) {
    let source = "- one\r\n- two";
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, _| {
            editor.set_selection(5, 5);
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("enter");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), "- one\r\n- \r\n- two");
        assert_eq!(editor.cursor(), 9);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
}

#[gpui::test]
fn markup_keys_follow_lone_cr_lines_without_rewriting_them(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::language::markdown::{MarkdownConfig, markdown};

    for (source, head, key, expected, caret) in [
        (
            "lead\r- alpha\r- beta",
            12,
            "enter",
            "lead\r- alpha\r- \r- beta",
            15,
        ),
        (
            "lead\r- one\r- two",
            13,
            "backspace",
            "lead\r- one\r  two",
            13,
        ),
        (
            "lead\r> one\r> two",
            13,
            "backspace",
            "lead\r> one\r> two",
            13,
        ),
    ] {
        let extension = markdown(MarkdownConfig {
            add_keymap: true,
            ..Default::default()
        });
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(source.into(), extension, window, cx).unwrap()
        });
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, _| {
                editor.set_selection(head, head);
                window.focus(&editor.focus);
            });
        });
        cx.simulate_keystrokes(key);
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), expected, "{key}");
            assert_eq!(editor.cursor(), caret, "{key}");
            assert_eq!(editor.view.undo_depth(), usize::from(source != expected));
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert_eq!(editor.cursor(), head);
        });
    }
}

fn carets(first: usize, second: usize) -> EditorSelection {
    EditorSelection::new(
        vec![
            SelectionRange::new(first, first),
            SelectionRange::new(second, second),
        ],
        1,
    )
    .unwrap()
}

#[gpui::test]
fn two_markdown_carets_continue_or_remove_markup_in_one_undo_step(cx: &mut TestAppContext) {
    for (source, selected, key, expected) in [
        (
            "- 猫\r\n- 犬\r\n\r\nOutside",
            carets(5, 12),
            "enter",
            "- 猫\r\n- \r\n- 犬\r\n- \r\n\r\nOutside",
        ),
        (
            "- 猫\r\n-    犬\r\n\r\nOutside",
            carets(2, 12),
            "backspace",
            "猫\r\n- 犬\r\n\r\nOutside",
        ),
    ] {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, _| {
                editor.view.set_selection(selected.clone()).unwrap();
                window.focus(&editor.focus);
            });
        });
        cx.simulate_keystrokes(key);
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), expected, "{key}");
            assert_eq!(editor.view.undo_depth(), 1);
            assert_eq!(editor.view.state().selection().ranges().len(), 2);
            assert_eq!(editor.view.state().selection().main_index(), 1);
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selected);
            assert!(!editor.view.can_undo());
        });
    }
}

#[gpui::test]
fn a_caret_in_a_protected_preview_rejects_a_multi_caret_markup_edit(cx: &mut TestAppContext) {
    let source = "- 猫\n\n```json\n[1]\n```\n";
    let selected = carets(source.find('\n').unwrap(), source.find('1').unwrap());
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, _| {
            editor.view.set_selection(selected.clone()).unwrap();
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("enter");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.view.state().selection(), &selected);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn configured_markdown_routes_keys_and_paste_without_changing_protection(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::language::markdown::{MarkdownConfig, markdown};

    let source = "- 猫\r\n- 犬\r\n\r\nlabel\r\n\r\n```json\r\n[1]\r\n```\r\n";
    let boundary = source.find('\r').unwrap();
    let second_item = source.find('犬').unwrap();
    let label = source.find("label").unwrap();
    for enabled in [true, false] {
        let extension = markdown(MarkdownConfig {
            add_keymap: enabled,
            paste_url_as_link: enabled,
            ..Default::default()
        });
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(source.into(), extension, window, cx).unwrap()
        });
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, _| {
                editor.set_selection(boundary, boundary);
                window.focus(&editor.focus);
            });
        });
        cx.simulate_keystrokes("enter");
        cx.update(|_, cx| {
            let prefix = if enabled { "- 猫\r\n- " } else { "- 猫\n" };
            assert_eq!(
                editor.read(cx).source(),
                format!("{prefix}{}", &source[boundary..])
            );
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert_eq!(editor.source(), source);
                editor.set_selection(second_item, second_item);
            });
        });
        cx.simulate_keystrokes("backspace");
        cx.update(|_, cx| {
            let changed = if enabled { "  犬" } else { "犬" };
            assert_eq!(editor.read(cx).source(), source.replace("- 犬", changed));
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string("https://example.test/".into()));
            editor.update(cx, |editor, _| {
                assert_eq!(editor.source(), source);
                editor.set_selection(label, label + "label".len());
            });
        });
        cx.simulate_keystrokes("secondary-v");
        cx.update(|_, cx| {
            let inserted = if enabled {
                "[label](https://example.test/)"
            } else {
                "https://example.test/"
            };
            assert_eq!(editor.read(cx).source(), source.replace("label", inserted));
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert_eq!(editor.source(), source);
                let from = source.find('1').unwrap();
                editor.set_selection(from, from + 1);
            });
        });
        cx.simulate_keystrokes("secondary-v enter backspace");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());
        });
    }
}

#[gpui::test]
fn backspace_removes_the_first_bullet_marker_in_one_edit(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new("- one".into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, _| {
            editor.set_selection(2, 2);
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("backspace");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), "one");
        assert_eq!(editor.cursor(), 0);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "- one"));
}
