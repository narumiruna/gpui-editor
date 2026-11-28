//! Replay the original HTML/XML handlers with a host default that changes parsers.
//! The ASCII fixture comes from SHA-512 verified pins in `docs/codemirror-upstream.json`.

use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Transaction,
    language_data::{LANGUAGES, LanguageDescription, NativeSyntax, attached_language_snapshot},
    transaction_filter,
    view::{EditorView, input::InputStep, update_listener},
};
use serde_json::{Value, json};

fn catalog_extension(name: &str) -> Extension {
    let description = LanguageDescription::match_name(LANGUAGES, name, false).unwrap();
    let mut load = description.catalog_support_load().unwrap().load();
    let Poll::Ready(Ok(support)) =
        Pin::new(&mut load).poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("missing catalog support for {name}");
    };
    support.extension().unwrap()
}

fn html_is_attached(state: &EditorState) -> bool {
    match attached_language_snapshot(state).unwrap().syntax() {
        NativeSyntax::Html(_) => true,
        NativeSyntax::Xml(_) => false,
        _ => panic!("expected a direct HTML or XML parser"),
    }
}

fn snapshot(state: &EditorState) -> Value {
    let selection = state.selection().main();
    json!({"doc": state.text().as_str(), "selection": [selection.anchor(), selection.head()]})
}

fn trace(step: &InputStep) -> Value {
    let tr = &step.transaction;
    let changes = tr
        .changes()
        .changes()
        .map(|(r, text)| json!([r.start, r.end, text]))
        .collect::<Vec<_>>();
    json!({"start": snapshot(&step.start_state), "end": snapshot(&step.state),
        "changes": changes, "event": tr.user_event()})
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

#[test]
fn direct_switch_keeps_old_handler_policy_on_accepted_syntax() {
    let html = catalog_extension("HTML");
    let xml = catalog_extension("XML");
    for line in include_str!("fixtures/direct_tag_switch.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        let old_html = row["old"] == "html";
        let (old, next) = if old_html {
            (&html, &xml)
        } else {
            (&xml, &html)
        };
        let source = row["doc"].as_str().unwrap();
        let at = byte(&row["at"]);
        let typed = row["typed"].as_str().unwrap();
        let rewrite = row["rewrite"].as_array().map(|spec| {
            (
                spec[0].as_str().unwrap().to_owned(),
                byte(&spec[1]),
                byte(&spec[2]),
            )
        });
        let filter = transaction_filter(move |start, tr| {
            if let Some((insert, anchor, head)) = &rewrite
                && tr.is_user_event("input.type")
            {
                return Ok(Transaction::new(ChangeSet::single(Change::new(
                    0..start.text().len(),
                    insert.as_str(),
                ))?)
                .with_selection(EditorSelection::single(*anchor, *head))
                .with_user_event("input.type")
                .with_effects(tr.effects().iter().cloned())
                .with_scroll_into_view());
            }
            Ok(tr)
        });
        let observations = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&observations);
        let listener = update_listener(move |update| {
            for (index, step) in update.input_steps.iter().enumerate() {
                assert_eq!(
                    html_is_attached(&step.start_state),
                    if index == 0 { old_html } else { !old_html }
                );
                assert_ne!(html_is_attached(&step.state), old_html);
            }
            log.lock()
                .unwrap()
                .push(update.input_steps.iter().map(trace).collect::<Vec<_>>());
            Ok(())
        });
        let slot = Compartment::new();
        let state = EditorState::create(
            source,
            Some(EditorSelection::single(at, at)),
            Extension::group([slot.of(old.clone()), filter, listener]),
        )
        .unwrap();
        let mut view = EditorView::new(state);
        view.apply_headless_input(
            at..at,
            typed,
            |start| {
                Ok(start
                    .replace_selection(typed)?
                    .with_effects([slot.reconfigure(next.clone())])
                    .with_user_event("input.type")
                    .with_scroll_into_view())
            },
            (),
        )
        .unwrap();
        let updates = observations.lock().unwrap();
        assert_eq!(updates.len(), 1, "{name}");
        assert_eq!(json!(updates[0]), row["steps"], "{name}");
    }
}
