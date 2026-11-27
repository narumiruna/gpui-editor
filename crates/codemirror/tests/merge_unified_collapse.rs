//! Captured from the SHA-512-verified @codemirror/merge 6.13.0 release in
//! headless Chrome. See `fixtures/merge_unified_collapse.mjs` to regenerate.

use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Change, ChangePermission, ChangeSet, Compartment, EditError, EditorState, Extension, READ_ONLY,
    Text, Transaction, TransactionCoordinates, change_filter,
    merge::{
        CollapseConfig, MergeView, MergeViewConfig, Side, UnifiedMerge, UnifiedMergeConfig,
        accept_chunk, get_chunks, go_to_next_chunk, original_doc_change_effect,
        unified_merge_state, unified_merge_view,
    },
};
use serde_json::{Value, json};

#[test]
fn unified_and_side_by_side_collapse_match_pinned_sessions() {
    for line in include_str!("fixtures/merge_unified_collapse.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let collapse = CollapseConfig {
            margin: byte(&case["collapse"]["margin"]),
            min_size: byte(&case["collapse"]["minSize"]),
        };
        if case["split"] == true {
            split_session(&case, collapse);
        } else {
            unified_session(&case, collapse);
        }
    }
}

fn unified_session(case: &Value, collapse: CollapseConfig) {
    let compartment = Compartment::new();
    let mut extension = unified_extension(Text::new(string(&case["a"])), Some(collapse));
    let mut state =
        EditorState::with_extensions(string(&case["b"]), compartment.of(extension.clone()))
            .unwrap();
    let snapshot = |state: &EditorState| {
        let merge = read_merge(state);
        snapshot(state, merge.original(), merge.collapsed_ranges(), None)
    };
    assert_eq!(
        snapshot(&state),
        case["initial"],
        "{}: initial",
        case["name"]
    );
    for (step, expected) in steps(case) {
        let expansion = || {
            Transaction::default().with_effects([UnifiedMerge::expand_effect(byte(&step["at"]))])
        };
        let tr = match string(&step["op"]) {
            "click" | "expand" => expansion(),
            "edit" if step["side"] == "a" => Transaction::default()
                .with_effects([original_doc_change_effect(&state, changes(step)).unwrap()]),
            "edit" => Transaction::new(changes(step)),
            "edit_and_expand" => Transaction::new(changes(step))
                .merge(
                    expansion(),
                    state.text().len(),
                    TransactionCoordinates::Original,
                )
                .unwrap(),
            "next" => go_to_next_chunk(&state).unwrap().unwrap(),
            "accept" => accept_chunk(&state, None).unwrap().unwrap(),
            "same" => {
                Transaction::default().with_effects([compartment.reconfigure(extension.clone())])
            }
            "enable" | "disable" => {
                extension = unified_extension(
                    read_merge(&state).original().clone(),
                    (step["op"] == "enable").then_some(collapse),
                );
                Transaction::default().with_effects([compartment.reconfigure(extension.clone())])
            }
            op => panic!("unknown operation: {op}"),
        };
        state.update(tr).unwrap();
        assert_eq!(&snapshot(&state), expected, "{}: {step}", case["name"]);
    }
}

fn split_session(case: &Value, collapse: CollapseConfig) {
    let mut config = MergeViewConfig {
        collapse_unchanged: Some(collapse),
        ..MergeViewConfig::default()
    };
    let mut view = MergeView::new(
        EditorState::new(string(&case["a"])),
        EditorState::new(string(&case["b"])),
        config,
    )
    .unwrap();
    let snapshot = |view: &MergeView| {
        snapshot(
            view.state(Side::B),
            view.state(Side::A).text(),
            view.projection().collapsed_ranges(Side::B),
            Some(view.projection().collapsed_ranges(Side::A)),
        )
    };
    assert_eq!(
        snapshot(&view),
        case["initial"],
        "{}: initial",
        case["name"]
    );
    for (step, expected) in steps(case) {
        match string(&step["op"]) {
            "click" => assert!(view.expand(Side::B, byte(&step["at"]))),
            "edit" => {
                let side = if step["side"] == "a" {
                    Side::A
                } else {
                    Side::B
                };
                view.dispatch(side, Transaction::new(changes(step)))
                    .unwrap();
            }
            "next" => {
                let tr = go_to_next_chunk(view.state(Side::B)).unwrap().unwrap();
                view.dispatch(Side::B, tr).unwrap();
            }
            "orientation" => {
                config.first = Side::B;
                view.reconfigure(config);
                assert_eq!(view.config().first, Side::B);
            }
            "same" => {
                let chunks = get_chunks(view.state(Side::A)).unwrap().unwrap().0;
                view.reconfigure_collapse(Some(collapse));
                let current = get_chunks(view.state(Side::A)).unwrap().unwrap().0;
                assert!(Arc::ptr_eq(&current, &chunks));
            }
            "enable" | "disable" => {
                config.collapse_unchanged = (step["op"] == "enable").then_some(collapse);
                view.reconfigure(config);
            }
            op => panic!("unknown operation: {op}"),
        }
        assert_eq!(&snapshot(&view), expected, "{}: {step}", case["name"]);
    }
}

