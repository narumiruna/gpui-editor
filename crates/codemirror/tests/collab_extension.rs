use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

use gpui_editor_codemirror::{
    Change, ChangePermission, ChangeSet, Compartment, ConfigurationError, EditError,
    EditorSelection, EditorState, Extension, History, StateEffectType, StateField, Text,
    Transaction, TransactionCoordinates, TransactionExtension, change_filter,
    collab::{
        CollabConfig, CollabError, Update, UpdateDescription, collab, collab_state, get_client_id,
        get_synced_version, is_remote, rebase_updates, rebase_updates_described, receive_updates,
        sendable_updates,
    },
    inverted_effects, transaction_extender, transaction_filter,
    view::EditorView,
};
use serde_json::{Value, json};

fn change(range: std::ops::Range<usize>, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(range, insert)).unwrap()
}

#[test]
fn ordinary_edits_and_undo_queue_updates_but_authority_traffic_does_not() {
    let state = EditorState::with_extensions("abcd", collab(CollabConfig::new("a", 4))).unwrap();
    let mut history = History::new(state);
    history
        .update(
            Transaction::new(change(2..2, "X")).with_user_event("input.type"),
            (),
            |_| (None, ()),
        )
        .unwrap();
    let a = collab_state(history.state()).unwrap().unwrap();
    assert_eq!(a.client_id(), "a");
    assert_eq!(a.synced_version(), 4);
    assert_eq!(a.sendable_updates().len(), 1);
    assert_eq!(history.state().text().as_str(), "abXcd");

    let remote = Update::new("b", change(0..0, "Y"), 4).unwrap();
    let apply = receive_updates(history.state(), 4, &[remote]).unwrap();
    assert!(is_remote(&apply));
    assert!(!apply.add_to_history());
    history.update(apply, (), |_| (None, ())).unwrap();
    let rebased = collab_state(history.state()).unwrap().unwrap();
    assert_eq!(rebased.synced_version(), 5);
    assert_eq!(rebased.sendable_updates()[0].changes(), &change(3..3, "X"));
    assert_eq!(history.state().text().as_str(), "YabXcd");

    history.undo().unwrap();
    assert_eq!(history.state().text().as_str(), "Yabcd");
    let a = collab_state(history.state()).unwrap().unwrap();
    assert_eq!(a.sendable_updates().len(), 2);
    assert!(
        a.sendable_updates()[1]
            .origin()
            .unwrap()
            .is_user_event("undo")
    );
    let ack = receive_updates(history.state(), 5, a.sendable_updates()).unwrap();
    assert!(!is_remote(&ack));
    history.update(ack, (), |_| (None, ())).unwrap();
    let accepted = collab_state(history.state()).unwrap().unwrap();
    assert_eq!(accepted.synced_version(), 7);
    assert!(accepted.sendable_updates().is_empty());
    assert!(history.can_redo());
    history.redo().unwrap();
    assert_eq!(history.state().text().as_str(), "YabXcd");
    assert_eq!(
        collab_state(history.state())
            .unwrap()
            .unwrap()
            .sendable_updates()
            .len(),
        1
    );
}

#[test]
fn redo_infers_the_selection_after_a_concurrent_insert() {
    let state = EditorState::with_extensions("a", collab(CollabConfig::new("a", 0))).unwrap();
    let mut history = History::new(state);
    let cursor = |pos| EditorSelection::single(pos, pos);
    history.set_selection(cursor(1)).unwrap();
    history
        .update(
            Transaction::new(change(1..1, "X"))
                .with_selection(cursor(2))
                .with_user_event("input.type"),
            "before",
            |_| (None, "after"),
        )
        .unwrap();
    assert!(
        history.to_json().unwrap()["done"][0]
            .get("oppositeSelection")
            .is_none()
    );

    let remote = Update::new("b", change(1..1, "B"), 1).unwrap();
    let receipt = receive_updates(history.state(), 0, &[remote]).unwrap();
    history.update(receipt, "", |_| (None, "")).unwrap();
    let pending = sendable_updates(history.state()).unwrap();
    assert_eq!(pending[0].changes(), &change(2..2, "X"));
    assert_eq!(history.state().text().as_str(), "aBX");
    assert_eq!(history.state().selection(), &cursor(3));

    assert_eq!(history.undo(), Some("before"));
    assert_eq!(history.state().text().as_str(), "aB");
    assert_eq!(history.state().selection(), &cursor(1));
    assert_eq!((history.undo_depth(), history.redo_depth()), (0, 1));
    assert_eq!(history.redo(), Some("after"));
    assert_eq!(history.state().text().as_str(), "aBX");
    assert_eq!(history.state().selection(), &cursor(1));
    assert_eq!((history.undo_depth(), history.redo_depth()), (1, 0));
    let pending = sendable_updates(history.state()).unwrap();
    assert_eq!(pending.len(), 3);
    assert_eq!(pending[2].changes(), &change(2..2, "X"));
    assert_eq!(pending[2].origin().unwrap().user_event(), Some("redo"));
}

