use gpui::{ClipboardItem, Modifiers, TestAppContext};

use super::*;
use crate::{
    document::{BlockPreview, Section, SectionKind},
    editor::bind_keys,
};

fn ranges(editor: &Editor) -> Vec<Range<usize>> {
    editor
        .view
        .state()
        .selection()
        .ranges()
        .iter()
        .map(|range| range.range())
        .collect()
}

#[gpui::test]
fn linewise_clipboard_preserves_fence_separators_and_undo(cx: &mut TestAppContext) {
    let source = "```css\r\np {}\r\nq {}\r\n```\r\n\r\nUnchanged.";
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            let source = editor
                .markdown()
                .sections()
                .into_iter()
                .find_map(|section| {
                    matches!(section.preview, Some(BlockPreview::Code { .. }))
                        .then_some(section.source)
                })
                .unwrap();
            editor.activate(
                Section::new(SectionKind::DraftCode, source.clone(), source),
                false,
                cx,
            );
            editor.move_to(editor.source().find("p {}").unwrap() + 1, false, cx);
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("secondary-x");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("p {}")
        );
        assert_eq!(editor.read(cx).source(), source.replace("p {}\r\n", ""));
        editor.update(cx, |editor, cx| {
            editor.move_to(editor.source().find("q {}").unwrap() + 1, false, cx);
        });
    });
    cx.simulate_keystrokes("secondary-v");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.cursor(), source.find("q {}").unwrap() + 1);
    });
    cx.simulate_keystrokes("secondary-x");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("q {}")
        );
        assert_eq!(editor.read(cx).source(), source.replace("q {}", ""));
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        use gpui_editor_codemirror::transaction_filter;

        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), source);
            editor.move_to(source.find("p {}").unwrap(), false, cx);
            let filter = transaction_filter(|_, transaction| {
                if transaction.user_event() == Some("delete.cut") {
                    Ok(Transaction::new(ChangeSet::single(Change::new(0..3, ""))?))
                } else {
                    Ok(transaction)
                }
            });
            let active = editor.active.clone();
            editor
                .view
                .update(
                    Transaction::default().with_effects([filter.append()]),
                    active.clone(),
                    |_| (None, active),
                )
                .unwrap();
        });
    });
    cx.simulate_keystrokes("secondary-x");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("q {}")
        );
        assert_eq!(editor.read(cx).source(), source);
    });
}

#[gpui::test]
fn linewise_clipboard_pastes_multiple_crlf_lines_above_target_carets(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::EditorSelection;

    let source = "Before.\r\n\r\n```txt\r\nalpha\r\nbeta\r\nthird\r\nfourth\r\n```\r\n\r\nAfter.";
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    let carets = |first, second| {
        let at = |line| {
            let offset = source.find(line).unwrap() + 1;
            SelectionRange::new(offset, offset)
        };
        EditorSelection::new(vec![at(first), at(second)], 0).unwrap()
    };
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            let block = editor
                .markdown()
                .sections()
                .into_iter()
                .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                .unwrap()
                .source;
            editor.activate(
                Section::new(SectionKind::DraftCode, block.clone(), block),
                false,
                cx,
            );
            editor.view.set_selection(carets("alpha", "beta")).unwrap();
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("secondary-c");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("alpha\r\nbeta")
        );
        editor.update(cx, |editor, _| {
            editor
                .view
                .set_selection(carets("third", "fourth"))
                .unwrap();
        });
    });
    cx.simulate_keystrokes("secondary-v");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(
            editor.source(),
            source.replace("third\r\nfourth", "alpha\r\nthird\r\nbeta\r\nfourth")
        );
        assert_eq!(editor.view.undo_depth(), 1);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
}

