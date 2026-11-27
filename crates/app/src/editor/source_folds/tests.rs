use gpui::{Modifiers, TestAppContext, Window, point, px, size};

use super::*;
use crate::{
    document::{BlockPreview, Section, SectionKind},
    editor::{bind_keys, layout::ItemPaint},
};

const CODE: &str = "Before\r\n\r\n```json\r\n{\r\n  \"word\": \"字\",\r\n  \"list\": [1, 2]\r\n}\r\n```\r\n\r\nAfter";

fn enter_code(editor: &mut Editor, window: &mut Window, cx: &mut Context<Editor>) {
    let code = editor
        .markdown()
        .sections()
        .into_iter()
        .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
        .unwrap();
    let range = code.source;
    editor.activate(
        Section::new(SectionKind::DraftCode, range.clone(), range),
        false,
        cx,
    );
    window.focus(&editor.focus);
}

fn only_fold(editor: &Editor) -> Range<usize> {
    let ranges = editor.source_fold_ranges();
    let [range] = ranges.as_slice() else {
        panic!("expected one fold, got {ranges:?}");
    };
    range.clone()
}

#[gpui::test]
fn code_fold_changes_layout_not_source_and_click_expands(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CODE.into(), window, cx));
    let start = CODE.find('{').unwrap() + 1;
    let end = CODE.find('}').unwrap();
    let count = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            assert!(!editor.toggle_source_fold(cx));
            enter_code(editor, window, cx);
            let head = CODE.find('字').unwrap();
            editor.set_selection(head, head);
            editor.shape(window, cx).rows.len()
        })
    });
    cx.simulate_keystrokes("ctrl-shift-[");
    cx.run_until_parked();
    let pointer = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), CODE);
        assert_eq!(editor.view.revision(), 0);
        assert_eq!(editor.cursor(), start);
        assert!(!editor.view.can_undo());
        assert_eq!(only_fold(editor), start..end);
        let layout = editor.layout.as_ref().unwrap();
        assert_eq!(layout.rows.len() + 3, count);
        assert!(layout.fold_hides(CODE.find('字').unwrap()));
        assert!(!layout.fold_hides(start));
        assert!(!layout.fold_hides(end));
        let (row, item) = layout
            .rows
            .iter()
            .find_map(|row| {
                row.items.iter().find(|item| {
                    matches!(&item.paint, ItemPaint::Fold(line) if line.text.as_ref() == " … ")
                }).map(|item| (row, item))
            })
            .unwrap();
        assert_eq!(item.source, start..end);
        layout.origin + point(item.left + item.width / 2., row.y + px(8.))
    });
    cx.simulate_click(pointer, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.layout.as_ref().unwrap().rows.len(), count);
        assert!(editor.source_fold_ranges().is_empty());
        assert_eq!(editor.source(), CODE);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn explicit_fold_announces_projected_lines_but_implicit_reveal_does_not(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::{
        Change, ChangeSet, Extension, TransactionCoordinates, transaction_filter,
        view::{ANNOUNCE, update_listener},
    };
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering::Relaxed},
    };

    let source = "Lead\r\n\r\n```json\r\n{\r  \"field\": \"字\"\r}\r\n```\r\n";
    let messages = Arc::new(Mutex::new(Vec::<String>::new()));
    let modify = Arc::new(AtomicBool::new(false));
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let listen = update_listener({
            let messages = messages.clone();
            move |update| {
                messages
                    .lock()
                    .unwrap()
                    .extend(update.announcements().map(str::to_owned));
                Ok(())
            }
        });
        let filter = transaction_filter({
            let modify = modify.clone();
            move |state, transaction| {
                if modify.load(Relaxed)
                    && transaction
                        .effects()
                        .iter()
                        .any(|effect| effect.get(&ANNOUNCE).is_some())
                {
                    transaction.merge(
                        Transaction::new(ChangeSet::single(Change::new(0..0, "Unexpected"))?),
                        state.text().len(),
                        TransactionCoordinates::Original,
                    )
                } else {
                    Ok(transaction)
                }
            }
        });
        Editor::with_native_extensions(
            source.into(),
            Extension::group([listen, filter]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            enter_code(editor, window, cx);
            let caret = source.find('{').unwrap();
            editor.set_selection(caret, caret);
            assert!(editor.toggle_source_fold(cx));
            assert!(editor.open_source_fold(only_fold(editor), cx));
            assert!(editor.toggle_source_fold(cx));
            assert!(editor.toggle_source_fold(cx));
            modify.store(true, Relaxed);
            assert!(!editor.toggle_source_fold(cx));
            assert!(editor.source_fold_ranges().is_empty());
            assert_eq!(editor.source(), source);
            assert_eq!(editor.cursor(), caret);
            assert_eq!(editor.view.revision(), 0);
        });
    });
    assert_eq!(
        *messages.lock().unwrap(),
        [
            "Folded lines 1 to 3.",
            "Folded lines 1 to 3.",
            "Unfolded lines 1 to 3."
        ]
    );
}