#[test]
fn collaboration_keeps_the_explicit_selection_of_a_restored_event() {
    let native = json!({
        "doc": "aX",
        "selection": {"ranges": [{"anchor": 2, "head": 2}], "main": 0},
        "history": {"done": [{
            "changes": [1, [1]],
            "startSelection": {"ranges": [{"anchor": 1, "head": 1}], "main": 0},
            "oppositeSelection": {"ranges": [{"anchor": 2, "head": 2}], "main": 0},
            "selectionsAfter": [],
        }], "undone": []},
    });
    let mut history =
        History::from_state_json(&native, collab(CollabConfig::new("a", 0)), "restored").unwrap();
    let remote = Update::new("b", change(1..1, "B"), 2).unwrap();
    let receipt = receive_updates(history.state(), 0, &[remote]).unwrap();
    history.update(receipt, "", |_| (None, "")).unwrap();
    assert_eq!(history.undo(), Some("restored"));
    assert_eq!(history.state().text().as_str(), "aB");
    assert_eq!(history.state().selection(), &EditorSelection::single(1, 1));
    assert_eq!(history.redo(), Some("restored"));
    assert_eq!(history.state().text().as_str(), "aBX");
    assert_eq!(history.state().selection(), &EditorSelection::single(3, 3));
}

#[test]
fn an_open_group_keeps_each_replay_mode_across_reconfiguration() {
    let slot = Compartment::new();
    let state = EditorState::with_extensions("a", slot.of(Extension::default())).unwrap();
    let mut history = History::new(state);
    let cursor = |pos| EditorSelection::single(pos, pos);
    history.set_selection(cursor(1)).unwrap();
    history.start_group();
    history
        .update(
            Transaction::new(change(1..1, "X"))
                .with_selection(cursor(0))
                .with_effects([slot.reconfigure(collab(CollabConfig::new("a", 0)))]),
            "initial",
            |_| (None, "enabled"),
        )
        .unwrap();
    history
        .update(
            Transaction::default().with_effects([slot.reconfigure(Extension::default())]),
            "enabled",
            |_| (None, "disabled"),
        )
        .unwrap();
    history
        .update(
            Transaction::new(change(2..2, "Y")).with_selection(cursor(0)),
            "disabled",
            |_| (None, "native"),
        )
        .unwrap();
    history.end_group();
    assert_eq!(history.undo_depth(), 2);
    assert_eq!(history.undo(), Some("disabled"));
    assert_eq!(history.undo(), Some("initial"));
    assert_eq!(history.redo(), Some("disabled"));
    assert_eq!(history.state().text().as_str(), "aX");
    assert_eq!(history.state().selection(), &cursor(2));
    assert_eq!(history.redo(), Some("native"));
    assert_eq!(history.state().text().as_str(), "aXY");
    assert_eq!(history.state().selection(), &cursor(0));
}

