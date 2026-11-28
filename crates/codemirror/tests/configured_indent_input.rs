use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Precedence,
    StateField, Transaction, TransactionExtension, change_filter,
    collab::{CollabConfig, collab, sendable_updates},
    language::{
        INDENT_UNIT, IndentContext, IndentDecision, IndentStyle, indent_service,
        lezer::{INDENT, LrIndentStrategy},
    },
    language_data::{json_language, lezer_language},
    setup::basic_setup,
    transaction_extender,
    view::keymap::KeyPlatform,
};
use serde_json::{Value, json};

#[test]
fn basic_setup_uses_provisional_state_for_configured_lr_input() {
    for entry in include_str!("fixtures/configured_indent_input.jsonl").lines() {
        let row: Value = serde_json::from_str(entry).unwrap();
        let language = row["language"].as_str().unwrap();
        let node = match language {
            "json" => "Object",
            "lezer" => "Body",
            _ => panic!("unknown language {language}"),
        };
        let field_calls = Arc::new(AtomicUsize::new(0));
        let edits = edit_counter(&field_calls);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observe = {
            let seen = Arc::clone(&seen);
            let edits = edits.reader();
            move |name: &str, state: &EditorState, cx: &IndentContext<'_>| {
                let count = state.context().field(&edits).unwrap().unwrap();
                seen.lock().unwrap().push(json!([
                    name,
                    cx.position(),
                    state.selection().main().head(),
                    cx.unit(),
                    *count,
                    state.text().as_str(),
                ]));
                *count
            }
        };
        let config = |mode: &str, unit: &str| {
            let stop = mode == "stop";
            let body: LrIndentStrategy = Arc::new({
                let observe = observe.clone();
                move |cx| {
                    let lines = cx.line_context();
                    let count = observe(node, cx.state().expect("post-input state"), lines);
                    if stop {
                        None
                    } else {
                        Some(cx.continue_indent()? + count + lines.unit())
                    }
                }
            });
            let root: LrIndentStrategy = Arc::new({
                let observe = observe.clone();
                move |cx| {
                    observe(
                        "root",
                        cx.state().expect("post-input state"),
                        cx.line_context(),
                    );
                    Some(1)
                }
            });
            let props = [INDENT.add(move |ty| {
                if ty.is(node) {
                    Some(Some(body.clone()))
                } else if ty.is_top() {
                    Some(Some(root.clone()))
                } else {
                    None
                }
            })];
            let configured = !mode.starts_with("stock-");
            let parser = match (language, configured) {
                ("json", true) => json_language().with_node_props(&props).extension(),
                ("lezer", true) => lezer_language().with_node_props(&props).extension(),
                ("json", false) => json_language().extension(),
                ("lezer", false) => lezer_language().extension(),
                _ => unreachable!(),
            };
            let service = |name, decision| {
                let observe = observe.clone();
                indent_service(move |state, cx| {
                    observe(name, state, cx);
                    decision
                })
            };
            let decision = match mode {
                "stock-blocked" => IndentDecision::Inherit,
                "stock-service" => IndentDecision::Column(5),
                _ => IndentDecision::Defer,
            };
            Extension::group([
                parser,
                INDENT_UNIT.of(IndentStyle::parse(unit).unwrap()),
                service("middle", decision),
                service("high", IndentDecision::Defer).with_precedence(Precedence::High),
                service("low", IndentDecision::Defer).with_precedence(Precedence::Low),
            ])
        };
        let slot = Compartment::new();
        let filters = Arc::new(AtomicUsize::new(0));
        let extenders = Arc::new(AtomicUsize::new(0));
        let extension = Extension::group([
            slot.of(config("continue", "\t")),
            edits.extension(),
            collab(CollabConfig::default()),
            change_filter({
                let filters = Arc::clone(&filters);
                move |_, _| {
                    filters.fetch_add(1, Ordering::Relaxed);
                    Default::default()
                }
            }),
            transaction_extender({
                let extenders = Arc::clone(&extenders);
                move |_, _| {
                    extenders.fetch_add(1, Ordering::Relaxed);
                    TransactionExtension::default()
                }
            }),
        ]);
        let state = initial_state(&row, extension);
        let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
        for (index, step) in row["steps"].as_array().unwrap().iter().enumerate() {
            let id = format!("{language}: step {index}");
            let view = editor.view_mut();
            let before = field_calls.load(Ordering::Relaxed);
            let previous_text = view.state().text().clone();
            let queued = sendable_updates(view.state()).unwrap().len();
            filters.store(0, Ordering::Relaxed);
            extenders.store(0, Ordering::Relaxed);
            if step["undo"] == true {
                assert_eq!(view.undo(), Some(()), "{id}");
            } else {
                let mut tr = if let Some(insert) = step["insert"].as_str() {
                    view.state()
                        .replace_selection(insert)
                        .unwrap()
                        .with_user_event(step["event"].as_str().unwrap())
                } else if let Some(edit) = step["edit"].as_array() {
                    Transaction::new(
                        ChangeSet::single(Change::new(
                            byte(&edit[0])..byte(&edit[1]),
                            edit[2].as_str().unwrap(),
                        ))
                        .unwrap(),
                    )
                } else {
                    Transaction::default()
                };
                if let Some(mode) = step["mode"].as_str() {
                    tr = tr.with_effects([
                        slot.reconfigure(config(mode, step["unit"].as_str().unwrap()))
                    ]);
                }
                view.update(tr.with_time(index as u64 * 1000), (), |_| (None, ()))
                    .unwrap();
                assert_eq!(filters.load(Ordering::Relaxed), 1, "{id}");
                assert_eq!(extenders.load(Ordering::Relaxed), 1, "{id}");
            }
            let calls = std::mem::take(&mut *seen.lock().unwrap());
            assert_eq!(json!(calls), step["seen"], "{id}");
            if calls.is_empty() && step["insert"].is_string() {
                assert_eq!(field_calls.load(Ordering::Relaxed) - before, 1, "{id}");
            }
            assert_eq!(view.state().text().as_str(), step["result"], "{id}");
            assert_eq!(
                view.state().selection().main().head(),
                byte(&step["head"]),
                "{id}"
            );
            assert_eq!(view.undo_depth(), byte(&step["undoDepth"]), "{id}");
            let updates = sendable_updates(view.state()).unwrap();
            let changed = view.state().text() != &previous_text;
            assert_eq!(updates.len(), queued + usize::from(changed), "{id}");
            if changed {
                let mut text = previous_text;
                updates[queued].changes().apply_to(&mut text).unwrap();
                assert_eq!(&text, view.state().text(), "{id}");
            }
        }
    }
}