#[gpui::test]
fn wast_fold_uses_lone_cr_before_the_close_delimiter(cx: &mut TestAppContext) {
    let source = "```wat\r\n(module\r  (func)\r)\r\n```";
    let open = source.find("(module").unwrap();
    let close = source.rfind(')').unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            enter_code(editor, window, cx);
            editor.set_selection(open, open);
            assert!(editor.toggle_source_fold(cx));
            assert_eq!(only_fold(editor), open + 1..close);
            assert!(editor.toggle_source_fold(cx));
            assert!(editor.source_fold_ranges().is_empty());
            assert_eq!(editor.source(), source);
            assert_eq!(editor.cursor(), open);
            assert!(!editor.view.can_undo());
        });
    });
}

#[gpui::test]
fn supported_fences_fold_on_lone_cr_lines_without_changing_bytes(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::language::fold::fold_gutter;

    for source in [
        "```cpp\r\nvoid f() {\r  call(\"字\");\r}\r\n```",
        "```rust\r\nfn f() {\r  call(\"字\");\r}\r\n```",
        "```java\r\nclass A {\r  String x = \"字\";\r}\r\n```",
        "```go\r\nfunc f() {\r  println(\"字\")\r}\r\n```",
        "```lezer\r\n@tokens {\r  Word { \"字\" }\r}\r\n```",
        "```json\r\n{\r  \"word\": \"字\"\r}\r\n```",
        "```yaml\r\n{\r  word: 字\r}\r\n```",
    ] {
        let open = source.find('{').unwrap();
        let close = source.rfind('}').unwrap();
        let (editor, window) =
            cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        window.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                enter_code(editor, window, cx);
                let body = editor.source_fence().unwrap().body;
                let head = body.start;
                editor.set_selection(head, head);
                assert!(editor.toggle_source_fold(cx));
                assert_eq!(only_fold(editor), open + 1..close);
                assert!(editor.toggle_source_fold(cx));
                assert!(editor.source_fold_ranges().is_empty());

                let active = editor.active.clone();
                editor
                    .view
                    .update(
                        Transaction::default()
                            .with_effects([fold_gutter(Default::default()).append()]),
                        active.clone(),
                        |_| (None, active),
                    )
                    .unwrap();
                let projected = editor.source_body_line_projection().unwrap();
                assert_eq!(projected.line_count(), 3);
                assert_eq!(projected.len(), body.len());
                let config = FoldGutterConfig::configured(editor.view.state()).unwrap();
                editor.click_source_fold_gutter(&body, head..open + 1, &config, cx);
                assert_eq!(only_fold(editor), open + 1..close);
                let layout = editor.shape(window, cx);
                assert!(layout.fold_hides(source.find('字').unwrap()));
                assert_eq!(editor.source(), source);
                assert_eq!(editor.cursor(), head);
                assert!(!editor.view.can_undo());
            });
        });
    }
}