#[test]
fn collaborative_effect_events_use_inferred_replay_and_still_reverse_effects() {
    let toggle = StateEffectType::new();
    let enabled = StateField::new(|_| Ok(false), {
        let toggle = toggle.clone();
        move |value, update| {
            Ok(value
                ^ update
                    .transaction()
                    .effects()
                    .iter()
                    .any(|e| e.get(&toggle).is_some()))
        }
    });
    let invert = inverted_effects({
        let toggle = toggle.clone();
        move |_, tr, _| {
            tr.effects()
                .iter()
                .filter(|e| e.get(&toggle).is_some())
                .cloned()
                .collect()
        }
    });
    let promote_selection = transaction_extender({
        let toggle = toggle.clone();
        move |_, tr| TransactionExtension {
            effects: tr
                .is_user_event("select.undo")
                .then(|| toggle.of(()))
                .into_iter()
                .collect(),
            ..TransactionExtension::default()
        }
    });
    let extensions = Extension::group([
        collab(CollabConfig::new("a", 0)),
        enabled.extension(),
        invert,
        promote_selection,
    ]);
    let mut history = History::new(EditorState::with_extensions("a", extensions).unwrap());
    let is_enabled =
        |history: &History<()>| *history.state().context().field(&enabled).unwrap().unwrap();
    let cursor = |pos| EditorSelection::single(pos, pos);
    history.set_selection(cursor(1)).unwrap();
    let update = Transaction::default()
        .with_selection(cursor(0))
        .with_effects([toggle.of(())]);
    history.update(update, (), |_| (None, ())).unwrap();
    assert!(is_enabled(&history));
    assert!(
        history.to_json().unwrap()["done"][0]
            .get("oppositeSelection")
            .is_none()
    );
    assert_eq!(history.undo(), Some(()));
    assert!(!is_enabled(&history));
    assert_eq!(history.redo(), Some(()));
    assert!(is_enabled(&history));
    assert_eq!(history.state().selection(), &cursor(1));

    let selection = Transaction::default().with_selection(cursor(0));
    history.update(selection, (), |_| (None, ())).unwrap();
    assert!(history.can_undo_selection());
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.undo_selection(()), Some(()));
    assert!(!is_enabled(&history));
    assert_eq!(history.redo_depth(), 1);
    assert!(
        history.to_json().unwrap()["undone"][0]
            .get("oppositeSelection")
            .is_none()
    );
    assert_eq!(history.redo(), Some(()));
    assert!(is_enabled(&history));
}

#[test]
fn filters_extenders_and_history_match_the_pinned_transaction_trace() {
    let oracle: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/collab_transaction_oracle.json")).unwrap();
    let hooks = Arc::new([(); 4].map(|()| AtomicUsize::new(0)));
    let effect: StateEffectType<&'static str> = StateEffectType::new();
    let config = CollabConfig::new("a", 7).with_shared_effects({
        let (hooks, effect) = (hooks.clone(), effect.clone());
        move |tr| {
            hooks[3].fetch_add(1, Relaxed);
            tr.effects()
                .iter()
                .filter(|item| item.get(&effect).is_some())
                .cloned()
                .collect()
        }
    });
    let state = EditorState::with_extensions(
        "abcd",
        Extension::group([
            collab(config),
            change_filter({
                let hooks = hooks.clone();
                move |_, tr| {
                    hooks[0].fetch_add(1, Relaxed);
                    if tr.is_user_event("input") {
                        ChangePermission::Protect(std::iter::once(1..3).collect())
                    } else {
                        ChangePermission::Allow
                    }
                }
            }),
            transaction_filter({
                let hooks = hooks.clone();
                move |_, tr| {
                    hooks[1].fetch_add(1, Relaxed);
                    Ok(tr)
                }
            }),
            transaction_extender({
                let (hooks, effect) = (hooks.clone(), effect.clone());
                move |_, _| {
                    hooks[2].fetch_add(1, Relaxed);
                    TransactionExtension {
                        effects: vec![effect.of("extended")],
                        ..TransactionExtension::default()
                    }
                }
            }),
        ]),
    )
    .unwrap();
    let mut history = History::new(state);
    for mut expected in oracle {
        let stage = expected["name"].as_str().unwrap().to_owned();
        let state = history.state();
        let version = get_synced_version(state).unwrap();
        let transaction = match stage.as_str() {
            "local" => Some(Transaction::new(change(0..4, "!")).with_user_event("input.type")),
            "remote" => {
                let remote = Update::new("b", change(1..2, "R"), 4).unwrap();
                Some(receive_updates(state, version, &[remote]).unwrap())
            }
            "undo" => {
                assert_eq!(history.undo(), Some(()));
                None
            }
            "redo" => {
                assert_eq!(history.redo(), Some(()));
                None
            }
            "ack" | "empty" => {
                assert_eq!(expected["hooks"], json!([1, 1, 1, 0]));
                assert_eq!(expected["history"], true);
                // Upstream filters can rewrite acknowledgement-only and empty
                // receipts. Rust bypasses filters so they cannot add unsent edits.
                expected["hooks"] = json!([0, 0, 1, 0]);
                expected["history"] = json!(false);
                let updates = if stage == "ack" {
                    sendable_updates(state).unwrap()
                } else {
                    Vec::new()
                };
                Some(receive_updates(state, version, &updates).unwrap())
            }
            _ => panic!("unknown stage: {stage}"),
        };
        if let Some(transaction) = transaction {
            assert_eq!(
                json!(is_remote(&transaction)),
                expected["remote"],
                "{stage}"
            );
            assert_eq!(
                json!(transaction.add_to_history()),
                expected["history"],
                "{stage}"
            );
            history.update(transaction, (), |_| (None, ())).unwrap();
        }
        let state = history.state();
        let pending: Vec<_> = sendable_updates(state)
            .unwrap()
            .iter()
            .map(|update| {
                let effects: Vec<_> = update
                    .effects()
                    .iter()
                    .filter_map(|e| e.get(&effect))
                    .collect();
                json!([
                    update
                        .changes()
                        .to_json(update.change_desc().length())
                        .unwrap(),
                    update.origin().unwrap().user_event(),
                    effects,
                ])
            })
            .collect();
        assert_eq!(json!(state.text().as_str()), expected["doc"], "{stage}");
        assert_eq!(
            json!(get_synced_version(state).unwrap()),
            expected["version"],
            "{stage}"
        );
        assert_eq!(
            json!([history.undo_depth(), history.redo_depth()]),
            expected["depth"],
            "{stage}"
        );
        assert_eq!(json!(pending), expected["pending"], "{stage}");
        let calls = std::array::from_fn::<_, 4, _>(|i| hooks[i].swap(0, Relaxed));
        assert_eq!(json!(calls), expected["hooks"], "{stage}");
    }
}