#[test]
fn stock_lr_input_does_not_need_provisional_state() {
    for entry in include_str!("fixtures/configured_indent_input.jsonl").lines() {
        let row: Value = serde_json::from_str(entry).unwrap();
        let lang = if row["language"] == "json" {
            json_language().extension()
        } else {
            lezer_language().extension()
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let state = initial_state(
            &row,
            Extension::group([lang, edit_counter(&calls).extension()]),
        );
        let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
        editor
            .insert_text("}", std::time::Duration::ZERO, ())
            .unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1, "{}", row["language"]);
        assert!(editor.view().state().text().as_str().ends_with("\r}\r\n"));
    }
}

fn edit_counter(calls: &Arc<AtomicUsize>) -> StateField<usize> {
    let calls = Arc::clone(calls);
    StateField::new(
        |_| Ok(0),
        move |count, update| {
            let changed = !update.transaction().changes().is_empty();
            calls.fetch_add(usize::from(changed), Ordering::Relaxed);
            Ok(count + usize::from(changed))
        },
    )
}

fn initial_state(row: &Value, extension: Extension) -> EditorState {
    let mut state =
        EditorState::with_extensions(row["source"].as_str().unwrap(), extension).unwrap();
    let caret = byte(&row["caret"]);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(caret, caret)))
        .unwrap();
    state
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}
