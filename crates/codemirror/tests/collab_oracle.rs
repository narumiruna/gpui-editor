//! Recorded with @codemirror/collab 6.1.1 and @codemirror/state 6.7.6, from
//! the SHA-512-verified tarballs in docs/codemirror-upstream.json. The scratch
//! oracle converts byte offsets to UTF-16 for inputs and back to bytes for
//! outputs. It runs authority rebase with both edits and descriptions.

use std::{
    ptr,
    sync::{Arc, Mutex},
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeDesc, ChangePermission, ChangeSet, EditorSelection, EditorState,
    Extension, History, MapMode, StateEffect, StateEffectType, Text, Transaction, change_filter,
    collab::{
        Collab, CollabConfig, CollabError, TransactionContext, Update, UpdateDescription, collab,
        collab_state, get_client_id, get_synced_version, is_remote, rebase_updates,
        rebase_updates_described, receive_updates, sendable_updates,
    },
};
use serde_json::{Value, json};

type Mark = StateEffectType<(String, usize)>;

#[test]
fn matches_pinned_receipts_and_rebase_except_for_documented_differences() {
    for line in include_str!("fixtures/collab_oracle.jsonl").lines() {
        check_case(&serde_json::from_str(line).unwrap());
    }
}

#[test]
fn collaboration_keeps_same_text_replacement_geometry_for_local_and_remote_edits() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/collab_replacement_oracle.json")).unwrap();
    let source = string(&fixture["source"]);
    let start = byte(&fixture["start"]["cursor"]);
    let selection = EditorSelection::single(start, start);
    let changes = ChangeSet::single(Change::new(0..source.len(), source)).unwrap();
    let foreign = Update::new("b", changes.clone(), source.len()).unwrap();
    let set_cursor = || Transaction::default().with_selection(selection.clone());
    let local_edit = || Transaction::new(changes.clone());
    let check = |state: &EditorState, pending: usize, version, stage: &str| {
        assert_eq!(
            json!({"text": state.text().as_str(), "cursor": state.selection().main().head(), "pending": pending, "version": version}),
            fixture[stage]
        );
    };
    let snapshot = |state: &EditorState, stage| {
        check(
            state,
            sendable_updates(state).unwrap().len(),
            get_synced_version(state).unwrap(),
            stage,
        )
    };
    let mut state =
        EditorState::with_extensions(source, collab(CollabConfig::new("a", 0))).unwrap();
    state.update(set_cursor()).unwrap();
    let mut remote = state.clone();
    state.update(local_edit()).unwrap();
    snapshot(&state, "local");
    let echoed = sendable_updates(&state).unwrap();
    assert_eq!(echoed[0].changes(), &changes);
    state
        .update(receive_updates(&state, 0, &echoed).unwrap())
        .unwrap();
    snapshot(&state, "echo");
    remote
        .update(receive_updates(&remote, 0, std::slice::from_ref(&foreign)).unwrap())
        .unwrap();
    snapshot(&remote, "remote");

    let mut standalone = EditorState::new(source);
    standalone.update(set_cursor()).unwrap();
    standalone.update(local_edit()).unwrap();
    assert_eq!(standalone.selection(), &selection);
    let mut direct_remote = standalone.clone();
    let mut session = Collab::new("a", 0, &standalone);
    let mut direct_client = session.clone();
    let standalone_snapshot = |state: &EditorState, session: &Collab, stage| {
        check(
            state,
            session.sendable_updates().len(),
            session.synced_version(),
            stage,
        )
    };
    session.apply_local(&mut standalone, local_edit()).unwrap();
    standalone_snapshot(&standalone, &session, "local");
    let echoed = session.sendable_updates().to_vec();
    session
        .receive_updates(&mut standalone, 0, &echoed)
        .unwrap();
    standalone_snapshot(&standalone, &session, "echo");
    direct_client
        .receive_updates(&mut direct_remote, 0, &[foreign])
        .unwrap();
    standalone_snapshot(&direct_remote, &direct_client, "remote");
}

