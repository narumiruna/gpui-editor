use std::{
    collections::BTreeMap,
    ptr,
    sync::{Arc, Mutex},
};

use gpui_editor_codemirror::{
    Assoc, ChangeSet, Compartment, EditorState, MapMode, StateEffectType, Transaction,
    collab::{
        CollabConfig, CollabError, Update, collab, get_client_id, get_synced_version,
        receive_updates, sendable_updates,
    },
};
use serde_json::{Value, json};

#[test]
fn retained_origins_read_their_historical_queues() {
    let mut rows = include_str!("fixtures/collab_origin_queue.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap());
    let first = rows.next().unwrap();
    let initial = &first["initial"];
    let slot = Compartment::new();
    let mark = StateEffectType::<usize>::with_mapping(|position, changes| {
        changes.map_pos(*position, Assoc::After, MapMode::Simple)
    });
    let callbacks = Arc::new(Mutex::new(BTreeMap::new()));
    let config = |client: &str, version| {
        let mark = mark.clone();
        let callbacks = callbacks.clone();
        collab(
            CollabConfig::new(client, version).with_shared_effects_state(move |context| {
                let transaction = context.transaction();
                let effects: Vec<_> = transaction
                    .effects()
                    .iter()
                    .filter(|effect| effect.get(&mark).is_some())
                    .cloned()
                    .collect();
                if !effects.is_empty() {
                    let session = context.start_collab();
                    let start = snapshot(
                        context.start().text().as_str(),
                        session.client_id(),
                        Ok(session.synced_version()),
                        Ok(session.sendable_updates().to_vec()),
                        &mark,
                    );
                    callbacks.lock().unwrap().insert(
                        transaction.time().unwrap(),
                        json!({
                            "start": start, "resultClient": context.result_client_id(),
                        }),
                    );
                }
                effects
            }),
        )
    };
    let mut state = EditorState::with_extensions(
        initial["doc"].as_str().unwrap(),
        slot.of(config(
            initial["client"].as_str().unwrap(),
            number(&initial["version"]),
        )),
    )
    .unwrap();
    let mut retained = BTreeMap::<u64, Update>::new();
    let live = |state: &EditorState| {
        snapshot(
            state.text().as_str(),
            &get_client_id(state).unwrap(),
            get_synced_version(state),
            sendable_updates(state),
            &mark,
        )
    };
    assert_eq!(live(&state), first["live"]);
    let mut authority = state.text().clone();
    let mut rows = rows.peekable();
    while rows.peek().is_some_and(|row| row.get("action").is_some()) {
        let row = rows.next().unwrap();
        let action = &row["action"];
        if action.get("peer").is_some() || action.get("echo").is_some() {
            let updates = if let Some(peer) = action["peer"].as_str() {
                let changes = ChangeSet::from_json(&action["changes"], &authority).unwrap();
                vec![Update::new(peer, changes, authority.len()).unwrap()]
            } else {
                sendable_updates(&state).unwrap()[..number(&action["echo"])].to_vec()
            };
            state
                .update(
                    receive_updates(&state, get_synced_version(&state).unwrap(), &updates).unwrap(),
                )
                .unwrap();
            for update in updates {
                update.apply_to(&mut authority).unwrap();
            }
        } else {
            let changes = action
                .get("changes")
                .map_or_else(ChangeSet::default, |wire| {
                    ChangeSet::from_json(wire, state.text()).unwrap()
                });
            let mut effects = Vec::new();
            if let Some(position) = action.get("mark") {
                effects.push(mark.of(number(position)));
            }
            if let Some(setting) = action.get("config") {
                effects.push(
                    slot.reconfigure(config(setting[0].as_str().unwrap(), number(&setting[1]))),
                );
            } else if action["remove"] == true {
                effects.push(slot.reconfigure(Default::default()));
            }
            let mut transaction = Transaction::new(changes).with_effects(effects);
            if let Some(time) = action["time"].as_u64() {
                transaction = transaction.with_time(time);
            }
            state.update(transaction).unwrap();
        }
        assert_eq!(live(&state), row["live"], "{action}");
        if let Ok(updates) = sendable_updates(&state) {
            for update in updates {
                let time = update.origin().unwrap().time().unwrap();
                if let Some(previous) = retained.get(&time) {
                    assert!(ptr::eq(
                        previous.origin().unwrap(),
                        update.origin().unwrap()
                    ));
                }
                retained.insert(time, update);
            }
        }
    }
    drop(state);
    let origins: Vec<_> = rows.collect();
    assert_eq!(origins.len(), retained.len());
    for row in origins {
        let time = row["time"].as_u64().unwrap();
        let callback = callbacks
            .lock()
            .unwrap()
            .remove(&time)
            .unwrap_or(Value::Null);
        assert_eq!(
            callback, row["callback"],
            "shared-effects callback at {time}"
        );
        let origin = retained[&time].origin_context().unwrap();
        let start = origin.start_collab_state().unwrap();
        let result = origin.collab_state().unwrap();
        for (stage, doc, queue) in [
            ("start", origin.start_text(), start),
            ("result", origin.new_text(), result),
        ] {
            let updates = queue.sendable_updates();
            for update in updates.iter().flatten() {
                let tr = update.origin().unwrap();
                assert!(ptr::eq(tr, retained[&tr.time().unwrap()].origin().unwrap()));
            }
            assert_eq!(
                snapshot(
                    doc.as_str(),
                    queue.client_id(),
                    queue.synced_version(),
                    updates,
                    &mark
                ),
                row[stage]
            );
        }
        let tail = result.sendable_updates().unwrap().pop().unwrap();
        assert!(ptr::eq(origin, tail.origin_context().unwrap()));
    }
}

fn snapshot(
    doc: &str,
    client: &str,
    version: Result<usize, CollabError>,
    updates: Result<Vec<Update>, CollabError>,
    mark: &StateEffectType<usize>,
) -> Value {
    if client.is_empty() {
        assert_eq!(version, Err(CollabError::NotConfigured));
        assert_eq!(updates, Err(CollabError::NotConfigured));
        return json!({"doc": doc, "client": client, "version": null, "queue": null});
    }
    let queue: Vec<_> = updates
        .unwrap()
        .iter()
        .map(|update| {
            json!([
                update.client_id(),
                update
                    .changes()
                    .to_json(update.change_desc().length())
                    .unwrap(),
                update
                    .effects()
                    .iter()
                    .map(|effect| effect.get(mark).unwrap())
                    .collect::<Vec<_>>(),
                update.origin().unwrap().time()
            ])
        })
        .collect();
    json!({"doc": doc, "client": client, "version": version.unwrap(), "queue": queue})
}

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}