// The pinned upstream invokes positional effect mappers on identity changes.
#[test]
fn an_empty_authority_update_still_maps_shared_effects() {
    let mark = StateEffectType::with_mapping(|pos, through| (!through.is_empty()).then_some(*pos));
    let mut state = EditorState::with_extensions(
        "x",
        collab(CollabConfig::new("a", 0).with_shared_effects(|tr| tr.effects().to_vec())),
    )
    .unwrap();
    let remote_mark = Update::new("b", ChangeSet::default(), 1)
        .unwrap()
        .with_effects([mark.of(0)]);
    let empty = Update::new("c", ChangeSet::default(), 1).unwrap();
    let receipt = receive_updates(&state, 0, std::slice::from_ref(&remote_mark)).unwrap();
    assert_eq!(receipt.effects()[0].get(&mark), Some(&0));
    let both = receive_updates(&state, 0, &[remote_mark, empty.clone()]).unwrap();
    assert!(both.effects().is_empty());

    state
        .update(Transaction::default().with_effects([mark.of(0)]))
        .unwrap();
    let local = collab_state(&state).unwrap().unwrap();
    let rebased = rebase_updates(local.sendable_updates(), std::slice::from_ref(&empty)).unwrap();
    assert_eq!(rebased.len(), 1);
    assert!(rebased[0].effects().is_empty());
    let described = UpdateDescription {
        client_id: empty.client_id(),
        changes: empty.change_desc(),
    };
    assert_eq!(
        rebase_updates_described(local.sendable_updates(), &[described]).unwrap(),
        rebased
    );

    let receipt = receive_updates(&state, 0, &[empty]).unwrap();
    state.update(receipt).unwrap();
    let current = collab_state(&state).unwrap().unwrap();
    assert_eq!(current.synced_version(), 1);
    assert_eq!(current.sendable_updates(), rebased);
    assert_eq!(
        current.sendable_updates()[0].origin().unwrap().effects()[0].get(&mark),
        Some(&0)
    );
}

