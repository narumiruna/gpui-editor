use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use gpui_editor_codemirror::{
    Compartment, ConfigurationError, EditError, EditorState, Extension, Precedence, Transaction,
    search::{
        SearchOptions, SearchPanel, SearchPanelInput, SearchQuery, close_search_panel, extension,
        get_search_query, open_search_panel, search_panel_factory, search_panel_id,
        search_panel_open, search_panel_placement, set_search_query,
    },
    transaction_filter,
    view::{
        EditorView, ViewUpdate,
        panels::{PanelPlacement, get_panel},
    },
};
use serde_json::{Value, json};

#[derive(Default)]
struct Trace {
    events: Mutex<Vec<Value>>,
    serial: AtomicUsize,
}

impl Trace {
    fn factory(self: &Arc<Self>, name: &'static str) -> Extension {
        let trace = self.clone();
        search_panel_factory(move |state| {
            assert!(search_panel_open(state));
            assert!(get_panel(state, search_panel_id()).is_some());
            let serial = trace.serial.fetch_add(1, Ordering::Relaxed) + 1;
            let id = format!("{name}{serial}");
            trace.record(json!(["create", id, get_search_query(state).search()]));
            RecordedPanel {
                id,
                trace: trace.clone(),
            }
        })
    }

    fn record(&self, event: Value) {
        self.events.lock().unwrap().push(event);
    }

    fn take(&self) -> Value {
        json!(std::mem::take(&mut *self.events.lock().unwrap()))
    }
}

struct RecordedPanel {
    id: String,
    trace: Arc<Trace>,
}

impl SearchPanel for RecordedPanel {
    fn mount(&mut self) {
        self.trace.record(json!(["mount", self.id]));
    }

    fn update(&mut self, update: &ViewUpdate<'_>) {
        self.trace.record(json!([
            "update",
            self.id,
            get_search_query(update.state).search(),
        ]));
    }

    fn destroy(&mut self) {
        self.trace.record(json!(["destroy", self.id]));
    }
}

fn open(state: &EditorState) -> Option<Transaction> {
    open_search_panel(state, SearchPanelInput::Missing)
}

fn apply(view: &mut EditorView<()>, transaction: Option<Transaction>) {
    if let Some(transaction) = transaction {
        view.update(transaction, (), |_| (None, ())).unwrap();
    }
}

#[test]
fn factory_lifecycle_matches_pinned_search() {
    let trace = Arc::new(Trace::default());
    let slot = Compartment::new();
    let a = trace.factory("a");
    let b = trace.factory("b");
    let config = |factory: &Extension, placement| {
        Extension::group([factory.clone(), search_panel_placement(placement)])
    };
    let state = EditorState::with_extensions(
        "cat dog cat",
        Extension::group([
            extension(SearchOptions::default()),
            slot.of(config(&a, PanelPlacement::Bottom)),
        ]),
    )
    .unwrap();
    let mut view = Some(EditorView::new(state));
    for row in factory_rows().filter(|row| row["step"] != "factory_config") {
        let step = row["step"].as_str().unwrap();
        if step == "destroy_view" {
            drop(view.take().unwrap());
        } else {
            let view = view.as_mut().unwrap();
            let reconfigure = |factory, placement| {
                Some(
                    Transaction::default()
                        .with_effects([slot.reconfigure(config(factory, placement))]),
                )
            };
            let transaction = match step {
                "before" => None,
                "open_a" | "open_again" | "reopen_b" => open(view.state()),
                "equivalent_a" => reconfigure(&a, PanelPlacement::Bottom),
                "change_top" => reconfigure(&a, PanelPlacement::Top),
                "change_factory" => reconfigure(&b, PanelPlacement::Top),
                "set_query" => Some(Transaction::default().with_effects([set_search_query(
                    SearchQuery::from_input("dog", SearchOptions::default()),
                )])),
                "close" => close_search_panel(view.state()),
                _ => panic!("Unrecognized step {step}"),
            };
            apply(view, transaction);
            let placement =
                get_panel(view.state(), search_panel_id()).map(|panel| match panel.placement() {
                    PanelPlacement::Top => "top",
                    PanelPlacement::Bottom => "bottom",
                });
            assert_eq!(json!(placement), row["placement"], "{step}");
            assert!(!view.can_undo());
        }
        assert_eq!(trace.take(), row["events"], "{step}");
    }
    assert!(view.is_none());
}