#[gpui::test]
fn configured_fold_label_updates_without_preparing_again(cx: &mut TestAppContext) {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use gpui_editor_codemirror::{
        Compartment,
        language::fold::{
            FoldPlaceholderConfig, PrepareFoldPlaceholder, code_folding_with_placeholders,
        },
    };

    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CODE.into(), window, cx));
    let open = CODE.find('{').unwrap();
    let close = CODE.find('}').unwrap();
    let slot = Compartment::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let logged = calls.clone();
    let prepare: PrepareFoldPlaceholder = Arc::new(move |state, range| {
        logged.fetch_add(1, Ordering::Relaxed);
        assert_eq!(range, &(open + 1..close));
        assert!(state.text().slice(range.clone()).unwrap().contains('字'));
        "字".into()
    });
    let options = |label| {
        code_folding_with_placeholders(FoldPlaceholderConfig {
            prepare: Some(prepare.clone()),
            render: Some(Arc::new(move |_, _, value| {
                Some(format!("{label}\n{}", value.unwrap()).into())
            })),
            ..Default::default()
        })
    };
    let paint = |layout: &super::super::layout::Layout, expected| {
        let (row, item) = layout
            .rows
            .iter()
            .flat_map(|row| row.items.iter().map(move |item| (row, item)))
            .find(|(_, item)| matches!(&item.paint, ItemPaint::Fold(line) if line.text.as_ref() == expected))
            .unwrap();
        assert_eq!(item.source, open + 1..close);
        layout.origin + point(item.left + item.width / 2., row.y + px(8.))
    };
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            enter_code(editor, window, cx);
            editor.set_selection(open, open);
            let active = editor.active.clone();
            editor
                .view
                .update(
                    Transaction::default().with_effects([slot.of(options("Hidden")).append()]),
                    active.clone(),
                    |_| (None, active),
                )
                .unwrap();
            assert!(editor.toggle_source_fold(cx));
            paint(&editor.shape(window, cx), " Hidden 字 ");
            let active = editor.active.clone();
            editor
                .view
                .update(
                    Transaction::default().with_effects([slot.reconfigure(options("Collapsed"))]),
                    active.clone(),
                    |_| (None, active),
                )
                .unwrap();
            cx.notify();
        });
    });
    cx.run_until_parked();
    let pointer = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(editor.cursor(), open);
        paint(editor.layout.as_ref().unwrap(), " Collapsed 字 ")
    });
    cx.simulate_click(pointer, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(editor.source_fold_ranges().is_empty());
        assert_eq!(editor.source(), CODE);
        assert_eq!(editor.view.revision(), 0);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn fold_services_take_precedence_without_hiding_a_markdown_fence(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::{Compartment, language::fold::fold_service};

    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CODE.into(), window, cx));
    let start = CODE.find('{').unwrap();
    let end = CODE.find("  \"list\"").unwrap();
    let config = Compartment::new();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            enter_code(editor, window, cx);
            editor.set_selection(start, start);
            let options = config.of(fold_service(move |state, from, to| {
                assert_eq!(state.text().as_str(), CODE);
                assert_eq!(from..to, start..start + 1);
                Some(start + 1..end)
            }));
            let active = editor.active.clone();
            editor
                .view
                .update(
                    Transaction::default().with_effects([options.append()]),
                    active.clone(),
                    |_| (None, active),
                )
                .unwrap();
            assert!(editor.toggle_source_fold(cx));
            assert_eq!(only_fold(editor), start + 1..end);
            assert!(editor.toggle_source_fold(cx));
            assert!(editor.source_fold_ranges().is_empty());
            let overlaps_header = config.reconfigure(fold_service(move |_, _, _| Some(0..end)));
            let active = editor.active.clone();
            editor
                .view
                .update(
                    Transaction::default().with_effects([overlaps_header]),
                    active.clone(),
                    |_| (None, active),
                )
                .unwrap();
            assert!(!editor.toggle_source_fold(cx));
            assert!(editor.source_fold_ranges().is_empty());
            assert_eq!(editor.source(), CODE);
            assert_eq!(editor.cursor(), start);
            assert!(!editor.view.can_undo());
        });
    });
}