#[test]
fn same_text_replacement_is_queued_when_collaboration_is_installed() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/collab_replacement_oracle.json")).unwrap();
    let case = &fixture["installation"];
    let source = string(&case["start"]["text"]);
    let cursor = byte(&case["start"]["cursor"]);
    let config = CollabConfig::new(
        string(&case["config"]["clientID"]),
        byte(&case["config"]["startVersion"]),
    );
    let mut state = EditorState::new(source);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(cursor, cursor)))
        .unwrap();
    let mut history = History::new(state.clone());
    let changes = ChangeSet::single(Change::new(0..source.len(), source)).unwrap();
    let transaction = Transaction::new(changes).with_effects([collab(config).append()]);
    state.update(transaction.clone()).unwrap();
    history.update(transaction, (), |_| (None, ())).unwrap();
    let snapshot = |state: &EditorState| {
        let queue = sendable_updates(state).unwrap();
        json!({
            "text": state.text().as_str(), "cursor": state.selection().main().head(),
            "pending": queue.len(), "version": get_synced_version(state).unwrap(),
            "client": get_client_id(state).unwrap(),
            "queue": queue.iter().map(|update| json!({
                "client": update.client_id(), "changes": update.changes().to_json(source.len()).unwrap(),
            })).collect::<Vec<_>>(),
        })
    };
    let expected = &case["installed"];
    assert_eq!(
        [snapshot(&state), snapshot(history.state())],
        [expected.clone(), expected.clone()],
    );
    assert_eq!(history.undo_depth(), 1);
}

#[test]
fn same_text_replacement_uses_collaboration_in_the_start_or_final_configuration() {
    let set_cursor = || Transaction::default().with_selection(EditorSelection::single(1, 1));
    let replace = || Transaction::new(ChangeSet::single(Change::new(0..2, "ab")).unwrap());
    let enabled = collab(CollabConfig::new("joined", 7));
    let disable = || Extension::default().reconfigure();

    let mut state = EditorState::new("ab");
    state.update(set_cursor()).unwrap();
    state
        .update(replace().with_effects([enabled.clone().append(), disable()]))
        .unwrap();
    assert_eq!(state.selection().main().head(), 1);
    assert!(collab_state(&state).unwrap().is_none());

    let mut state = EditorState::with_extensions("ab", enabled).unwrap();
    state.update(set_cursor()).unwrap();
    state.update(replace().with_effects([disable()])).unwrap();
    assert_eq!(state.selection().main().head(), 0);
    assert!(collab_state(&state).unwrap().is_none());
}

#[test]
fn contextual_effects_and_origins_match_pinned_local_edits_and_receipts() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/collab_origin_oracle.json")).unwrap();
    for integrated in [false, true] {
        check_contextual_origin(&fixture, integrated);
    }
}

