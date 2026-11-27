use std::{cell::RefCell, rc::Rc};

use gpui_editor_codemirror::{
    Compartment, EditorSelection, EditorState, Extension, Precedence, Transaction,
    search::{
        SearchPanelInput, close_search_panel, get_search_query, open_search_panel, search_panel_id,
        search_panel_open, search_panel_placement,
    },
    transaction_filter,
    view::{
        EditorView,
        panels::{PanelEvent, PanelLifecycle, PanelPlacement, active_panels, get_panel},
    },
};
use serde_json::{Value, json};

#[test]
fn search_opens_registers_and_reconfigures_a_stable_panel() {
    let layout = Compartment::new();
    let mut state =
        EditorState::with_extensions("猫 kitten\n猫", layout.of(Extension::default())).unwrap();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(0, 3)))
        .unwrap();
    let mut view = EditorView::new(state);
    let events = Rc::new(RefCell::new(Vec::new()));
    let capture = events.clone();
    view.add_plugin(PanelLifecycle::new(view.state(), move |event| {
        capture.borrow_mut().push(event)
    }));
    let id = search_panel_id();
    for row in include_str!("fixtures/search_panel_registration.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let step = case["step"].as_str().unwrap();
        let transaction = match step {
            "before" => None,
            "open_default" | "open_top" | "reopen_bottom" => {
                open_search_panel(view.state(), SearchPanelInput::Missing)
            }
            "close_bottom" | "close_top" => close_search_panel(view.state()),
            "configure_top_while_open" | "configure_bottom_while_open" => {
                let placement = if step == "configure_top_while_open" {
                    PanelPlacement::Top
                } else {
                    PanelPlacement::Bottom
                };
                Some(
                    Transaction::default()
                        .with_effects([layout.reconfigure(search_panel_placement(placement))]),
                )
            }
            _ => panic!("Unrecognized step {step}"),
        };
        if let Some(transaction) = transaction {
            view.update(transaction, (), |_| (None, ())).unwrap();
        }
        let state = view.state();
        assert_eq!(json!(search_panel_open(state)), case["open"], "{step}");
        assert_eq!(
            get_search_query(state).search(),
            case["query"].as_str().unwrap()
        );
        assert_eq!(state.text().as_str(), case["doc"].as_str().unwrap());
        assert_eq!(state.selection().main().head(), 3);
        assert!(!view.can_undo());
        let placement = get_panel(state, id).map(|panel| match panel.placement() {
            PanelPlacement::Top => "top",
            PanelPlacement::Bottom => "bottom",
        });
        assert_eq!(json!(placement), case["placement"], "{step}");
        let events = std::mem::take(&mut *events.borrow_mut());
        let outcome = match events.as_slice() {
            [] => "none",
            [PanelEvent::Mounted(slot)] if slot.panel.id() == id => "mounted",
            [PanelEvent::Updated(slot)] if slot.panel.id() == id => "updated",
            [PanelEvent::Destroyed(slot)] if slot.panel.id() == id => "destroyed",
            _ => panic!("Unexpected events at {step}: {events:?}"),
        };
        assert_eq!(
            outcome,
            if step == "before" {
                "none"
            } else if step.starts_with("close") {
                "destroyed"
            } else if step.starts_with("configure") {
                "updated"
            } else {
                "mounted"
            }
        );
    }
}

#[test]
fn placement_is_opt_in_and_filter_can_reject_opening() {
    let top = search_panel_placement(PanelPlacement::Top).with_precedence(Precedence::High);
    let bottom = search_panel_placement(PanelPlacement::Bottom);
    let mut state =
        EditorState::with_extensions("source", Extension::group([bottom, top])).unwrap();
    assert!(active_panels(&state).is_empty());
    state
        .update(open_search_panel(&state, SearchPanelInput::Missing).unwrap())
        .unwrap();
    assert_eq!(
        get_panel(&state, search_panel_id()).unwrap().placement(),
        PanelPlacement::Top
    );

    let mut rejected = EditorState::with_extensions(
        "source",
        transaction_filter(|_, _| Ok(Transaction::default())),
    )
    .unwrap();
    rejected
        .update(open_search_panel(&rejected, SearchPanelInput::Missing).unwrap())
        .unwrap();
    assert!(!search_panel_open(&rejected));
    assert!(active_panels(&rejected).is_empty());
}
