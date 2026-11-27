use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditorState, Extension, Precedence, Transaction,
    view::{
        EditorView, ViewException, ViewPlugin, ViewPluginType, ViewUpdate, Viewport,
        exception_sink, update_listener,
    },
};

type Events = Arc<Mutex<Vec<String>>>;
struct Probe {
    log: Events,
    label: String,
    updates: usize,
}

impl ViewPlugin for Probe {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        self.updates += 1;
        self.log.lock().unwrap().push(format!(
            "update:{}:{}:{}",
            self.label,
            update.state.text().as_str(),
            update.doc_changed()
        ));
    }

    fn destroy(&mut self) {
        self.log
            .lock()
            .unwrap()
            .push(format!("destroy:{}", self.label));
    }
}

fn probe(name: &'static str, log: Events) -> ViewPluginType<&'static str, Probe> {
    ViewPluginType::define(move |state, argument| {
        let label = format!("{name}/{argument}");
        log.lock()
            .unwrap()
            .push(format!("create:{label}:{}", state.text().as_str()));
        Ok(Probe {
            label,
            log: log.clone(),
            updates: 0,
        })
    })
}

fn dispatch(view: &mut EditorView<()>, transaction: Transaction) {
    view.update(transaction, (), |_| (None, ())).unwrap();
}

#[test]
fn typed_extensions_match_pinned_plugin_order_and_lifecycle() {
    let log = Events::default();
    let a = probe("a", log.clone());
    let b = probe("b", log.clone());
    let compartment = Compartment::new();
    let keep = a.of("keep");
    let beta = b.of("b");
    let state = EditorState::with_extensions(
        "😀",
        compartment.of(Extension::group([
            a.of("ignored"),
            keep.clone().with_precedence(Precedence::High),
            beta.clone(),
        ])),
    )
    .unwrap();
    let mut view = EditorView::<()>::new(state);
    assert_eq!(view.plugin(&a).unwrap().label, "a/keep");
    assert_eq!(view.plugin(&a).unwrap().updates, 0);
    dispatch(
        &mut view,
        Transaction::new(ChangeSet::single(Change::new(4..4, "!")).unwrap()),
    );
    dispatch(
        &mut view,
        Transaction::default()
            .with_effects([compartment.reconfigure(Extension::group([beta.clone(), keep]))]),
    );
    assert_eq!(view.plugin(&a).unwrap().updates, 2);
    dispatch(
        &mut view,
        Transaction::new(ChangeSet::single(Change::new(0..0, "x")).unwrap()).with_effects([
            compartment.reconfigure(Extension::group([beta.clone(), a.of("fresh")])),
        ]),
    );
    assert_eq!(view.plugin(&a).unwrap().updates, 0);
    assert_eq!(view.plugin(&b).unwrap().updates, 3);
    assert!(view.can_undo());
    let revision = view.revision();
    view.set_state(EditorState::with_extensions("other", beta).unwrap());
    assert!(view.plugin(&a).is_none());
    assert_eq!(view.plugin(&b).unwrap().updates, 0);
    assert_eq!(view.viewport().interval(), 0..5);
    assert!(!view.can_undo());
    assert_ne!(view.revision(), revision);
    drop(view);
    // Generated in JSDOM from the actual pinned view 6.44.0 and state 6.7.6.
    assert_eq!(
        *log.lock().unwrap(),
        [
            "create:a/keep:😀",
            "create:b/b:😀",
            "update:a/keep:😀!:true",
            "update:b/b:😀!:true",
            "update:b/b:😀!:false",
            "update:a/keep:😀!:false",
            "destroy:a/keep",
            "update:b/b:x😀!:true",
            "create:a/fresh:x😀!",
            "destroy:b/b",
            "destroy:a/fresh",
            "create:b/b:other",
            "destroy:b/b",
        ]
    );
}

struct Failed;
impl ViewPlugin for Failed {
    fn try_update(&mut self, _: &ViewUpdate<'_>) -> Result<(), ViewException> {
        Err(ViewException::new("configured plugin update failed"))
    }
}

#[test]
fn configured_plugin_failures_do_not_retry_until_a_new_binding() {
    let constructed = Arc::new(AtomicUsize::new(0));
    let errors = Arc::new(AtomicUsize::new(0));
    let broken = ViewPluginType::define({
        let constructed = constructed.clone();
        move |_, &fail_at_create: &bool| {
            constructed.fetch_add(1, Ordering::SeqCst);
            if fail_at_create {
                Err(ViewException::new("configured plugin construction failed"))
            } else {
                Ok(Failed)
            }
        }
    });
    let compartment = Compartment::new();
    let state = EditorState::with_extensions(
        "a",
        Extension::group([
            compartment.of(Extension::default()),
            exception_sink({
                let errors = errors.clone();
                move |_| {
                    errors.fetch_add(1, Ordering::SeqCst);
                }
            }),
        ]),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    dispatch(
        &mut view,
        Transaction::default().with_effects([compartment.reconfigure(broken.of(true))]),
    );
    assert!(view.plugin(&broken).is_none());
    dispatch(&mut view, Transaction::default());
    assert_eq!(constructed.load(Ordering::SeqCst), 1);
    assert_eq!(errors.load(Ordering::SeqCst), 1);
    dispatch(
        &mut view,
        Transaction::default().with_effects([compartment.reconfigure(broken.of(false))]),
    );
    assert!(view.plugin(&broken).is_some());
    view.set_viewport(Viewport::complete(view.state().text()), true)
        .unwrap();
    assert!(view.plugin(&broken).is_none());
    dispatch(
        &mut view,
        Transaction::new(ChangeSet::single(Change::new(0..1, "b")).unwrap()),
    );
    assert!(view.plugin(&broken).is_none());
    assert_eq!(constructed.load(Ordering::SeqCst), 2);
    assert_eq!(errors.load(Ordering::SeqCst), 2);
    assert_eq!(view.state().text().as_str(), "b");
}

#[test]
fn listeners_observe_the_reconfiguration_that_installs_them_after_plugins() {
    let log = Events::default();
    let a = probe("a", log.clone());
    let c = Compartment::new();
    let listener = |label| {
        let log = log.clone();
        update_listener(move |update| {
            log.lock().unwrap().push(format!(
                "listener:{label}:{}:{}",
                update.start_state.text().as_str(),
                update.state.text().as_str()
            ));
            if label == "first" {
                Err(ViewException::new("later listener must still run"))
            } else {
                Ok(())
            }
        })
    };
    let state = EditorState::with_extensions("a", c.of(Extension::default())).unwrap();
    let mut view = EditorView::new(state);
    dispatch(
        &mut view,
        Transaction::new(ChangeSet::single(Change::new(1..1, "b")).unwrap()).with_effects([c
            .reconfigure(Extension::group([
                a.of("installed"),
                listener("later"),
                listener("first").with_precedence(Precedence::High),
            ]))]),
    );
    dispatch(
        &mut view,
        Transaction::new(ChangeSet::single(Change::new(2..2, "c")).unwrap()),
    );
    assert_eq!(
        *log.lock().unwrap(),
        [
            "create:a/installed:ab",
            "listener:first:a:ab",
            "listener:later:a:ab",
            "update:a/installed:abc:true",
            "listener:first:ab:abc",
            "listener:later:ab:abc",
        ]
    );
}
