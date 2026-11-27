use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use gpui::{Modifiers, TestAppContext};
use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, Extension, StateEffect, Transaction,
    language::fold::{fold_effect, fold_gutter, fold_service, folded_ranges},
    transaction_filter,
    view::gutters::GutterMarker,
};

use super::*;
use crate::document::{BlockPreview, Section, SectionKind};

const CODE: &str = "Before\r\n\r\n```json\r\n{\r\n  \"word\": \"字\",\r\n  \"list\": [1, 2]\r\n}\r\n```\r\n\r\nAfter";

fn enter_code(editor: &mut Editor, cx: &mut Context<Editor>) {
    let range = editor
        .markdown()
        .sections()
        .into_iter()
        .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
        .unwrap()
        .source;
    editor.activate(
        Section::new(SectionKind::DraftCode, range.clone(), range),
        false,
        cx,
    );
}

fn configure(editor: &mut Editor, effect: StateEffect, cx: &mut Context<Editor>) {
    let active = editor.active.clone();
    editor
        .view
        .update(
            Transaction::default().with_effects([effect]),
            active.clone(),
            |_| (None, active),
        )
        .unwrap();
    cx.notify();
}

fn frame(editor: &Editor) -> (&Layout, &SourceGutter) {
    let layout = editor.layout.as_ref().unwrap();
    let gutter = layout
        .source_frame
        .as_ref()
        .unwrap()
        .gutter
        .as_ref()
        .unwrap();
    (layout, gutter)
}

fn row_hit(editor: &Editor, start: usize, first: bool) -> Point<Pixels> {
    let (layout, gutter) = frame(editor);
    let row = layout
        .rows
        .iter()
        .find(|row| {
            let Some((number, is_first)) = gutter.line_at_row(row) else {
                return false;
            };
            is_first == first
                && gutter.state.text().line(number).unwrap().range.start + gutter.body.start
                    == start
        })
        .unwrap();
    layout.origin + gutter.fold_bounds(row).center()
}

fn assert_only_fold(editor: &Editor, range: Range<usize>) {
    assert_eq!(editor.source_fold_ranges(), std::slice::from_ref(&range));
}

fn markers(editor: &Editor) -> Vec<(usize, FoldGutterState, &str)> {
    frame(editor)
        .1
        .folds
        .as_ref()
        .unwrap()
        .markers
        .iter()
        .map(|(&start, marker)| {
            (
                start,
                marker.state,
                marker.marker.content.as_deref().unwrap(),
            )
        })
        .collect()
}

