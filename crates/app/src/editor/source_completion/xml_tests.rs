use gpui::{EntityInputHandler, Modifiers, TestAppContext};
use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, Extension, StateEffect, Transaction,
    autocomplete::Completion,
    language::xml::{XmlAttrSpec, XmlElementSpec, XmlSchema, XmlValue},
    transaction_filter,
};

use super::{tests::activate_draft, *};
use crate::editor::bind_keys;

fn schema_with_element(name: &str, insert: Option<&str>) -> Extension {
    NativeXmlSchemaExtension::new(XmlSchema {
        elements: &[XmlElementSpec {
            name,
            completion: Some(Completion {
                insert,
                ..Completion::new("")
            }),
            ..Default::default()
        }],
        ..Default::default()
    })
    .extension()
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
fn xml_schema_typing_reuses_the_quoted_range_and_undo_restores_exact_bytes(
    cx: &mut TestAppContext,
) {
    let draft = "前\r\n\r\n```XML info\r\n<!-- 😀 -->\r\n<root mode=\"o\"/>\r\n```\r\n\r\n後";
    let start = draft.find("\"o\"").unwrap();
    let head = start + 2;
    let split_crlf = draft.find("-->\r\n").unwrap() + 4;
    let extension = NativeXmlSchemaExtension::new(XmlSchema {
        attributes: &[XmlAttrSpec {
            name: "mode",
            values: Some(&[XmlValue::Completion(Completion {
                insert: Some("\"ordered value\""),
                ..Completion::new("order")
            })]),
            ..Default::default()
        }],
        ..Default::default()
    })
    .extension();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    let executor = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            configure(editor, extension.append(), cx);
            editor.move_to(head, false, cx);
            assert!(!editor.request_source_completion(true, cx)); // Read-only preview.
            activate_draft(editor, split_crlf, window, cx);
            assert_eq!(editor.cursor(), split_crlf);
            assert!(!editor.request_source_completion(true, cx));
            editor.move_to(head, false, cx);
        });
        cx.background_executor().clone()
    });
    cx.run_until_parked();
    cx.simulate_input("r");
    executor.advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    let id = cx.update(|_, cx| {
        let current = editor.read(cx).current_source_completion().unwrap();
        let option = current.session.selected_completion().unwrap().unwrap();
        assert_eq!(option.id.source(), SOURCE);
        assert_eq!(option.item.label, "\"order\"");
        assert_eq!(option.range, start..head + 2);
        option.id
    });
    cx.simulate_input("d");
    executor.advance_clock(Duration::from_millis(75));
    let typed = draft.replace("\"o\"", "\"ord\"");
    let undo_before = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source().as_bytes(), typed.as_bytes());
        let current = editor.current_source_completion().unwrap();
        let option = current.session.selected_completion().unwrap().unwrap();
        assert_eq!(option.id, id);
        assert_eq!(option.range, start..head + 3);
        editor.view.undo_depth()
    });
    cx.simulate_keystrokes("enter");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(
            editor.source().as_bytes(),
            draft.replace("\"o\"", "\"ordered value\"").as_bytes()
        );
        assert_eq!(editor.cursor(), start + "\"ordered value\"".len());
        assert_eq!(editor.view.undo_depth(), undo_before + 1);
    });
    cx.simulate_keystrokes("cmd-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source().as_bytes(), typed.as_bytes()));
}

#[gpui::test]
fn replacing_and_removing_xml_schema_discards_popup_and_in_flight_results(cx: &mut TestAppContext) {
    let draft = "```xml\r\n<root> \r\n```\r\n\r\nOutside.";
    let head = draft.find("<root> ").unwrap() + "<root> ".len();
    let compartment = Compartment::new();
    let old = schema_with_element("choiceOld", None);
    let new = schema_with_element("choiceNew", None);
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    let executor = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_draft(editor, head, window, cx);
            configure(editor, compartment.of(old).append(), cx);
        });
        cx.background_executor().clone()
    });
    cx.simulate_keystrokes("ctrl-space");
    executor.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    assert!(cx.debug_bounds("source-completion: <choiceOld").is_some());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let options = editor
                .current_source_completion()
                .unwrap()
                .session
                .options()
                .unwrap();
            assert!(options.iter().all(|option| option.id.source() == SOURCE));
            assert_eq!(
                options
                    .iter()
                    .filter(|opt| opt.item.label == "</root>")
                    .count(),
                1
            );
            let old_option = options
                .into_iter()
                .find(|opt| opt.item.label == "<choiceOld")
                .unwrap()
                .id;
            configure(editor, compartment.reconfigure(new), cx);
            assert!(editor.current_source_completion().is_none());
            assert!(!editor.accept_source_completion(Some(old_option), cx));
            assert!(editor.request_source_completion(true, cx));
        })
    });
    executor.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    assert!(cx.debug_bounds("source-completion: <choiceNew").is_some());
    cx.update(|_, cx| {
        let current = editor.read(cx).current_source_completion().unwrap();
        assert!(
            current
                .session
                .options()
                .unwrap()
                .iter()
                .all(|option| option.item.label != "<choiceOld")
        );
    });

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.request_source_completion(true, cx));
            editor.completion.as_mut().unwrap().timer = None;
        })
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
            configure(editor, compartment.reconfigure(Extension::default()), cx);
            assert!(editor.current_source_completion().is_none());
            assert!(editor.request_source_completion(true, cx));
        })
    });
    cx.run_until_parked();
    executor.advance_clock(Duration::from_millis(49));
    cx.update(|_, cx| {
        let current = editor.read(cx).current_source_completion().unwrap();
        assert!(current.session.options().unwrap().is_empty());
    });
    executor.advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    cx.update(|_, cx| {
        let current = editor.read(cx).current_source_completion().unwrap();
        let options = current.session.options().unwrap();
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].item.label, "</root>");
    });
    executor.advance_clock(Duration::from_millis(75));
    cx.simulate_keystrokes("enter");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), draft.replace("<root> ", "<root> </root>"));
        assert_eq!(editor.view.undo_depth(), 1);
    });
}

