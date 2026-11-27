//! Recorded with @codemirror/merge 6.13.0 and the state, language, and view
//! tarballs verified against docs/codemirror-upstream.json. The scratch oracle
//! ran the real EditorState and commands with an inert view dispatch stub. It
//! used inert elements to record control kinds and confirm callback dispatch,
//! without a browser, layout, or visual rendering. Positions are UTF-8 bytes,
//! with upstream chunk ends clipped to EOF.

use gpui_editor_codemirror::{
    Change, ChangePermission, ChangeSet, Compartment, EditorSelection, EditorState, Extension,
    Text, Transaction, change_filter,
    merge::{
        MergeControlKind, Side, UnifiedMergeConfig, accept_chunk, get_chunks, go_to_next_chunk,
        go_to_previous_chunk, original_doc_change_effect, reject_chunk, unified_merge_state,
        unified_merge_view,
    },
};
use serde_json::{Value, json};

#[test]
fn unified_merge_matches_pinned_state_and_commands() {
    for line in include_str!("fixtures/merge_session_oracle.jsonl").lines() {
        let mut case: Value = serde_json::from_str(line).unwrap();
        if case["name"] == "insertion and deletion of complete lines" {
            normalize_shifted_line_insertion(&mut case);
        }
        check_case(&case);
    }
}

fn normalize_shifted_line_insertion(value: &mut Value) {
    if let Some(obj) = value.as_object_mut() {
        for child in obj.values_mut() {
            normalize_shifted_line_insertion(child);
        }
    } else if let Some(list) = value.as_array_mut() {
        for child in list {
            if *child == json!([-1, -1, -1, 8]) {
                // The pin moves the chunk start but not the insertion past
                // the shared LF. Native changes rotate the insertion too.
                *child = json!([0, 0, 0, 9]);
            } else {
                normalize_shifted_line_insertion(child);
            }
        }
    }
}

fn check_case(case: &Value) {
    let name = string(&case["name"]);
    let compact = case["compact"] == true;
    let expand = |doc: &Value| {
        let doc = string(doc);
        match case["pad"].as_str() {
            Some(pad) => doc.replace("{pad}", &pad.repeat(byte(&case["count"]))),
            None => doc.to_owned(),
        }
    };
    let compartment = Compartment::new();
    let mut extension = merge_extension(expand(&case["a"]), case);
    let installed = if case["compartment"] == true {
        compartment.of(extension.clone())
    } else {
        extension.clone()
    };
    let filter = if case["filtered"] == true {
        change_filter(|_, tr| {
            if tr.is_user_event("revert") {
                ChangePermission::Reject
            } else {
                ChangePermission::Protect(std::iter::once(5..8).collect())
            }
        })
    } else {
        Extension::default()
    };
    let mut state =
        EditorState::with_extensions(expand(&case["b"]), Extension::group([installed, filter]))
            .unwrap();
    assert_eq!(
        snapshot(&state, compact),
        case["initial"],
        "{name}: initial"
    );
    for (index, (step, expected)) in items(&case["steps"])
        .zip(items(&case["oracle"]))
        .enumerate()
    {
        let context = format!("{name}: step {index}");
        let merge = unified_merge_state(&state).unwrap().unwrap();
        let mut tr = Transaction::new(edits(step, "b", "bNamed", state.text()));
        let a = edits(step, "a", "aNamed", merge.original());
        if !a.is_empty() {
            tr = tr.with_effects([original_doc_change_effect(&state, a).unwrap()]);
        }
        if let Some(head) = step["head"].as_u64() {
            let head = usize::try_from(head).unwrap();
            tr = tr.with_selection(EditorSelection::single(head, head));
        }
        if let Some(kind) = step["reconfigure"].as_str() {
            match kind {
                "same" => (),
                "new" => {
                    extension = merge_extension(string(&step["original"]), step);
                }
                "remove" => extension = Extension::default(),
                _ => panic!("unknown reconfiguration: {kind}"),
            }
            tr = tr.with_effects([compartment.reconfigure(extension.clone())]);
        }
        if step["apply"] == "accept" {
            tr = accept_chunk(&state, None).unwrap().unwrap();
        }
        state
            .update(tr)
            .unwrap_or_else(|error| panic!("{context}: {error:?}"));
        assert_eq!(
            snapshot(&state, compact),
            expected["state"],
            "{context}: state"
        );
        if let Some(checks) = expected["checks"].as_array() {
            for check in checks {
                let at = (!check["at"].is_null()).then(|| byte(&check["at"]));
                for (kind, tr) in [
                    ("accept", accept_chunk(&state, at)),
                    ("reject", reject_chunk(&state, at)),
                ] {
                    assert_eq!(
                        action(&state, tr.unwrap(), false),
                        check[kind],
                        "{context}: {kind} at {at:?}"
                    );
                }
            }
        }
        if let Some(nav) = expected["nav"].as_array() {
            for (tr, oracle) in [go_to_next_chunk(&state), go_to_previous_chunk(&state)]
                .into_iter()
                .zip(nav)
            {
                assert_eq!(
                    &action(&state, tr.unwrap(), true),
                    oracle,
                    "{context}: navigation"
                );
            }
        }
    }
}