fn check_contextual_origin(fixture: &Value, integrated: bool) {
    let mark = Mark::with_mapping(|(label, pos), changes| {
        changes
            .map_pos(*pos, Assoc::After, MapMode::Simple)
            .map(|pos| (label.clone(), pos))
    });
    let seen = Arc::new(Mutex::new(Vec::new()));
    let share = {
        let (seen, mark) = (seen.clone(), mark.clone());
        move |context: &TransactionContext| {
            let transaction = context.transaction();
            seen.lock().unwrap().push(ptr::from_ref(transaction).addr());
            if transaction.changes().is_empty() {
                return Vec::new();
            }
            let label = format!(
                "{}:{}>{}:{}",
                context.start_text().as_str(),
                context.start_selection().main().head(),
                context.new_text().as_str(),
                context.new_selection().main().head(),
            );
            vec![mark.of((label, context.new_selection().main().head()))]
        }
    };
    let protect = byte(&fixture["protect"][0])..byte(&fixture["protect"][1]);
    let filter = change_filter(move |_, tr| {
        if tr.is_user_event("delete") {
            ChangePermission::Protect(std::iter::once(protect.clone()).collect())
        } else {
            ChangePermission::Allow
        }
    });
    let start = &fixture["start"];
    let version = byte(&start["version"]);
    let cursor = byte(&start["cursor"]);
    let mut direct = None;
    let create = |extension| {
        EditorState::create(
            string(&start["doc"]),
            Some(EditorSelection::single(cursor, cursor)),
            extension,
        )
        .unwrap()
    };
    let mut state = if integrated {
        create(Extension::group([
            filter,
            collab(CollabConfig::new("a", version).with_shared_effects_context(share)),
        ]))
    } else {
        let state = create(filter);
        direct = Some(Collab::new("a", version, &state).with_shared_effects_context(share));
        state
    };
    let current = |state: &EditorState, direct: Option<&Collab>| match direct {
        Some(session) => (
            session.synced_version(),
            session.sendable_updates().to_vec(),
        ),
        None => (
            get_synced_version(state).unwrap(),
            sendable_updates(state).unwrap(),
        ),
    };
    let snapshot = |state: &EditorState, direct: Option<&Collab>| {
        let (version, pending) = current(state, direct);
        json!({
            "doc": state.text().as_str(), "cursor": state.selection().main().head(),
            "version": version, "calls": seen.lock().unwrap().len(),
            "pending": contextual_queue(&pending, &mark, true),
        })
    };
    assert_eq!(snapshot(&state, direct.as_ref()), *start);
    for (request, expected) in items(&fixture["requests"]).zip(items(&fixture["local"])) {
        let changes = ChangeSet::from_json(&request["changes"], state.text()).unwrap();
        let tr = Transaction::new(changes).with_user_event(string(&request["userEvent"]));
        if let Some(session) = &mut direct {
            session.apply_local(&mut state, tr).unwrap();
        } else {
            state.update(tr).unwrap();
        }
        assert_eq!(snapshot(&state, direct.as_ref()), *expected);
    }
    let (_, originals) = current(&state, direct.as_ref());
    let seen_transactions = seen.lock().unwrap().clone();
    assert_eq!(originals.len(), seen_transactions.len());
    for (update, address) in originals.iter().zip(seen_transactions) {
        assert_eq!(ptr::from_ref(update.origin().unwrap()).addr(), address);
        assert!(ptr::eq(
            update.origin().unwrap(),
            update.origin_context().unwrap().transaction()
        ));
    }
    let peer = &fixture["peer"];
    let source = Text::new(string(&start["doc"]));
    let peer = Update::new(
        string(&peer["client"]),
        ChangeSet::from_json(&peer["changes"], &source).unwrap(),
        source.len(),
    )
    .unwrap();
    assert!(peer.origin_context().is_none());
    let peer = [peer];
    for rebased in [
        rebase_updates(&originals, &peer).unwrap(),
        rebase_updates_described(&originals, &peer.each_ref().map(describe)).unwrap(),
    ] {
        // Authority-side JavaScript rebase drops origins. Native rebase keeps them.
        assert_eq!(
            contextual_queue(&rebased, &mark, false),
            fixture["authority"]
        );
        assert_same_origins(&rebased, &originals);
    }

    let receive = |state: &mut EditorState, direct: Option<&mut Collab>, updates: &[Update]| {
        if let Some(session) = direct {
            session
                .receive_updates(state, session.synced_version(), updates)
                .unwrap();
        } else {
            let receipt =
                receive_updates(state, get_synced_version(state).unwrap(), updates).unwrap();
            state.update(receipt).unwrap();
        }
    };
    receive(&mut state, direct.as_mut(), &peer);
    assert_eq!(snapshot(&state, direct.as_ref()), fixture["received"]);
    let (_, rebased) = current(&state, direct.as_ref());
    assert_same_origins(&rebased, &originals);
    for (update, stage) in rebased.iter().zip(["partial", "acknowledged"]) {
        let echo = Update::new(
            update.client_id(),
            update.changes().clone(),
            update.change_desc().length(),
        )
        .unwrap()
        .with_effects(update.effects().iter().cloned());
        receive(&mut state, direct.as_mut(), &[echo]);
        assert_eq!(snapshot(&state, direct.as_ref()), fixture[stage]);
    }
}

fn contextual_queue(updates: &[Update], mark: &Mark, include_origin: bool) -> Value {
    Value::Array(
        updates
            .iter()
            .map(|update| {
                let origin = update.origin_context().filter(|_| include_origin).map(|context| {
            let transaction = context.transaction();
            json!({
                "start": [context.start_text().as_str(), context.start_selection().main().head()],
                "result": [context.new_text().as_str(), context.new_selection().main().head()],
                "changes": transaction.changes().to_json(context.start_text().len()).unwrap(),
                "explicit": transaction.selection().map(|selection| selection.main().head()),
            })
        });
                json!({
                    "client": update.client_id(),
                    "changes": update.changes().to_json(update.change_desc().length()).unwrap(),
                    "marks": marks(update.effects(), mark), "origin": origin,
                })
            })
            .collect(),
    )
}

