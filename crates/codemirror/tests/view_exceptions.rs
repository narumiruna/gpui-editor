use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorState, Extension, Transaction,
    view::{EditorView, ViewException, ViewPlugin, ViewUpdate, exception_sink, log_exception},
};
use serde_json::{Value, json};

type Events = Arc<Mutex<Vec<String>>>;

struct Observer {
    events: Events,
    fail: bool,
}

impl ViewPlugin for Observer {
    fn try_update(&mut self, update: &ViewUpdate<'_>) -> Result<(), ViewException> {
        if !update.doc_changed() {
            return Ok(());
        }
        if self.fail {
            self.events.lock().unwrap().push("update:failure".into());
            Err(ViewException::new("update"))
        } else {
            self.events
                .lock()
                .unwrap()
                .push(format!("update:{}", update.state.text().as_str()));
            Ok(())
        }
    }

    fn destroy(&mut self) {
        let name = if self.fail { "failure" } else { "survivor" };
        self.events.lock().unwrap().push(format!("destroy:{name}"));
    }
}

#[test]
fn failed_plugins_are_removed_without_rolling_back_the_accepted_edit() {
    let events = Events::default();
    let sink = |name| {
        let events = events.clone();
        exception_sink(move |error| {
            events
                .lock()
                .unwrap()
                .push(format!("{name}:{}", error.message))
        })
    };
    let state =
        EditorState::with_extensions("a", Extension::group([sink("first"), sink("second")]))
            .unwrap();
    let mut view = EditorView::<()>::new(state);
    let creation = view.try_add_plugin::<Observer>(|_| {
        events.lock().unwrap().push("create:failure".into());
        Err(ViewException::new("construction"))
    });
    let failed = view.add_plugin(Observer {
        events: events.clone(),
        fail: true,
    });
    let survivor = view.add_plugin(Observer {
        events: events.clone(),
        fail: false,
    });
    assert!(log_exception(
        view.state(),
        &ViewException::new("manual").with_context("caller context")
    ));
    for (at, value) in [(1, "b"), (2, "c")] {
        let changes = ChangeSet::new(vec![Change::new(at..at, value)]).unwrap();
        view.update(Transaction::new(changes), (), |_| (None, ()))
            .unwrap();
    }
    let final_doc = view.state().text().as_str().to_owned();
    let active = [
        creation.is_some(),
        view.remove_plugin(failed),
        view.remove_plugin(survivor),
    ];
    drop(view);
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/view_exceptions.jsonl")).unwrap();
    assert_eq!(
        json!({"events": *events.lock().unwrap(), "final": final_doc, "active": active}),
        expected
    );
    assert!(!log_exception(
        &EditorState::new(""),
        &ViewException::new("no sink")
    ));
}
