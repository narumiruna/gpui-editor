use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    ChangeDesc, ChangeSet, StateEffect, StateEffectType, Text,
    collab::{Update, UpdateDescription, rebase_updates_described},
};
use serde_json::{Value, json};

#[test]
fn canonical_and_wire_identity_descriptions_match_pinned_state() {
    for row in include_str!("fixtures/state_effect_identity.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let name = row["name"].as_str().unwrap();
        let desc = ChangeDesc::from_json(&row["wire"]).unwrap();
        assert_eq!(json!(desc.is_empty()), row["empty"], "{name}");
        let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
        let point = StateEffectType::with_mapping({
            let calls = calls.clone();
            move |label: &String, mapping| {
                calls
                    .lock()
                    .unwrap()
                    .push(json!([label, mapping.is_empty(), mapping.to_json()]));
                (label != "drop").then(|| format!("{label}:{}", mapping.is_empty()))
            }
        });
        let marks = ["first", "drop", "last"].map(|label| point.of(label.to_owned()));
        let effects = StateEffect::map_all(&marks, &desc);
        let actual: Vec<_> = effects
            .iter()
            .map(|effect| effect.get(&point).unwrap())
            .collect();
        assert_eq!(json!(actual), row["values"], "{name}");
        assert_eq!(json!(*calls.lock().unwrap()), row["calls"], "{name}");
        calls.lock().unwrap().clear();
        let pending = Update::new("a", ChangeSet::default(), desc.length())
            .unwrap()
            .with_effects(marks);
        let rebased = rebase_updates_described(
            &[pending],
            &[UpdateDescription {
                client_id: "b",
                changes: &desc,
            }],
        )
        .unwrap();
        let [update] = rebased.as_slice() else {
            panic!("{name}: missing pending update");
        };
        let mapped: Vec<_> = update
            .effects()
            .iter()
            .map(|effect| effect.get(&point).unwrap())
            .collect();
        assert_eq!(json!(mapped), row["rebased"]["values"], "{name}");
        assert_eq!(
            json!(*calls.lock().unwrap()),
            row["rebased"]["calls"],
            "{name}"
        );
        let identity = ChangeSet::default().desc(desc.length()).unwrap();
        let later = ChangeSet::default().desc(desc.new_length()).unwrap();
        assert_eq!(
            identity.compose(&desc).unwrap().to_json(),
            row["before"],
            "{name}"
        );
        assert_eq!(
            desc.compose(&later).unwrap().to_json(),
            row["after"],
            "{name}"
        );
        assert_eq!(
            desc.map(&identity, false).unwrap().to_json(),
            row["mapped"],
            "{name}"
        );
        let source = Text::new("x".repeat(desc.length()));
        let edits = ChangeSet::from_json(&row["edits"]["input"], &source).unwrap();
        let empty = ChangeSet::default();
        let encoded = |set: ChangeSet| set.to_json(source.len()).unwrap();
        assert_eq!(
            encoded(empty.compose(&edits, source.len()).unwrap()),
            row["edits"]["before"],
            "{name}"
        );
        assert_eq!(
            encoded(edits.compose(&empty, source.len()).unwrap()),
            row["edits"]["after"],
            "{name}"
        );
        assert_eq!(
            encoded(edits.map(&identity, false).unwrap()),
            row["edits"]["mapped"],
            "{name}"
        );
        assert_eq!(
            encoded(edits.map_desc(&identity, false).unwrap()),
            row["edits"]["forced"],
            "{name}"
        );
    }
}