fn assert_same_origins(updates: &[Update], originals: &[Update]) {
    assert_eq!(updates.len(), originals.len());
    for (mapped, original) in updates.iter().zip(originals) {
        assert!(ptr::eq(
            mapped.origin_context().unwrap(),
            original.origin_context().unwrap()
        ));
    }
}

#[test]
fn retains_wire_boundaries_for_shared_effects_and_checks_non_adjacent_echoes() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/collab_wire_oracle.json")).unwrap();
    let source = Text::new(string(&fixture["doc"]));
    let version = byte(&fixture["version"]);
    let pos = byte(&fixture["position"]);
    let mark = Mark::with_mapping(|(name, pos), desc| {
        desc.map_pos(*pos, Assoc::After, MapMode::TrackDel)
            .map(|pos| (name.clone(), pos))
    });
    let start = || {
        let config = CollabConfig::new("a", version)
            .with_shared_effects(|transaction| transaction.effects().to_vec());
        EditorState::with_extensions(source.as_str(), collab(config)).unwrap()
    };
    let mut initial = start();
    initial
        .update(Transaction::default().with_effects([mark.of(("local".into(), pos))]))
        .unwrap();
    let queued = sendable_updates(&initial).unwrap();
    let remote_mark = Update::new("b", ChangeSet::default(), source.len())
        .unwrap()
        .with_effects([mark.of(("remote".into(), pos))]);
    for case in items(&fixture["cases"]) {
        let edits = ChangeSet::from_json(&case["bytes"], &source).unwrap();
        let wire = ChangeDesc::from_json(&case["description"]).unwrap();
        let over = [
            remote_mark.clone(),
            Update::new("c", edits, source.len()).unwrap(),
        ];
        assert_eq!(over[1].change_desc(), &wire);
        let described = over.each_ref().map(describe);
        assert_eq!(
            wire_marks(&rebase_updates(&queued, &over).unwrap(), &mark),
            case["full"]
        );
        assert_eq!(
            wire_receipt(initial.clone(), &over, &mark),
            case["receipt"],
            "receipt for {}",
            case["bytes"]
        );
        assert_eq!(
            wire_marks(
                &rebase_updates_described(&queued, &described).unwrap(),
                &mark
            ),
            case["described"]
        );
    }

    let split = &fixture["cases"][0];
    let merged = &fixture["cases"][1];
    let mut local = start();
    let decode = |case: &Value| ChangeSet::from_json(&case["bytes"], &source).unwrap();
    local.update(Transaction::new(decode(split))).unwrap();
    local
        .update(Transaction::new(
            ChangeSet::single(Change::new(1..1, "!")).unwrap(),
        ))
        .unwrap();
    let queued = sendable_updates(&local).unwrap();
    let gap = Update::new("b", ChangeSet::single(Change::new(1..1, "?")).unwrap(), 1).unwrap();
    let last_echo =
        Update::new("a", ChangeSet::single(Change::new(2..2, "!")).unwrap(), 2).unwrap();
    let matching = [queued[0].clone(), gap.clone(), last_echo.clone()];
    let described = matching.each_ref().map(describe);
    let expected = &fixture["echoes"][0];
    assert_eq!(
        wire_receipt(local.clone(), &matching, &mark),
        expected["receipt"]
    );
    assert_eq!(
        rebase_updates(&queued, &matching).unwrap().len(),
        byte(&expected["full"])
    );
    assert_eq!(
        rebase_updates_described(&queued, &described).unwrap().len(),
        byte(&expected["described"])
    );

    // Upstream accepts the same client ID even with different deletion geometry.
    // Rust does not treat it as an echo even when the resulting text is the same.
    let wrong = [
        Update::new("a", decode(merged), source.len()).unwrap(),
        gap,
        last_echo,
    ];
    assert_eq!(fixture["echoes"][1], *expected);
    assert_eq!(
        receive_updates(&local, version, &wrong).unwrap_err(),
        CollabError::MismatchedAcknowledgement
    );
    assert_eq!(
        rebase_updates(&queued, &wrong),
        Err(CollabError::MismatchedAcknowledgement)
    );
    assert_eq!(
        rebase_updates_described(&queued, &wrong.each_ref().map(describe)),
        Err(CollabError::MismatchedAcknowledgement)
    );
    assert_eq!(local.text().as_str(), "z!");
    assert_eq!(get_synced_version(&local), Ok(version));
    assert_eq!(sendable_updates(&local).unwrap(), queued);
}

