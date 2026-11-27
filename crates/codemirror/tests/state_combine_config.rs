use gpui_editor_codemirror::combine_config;
use serde_json::{Value, json};

fn entries(value: &Value) -> Vec<(&str, Option<Value>)> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let entry = entry.as_array().unwrap();
            (entry[0].as_str().unwrap(), entry.get(1).cloned())
        })
        .collect()
}

#[test]
fn config_defaults_conflicts_and_merge_order_follow_pinned_state() {
    for line in include_str!("fixtures/state_combine_config.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        let inputs: Vec<_> = row["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(entries)
            .collect();
        let inputs: Vec<_> = inputs.iter().map(Vec::as_slice).collect();
        let defaults = entries(&row["defaults"]);
        let mut calls = Vec::new();
        let result = combine_config(&inputs, &defaults, |key, left, right| {
            match row["rules"][key].as_str()? {
                "numbers" => {
                    calls.push(json!([key, left, right]));
                    Some(Some(json!(
                        left.as_i64().unwrap() * 10 + right.as_i64().unwrap()
                    )))
                }
                "reset" => {
                    calls.push(json!([key, left, right]));
                    Some(None)
                }
                rule => unreachable!("{rule}"),
            }
        });
        assert_eq!(json!(calls), row["calls"], "{name}");
        match result {
            Ok(result) => {
                let normalized: Vec<_> = result
                    .into_iter()
                    .map(|(key, value)| match value {
                        Some(value) => json!([key, value]),
                        None => json!([key]),
                    })
                    .collect();
                assert_eq!(json!(normalized), row["values"], "{name}");
                assert!(row["conflict"].is_null(), "{name}");
            }
            Err(key) => assert_eq!(json!(key), row["conflict"], "{name}"),
        }
    }
}
