//! Chrome Headless 142 oracle: stock `findReferences`, `closeReferencePanel`,
//! and panel DOM with the manifest-verified 6.3.1 dist and an in-memory peer.
//! Native comparisons cover state and transactions, not browser focus.

use std::{cell::RefCell, rc::Rc, time::Duration};

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Text, Transaction,
    lsp::{
        DocumentWorkspace, LanguageServerCommand, LspClient,
        features::FeatureError,
        language_server_extensions,
        references::{
            ReferencePanel, ReferencePanelKeyAction, close_reference_panel, find_references,
            get_reference_panel, references_request,
        },
    },
    view::{
        EditorView,
        keymap::{KeyEvent, KeyModifiers, KeyPlatform, KeySequence, Keymap},
        panels::{Panel, PanelEvent, PanelLifecycle, PanelPlacement, active_panels},
    },
};
use serde_json::{Value, json};

fn dispatch(view: &mut EditorView<()>, transaction: Transaction) {
    view.update(transaction, (), |_| (None, ())).unwrap();
}

fn display(panel: &ReferencePanel) -> Value {
    let mut rows = Vec::new();
    for location in panel.entries() {
        let preview = location.preview.as_ref().unwrap();
        if preview.starts_file_group {
            rows.push(json!({"header": preview.file_name}));
        }
        rows.push(
            json!({"line": preview.line_number, "before": preview.before,
            "matched": preview.matched, "after": preview.after}),
        );
    }
    json!(rows)
}