#[gpui::test]
fn fold_gutter_opt_in_click_interception_and_reconfiguration(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CODE.into(), window, cx));
    cx.simulate_resize(size(px(420.), px(420.)));
    cx.run_until_parked();
    let open = CODE.find('{').unwrap();
    let close = CODE.find('}').unwrap();
    let end = CODE.find("  \"list\"").unwrap();
    let config = Compartment::new();
    let intercept = Arc::new(AtomicBool::new(true));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let stock = FoldGutterConfig {
        open_text: "hide".into(),
        closed_text: "show".into(),
        handle_click: Some(Arc::new({
            let calls = calls.clone();
            let intercept = intercept.clone();
            move |state, line| {
                assert_eq!(state.text().as_str().len(), CODE.len());
                calls.lock().unwrap().push(line.clone());
                intercept.load(Ordering::Relaxed)
            }
        })),
        ..Default::default()
    };
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.layout.as_ref().unwrap().source_frame.is_none());
            enter_code(editor, cx);
            let word = CODE.find("word").unwrap();
            assert!(editor.replace(word..word + 4, "name", cx));
            // Keep an existing selection inside the range, with nonempty undo history.
            editor.set_selection(word + 3, word);
        })
    });
    cx.run_until_parked();
    let (numbers_width, selection, revision) = cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let (_, gutter) = frame(editor);
            assert!(gutter.folds.is_none());
            let width = gutter.width;
            let selection = editor.view.state().selection().clone();
            let revision = editor.view.revision();
            configure(
                editor,
                config
                    .of(Extension::group([
                        fold_gutter(stock),
                        fold_service(move |_, from, to| {
                            ((from..to) == (open..open + 1)).then_some(open + 1..end)
                        }),
                    ]))
                    .append(),
                cx,
            );
            (width, selection, revision)
        })
    });
    cx.run_until_parked();
    let (hit, width) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        let (_, gutter) = frame(editor);
        assert_eq!(gutter.numbers_width, numbers_width);
        assert!(gutter.width > numbers_width);
        assert_eq!(markers(editor), [(open, FoldGutterState::Open, "hide")]);
        (row_hit(editor, open, true), gutter.width)
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| assert!(editor.read(cx).source_fold_ranges().is_empty()));
    assert_eq!(
        *calls.lock().unwrap(),
        std::slice::from_ref(&(open..open + 1))
    );
    intercept.store(false, Ordering::Relaxed);
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    let hit = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_only_fold(editor, open + 1..end);
        assert_eq!(markers(editor), [(open, FoldGutterState::Closed, "show")]);
        assert_eq!(frame(editor).1.width, width);
        assert_eq!(editor.view.state().selection(), &selection);
        row_hit(editor, open, true)
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        *calls.lock().unwrap(),
        [open..open + 1, open..open + 1, open..open + 1]
    );
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.source_fold_ranges().is_empty());
            configure(
                editor,
                config.reconfigure(fold_gutter(FoldGutterConfig {
                    render_marker: Some(Arc::new(|_, kind| {
                        GutterMarker::text(match kind {
                            FoldGutterState::Open => "custom hide",
                            FoldGutterState::Closed => "custom show",
                        })
                    })),
                    ..Default::default()
                })),
                cx,
            );
        })
    });
    cx.run_until_parked();
    let hit = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(
            markers(editor),
            [(open, FoldGutterState::Open, "custom hide")]
        );
        row_hit(editor, open, true)
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_only_fold(editor, open + 1..close);
            assert_eq!(
                markers(editor),
                [(open, FoldGutterState::Closed, "custom show")]
            );
            configure(editor, config.reconfigure(Extension::default()), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let (_, gutter) = frame(editor);
        assert!(gutter.folds.is_none());
        assert_eq!(gutter.width, numbers_width);
        assert_only_fold(editor, open + 1..close);
        assert_eq!(editor.source(), CODE.replacen("word", "name", 1));
        assert_eq!(editor.view.state().selection(), &selection);
        assert_eq!(editor.view.revision(), revision);
        assert_eq!(editor.view.undo_depth(), 1);
        assert_eq!(editor.view.redo_depth(), 0);
        assert!(!editor.view.can_undo_selection());
    });
}

#[gpui::test]
fn fold_gutter_first_row_hit_does_not_act_during_composition(cx: &mut TestAppContext) {
    let source = "```js\r\nconst some_long_name_here = {\r\n  word: '字',\r\n};\r\n```";
    let start = source.find("const").unwrap();
    let open = source.find('{').unwrap();
    let close = source.find('}').unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.simulate_resize(size(px(214.), px(430.)));
    let calls = Arc::new(Mutex::new(Vec::new()));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            enter_code(editor, cx);
            window.focus(&editor.focus);
            configure(
                editor,
                fold_gutter(FoldGutterConfig {
                    handle_click: Some(Arc::new({
                        let calls = calls.clone();
                        move |_, line| {
                            calls.lock().unwrap().push(line.clone());
                            false
                        }
                    })),
                    ..Default::default()
                })
                .append(),
                cx,
            );
        })
    });
    cx.run_until_parked();
    let continuation = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(markers(editor), [(start, FoldGutterState::Open, "⌄")]);
        row_hit(editor, start, false)
    });
    cx.simulate_click(continuation, Modifiers::none());
    cx.run_until_parked();
    assert!(calls.lock().unwrap().is_empty());
    let (first, selection) = cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            let hit = row_hit(editor, start, true);
            let head = source.find('字').unwrap();
            editor.set_selection(head, head);
            editor.marked = Some(head..head + '字'.len_utf8());
            (hit, editor.view.state().selection().clone())
        })
    });
    cx.simulate_click(first, Modifiers::none());
    cx.run_until_parked();
    assert!(calls.lock().unwrap().is_empty());
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert!(editor.is_composing());
            assert!(editor.source_fold_ranges().is_empty());
            assert_eq!(editor.view.state().selection(), &selection);
            editor.finish_composition();
        })
    });
    cx.simulate_click(first, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_only_fold(editor, open + 1..close);
        assert_eq!(editor.view.state().selection(), &selection);
        assert!(!editor.view.can_undo());
        assert_eq!(editor.source(), source);
    });
    assert_eq!(
        *calls.lock().unwrap(),
        std::slice::from_ref(&(start..open + 1))
    );
}