fn factory_rows() -> impl Iterator<Item = Value> {
    include_str!("fixtures/search_panel_factory.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

#[test]
fn factory_conflicts_reject_creation_and_live_reconfiguration() {
    let expected = factory_rows()
        .find(|row| row["step"] == "factory_config")
        .unwrap();
    let trace = Arc::new(Trace::default());
    let a = trace.factory("a");
    let b = trace.factory("b").with_precedence(Precedence::High);
    let initial = |extensions| EditorState::with_extensions("source", extensions);
    let duplicate = Extension::group([a.clone(), a.clone().with_precedence(Precedence::High)]);
    let conflict = Extension::group([a.clone(), b]);
    let rejection = ConfigurationError::InvalidFieldData;
    assert_eq!(
        json!(initial(duplicate.clone()).is_err()),
        expected["duplicate"]
    );
    let rejected = initial(conflict.clone());
    assert_eq!(json!(rejected.is_err()), expected["conflict"]);
    assert_eq!(rejected.unwrap_err(), rejection);

    let slot = Compartment::new();
    let mut view = EditorView::new(initial(slot.of(duplicate)).unwrap());
    let prepare = open(view.state());
    apply(&mut view, prepare);
    assert_eq!(trace.take(), json!([["create", "a1", ""], ["mount", "a1"]]));
    let invalid = Transaction::default().with_effects([slot.reconfigure(conflict)]);
    assert_eq!(
        view.update(invalid, (), |_| (None, ())),
        Err(EditError::Configuration(rejection))
    );
    assert!(search_panel_open(view.state()));
    assert_eq!(trace.take(), json!([]));
    assert!(!view.can_undo());
}

#[test]
fn adding_and_removing_factory_only_affects_the_next_open() {
    let trace = Arc::new(Trace::default());
    let slot = Compartment::new();
    let mut view = EditorView::new(
        EditorState::with_extensions("source", slot.of(Extension::default())).unwrap(),
    );
    let transaction = open(view.state());
    apply(&mut view, transaction);
    let configure =
        |extension| Some(Transaction::default().with_effects([slot.reconfigure(extension)]));
    apply(&mut view, configure(trace.factory("a")));
    assert_eq!(trace.take(), json!([]));
    let close = close_search_panel(view.state());
    apply(&mut view, close);
    let reopen = open(view.state());
    apply(&mut view, reopen);
    assert_eq!(trace.take(), json!([["create", "a1", ""], ["mount", "a1"]]));
    apply(&mut view, configure(Extension::default()));
    assert_eq!(trace.take(), json!([["update", "a1", ""]]));
    let close = close_search_panel(view.state());
    apply(&mut view, close);
    let reopen = open(view.state());
    apply(&mut view, reopen);
    drop(view);
    assert_eq!(trace.take(), json!([["destroy", "a1"]]));
}

#[test]
fn only_a_view_of_an_accepted_open_state_creates_a_panel() {
    let trace = Arc::new(Trace::default());
    let state = EditorState::with_extensions(
        "source",
        Extension::group([
            trace.factory("a"),
            transaction_filter(|_, _| Ok(Transaction::default())),
        ]),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    let transaction = open(view.state());
    apply(&mut view, transaction);
    assert!(!search_panel_open(view.state()));
    assert_eq!(trace.take(), json!([]));

    let mut state = EditorState::with_extensions("source", trace.factory("a")).unwrap();
    state.update(open(&state).unwrap()).unwrap();
    assert!(search_panel_open(&state));
    assert_eq!(trace.take(), json!([]));
    let a = EditorView::<()>::new(state.clone());
    let b = EditorView::<()>::new(state);
    drop(a);
    drop(b);
    assert_eq!(
        trace.take(),
        json!([
            ["create", "a1", ""],
            ["mount", "a1"],
            ["create", "a2", ""],
            ["mount", "a2"],
            ["destroy", "a1"],
            ["destroy", "a2"],
        ])
    );
}