#[test]
fn incompatible_echo_keeps_native_revision_selection_and_history() {
    let mut view = EditorView::new(
        EditorState::with_extensions("é中🦊z", collab(CollabConfig::new("a", 9))).unwrap(),
    );
    view.update(
        Transaction::new(change(2..2, "🤖"))
            .with_selection(EditorSelection::single(6, 6))
            .with_user_event("input.type"),
        (),
        |_| (None, ()),
    )
    .unwrap();
    let stale = collab_state(view.state()).unwrap().unwrap();
    let remote = [
        Update::new("b", change(2..2, "β"), 10).unwrap(),
        Update::new("c", change(2..7, "語🙂"), 12).unwrap(),
    ];
    let batch_echo = rebase_updates(stale.sendable_updates(), &remote).unwrap();
    assert_eq!(batch_echo[0].changes(), &change(2..2, "🤖"));
    for (version, update) in (9..).zip(&remote) {
        let receipt = receive_updates(view.state(), version, std::slice::from_ref(update)).unwrap();
        assert!(is_remote(&receipt));
        assert!(!receipt.add_to_history());
        view.update(receipt, (), |_| (None, ())).unwrap();
    }
    let current = collab_state(view.state()).unwrap().unwrap();
    let selection = view.state().selection().clone();
    assert_eq!(view.revision(), 3);
    assert_eq!(view.state().text().as_str(), "é語🙂🤖🦊z");
    assert_eq!(current.synced_version(), 11);
    assert_eq!(current.sendable_updates()[0].changes(), &change(9..9, "🤖"));
    assert_eq!(
        receive_updates(view.state(), 11, &batch_echo).unwrap_err(),
        CollabError::MismatchedAcknowledgement,
    );
    assert_eq!(view.revision(), 3);
    assert_eq!(view.state().selection(), &selection);
    assert!(view.can_undo());
    assert!(!view.can_redo());
    assert!(std::sync::Arc::ptr_eq(
        &current,
        &collab_state(view.state()).unwrap().unwrap()
    ));

    // The host instead accepts the exact-version submission. Acknowledging it
    // does not edit the document or add a history entry.
    let receipt = receive_updates(view.state(), 11, current.sendable_updates()).unwrap();
    assert!(!is_remote(&receipt));
    view.update(receipt, (), |_| (None, ())).unwrap();
    assert_eq!(view.revision(), 3);
    assert_eq!(view.state().selection(), &selection);
    assert_eq!(
        collab_state(view.state())
            .unwrap()
            .unwrap()
            .synced_version(),
        12
    );
    assert_eq!(view.undo(), Some(()));
    assert_eq!(view.revision(), 4);
    assert_eq!(view.state().text().as_str(), "é語🙂🦊z");
    assert!(!view.can_undo());
    let after_undo = collab_state(view.state()).unwrap().unwrap();
    assert_eq!(after_undo.sendable_updates().len(), 1);
    assert_eq!(
        after_undo.sendable_updates()[0]
            .origin()
            .unwrap()
            .user_event(),
        Some("undo")
    );
    assert_eq!(view.redo(), Some(()));
    assert_eq!(view.revision(), 5);
    assert_eq!(view.state().text().as_str(), "é語🙂🤖🦊z");
}