#[test]
fn wire_composition_preserves_split_boundaries_and_only_accepts_the_correct_echo() {
    for line in include_str!("fixtures/collab_composition_oracle.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let name = string(&case["name"]);
        let mark = StateEffectType::with_mapping(|pos, desc| {
            desc.map_pos(*pos, Assoc::After, MapMode::Simple)
        });
        let version = byte(&case["version"]);
        let config = CollabConfig::new("a", version)
            .with_shared_effects(|transaction| transaction.effects().to_vec());
        let mut state =
            EditorState::with_extensions(string(&case["source"]), collab(config)).unwrap();
        let local = ChangeSet::from_json(&case["local"], state.text()).unwrap();
        state
            .update(Transaction::new(local).with_effects([mark.of(byte(&case["localMark"]))]))
            .unwrap();
        let pending = sendable_updates(&state).unwrap();
        let text = state.text().clone();
        let selection = state.selection().clone();
        let mut authority = Text::new(string(&case["source"]));
        let over: Vec<_> = ["b", "c"]
            .into_iter()
            .zip(items(&case["remote"]))
            .map(|(id, wire)| {
                let changes = ChangeSet::from_json(wire, &authority).unwrap();
                let update = Update::new(id, changes, authority.len()).unwrap();
                update.apply_to(&mut authority).unwrap();
                update
            })
            .collect();
        assert_eq!(authority.as_str(), string(&case["authority"]), "{name}");
        let receipt = receive_updates(&state, version, &over).unwrap();
        assert_eq!(
            receipt.changes().to_json(state.text().len()).unwrap(),
            case["receipt"],
            "{name}"
        );
        let full = rebase_updates(&pending, &over).unwrap();
        let descriptions: Vec<_> = over.iter().map(describe).collect();
        let described = rebase_updates_described(&pending, &descriptions).unwrap();
        for (kind, updates) in [("echo", &full), ("described", &described)] {
            assert_eq!(updates.len(), 1, "{name}, {kind}");
            assert_eq!(
                updates[0].changes().to_json(authority.len()).unwrap(),
                case[kind],
                "{name}, {kind}"
            );
            assert_eq!(
                updates[0].effects()[0].get(&mark),
                Some(&byte(&case["mark"])),
                "{name}, {kind}"
            );
        }

        let mut batch = over;
        if case["echo"] != case["other"] {
            let other = ChangeSet::from_json(&case["other"], &authority).unwrap();
            let other = Update::new("a", other, authority.len()).unwrap();
            let mut other_authority = authority.clone();
            other.apply_to(&mut other_authority).unwrap();
            assert_eq!(other_authority.as_str(), string(&case["otherAuthority"]));
            batch.push(other);
            assert_eq!(
                receive_updates(&state, version, &batch).unwrap_err(),
                CollabError::MismatchedAcknowledgement,
                "{name}"
            );
            assert_eq!(
                rebase_updates(&pending, &batch),
                Err(CollabError::MismatchedAcknowledgement),
                "{name}"
            );
            let described: Vec<_> = batch.iter().map(describe).collect();
            assert_eq!(
                rebase_updates_described(&pending, &described),
                Err(CollabError::MismatchedAcknowledgement),
                "{name}"
            );
            assert_eq!(state.text(), &text);
            assert_eq!(state.selection(), &selection);
            assert_eq!(get_synced_version(&state), Ok(version));
            assert_eq!(sendable_updates(&state).unwrap(), pending);
            assert_ne!(case["wrongUpstream"]["doc"], case["otherAuthority"]);
            assert_eq!(case["wrongUpstream"]["version"], case["finalVersion"]);
            assert_eq!(case["wrongUpstream"]["pending"], 0);
            batch.pop();
        }
        batch.extend(full);
        assert!(rebase_updates(&pending, &batch).unwrap().is_empty());
        let described: Vec<_> = batch.iter().map(describe).collect();
        assert!(
            rebase_updates_described(&pending, &described)
                .unwrap()
                .is_empty()
        );
        let receipt = receive_updates(&state, version, &batch).unwrap();
        state.update(receipt).unwrap();
        assert_eq!(state.text().as_str(), string(&case["doc"]), "{name}");
        assert_eq!(get_synced_version(&state), Ok(byte(&case["finalVersion"])));
        assert!(sendable_updates(&state).unwrap().is_empty());
    }
}