#[gpui::test]
fn predefined_entities_stay_available_with_or_without_an_xml_schema(cx: &mut TestAppContext) {
    let draft = "```xml\r\n<root>&am;</root>\r\n```\r\n";
    let head = draft.find('&').unwrap() + "&am".len();
    for extension in [Extension::default(), schema_with_element("root", None)] {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_draft(editor, head, window, cx);
                configure(editor, extension.append(), cx);
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("source-completion: &amp;").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), draft.replace("&am;", "&amp;"));
            assert_eq!(editor.cursor(), head + 2);
            assert_eq!(editor.view.undo_depth(), 1);
        });
        cx.simulate_keystrokes("cmd-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), draft));
    }
}

#[gpui::test]
fn xml_schema_ignores_other_fences_and_stops_at_markers_or_composition(cx: &mut TestAppContext) {
    for fence in ["html", "xml"] {
        let draft = format!("```{fence}\r\n<roo\r\n```\r\n\r\nOutside.");
        let head = draft.find("<roo").unwrap() + 4;
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.clone(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_draft(editor, 3, window, cx);
                configure(editor, schema_with_element("rootCustom", None).append(), cx);
                assert!(!editor.request_source_completion(true, cx));
                editor.move_to(head + 2, false, cx); // Closing fence.
                assert!(!editor.request_source_completion(true, cx));
                editor.move_to(head, false, cx);
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("source-completion: rootCustom").is_some(),
            fence == "xml"
        );
        if fence == "xml" {
            cx.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    let id = editor
                        .current_source_completion()
                        .unwrap()
                        .session
                        .selected()
                        .unwrap();
                    editor.replace_and_mark_text_in_range(None, "中", None, window, cx);
                    assert!(editor.is_composing());
                    assert!(!editor.request_source_completion(true, cx));
                    assert!(!editor.accept_source_completion(Some(id), cx));
                    assert!(editor.current_source_completion().is_none());
                    assert_eq!(editor.source(), draft.replace("<roo", "<roo中"));
                })
            });
        }
    }
}

#[gpui::test]
fn xml_schema_cannot_insert_a_fence_or_redirect_a_pick_to_protected_source(
    cx: &mut TestAppContext,
) {
    let draft = "```xml\r\n<roo\r\n```\r\n\r\n```text\r\nProtected.\r\n```";
    let head = draft.find("<roo").unwrap() + 4;
    let protected = draft.find("Protected").unwrap();
    for (insert, redirected) in [
        (Some("root\r\n```\r\nOutside"), None),
        (None, Some(0..3)),
        (None, Some(protected..protected + 1)),
    ] {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
        let executor = cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_draft(editor, head, window, cx);
                configure(
                    editor,
                    Extension::group([
                        schema_with_element("root", insert),
                        transaction_filter(move |_, tx| {
                            if let Some(range) = &redirected
                                && tx.annotation(picked_completion()).is_some()
                            {
                                return Ok(Transaction::new(
                                    ChangeSet::single(Change::new(range.clone(), "bad")).unwrap(),
                                ));
                            }
                            Ok(tx)
                        }),
                    ])
                    .append(),
                    cx,
                );
            });
            cx.background_executor().clone()
        });
        cx.simulate_keystrokes("ctrl-space");
        executor.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("source-completion: root").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source().as_bytes(), draft.as_bytes());
            assert_eq!(editor.cursor(), head);
            assert!(!editor.view.can_undo());
        });
    }
}