#[test]
fn wire_batches_validate_atomically_and_preserve_revision_and_history() {
    let mut view = EditorView::new(
        EditorState::with_extensions("éab", collab(CollabConfig::new("a", 7))).unwrap(),
    );
    view.update(
        Transaction::new(change(4..4, "!\r\n")).with_selection(EditorSelection::single(7, 7)),
        (),
        |_| (None, ()),
    )
    .unwrap();
    let before = collab_state(view.state()).unwrap().unwrap();
    let source = Text::new("éab");
    let update = |id, wire, source: &Text| {
        Update::new(
            id,
            ChangeSet::from_json(&wire, source).unwrap(),
            source.len(),
        )
        .unwrap()
    };
    let replacement = update("b", json!([2, [1, "X"], 1]), &source);
    let invalid_boundary = update("c", json!([1, 3]), &Text::new("abcd"));
    let split = update("b", json!([2, 2]), &source);
    assert_eq!(
        Update::new("b", split.changes().clone(), 7),
        Err(EditError::MismatchedLength)
    );
    let wrong_echo = update("a", json!([2, 2, [0, "?\r", ""]]), &source);
    for (batch, error) in [
        (
            [replacement, invalid_boundary],
            CollabError::Edit(EditError::InvalidRange),
        ),
        (
            [split.clone(), wrong_echo],
            CollabError::MismatchedAcknowledgement,
        ),
    ] {
        assert_eq!(receive_updates(view.state(), 7, &batch).unwrap_err(), error);
        assert_eq!(view.state().text().as_str(), "éab!\r\n");
        assert_eq!(view.state().selection(), &EditorSelection::single(7, 7));
        assert_eq!(view.revision(), 1);
        assert!(view.can_undo());
        assert!(!view.can_redo());
        assert!(Arc::ptr_eq(
            &before,
            &collab_state(view.state()).unwrap().unwrap()
        ));
        if error == CollabError::MismatchedAcknowledgement {
            assert_eq!(
                rebase_updates(before.sendable_updates(), &batch),
                Err(error)
            );
        }
    }

    let echo = update("a", json!([2, 2, [0, "!"], [0, "\r", ""]]), &source);
    let identity = update("c", json!([2, 5]), view.state().text());
    let batch = [split, echo, identity];
    assert!(
        rebase_updates(before.sendable_updates(), &batch)
            .unwrap()
            .is_empty()
    );
    let receipt = receive_updates(view.state(), 7, &batch).unwrap();
    assert_eq!(receipt.changes().to_json(7).unwrap(), json!([2, 5]));
    assert!(is_remote(&receipt));
    assert!(!receipt.add_to_history());
    view.update(receipt, (), |_| (None, ())).unwrap();
    assert_eq!(get_synced_version(view.state()), Ok(10));
    assert!(sendable_updates(view.state()).unwrap().is_empty());
    assert_eq!(view.revision(), 2);
    assert_eq!(view.state().text().as_str(), "éab!\r\n");
    assert_eq!(view.state().selection(), &EditorSelection::single(7, 7));
    assert_eq!(view.undo(), Some(()));
    assert_eq!(view.state().text().as_str(), "éab");
    assert!(!view.can_undo());
    assert_eq!(view.redo(), Some(()));
    assert_eq!(view.state().text().as_str(), "éab!\r\n");
    assert_eq!(view.revision(), 4);
    assert_eq!(get_synced_version(view.state()), Ok(10));
    assert_eq!(sendable_updates(view.state()).unwrap().len(), 2);
}

#[test]
fn only_an_unchanged_update_queue_can_apply_a_prepared_receipt() {
    let mut state =
        EditorState::with_extensions("abcd", collab(CollabConfig::new("a", 0))).unwrap();
    let remote = Update::new("b", change(1..2, "Q"), 4).unwrap();
    let receipt = receive_updates(&state, 0, std::slice::from_ref(&remote)).unwrap();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(2, 2)))
        .unwrap();
    state.update(receipt).unwrap();
    assert_eq!(state.text().as_str(), "aQcd");
    let remote = Update::new("b", change(1..2, "R"), 4).unwrap();
    let stale = receive_updates(&state, 1, &[remote]).unwrap();
    state.update(Transaction::new(change(0..1, "X"))).unwrap();
    assert_eq!(
        state.update(stale),
        Err(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        ))
    );
    assert_eq!(state.text().as_str(), "XQcd");
    assert_eq!(
        collab_state(&state)
            .unwrap()
            .unwrap()
            .sendable_updates()
            .len(),
        1
    );
}

#[test]
fn a_receipt_requires_collaboration_for_the_whole_transaction() {
    let slot = Compartment::new();
    let mut state =
        EditorState::with_extensions("abcd", slot.of(collab(CollabConfig::new("a", 2)))).unwrap();
    let update = Update::new("b", change(1..2, "!"), 4).unwrap();
    let receipt = receive_updates(&state, 2, &[update]).unwrap();
    let remove = || slot.reconfigure(Extension::default());
    let rejected = Err(EditError::Configuration(
        ConfigurationError::InvalidFieldData,
    ));

    assert_eq!(
        state.update(receipt.clone().with_effects([remove()])),
        rejected
    );
    assert_eq!(state.text().as_str(), "abcd");
    assert_eq!(get_synced_version(&state), Ok(2));
    state
        .update(Transaction::default().with_effects([remove()]))
        .unwrap();
    assert_eq!(state.update(receipt), rejected);
    assert_eq!(state.text().as_str(), "abcd");
    assert!(collab_state(&state).unwrap().is_none());
}

