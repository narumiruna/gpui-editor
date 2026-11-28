use gpui_editor_codemirror::{
    ChangeSet, ChangeSpec, EditorState, StateEffect, StateEffectType, Text, Transaction,
    collab::{
        CollabConfig, Update, UpdateDescription, collab, get_synced_version, rebase_updates,
        rebase_updates_described, receive_updates, sendable_updates,
    },
};
use serde_json::{Value, json};

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/change_wire_oracle.json")).unwrap()
}

#[test]
fn concrete_wire_matches_pinned_state_sections() {
    let fixture = oracle();
    for case in fixture["sets"].as_array().unwrap() {
        let state = EditorState::new(case["doc"].as_str().unwrap());
        let source = state.text();
        let set = ChangeSet::from_json(&case["input"], source).unwrap();
        assert_eq!(set.to_json(source.len()).unwrap(), case["wire"]);
        let desc = set.desc(source.len()).unwrap();
        assert_eq!(desc.to_json(), case["desc"]);
        assert_eq!(json!(desc.is_empty()), case["empty"]);
        assert_eq!(desc.is_empty(), set.is_empty());
        let gaps: Vec<_> = desc
            .gaps()
            .map(|(old, new)| (old.start, new.start, old.len()))
            .collect();
        assert_eq!(json!(gaps), case["gaps"]);
        let spec = state.changes_from_specs(&[ChangeSpec::Set(&set)]).unwrap();
        assert_eq!(spec.to_json(source.len()).unwrap(), case["spec"]);
        let mut edited = source.clone();
        set.apply_to(&mut edited).unwrap();
        assert_eq!(edited.as_str(), case["applied"].as_str().unwrap());
        let inverse = set.inverted(source).unwrap();
        assert_eq!(inverse.to_json(edited.len()).unwrap(), case["inverse"]);
        inverse.apply_to(&mut edited).unwrap();
        assert_eq!(&edited, source);
    }

    let source = Text::new("abcd");
    let deletions = ChangeSet::from_json(&json!([[1], [1], 2]), &source).unwrap();
    let identity = ChangeSet::from_json(&json!([1, 3]), &source).unwrap();
    assert_eq!(
        deletions
            .map(&identity.desc(4).unwrap(), false)
            .unwrap()
            .to_json(4)
            .unwrap(),
        fixture["transform"]["map"]
    );
    let after = ChangeSet::from_json(&json!([1, 1]), &Text::new("cd")).unwrap();
    assert_eq!(
        deletions.compose(&after, 4).unwrap().to_json(4).unwrap(),
        fixture["transform"]["compose"]
    );
}

#[test]
fn collab_preserves_wire_markers_in_accepted_updates() {
    let doc = "é😀尾";
    let wire = json!([1, [0], 3]);
    let mut sender = EditorState::with_extensions(doc, collab(CollabConfig::new("a", 7))).unwrap();
    let mut receiver =
        EditorState::with_extensions(doc, collab(CollabConfig::new("b", 7))).unwrap();
    let marker = ChangeSet::from_utf16_json(&wire, sender.text()).unwrap();
    sender.update(Transaction::new(marker)).unwrap();
    assert_eq!(sender.text().as_str(), doc);
    let pending = sendable_updates(&sender).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].changes().to_utf16_json(sender.text()).unwrap(),
        wire
    );

    let receipt = receive_updates(&receiver, 7, &pending).unwrap();
    assert_eq!(
        receipt.changes().to_utf16_json(receiver.text()).unwrap(),
        wire
    );
    receiver.update(receipt).unwrap();
    assert_eq!(receiver.text().as_str(), doc);
    assert_eq!(get_synced_version(&receiver).unwrap(), 8);
    assert!(sendable_updates(&receiver).unwrap().is_empty());
}

#[test]
fn concrete_receipts_and_full_rebase_match_pinned_effect_maps() {
    let mark = StateEffectType::with_mapping(|values: &Vec<Value>, desc| {
        let mut values = values.clone();
        values.push(json!([desc.is_empty(), desc.to_json()]));
        Some(values)
    });
    let effects = |effects: &[StateEffect]| {
        json!(
            effects
                .iter()
                .map(|e| e.get(&mark).unwrap())
                .collect::<Vec<_>>()
        )
    };
    let queue = |updates: &[Update]| {
        json!(
            updates
                .iter()
                .map(|update| json!([
                    update
                        .changes()
                        .to_json(update.change_desc().length())
                        .unwrap(),
                    effects(update.effects())
                ]))
                .collect::<Vec<_>>()
        )
    };
    for case in oracle()["cases"].as_array().unwrap() {
        let config = CollabConfig::new("a", 7).with_shared_effects(|tr| tr.effects().to_vec());
        let doc = case["doc"].as_str().unwrap();
        let mut state = EditorState::with_extensions(doc, collab(config)).unwrap();
        let source = state.text().clone();
        if !case["local"].is_null() {
            let local = ChangeSet::from_json(&case["local"], &source).unwrap();
            state
                .update(Transaction::new(local).with_effects([mark.of(Vec::new())]))
                .unwrap();
        }
        let pending = sendable_updates(&state).unwrap();
        let remote = ChangeSet::from_json(&case["remote"], &source).unwrap();
        let batch = [
            Update::new("b", ChangeSet::default(), source.len())
                .unwrap()
                .with_effects([mark.of(Vec::new())]),
            Update::new("c", remote, source.len()).unwrap(),
        ];
        assert_eq!(batch[1].change_desc().to_json(), case["desc"]);
        let receipt = receive_updates(&state, 7, &batch).unwrap();
        let wire = receipt.changes().to_json(state.text().len()).unwrap();
        let mapped = effects(receipt.effects());
        state.update(receipt).unwrap();
        assert_eq!(
            json!({
                "wire": wire, "effects": mapped, "doc": state.text().as_str(),
                "version": get_synced_version(&state).unwrap(),
                "pending": queue(&sendable_updates(&state).unwrap()),
            }),
            case["receipt"],
            "{}",
            case["local"]
        );
        assert_eq!(
            queue(&rebase_updates(&pending, &batch).unwrap()),
            case["full"]
        );
        let descriptions = batch.each_ref().map(|update| UpdateDescription {
            client_id: update.client_id(),
            changes: update.change_desc(),
        });
        assert_eq!(
            queue(&rebase_updates_described(&pending, &descriptions).unwrap()),
            case["described"]
        );
    }
}