#[gpui::test]
fn fold_gutter_rejects_invalid_open_closed_and_filtered_effects(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(CODE.into(), window, cx));
    cx.simulate_resize(size(px(600.), px(430.)));
    let open = CODE.find('{').unwrap();
    let close = CODE.find('}').unwrap();
    let outside = CODE.rfind("```").unwrap() + 1;
    let word = CODE.find('字').unwrap();
    let config = Compartment::new();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            enter_code(editor, cx);
            editor.set_selection(open, open);
            configure(editor, fold_gutter(Default::default()).append(), cx);
            configure(editor, config.of(Extension::default()).append(), cx);
        })
    });
    cx.run_until_parked();
    for rejected in [
        0..close,
        open + 1..outside,
        open + 1..word + 1,
        open + 2..close,
    ] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(
                    editor,
                    config.reconfigure(fold_service(move |_, from, _| {
                        (from == open).then(|| rejected.clone())
                    })),
                    cx,
                )
            })
        });
        cx.run_until_parked();
        let hit = cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert!(markers(editor).is_empty());
            row_hit(editor, open, true)
        });
        cx.simulate_click(hit, Modifiers::none());
        cx.run_until_parked();
        cx.update(|_, cx| assert!(folded_ranges(editor.read(cx).view.state()).is_empty()));
    }
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            configure(
                editor,
                config.reconfigure(transaction_filter(move |_, tr| {
                    if tr
                        .effects()
                        .iter()
                        .any(|effect| effect.get(fold_effect()).is_some())
                    {
                        return Ok(Transaction::new(ChangeSet::single(Change::new(0..1, "X"))?));
                    }
                    Ok(tr)
                })),
                cx,
            );
        })
    });
    cx.run_until_parked();
    let hit = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(markers(editor), [(open, FoldGutterState::Open, "⌄")]);
        row_hit(editor, open, true)
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.source_fold_ranges().is_empty());
            assert_eq!(editor.source(), CODE);
            configure(editor, config.reconfigure(Extension::default()), cx);
            configure(editor, fold_effect().of(open + 1..outside), cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(markers(editor).is_empty());
        assert_eq!(hit, row_hit(editor, open, true));
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(folded_ranges(editor.view.state()).len(), 1);
        assert!(editor.source_fold_ranges().is_empty());
        assert_eq!(editor.source(), CODE);
        assert_eq!(editor.cursor(), open);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn fold_gutter_projects_wast_lone_cr_rows_without_editing_bytes(cx: &mut TestAppContext) {
    let source = "```wat\r\n(module\r  (func\r    nop)\r)\r\n```";
    let open = source.find("(module").unwrap();
    let func = source.find("  (func").unwrap();
    let close = source.rfind(')').unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.simulate_resize(size(px(600.), px(420.)));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            enter_code(editor, cx);
            editor.set_selection(open, open);
            configure(editor, fold_gutter(Default::default()).append(), cx);
        })
    });
    cx.run_until_parked();
    let hit = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(
            markers(editor),
            [
                (open, FoldGutterState::Open, "⌄"),
                (func, FoldGutterState::Open, "⌄")
            ]
        );
        row_hit(editor, open, true)
    });
    cx.simulate_click(hit, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(markers(editor), [(open, FoldGutterState::Closed, "›")]);
        assert_only_fold(editor, open + 1..close);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}