#[test]
fn a_transaction_accepts_at_most_one_authority_receipt() {
    let mut state =
        EditorState::with_extensions("abcd", collab(CollabConfig::new("a", 2))).unwrap();
    let receipt = receive_updates(&state, 2, &[]).unwrap();
    let duplicate = receipt
        .clone()
        .merge(receipt.clone(), 4, TransactionCoordinates::Original)
        .unwrap();
    assert_eq!(
        state.update(duplicate),
        Err(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        ))
    );
    assert_eq!(state.text().as_str(), "abcd");
    assert_eq!(get_synced_version(&state), Ok(2));
    state.update(receipt).unwrap();
}

#[test]
fn rejected_shared_effect_does_not_commit_a_prepared_receipt() {
    let rejected = StateEffectType::new();
    let validator = StateField::new(|_| Ok(()), {
        let rejected = rejected.clone();
        move |(), update| {
            if update
                .transaction()
                .effects()
                .iter()
                .any(|effect| effect.get(&rejected).is_some())
            {
                Err(ConfigurationError::InvalidFieldData)
            } else {
                Ok(())
            }
        }
    });
    let mut state = EditorState::with_extensions(
        "éab",
        Extension::group([collab(CollabConfig::new("a", 2)), validator.extension()]),
    )
    .unwrap();
    state
        .update(Transaction::new(change(4..4, "!")).with_selection(EditorSelection::single(5, 5)))
        .unwrap();
    let before = collab_state(&state).unwrap().unwrap();
    let remote = Update::new("b", change(0..2, "猫"), 4).unwrap();
    let malformed = remote.clone().with_effects([rejected.of(())]);
    let receipt = receive_updates(&state, 2, &[malformed]).unwrap();
    assert_eq!(
        state.update(receipt),
        Err(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        ))
    );
    assert_eq!(state.text().as_str(), "éab!");
    assert_eq!(state.selection(), &EditorSelection::single(5, 5));
    assert!(std::sync::Arc::ptr_eq(
        &before,
        &collab_state(&state).unwrap().unwrap()
    ));

    let receipt = receive_updates(&state, 2, &[remote]).unwrap();
    state.update(receipt).unwrap();
    assert_eq!(state.text().as_str(), "猫ab!");
    assert_eq!(state.selection(), &EditorSelection::single(6, 6));
    assert_eq!(get_synced_version(&state), Ok(3));
    assert_eq!(
        sendable_updates(&state).unwrap()[0].changes(),
        &change(5..5, "!")
    );
}

#[test]
fn an_extension_has_a_stable_generated_id_and_an_unconfigured_editor_has_no_state() {
    let config = collab(CollabConfig::default());
    let editor = EditorState::with_extensions("abc", config.clone()).unwrap();
    let copy = EditorState::with_extensions("", config).unwrap();
    let a = collab_state(&editor).unwrap().unwrap();
    let b = collab_state(&copy).unwrap().unwrap();
    assert!(!a.client_id().is_empty());
    assert_eq!(a.client_id(), b.client_id());
    assert_eq!(a.synced_version(), 0);
    assert_eq!(get_client_id(&editor).unwrap(), a.client_id());
    assert_eq!(get_synced_version(&editor), Ok(0));
    assert_eq!(sendable_updates(&editor), Ok(Vec::new()));
    let plain = EditorState::new("abc");
    assert!(collab_state(&plain).unwrap().is_none());
    assert_eq!(get_client_id(&plain).unwrap(), "");
    assert_eq!(get_synced_version(&plain), Err(CollabError::NotConfigured));
    assert_eq!(sendable_updates(&plain), Err(CollabError::NotConfigured));
    assert_eq!(
        receive_updates(&plain, 0, &[]).unwrap_err(),
        CollabError::NotConfigured
    );
}