#[gpui::test]
fn linewise_clipboard_uses_displayed_lone_cr_breaks(cx: &mut TestAppContext) {
    let source = "Intro\r\n\r\n```json\r\n[\r  \"猫\",\r  \"dog\"\r\n]\r\n```\r\n\r\nAfter";
    let removed = "  \"猫\",\r";
    let caret = source.find('猫').unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            let block = editor
                .markdown()
                .sections()
                .into_iter()
                .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                .unwrap()
                .source;
            editor.activate(
                Section::new(SectionKind::DraftCode, block.clone(), block),
                false,
                cx,
            );
            editor.move_to(caret, false, cx);
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("secondary-x");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("  \"猫\",")
        );
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source.replace(removed, ""));
        assert_eq!(editor.view.undo_depth(), 1);
    });
    cx.simulate_keystrokes("secondary-v");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        // The new line uses the host clipboard separator. Existing breaks are untouched.
        assert_eq!(editor.source(), source.replace(removed, "  \"猫\",\r\n"));
        assert_eq!(editor.view.undo_depth(), 2);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source.replace(removed, "")));
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.cursor(), caret);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn clipboard_filters_keep_crlf_and_do_not_copy_empty_ranges(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::{
        EditorSelection, Extension,
        view::{clipboard_input_filter, clipboard_output_filter},
    };

    let source = "猫\r\n\r\n猫";
    let second = source.rfind('猫').unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    let chosen = |include_empty: bool| {
        let mut ranges = vec![
            SelectionRange::new(0, 3),
            SelectionRange::new(second, second + 3),
        ];
        if include_empty {
            ranges.insert(1, SelectionRange::new(5, 5));
        }
        EditorSelection::new(ranges, 0).unwrap()
    };
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, _| {
            let filters = Extension::group([
                clipboard_output_filter(|text, _| format!("[{text}]")),
                clipboard_input_filter(move |text, state| {
                    assert_eq!(text, "first|second");
                    assert_eq!(state.text().as_str(), source);
                    text.replace('|', "\r\n")
                }),
            ]);
            let transaction = Transaction::default()
                .with_effects([filters.append()])
                .with_selection(chosen(true));
            editor
                .view
                .update(transaction, None, |_| (None, None))
                .unwrap();
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("secondary-c");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "[猫\r\n猫]"
        );
        assert_eq!(editor.read(cx).source(), source);
        editor.update(cx, |editor, _| {
            editor.view.set_selection(chosen(false)).unwrap()
        });
        cx.write_to_clipboard(ClipboardItem::new_string("first|second".into()));
    });
    cx.simulate_keystrokes("secondary-v");
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).source(), "first\r\n\r\nsecond");
        assert_eq!(editor.read(cx).view.undo_depth(), 1);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn handled_input_can_edit_prose_but_cannot_bypass_markdown_rules(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    use gpui_editor_codemirror::{
        EditorSelection, Extension, transaction_filter,
        view::input::{InputHandling, input_handler},
    };

    let source = "Before\r\n\r\n```txt\r\nprotected\r\n```\r\n";
    let protected = source.find("protected").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, _| {
            let hooks = Extension::group([
                input_handler(move |change, default| match change.text() {
                    "h" => {
                        let range = change.range();
                        let at = range.start + 1;
                        let changes = ChangeSet::single(Change::new(range, "H")).unwrap();
                        InputHandling::Handled(Some(
                            Transaction::new(changes)
                                .with_selection(EditorSelection::single(at, at)),
                        ))
                    }
                    "p" => {
                        let changes =
                            ChangeSet::single(Change::new(protected..protected + 1, "!")).unwrap();
                        InputHandling::Handled(Some(Transaction::new(changes)))
                    }
                    "x" => InputHandling::Handled(Some(default().unwrap().clone())),
                    "q" => InputHandling::Handled(None),
                    _ => InputHandling::Pass,
                }),
                transaction_filter(move |_, transaction| {
                    if transaction
                        .changes()
                        .changes()
                        .any(|(_, added)| matches!(added, "x" | "w"))
                    {
                        let changes =
                            ChangeSet::single(Change::new(protected..protected + 1, "!")).unwrap();
                        Ok(Transaction::new(changes))
                    } else {
                        Ok(transaction)
                    }
                }),
            ]);
            editor
                .view
                .update(
                    Transaction::default().with_effects([hooks.append()]),
                    None,
                    |_| (None, None),
                )
                .unwrap();
            window.focus(&editor.focus);
        });
    });
    cx.simulate_input("h");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), format!("H{source}"));
        assert_eq!(editor.cursor(), 1);
        assert_eq!(editor.view.undo_depth(), 1);
    });
    cx.simulate_keystrokes("secondary-z");
    for input in ["p", "x", "q", "w"] {
        cx.simulate_input(input);
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert_eq!(editor.cursor(), 0);
            assert!(!editor.view.can_undo());
        });
    }
    cx.simulate_input("y");
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), format!("y{source}"));
            editor.replace_and_mark_text_in_range(None, "q", Some(1..1), window, cx);
            assert_eq!(editor.source(), format!("yq{source}"));
            assert!(editor.is_composing());
            editor.replace_text_in_range(None, "h", window, cx);
            assert_eq!(editor.source(), format!("yh{source}"));
            assert!(!editor.is_composing());
        })
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), format!("y{source}")));
}

