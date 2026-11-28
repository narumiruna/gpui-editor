use gpui::TestAppContext;
use gpui_editor_codemirror::{Compartment, Extension, StateEffect, Transaction};

use super::{tests::activate_draft, *};
use crate::editor::bind_keys;

fn properties(keys: &[&str]) -> Extension {
    CssPropertySnapshot::from_style_keys(keys).extension()
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

#[gpui::test]
fn installed_property_names_stay_in_the_draft_and_undo_exactly(cx: &mut TestAppContext) {
    for (language, body) in [
        ("css", "p {\r\n  col\r\n}"),
        ("less", "p {\r\n  col\r\n}"),
        ("scss", "p {\r\n  col\r\n}"),
        ("sass", "p\r\n  col"),
        ("html", "<style>p { col }</style>"),
        ("html", "<p style=\"col\">text</p>"),
    ] {
        let draft = format!("前😀\r\n\r\n```{language}\r\n{body}\r\n```\r\n\r\nOutside.");
        let from = draft.find("col").unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(draft.clone(), properties(&["colorHost"]), window, cx)
                .unwrap()
        });
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_draft(editor, from + 3, window, cx)
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        cx.run_until_parked();
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let source = editor
                    .current_source_completion()
                    .unwrap_or_else(|| panic!("no completion for {language}: {body}"));
                let options = source.session.options().unwrap();
                assert_eq!(options.len(), 1, "{language}: {body}");
                assert_eq!(options[0].item.label, "color-host");
                assert_eq!(options[0].item.kind.as_deref(), Some("property"));
                assert_eq!(options[0].range, from..from + 3);
                assert!(editor.accept_source_completion(Some(options[0].id), cx));
                assert_eq!(editor.source(), draft.replacen("col", "color-host: ", 1));
                assert_eq!(editor.view.undo_depth(), 1);
            });
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), draft));
    }
}

#[gpui::test]
fn replaced_property_snapshot_rejects_open_and_in_flight_choices(cx: &mut TestAppContext) {
    let draft = "```css\r\np { col }\r\n```\r\n\r\nOutside.";
    let head = draft.find("col").unwrap() + 3;
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(draft.into(), slot.of(properties(&["colorOld"])), window, cx)
            .unwrap()
    });
    let executor = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| activate_draft(editor, head, window, cx));
        cx.background_executor().clone()
    });
    cx.simulate_keystrokes("ctrl-space");
    executor.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let current = editor.current_source_completion().unwrap();
            let choices = current.session.options().unwrap();
            assert_eq!(choices.len(), 1);
            assert_eq!(choices[0].item.label, "color-old");
            let stale_id = choices[0].id;
            configure(editor, slot.reconfigure(properties(&["colorNew"])), cx);
            assert!(editor.current_source_completion().is_none());
            assert!(!editor.accept_source_completion(Some(stale_id), cx));
            assert!(editor.request_source_completion(true, cx));
            editor.completion.as_mut().unwrap().timer = None;
        });
    });
    executor.advance_clock(Duration::from_millis(50));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.run_source_completion_timer(cx);
            assert_eq!(
                editor
                    .current_source_completion()
                    .unwrap()
                    .session
                    .deadline(),
                None
            );
            configure(editor, slot.reconfigure(properties(&[])), cx);
            assert!(editor.current_source_completion().is_none());
            assert!(editor.request_source_completion(true, cx));
        });
    });
    cx.run_until_parked();
    executor.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.current_source_completion().is_none_or(|current| {
                current
                    .session
                    .options()
                    .is_ok_and(|options| options.is_empty())
            }));
            assert_eq!(editor.source(), draft);
            assert_eq!(editor.view.undo_depth(), 0);
            configure(editor, slot.reconfigure(Extension::default()), cx);
            assert!(editor.request_source_completion(true, cx));
        });
    });
    executor.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    cx.update(|_, cx| {
        let current = editor.read(cx).current_source_completion().unwrap();
        assert!(
            current
                .session
                .options()
                .unwrap()
                .iter()
                .any(|option| option.item.label == "color")
        );
        assert!(
            current
                .session
                .options()
                .unwrap()
                .iter()
                .all(|option| option.item.label != "color-new")
        );
    });
}

#[gpui::test]
fn host_property_picker_cannot_add_a_closing_fence(cx: &mut TestAppContext) {
    let draft = "```css\np { col }\n```\n\nOutside.";
    let head = draft.find("col").unwrap() + 3;
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(draft.into(), properties(&["col\n```\noutside"]), window, cx)
            .unwrap()
    });
    let executor = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| activate_draft(editor, head, window, cx));
        cx.background_executor().clone()
    });
    cx.simulate_keystrokes("ctrl-space");
    executor.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let current = editor.current_source_completion().unwrap();
            let choices = current.session.options().unwrap();
            assert_eq!(
                choices[0].item.insert.as_deref(),
                Some("col\n```\noutside: ")
            );
            assert!(!editor.accept_source_completion(Some(choices[0].id), cx));
            assert_eq!(editor.source(), draft);
            assert_eq!(editor.view.undo_depth(), 0);
        });
    });
}