#[test]
fn matches_browser_reference_panel_state_and_keys() {
    let mut fixture = include_str!("fixtures/lsp_reference_panel.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap());
    let meta = fixture.next().unwrap();
    let manifest: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let pin = manifest["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "@codemirror/lsp-client")
        .unwrap();
    assert_eq!(
        meta["package"],
        format!(
            "@codemirror/lsp-client@{}",
            pin["version"].as_str().unwrap()
        )
    );
    assert_eq!(meta["integrity"], pin["integrity"]);
    let reference_keys: Vec<_> = language_server_extensions()
        .keymap()
        .into_iter()
        .filter(|binding| {
            matches!(
                binding.run,
                Some(
                    LanguageServerCommand::FindReferences
                        | LanguageServerCommand::CloseReferencePanel
                )
            )
        })
        .collect();
    assert_eq!(
        json!(
            reference_keys
                .iter()
                .map(|b| (&b.key, b.prevent_default))
                .collect::<Vec<_>>()
        ),
        meta["keys"]
    );
    let keymap = Keymap::new(KeyPlatform::Linux, reference_keys).unwrap();
    let uri = meta["uri"].as_str().unwrap();
    let target = meta["otherUri"].as_str().unwrap();
    let mut view = EditorView::new(EditorState::new(meta["doc"].as_str().unwrap()));
    let mut target_view =
        EditorView::<()>::new(EditorState::new(meta["otherDoc"].as_str().unwrap()));
    dispatch(
        &mut view,
        Transaction::default().with_selection(EditorSelection::single(0, 10)),
    );
    let mut client = LspClient::default();
    client.attach(&mut view, uri, "rust").unwrap();
    client.attach(&mut target_view, target, "rust").unwrap();
    client.connected(&json!({"textDocumentSync": 2})).unwrap();
    client.flush().unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let queued = events.clone();
    view.add_plugin(PanelLifecycle::new(view.state(), move |event| {
        queued.borrow_mut().push(event)
    }));
    let mut ids = Vec::new();
    let mut pending = None;
    for step in fixture {
        let name = step["name"].as_str().unwrap();
        let old = get_reference_panel(view.state());
        let before_selection = view.state().selection().clone();
        let before_text = view.state().text().clone();
        if let Some(result) = step.get("result") {
            let request = pending.take().unwrap_or_else(|| {
                client.flush().unwrap();
                client.with_workspace(|ws| find_references(view.state(), ws, uri).unwrap())
            });
            if name == "open-mixed" {
                assert_eq!(*request.params(), meta["params"]);
            }
            let tx = client
                .with_workspace(|ws| request.resolve_panel(view.state(), ws, result))
                .unwrap();
            assert_eq!(
                tx.is_some(),
                step["panel"].as_u64() == Some(ids.len() as u64),
                "{name}"
            );
            if let Some(tx) = tx {
                dispatch(&mut view, tx);
            }
        } else if let Some(key) = step["key"].as_str() {
            let panel = old.as_ref().unwrap();
            let action = panel.key_action(key);
            assert_eq!(json!(action.is_some()), step["handled"], "{name}");
            match action {
                Some(ReferencePanelKeyAction::Update(tx)) => dispatch(&mut view, tx),
                Some(ReferencePanelKeyAction::Activate) => {
                    let (target, tx) = client.with_workspace(|ws| panel.activate(ws)).unwrap();
                    assert_eq!(target, uri);
                    assert!(tx.scroll_into_view());
                    assert!(tx.changes().is_empty());
                    dispatch(&mut view, tx);
                }
                Some(ReferencePanelKeyAction::Keep) => {
                    assert_eq!(json!(panel.selected_index()), step["selected"]);
                }
                None => {}
            }
        } else if let Some(index) = step["click"].as_u64() {
            let panel = old.as_ref().unwrap();
            let tx = panel.select_row(usize::try_from(index).unwrap()).unwrap();
            assert!(tx.selection().is_none() && tx.changes().is_empty());
            dispatch(&mut view, tx);
            let selected = get_reference_panel(view.state()).unwrap();
            let (target, reveal) = client.with_workspace(|ws| selected.activate(ws)).unwrap();
            assert_eq!(target, uri);
            assert!(reveal.scroll_into_view() && reveal.changes().is_empty());
            dispatch(&mut view, reveal);
            assert_eq!(step["handled"], true);
        } else if let Some(text) = step["inserted"].as_str() {
            dispatch(
                &mut view,
                Transaction::new(ChangeSet::single(Change::new(0..0, text)).unwrap()),
            );
        } else if name.starts_with("escape-in-editor") {
            let handled = keymap.dispatch(
                "editor",
                KeyEvent::new("Escape", KeyModifiers::default()),
                Duration::ZERO,
                &mut KeySequence::default(),
                |command| {
                    assert_eq!(*command, LanguageServerCommand::CloseReferencePanel);
                    close_reference_panel(view.state()).is_some_and(|tx| {
                        dispatch(&mut view, tx);
                        true
                    })
                },
            );
            assert_eq!(json!(handled.handled), step["handled"], "{name}");
        } else {
            if name == "close-while-pending" {
                let modifiers = KeyModifiers {
                    shift: true,
                    ..KeyModifiers::default()
                };
                let handled = keymap.dispatch(
                    "editor",
                    KeyEvent::new("F12", modifiers),
                    Duration::ZERO,
                    &mut KeySequence::default(),
                    |command| {
                        assert_eq!(*command, LanguageServerCommand::FindReferences);
                        client.flush().unwrap();
                        pending =
                            Some(client.with_workspace(|ws| {
                                find_references(view.state(), ws, uri).unwrap()
                            }));
                        true
                    },
                );
                assert_eq!(json!(handled.handled), step["pending"]);
            }
            let tx = close_reference_panel(view.state());
            assert_eq!(json!(tx.is_some()), step["handled"], "{name}");
            if let Some(tx) = tx {
                dispatch(&mut view, tx);
            }
        }
        let current = get_reference_panel(view.state());
        let slots = active_panels(view.state());
        let ordinal = current.as_ref().map(|panel| {
            let id = panel.id();
            assert_eq!(
                *slots,
                [Panel::Custom {
                    id,
                    placement: PanelPlacement::Bottom
                }]
            );
            if let Some(expected) = step.get("display") {
                assert_eq!(&display(panel), expected, "{name}");
            }
            ids.iter()
                .position(|&known| known == id)
                .unwrap_or_else(|| {
                    ids.push(id);
                    ids.len() - 1
                })
        });
        if current.is_none() {
            assert!(slots.is_empty());
        }
        assert_eq!(json!(ordinal), step["panel"], "{name}");
        assert_eq!(
            json!(current.as_ref().map(ReferencePanel::selected_index)),
            step["selected"],
            "{name}"
        );
        let old_id = old.map(|panel| panel.id());
        let new_id = current.map(|panel| panel.id());
        let mut events = events.borrow_mut();
        let transitions: Vec<_> = events
            .drain(..)
            .filter_map(|event| match event {
                PanelEvent::Destroyed(slot) => Some((false, slot.panel.id())),
                PanelEvent::Mounted(slot) => Some((true, slot.panel.id())),
                _ => None,
            })
            .collect();
        if old_id == new_id {
            assert!(transitions.is_empty(), "{name}");
        } else {
            assert_eq!(
                transitions,
                old_id
                    .map(|id| (false, id))
                    .into_iter()
                    .chain(new_id.map(|id| (true, id)))
                    .collect::<Vec<_>>(),
                "{name}"
            );
        }
        if let Some(position) = step.get("position") {
            assert_eq!(
                json!(view.state().selection().main().head()),
                *position,
                "{name}"
            );
        }
        if name != "edit-before-activation" {
            assert_eq!(view.state().text(), &before_text, "{name}");
            if name != "enter" && step["click"].is_null() {
                assert_eq!(view.state().selection(), &before_selection, "{name}");
            }
        }
    }
    assert!(pending.is_none());
}

fn reference(uri: &str, start: u32, end: u32) -> Value {
    json!({"uri": uri, "range": {"start": {"line": 0, "character": start},
        "end": {"line": 0, "character": end}}})
}

#[test]
fn invalid_or_stale_replies_leave_the_live_panel_unchanged() {
    let (uri, target, late) = ("file:///source", "file:///target", "file:///late");
    let mut workspace = DocumentWorkspace::default();
    workspace
        .open_file(uri, "rust", Text::new("😀word"))
        .unwrap();
    workspace
        .open_file(target, "rust", Text::new("x word"))
        .unwrap();
    let mut state = EditorState::new("😀word");
    let request = references_request(&workspace, uri, 4).unwrap();
    let wire = json!([reference(uri, 2, 6), reference(target, 2, 6)]);
    let tx = request
        .resolve_panel(&state, &workspace, &wire)
        .unwrap()
        .unwrap();
    assert!(
        state
            .update(
                Transaction::new(ChangeSet::single(Change::new(99..99, "!")).unwrap())
                    .with_effects(tx.effects().iter().cloned())
            )
            .is_err()
    );
    assert!(get_reference_panel(&state).is_none());
    state.update(tx).unwrap();
    let panel = get_reference_panel(&state).unwrap();
    let key = panel.key_action("ArrowDown").unwrap();
    let ReferencePanelKeyAction::Update(tx) = key else {
        panic!("expected row selection")
    };
    state.update(tx).unwrap();
    let selected = get_reference_panel(&state).unwrap();
    let change = ChangeSet::single(Change::new(0..0, "é")).unwrap();
    workspace.record_change(target, &change).unwrap();
    let (activation_uri, tx) = selected.activate(&workspace).unwrap();
    assert_eq!(activation_uri, target);
    assert_eq!(tx.selection(), Some(&EditorSelection::single(4, 4)));
    assert!(tx.scroll_into_view());
    workspace
        .open_file(late, "rust", Text::new("word"))
        .unwrap();
    assert!(
        request
            .resolve_panel(&state, &workspace, &json!([reference(late, 0, 4)]))
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        request.resolve_panel(&state, &workspace, &json!([reference(uri, 1, 2)])),
        Err(FeatureError::Protocol(_))
    ));
    assert!(
        matches!(request.resolve_panel(&EditorState::new("wrong snapshot"), &workspace, &wire), Err(FeatureError::StaleResponse { uri: rejected, .. }) if rejected == uri)
    );
    workspace
        .record_change(target, &ChangeSet::single(Change::new(5..6, "!")).unwrap())
        .unwrap();
    assert!(
        matches!(request.resolve_panel(&state, &workspace, &wire), Err(FeatureError::StaleResponse { uri, .. }) if uri == target)
    );
    assert!(
        matches!(selected.activate(&workspace), Err(FeatureError::StaleResponse { uri, .. }) if uri == target)
    );
    workspace.close_file(target).unwrap();
    workspace
        .open_file(target, "rust", Text::new("x word"))
        .unwrap();
    assert!(
        matches!(request.resolve_panel(&state, &workspace, &wire), Err(FeatureError::StaleResponse { uri, current: Some(1), .. }) if uri == target)
    );
    workspace.close_file(uri).unwrap();
    assert!(
        matches!(request.resolve_panel(&state, &workspace, &wire), Err(FeatureError::StaleResponse { uri: rejected, current: None, .. }) if rejected == uri)
    );
    let live = get_reference_panel(&state).unwrap();
    assert_eq!(live.id(), panel.id());
    assert_eq!(live.selected_index(), 1);
}

#[test]
fn actions_from_replaced_panels_do_not_affect_the_new_panel() {
    let uri = "file:///source";
    let mut state = EditorState::new("word word");
    let mut ws = DocumentWorkspace::default();
    ws.open_file(uri, "rust", state.text().clone()).unwrap();
    let request = find_references(&state, &ws, uri).unwrap();
    let result = json!([reference(uri, 0, 4), reference(uri, 5, 9)]);
    let open = |state: &EditorState| request.resolve_panel(state, &ws, &result).unwrap().unwrap();
    state.update(open(&state)).unwrap();
    let old = get_reference_panel(&state).unwrap();
    assert!(old.select_row(0).is_none());
    assert!(old.select_row(old.entries().len()).is_none());
    let select = old.select_row(1).unwrap();
    let close = close_reference_panel(&state).unwrap();
    state.update(open(&state)).unwrap();
    let new = get_reference_panel(&state).unwrap();
    assert_ne!(old.id(), new.id());
    state.update(select).unwrap();
    state.update(close).unwrap();
    let current = get_reference_panel(&state).unwrap();
    assert_eq!(current.id(), new.id());
    assert_eq!(current.selected_index(), 0);
}