fn describe(update: &Update) -> UpdateDescription<'_> {
    UpdateDescription {
        client_id: update.client_id(),
        changes: update.change_desc(),
    }
}

fn wire_marks(updates: &[Update], mark: &Mark) -> Value {
    Value::Array(
        updates
            .iter()
            .map(|update| marks(update.effects(), mark))
            .collect(),
    )
}

fn wire_receipt(mut state: EditorState, batch: &[Update], mark: &Mark) -> Value {
    let transaction = receive_updates(&state, get_synced_version(&state).unwrap(), batch).unwrap();
    let remote = is_remote(&transaction);
    let history = transaction.add_to_history();
    let marks = marks(transaction.effects(), mark);
    state.update(transaction).unwrap();
    json!({
        "doc": state.text().as_str(), "version": get_synced_version(&state).unwrap(),
        "client": get_client_id(&state).unwrap(), "remote": remote, "history": history,
        "marks": marks, "pending": wire_marks(&sendable_updates(&state).unwrap(), mark),
    })
}

fn check_case(case: &Value) {
    let name = string(&case["name"]);
    let mark = Mark::with_mapping(|(name, pos), through| {
        if name.starts_with('#') && through.is_empty() {
            return None;
        }
        through
            .map_pos(*pos, Assoc::After, MapMode::TrackDel)
            .map(|pos| (name.clone(), pos))
    });
    let private_effect = StateEffectType::new();
    let config = CollabConfig::new("a", byte(&case["version"])).with_shared_effects({
        let mark = mark.clone();
        move |tr| {
            tr.effects()
                .iter()
                .filter(|effect| effect.get(&mark).is_some())
                .cloned()
                .collect()
        }
    });
    let mut authority = Text::new(string(&case["doc"]));
    let mut state = EditorState::with_extensions(authority.as_str(), collab(config)).unwrap();
    for (index, edit) in items(&case["local"]).enumerate() {
        state
            .update(
                Transaction::new(changes(edit))
                    .with_effects(effects(&mark, edit))
                    .with_effects([private_effect.of(index)])
                    .with_user_event(format!("input.local{index}")),
            )
            .unwrap();
    }
    assert_eq!(
        queue(&sendable_updates(&state).unwrap(), &mark),
        case["initial"],
        "{name}: initial queue",
    );

    for (index, (batch, oracle)) in items(&case["batches"])
        .zip(items(&case["oracle"]))
        .enumerate()
    {
        let context = format!("{name}, batch {index}");
        let version = get_synced_version(&state).unwrap();
        let pending = sendable_updates(&state).unwrap();
        let batch = items(batch)
            .map(|edit| {
                let update = Update::new(string(&edit["id"]), changes(edit), authority.len())
                    .unwrap()
                    .with_effects(effects(&mark, edit));
                update.apply_to(&mut authority).unwrap();
                update
            })
            .collect::<Vec<_>>();
        assert_eq!(
            authority.as_str(),
            string(&oracle["authority"]),
            "{context}"
        );
        let descriptions = batch
            .iter()
            .map(|update| UpdateDescription {
                client_id: update.client_id(),
                changes: update.change_desc(),
            })
            .collect::<Vec<_>>();
        let errors = &case["nativeErrors"][index.to_string()];
        for (kind, result) in [
            ("full", rebase_updates(&pending, &batch)),
            (
                "described",
                rebase_updates_described(&pending, &descriptions),
            ),
        ] {
            if let Some(error) = errors[kind].as_str() {
                assert_eq!(
                    format!("{:?}", result.unwrap_err()),
                    error,
                    "{context}, {kind}"
                );
            } else {
                let mut expected = oracle["rebase"].clone();
                // Only JS drops the origin in authority-side mapped updates.
                for (update, local) in expected
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .zip(items(&oracle["queue"]))
                {
                    if update["origin"].is_null() {
                        update["origin"] = local["origin"].clone();
                    }
                }
                assert_eq!(
                    queue(&result.unwrap(), &mark),
                    expected,
                    "{context}, {kind}"
                );
            }
        }

        let received = receive_updates(&state, version, &batch);
        if let Some(error) = errors["receive"].as_str() {
            let expected = match error {
                "MismatchedAcknowledgement" => CollabError::MismatchedAcknowledgement,
                "UnexpectedAcknowledgement" => CollabError::UnexpectedAcknowledgement,
                _ => panic!("unrecognized receipt error {error}"),
            };
            assert_eq!(received.unwrap_err(), expected, "{context}");
            assert_eq!(get_synced_version(&state), Ok(version), "{context}");
            assert_eq!(sendable_updates(&state).unwrap(), pending, "{context}");
            assert!(oracle["queue"].as_array().unwrap().is_empty(), "{context}");
            assert_eq!(byte(&oracle["version"]), version + batch.len(), "{context}");
            break;
        }
        let received = received.unwrap();
        let mut expected = oracle["recv"].clone();
        // JS does not set addToHistory on acknowledgements and empty receipts.
        if expected["remote"] == false {
            assert_eq!(expected["history"], true, "{context}: upstream default");
            expected["history"] = json!(false);
        }
        assert_eq!(
            json!({
                "remote": is_remote(&received),
                "history": received.add_to_history(),
                "edits": edits(received.changes()),
                "marks": marks(received.effects(), &mark),
            }),
            expected,
            "{context}: receipt",
        );
        state.update(received).unwrap();
        assert_eq!(state.text().as_str(), string(&oracle["doc"]), "{context}");
        assert_eq!(
            get_client_id(&state).unwrap(),
            string(&oracle["client"]),
            "{context}"
        );
        assert_eq!(
            get_synced_version(&state),
            Ok(byte(&oracle["version"])),
            "{context}"
        );
        let next = sendable_updates(&state).unwrap();
        assert_eq!(queue(&next, &mark), oracle["queue"], "{context}: queue");
        let mut optimistic = authority.clone();
        for update in next {
            update.apply_to(&mut optimistic).unwrap();
        }
        assert_eq!(state.text(), &optimistic, "{context}: authority plus queue");
    }
}