#[test]
fn collapse_preserves_crlf_and_checked_edits_in_a_read_only_state() {
    let a = "😀\r\nold\r\ncontext\r\n🍎\r\nlast\r\n";
    let b = a.replace("old", "new");
    let mut state = EditorState::with_extensions(
        &b,
        Extension::group([
            unified_extension(
                Text::new(a),
                Some(CollapseConfig {
                    margin: 1,
                    min_size: 2,
                }),
            ),
            READ_ONLY.of(true),
            change_filter(|_, _| ChangePermission::Reject),
        ]),
    )
    .unwrap();
    let start = b.find('🍎').unwrap();
    let range = start..b.len();
    assert_eq!(
        read_merge(&state).collapsed_ranges(),
        std::slice::from_ref(&range)
    );

    let edit = |range| ChangeSet::single(Change::new(range, "!")).unwrap();
    assert!(matches!(
        original_doc_change_effect(&state, edit(start + 1..start + 2)),
        Err(EditError::InvalidRange),
    ));
    assert_eq!(
        state.update(Transaction::new(edit(start + 1..start + 2))),
        Err(EditError::InvalidRange),
    );
    state.update(Transaction::new(edit(start..start))).unwrap();
    state
        .update(Transaction::default().with_effects([UnifiedMerge::expand_effect(start + 1)]))
        .unwrap();
    assert_eq!(
        read_merge(&state).collapsed_ranges(),
        std::slice::from_ref(&range)
    );

    state
        .update(Transaction::default().with_effects([UnifiedMerge::expand_effect(start)]))
        .unwrap();
    let merge = read_merge(&state);
    assert!(merge.collapsed_ranges().is_empty());
    assert_eq!(merge.original().as_str(), a);
    assert_eq!(state.text().as_str(), b);
    assert!(state.read_only());
}

fn read_merge(state: &EditorState) -> std::sync::Arc<UnifiedMerge> {
    unified_merge_state(state).unwrap().unwrap()
}

fn unified_extension(original: Text, collapse: Option<CollapseConfig>) -> Extension {
    let mut config = UnifiedMergeConfig::new(original);
    config.collapse_unchanged = collapse;
    unified_merge_view(config)
}

fn snapshot(
    state: &EditorState,
    original: &Text,
    collapsed: &[Range<usize>],
    peer: Option<&[Range<usize>]>,
) -> Value {
    let (chunks, side) = get_chunks(state).unwrap().unwrap();
    assert_eq!(side, Side::B);
    let mut result = json!({
        "a": original.as_str(), "b": state.text().as_str(), "head": state.selection().main().head(),
        "chunks": chunks.iter().map(|c| {
            let a = c.range(Side::A);
            let b = c.range(Side::B);
            [a.start, a.end, b.start, b.end]
        }).collect::<Vec<_>>(),
        "collapsed": ranges(collapsed),
    });
    if let Some(peer) = peer {
        result["peer"] = json!(ranges(peer));
    }
    result
}

fn ranges(ranges: &[Range<usize>]) -> Vec<[usize; 2]> {
    ranges
        .iter()
        .map(|range| [range.start, range.end])
        .collect()
}

fn changes(step: &Value) -> ChangeSet {
    ChangeSet::single(Change::new(
        byte(&step["from"])..byte(&step["to"]),
        string(&step["insert"]),
    ))
    .unwrap()
}

fn steps(case: &Value) -> impl Iterator<Item = (&Value, &Value)> {
    let steps = case["steps"].as_array().unwrap();
    let oracle = case["oracle"].as_array().unwrap();
    assert_eq!(steps.len(), oracle.len());
    steps.iter().zip(oracle)
}

fn string(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}