// The pinned extension combines supplied fields, rather than picking one config.
#[test]
fn combines_config_inputs_and_rejects_conflicts_atomically() {
    let mark = StateEffectType::new();
    let mut settings = CollabConfig::default();
    settings.start_version = 4;
    let mut policy = CollabConfig::default().with_shared_effects(|tr| tr.effects().to_vec());
    policy.client_id = Some("a".into());
    let mut state = EditorState::with_extensions(
        "abc",
        Extension::group([collab(settings), collab(policy.clone()), collab(policy)]),
    )
    .unwrap();
    assert_eq!(get_client_id(&state).unwrap(), "a");
    assert_eq!(get_synced_version(&state), Ok(4));
    state
        .update(Transaction::default().with_effects([mark.of(1)]))
        .unwrap();
    let queued = sendable_updates(&state).unwrap();
    assert!(queued[0].changes().is_empty());
    assert_eq!(queued[0].effects()[0].get(&mark), Some(&1));

    for config in [
        CollabConfig::new("b", 4),
        CollabConfig::new("a", 0),
        CollabConfig::default().with_start_version(0),
        CollabConfig::default().with_shared_effects(|_| vec![]),
    ] {
        assert_eq!(
            state.update(
                Transaction::new(change(1..2, "X")).with_effects([collab(config).append()]),
            ),
            Err(EditError::Configuration(
                ConfigurationError::InvalidFieldData
            ))
        );
        assert_eq!(state.text().as_str(), "abc");
        assert_eq!(get_client_id(&state).unwrap(), "a");
        assert_eq!(get_synced_version(&state), Ok(4));
        assert_eq!(sendable_updates(&state).unwrap(), queued);
    }
}

// Pinned upstream uses the old sharing policy on a reconfigure and retains
// the queue and version. A later receive labels mapped entries with the new ID.
#[test]
fn reconfiguration_preserves_version_and_uses_the_start_policy() {
    let old_mark = StateEffectType::new();
    let new_mark = StateEffectType::new();
    let policy = |id, version, mark: StateEffectType<&'static str>| {
        collab(
            CollabConfig::new(id, version).with_shared_effects(move |tr| {
                tr.effects()
                    .iter()
                    .filter(|item| item.get(&mark).is_some())
                    .cloned()
                    .collect()
            }),
        )
    };
    let compartment = Compartment::new();
    let mut state =
        EditorState::with_extensions("abc", compartment.of(policy("old", 4, old_mark.clone())))
            .unwrap();
    state
        .update(Transaction::new(change(1..1, "!")).with_effects([old_mark.of("x1")]))
        .unwrap();
    state
        .update(Transaction::default().with_effects([
            compartment.reconfigure(policy("new", 80, new_mark.clone())),
            old_mark.of("x2"),
            new_mark.of("y2"),
        ]))
        .unwrap();
    state
        .update(Transaction::default().with_effects([old_mark.of("x3"), new_mark.of("y3")]))
        .unwrap();
    let queued = sendable_updates(&state).unwrap();
    assert_eq!(get_client_id(&state).unwrap(), "new");
    assert_eq!(get_synced_version(&state), Ok(4));
    assert_eq!(
        queued.iter().map(Update::client_id).collect::<Vec<_>>(),
        ["old", "old", "new"]
    );
    assert_eq!(queued[0].effects()[0].get(&old_mark), Some(&"x1"));
    assert_eq!(queued[1].effects()[0].get(&old_mark), Some(&"x2"));
    assert_eq!(queued[2].effects()[0].get(&new_mark), Some(&"y3"));

    let remote = Update::new("remote", change(0..0, "?"), 3).unwrap();
    state
        .update(receive_updates(&state, 4, &[remote]).unwrap())
        .unwrap();
    let mapped = sendable_updates(&state).unwrap();
    assert_eq!(state.text().as_str(), "?a!bc");
    assert_eq!(get_synced_version(&state), Ok(5));
    assert_eq!(
        mapped.iter().map(Update::client_id).collect::<Vec<_>>(),
        ["new"; 3]
    );
    assert_eq!(mapped[0].changes(), &change(2..2, "!"));
    for (current, original) in mapped.iter().zip(queued) {
        assert!(std::ptr::eq(
            current.origin().unwrap(),
            original.origin().unwrap()
        ));
        assert_eq!(current.effects().len(), 1);
    }
}