#[gpui::test]
fn folding_above_the_viewport_preserves_the_displayed_source_row(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::view::{covered_margins, scroll_handler};

    let draft = format!(
        "```json\r\n{{\r\n  \"fold\": [\r\n{}    0\r\n  ],\r\n  \"keep\": [\r\n    1234,\r\n{}    0\r\n  ]\r\n}}\r\n```",
        "    1,\r\n".repeat(24),
        "    2,\r\n".repeat(24),
    );
    let anchor = draft.find("    1234,").unwrap();
    let head = draft.find("\"fold\"").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.clone(), window, cx));
    cx.simulate_resize(size(px(420.), px(350.)));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            enter_code(editor, window, cx);
            editor.set_selection(head, head);
        });
    });
    cx.run_until_parked();
    let anchor_y = |editor: &Editor| {
        editor
            .layout
            .as_ref()
            .unwrap()
            .source_bounds(anchor)
            .unwrap()
            .top()
            - editor.scroll.bounds().top()
    };
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        let next = editor.scroll.offset().y - anchor_y(editor) + px(7.);
        editor.scroll.set_offset(point(px(0.), next));
        window.refresh();
    });
    cx.run_until_parked();
    let original_offset = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(anchor_y(editor), px(7.));
            assert_eq!(editor.capture_source_scroll().unwrap().anchor(), anchor);
            // Restoration bypasses the ordinary reveal callbacks.
            let callbacks = gpui_editor_codemirror::Extension::group([
                scroll_handler(|_, _| panic!("snapshot called a reveal handler")),
                covered_margins(|_, _| panic!("snapshot queried a covered margin")),
            ]);
            let transaction = Transaction::default().with_effects([callbacks.append()]);
            let active = editor.active.clone();
            editor
                .view
                .update(transaction, active.clone(), |_| (None, active))
                .unwrap();
            let old = editor.scroll.offset().y;
            assert!(editor.toggle_source_fold(cx));
            old
        })
    });
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(anchor_y(editor), px(7.));
            assert!(editor.scroll.offset().y > original_offset);
            assert_eq!(editor.cursor(), head);
            assert!(editor.toggle_source_fold(cx));
        });
    });
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(anchor_y(editor), px(7.));
        assert_eq!(editor.scroll.offset().y, original_offset);
        assert_eq!(editor.source(), draft);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn delete_to_end_of_source_row_unfolds_before_editing(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CODE.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            enter_code(editor, window, cx);
            let head = CODE.find('{').unwrap();
            editor.set_selection(head, head);
            assert!(editor.toggle_source_fold(cx));
        });
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-delete");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(editor.source_fold_ranges().is_empty());
        assert_eq!(editor.source(), CODE);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn folded_source_is_not_deleted_by_an_empty_selection_and_reveals_selected_text(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CODE.into(), window, cx));
    let from = CODE.find('{').unwrap() + 1;
    let to = CODE.find('}').unwrap();
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            enter_code(editor, window, cx);
            editor.set_selection(from, from);
            assert!(editor.toggle_source_fold(cx));
        });
    });
    cx.simulate_keystrokes("delete");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.source_fold_ranges().is_empty());
            assert_eq!(editor.source(), CODE);
            assert_eq!(editor.cursor(), from);
            assert!(editor.toggle_source_fold(cx));
            editor.set_selection(to, to);
        });
    });
    cx.simulate_keystrokes("backspace");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.source_fold_ranges().is_empty());
            assert_eq!(editor.source(), CODE);
            assert_eq!(editor.cursor(), to);
            assert!(!editor.view.can_undo());
            editor.set_selection(from, from);
            assert!(editor.toggle_source_fold(cx));
            let word = CODE.find('字').unwrap();
            editor.move_to(word, false, cx);
            assert_eq!(editor.cursor(), word);
            assert!(editor.source_fold_ranges().is_empty());
        });
    });
}

#[gpui::test]
fn find_expands_a_fold_before_revealing_its_match(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CODE.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            enter_code(editor, window, cx);
            let open = CODE.find('{').unwrap();
            editor.set_selection(open, open);
            assert!(editor.toggle_source_fold(cx));
        });
    });
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    cx.simulate_input("字");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let start = CODE.find('字').unwrap();
        assert_eq!(editor.selection(), start..start + '字'.len_utf8());
        assert!(editor.source_fold_ranges().is_empty());
        assert_eq!(editor.source(), CODE);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn a_fold_tracks_accepted_edits_and_undo(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CODE.into(), window, cx));
    let open = CODE.find('{').unwrap();
    let close = CODE.find('}').unwrap();
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            enter_code(editor, window, cx);
            editor.set_selection(open, open);
            assert!(editor.toggle_source_fold(cx));
            assert!(editor.replace(open..open, " ", cx));
            assert_eq!(only_fold(editor), open + 2..close + 1);
            assert_eq!(editor.source(), CODE.replacen('{', " {", 1));
        });
    });
    cx.simulate_keystrokes("cmd-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(only_fold(editor), open + 1..close);
        assert_eq!(editor.source(), CODE);
        assert!(!editor.view.can_undo());
        assert!(editor.view.can_redo());
    });
}

#[gpui::test]
fn folding_cannot_touch_fences_prose_or_composition(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CODE.into(), window, cx));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            enter_code(editor, window, cx);
            let start = editor.active.as_ref().unwrap().content.start;
            editor.set_selection(start, start);
            assert!(!editor.toggle_source_fold(cx));
            let head = CODE.find('字').unwrap();
            editor.set_selection(head, head);
            editor.marked = Some(head..head);
            assert!(!editor.toggle_source_fold(cx));
            editor.finish_composition();
            editor.set_input_enabled(false, cx);
            assert!(!editor.toggle_source_fold(cx));
            assert!(editor.source_fold_ranges().is_empty());
            assert_eq!(editor.source(), CODE);
        });
    });
}