#[gpui::test]
fn next_occurrence_keeps_the_primary_and_pastes_one_line_per_range(cx: &mut TestAppContext) {
    let source = "猫 猫咪\r\n\r\n猫\r\n";
    let second = source.rfind('猫').unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        window.focus(&editor.read(cx).focus);
    });
    cx.simulate_keystrokes("cmd-d cmd-d");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(ranges(editor), [0..3, second..second + 3]);
        assert_eq!(editor.selection(), 0..3);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
    cx.simulate_keystrokes("secondary-c");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "猫\r\n猫"
        );
        cx.write_to_clipboard(ClipboardItem::new_string("first\r\nsecond".into()));
    });
    cx.simulate_keystrokes("secondary-v");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), "first 猫咪\r\n\r\nsecond\r\n");
        assert_eq!(ranges(editor).len(), 2);
        assert!(ranges(editor).iter().all(Range::is_empty));
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(ranges(editor), [0..3, second..second + 3]);
        assert!(!editor.view.can_undo());
    });
    cx.simulate_keystrokes("escape");
    cx.update(|_, cx| assert_eq!(ranges(editor.read(cx)).len(), 1));
}

#[gpui::test]
fn next_occurrence_uses_the_source_languages_word_characters(cx: &mut TestAppContext) {
    let name = "--accent-color";
    let source = "--accent-color\r\n\r\n```css\r\np { --accent-color: red; color: var(--accent-color) }\r\n```\r\n";
    let positions: Vec<_> = source
        .match_indices(name)
        .map(|(start, _)| start..start + name.len())
        .collect();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            let fence = editor
                .markdown()
                .sections()
                .into_iter()
                .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                .unwrap()
                .source;
            editor.activate(
                Section::new(SectionKind::DraftCode, fence.clone(), fence),
                false,
                cx,
            );
            let caret = positions[1].start + 5;
            editor.set_selection(caret, caret);
            window.focus(&editor.focus);
        });
    });
    cx.simulate_keystrokes("cmd-d cmd-d cmd-d");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(ranges(editor), positions[1..]);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn a_read_only_match_blocks_the_entire_edit_and_cut(cx: &mut TestAppContext) {
    let source = "cat\r\n\r\n```\r\ncat\r\n```\r\n\r\ncat";
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        window.focus(&editor.read(cx).focus);
    });
    cx.simulate_keystrokes("cmd-d cmd-shift-l");
    cx.update(|_, cx| {
        assert_eq!(ranges(editor.read(cx)).len(), 3);
        cx.write_to_clipboard(ClipboardItem::new_string("keep".into()));
    });
    cx.simulate_keystrokes("secondary-x backspace");
    cx.simulate_input("X");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(ranges(editor).len(), 3);
        assert!(!editor.view.can_undo());
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "keep");
    });
}

#[gpui::test]
fn find_selects_all_matches_then_focuses_the_editor(cx: &mut TestAppContext) {
    let (editor, cx) =
        cx.add_window_view(|window, cx| Editor::new("alpha and alpha".into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        window.focus(&editor.read(cx).focus);
    });
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    cx.simulate_input("alpha");
    cx.run_until_parked();
    let button = cx
        .debug_bounds("document-find-select-matches")
        .unwrap()
        .center();
    cx.simulate_click(button, Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert_eq!(ranges(editor), [0..5, 10..15]);
        assert!(editor.focus.is_focused(window));
    });
    cx.simulate_input("Z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "Z and Z"));
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "alpha and alpha"));
}