fn queue(updates: &[Update], mark: &Mark) -> Value {
    Value::Array(
        updates
            .iter()
            .map(|update| {
                json!({
                    "id": update.client_id(),
                    "edits": edits(update.changes()),
                    "marks": marks(update.effects(), mark),
                    "origin": update.origin().and_then(Transaction::user_event),
                })
            })
            .collect(),
    )
}

fn changes(edit: &Value) -> ChangeSet {
    ChangeSet::new(
        items(&edit["edits"])
            .map(|edit| Change::new(byte(&edit[0])..byte(&edit[1]), string(&edit[2])))
            .collect(),
    )
    .unwrap()
}

fn edits(changes: &ChangeSet) -> Value {
    Value::Array(
        changes
            .changes()
            .map(|(range, insert)| json!([range.start, range.end, insert]))
            .collect(),
    )
}

fn effects<'a>(mark: &'a Mark, edit: &'a Value) -> impl Iterator<Item = StateEffect> + 'a {
    items(&edit["marks"]).map(|data| mark.of((string(&data[0]).to_owned(), byte(&data[1]))))
}

fn marks(effects: &[StateEffect], mark: &Mark) -> Value {
    Value::Array(
        effects
            .iter()
            .filter_map(|effect| effect.get(mark))
            .map(|(name, pos)| json!([name, pos]))
            .collect(),
    )
}

fn items(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

fn string(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}