fn merge_extension(original: impl Into<String>, settings: &Value) -> Extension {
    let mut config = UnifiedMergeConfig::new(Text::new(original));
    config.merge_controls = settings["controls"] != false;
    if !settings["scanLimit"].is_null() {
        config.diff_config.scan_limit = byte(&settings["scanLimit"]);
    }
    unified_merge_view(config)
}

fn edits(step: &Value, key: &str, named: &str, doc: &Text) -> ChangeSet {
    let changes = if let Some(edits) = step[key].as_array() {
        edits
            .iter()
            .map(|edit| Change::new(byte(&edit[0])..byte(&edit[1]), string(&edit[2])))
            .collect()
    } else if let Some(edits) = step[named].as_array() {
        edits
            .iter()
            .map(|edit| {
                let needle = string(&edit[0]);
                let from = doc.as_str().find(needle).unwrap();
                Change::new(from..from + needle.len(), string(&edit[1]))
            })
            .collect()
    } else if key == "b"
        && let Some(tail) = step["bTail"].as_str()
    {
        vec![Change::new(doc.len()..doc.len(), tail)]
    } else {
        Vec::new()
    };
    ChangeSet::new(changes).unwrap()
}

fn snapshot(state: &EditorState, compact: bool) -> Value {
    let Some(merge) = unified_merge_state(state).unwrap() else {
        assert!(get_chunks(state).unwrap().is_none());
        return Value::Null;
    };
    assert_eq!(get_chunks(state).unwrap().unwrap().1, Side::B);
    let chunks = merge.chunks().iter().map(|chunk| {
        let a = chunk.range(Side::A);
        let b = chunk.range(Side::B);
        let changes = chunk.changes().iter().map(|ch| [ch.a.start, ch.a.end, ch.b.start, ch.b.end]).collect::<Vec<_>>();
        json!({"a":[a.start,a.end], "b":[b.start,b.end], "precise":chunk.precise(), "changes":changes})
    }).collect::<Vec<_>>();
    let controls = merge
        .controls()
        .map(|control| {
            let kind = match control.kind {
                MergeControlKind::Accept => "accept",
                MergeControlKind::Reject => "reject",
            };
            json!([kind, control.position])
        })
        .collect::<Vec<_>>();
    let mut snapshot = json!({"side":"b", "head":state.selection().main().head(), "chunks":chunks, "controls":controls});
    if !compact {
        snapshot["a"] = json!(merge.original().as_str());
        snapshot["b"] = json!(state.text().as_str());
    }
    snapshot
}

fn action(state: &EditorState, tr: Option<Transaction>, nav: bool) -> Value {
    let Some(tr) = tr else { return Value::Null };
    let edit = tr
        .changes()
        .changes()
        .map(|(range, insert)| json!([range.start, range.end, insert]))
        .collect::<Vec<_>>();
    let mut result = json!({"event":tr.user_event(), "scroll":tr.scroll_into_view(), "edit":edit});
    let mut state = state.clone();
    state.update(tr).unwrap();
    if nav {
        result["head"] = json!(state.selection().main().head());
    } else {
        result["state"] = snapshot(&state, false);
    }
    result
}

fn string(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn items(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().unwrap().iter()
}
