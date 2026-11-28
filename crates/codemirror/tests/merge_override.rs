use std::{
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
};

use gpui_editor_codemirror::{
    Change, ChangePermission, ChangeSet, Compartment, ConfigurationError, EditError,
    EditorSelection, EditorState, Text, Transaction, change_filter,
    merge::{
        Chunk, CollapseConfig, DiffChange, DiffConfig, DiffOverride, InvalidDiffOverride,
        MergeView, MergeViewConfig, Side, UnifiedMerge, UnifiedMergeConfig, get_chunks,
        original_doc_change_effect, unified_merge_state, unified_merge_view_with_override,
    },
};
use serde_json::{Value, json};

fn cases() -> impl Iterator<Item = Value> {
    include_str!("fixtures/merge_override.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn items(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().unwrap().iter()
}

fn string(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn positions(value: &Value) -> [usize; 4] {
    serde_json::from_value(value.clone()).unwrap()
}

fn change([from_a, to_a, from_b, to_b]: [usize; 4]) -> DiffChange {
    DiffChange {
        a: from_a..to_a,
        b: from_b..to_b,
    }
}

fn changes(value: &Value) -> Vec<DiffChange> {
    items(value).map(|row| change(positions(row))).collect()
}

fn range_pair(a: &Range<usize>, b: &Range<usize>) -> [usize; 4] {
    [a.start, a.end, b.start, b.end]
}

fn chunk_snapshot(chunks: &[Chunk]) -> Value {
    json!(chunks
        .iter()
        .map(|chunk| json!({
            "range": range_pair(&chunk.range(Side::A), &chunk.range(Side::B)),
            "changes": chunk.changes().iter().map(|ch| range_pair(&ch.a, &ch.b)).collect::<Vec<_>>(),
            "precise": chunk.precise(),
        }))
        .collect::<Vec<_>>())
}

#[test]
fn raw_display_and_build_match_selected_pinned_cases() {
    for row in cases().filter(|row| row.get("raw").is_some()) {
        let name = string(&row["name"]);
        let (a, b) = (string(&row["a"]), string(&row["b"]));
        let (text_a, text_b) = (Text::new(a), Text::new(b));
        let expected = (a.to_owned(), b.to_owned());
        let script = changes(&row["raw"]);
        let calls = Arc::new(AtomicUsize::new(0));
        let diff_override = DiffOverride::new({
            let calls = calls.clone();
            move |a, b| {
                assert_eq!((a, b), (expected.0.as_str(), expected.1.as_str()));
                calls.fetch_add(1, Relaxed);
                script.clone()
            }
        });
        let raw = diff_override.diff(a, b).unwrap();
        assert_eq!(calls.swap(0, Relaxed), 1, "{name}: raw calls");
        assert_eq!(raw.changes, changes(&row["raw"]), "{name}");
        assert!(raw.precise, "{name}");
        let display = diff_override.presentable_diff(a, b).unwrap();
        assert_eq!(calls.swap(0, Relaxed), 1, "{name}: display calls");
        assert_eq!(display.changes, changes(&row["display"]), "{name}");
        assert!(display.precise, "{name}");
        let chunks = Chunk::build_with_override(&text_a, &text_b, &diff_override).unwrap();
        assert_eq!(calls.load(Relaxed), 1, "{name}: build calls");
        assert_eq!(chunk_snapshot(&chunks), row["chunks"], "{name}");
    }
}

#[test]
fn update_matches_selected_pinned_windows_and_chunks() {
    let row = cases()
        .find(|row| row["name"] == "aligned-update-windows")
        .unwrap();
    let pad = string(&row["pad"]).repeat(row["repeat"].as_u64().unwrap().try_into().unwrap());
    let [mut a, mut b] =
        [0, 1].map(|index| Text::new(string(&row["template"][index]).replace("{pad}", &pad)));
    let mut chunks = Vec::new();
    for step in items(&row["steps"]) {
        let side = match string(&step["side"]) {
            "A" => Some(Side::A),
            "B" => Some(Side::B),
            "build" => None,
            other => panic!("unknown side in oracle: {other}"),
        };
        let desc = side.map(|side| {
            let doc = if side == Side::A { &mut a } else { &mut b };
            let edits = ChangeSet::new(
                items(&step["edits"])
                    .map(|edit| {
                        let before = string(&edit[0]);
                        let start = doc.as_str().find(before).unwrap();
                        Change::new(start..start + before.len(), string(&edit[1]))
                    })
                    .collect(),
            )
            .unwrap();
            let desc = edits.desc(doc.len()).unwrap();
            edits.apply_to(doc).unwrap();
            desc
        });
        let expected: Vec<_> = items(&step["calls"])
            .map(|call| {
                let [from_a, to_a, from_b, to_b] = positions(&call["window"]);
                (
                    a.as_str()[from_a..to_a].to_owned(),
                    b.as_str()[from_b..to_b].to_owned(),
                    changes(&call["raw"]),
                )
            })
            .collect();
        let expected_calls = expected.len();
        let calls = Arc::new(AtomicUsize::new(0));
        let diff_override = DiffOverride::new({
            let calls = calls.clone();
            move |a, b| {
                let index = calls.fetch_add(1, Relaxed);
                let (expected_a, expected_b, raw) = &expected[index];
                assert_eq!((a, b), (expected_a.as_str(), expected_b.as_str()));
                raw.clone()
            }
        });
        chunks = match side.zip(desc) {
            Some((side, desc)) => {
                Chunk::update_with_override(&chunks, &a, &b, side, &desc, &diff_override).unwrap()
            }
            None => Chunk::build_with_override(&a, &b, &diff_override).unwrap(),
        };
        assert_eq!(calls.load(Relaxed), expected_calls, "side {side:?}");
        assert_eq!(chunk_snapshot(&chunks), step["chunks"], "side {side:?}");
    }
}

#[test]
fn invalid_outputs_fail_before_display_or_chunk_build() {
    for (name, a, b, script) in [
        ("out of bounds", "abc", "abc", vec![[0, 0, 0, usize::MAX]]),
        ("reversed", "abc", "abc", vec![[2, 1, 2, 2]]),
        ("inside scalar start", "a😀b", "a😀b", vec![[2, 5, 1, 5]]),
        ("inside scalar end", "aéb", "aéb", vec![[1, 3, 1, 2]]),
        (
            "unordered A",
            "abc",
            "abc",
            vec![[1, 2, 1, 2], [0, 1, 2, 3]],
        ),
        ("overlap B", "abc", "abc", vec![[0, 2, 0, 2], [2, 3, 1, 3]]),
        ("unequal prefix", "xab", "ycb", vec![[1, 2, 1, 2]]),
        (
            "unequal gap",
            "abc",
            "xyz",
            vec![[0, 1, 0, 1], [2, 3, 2, 3]],
        ),
        ("unequal suffix", "ax", "by", vec![[0, 1, 0, 1]]),
        ("no differences reported", "a", "b", vec![]),
        ("no Unicode folding", "e\u{301}", "é", vec![]),
    ] {
        let script: Vec<_> = script.into_iter().map(change).collect();
        let diff_override = DiffOverride::new(move |_, _| script.clone());
        assert_eq!(diff_override.diff(a, b), Err(InvalidDiffOverride), "{name}");
        assert_eq!(
            diff_override.presentable_diff(a, b),
            Err(InvalidDiffOverride),
            "{name}"
        );
        assert_eq!(
            Chunk::build_with_override(&Text::new(a), &Text::new(b), &diff_override),
            Err(InvalidDiffOverride),
            "{name}"
        );
    }
}

#[test]
fn invalid_later_window_returns_an_error_without_a_fallback_call() {
    let pad = "012345678\n".repeat(240);
    let a = Text::new(format!("left\n{pad}cat\n{pad}dog\n{pad}right\n"));
    let mut b = a.clone();
    let initial = Chunk::build(&a, &b);
    let edit = ChangeSet::new(
        [("cat", "cut"), ("dog", "dig")]
            .into_iter()
            .map(|(before, after)| {
                let start = b.as_str().find(before).unwrap();
                Change::new(start..start + before.len(), after)
            })
            .collect(),
    )
    .unwrap();
    let desc = edit.desc(b.len()).unwrap();
    edit.apply_to(&mut b).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let diff_override = DiffOverride::new({
        let calls = calls.clone();
        move |a, b| match calls.fetch_add(1, Relaxed) {
            0 => vec![change([0, a.len(), 0, b.len()])],
            1 => Vec::new(),
            _ => panic!("invalid override must not trigger a full rebuild"),
        }
    });
    assert_eq!(
        Chunk::update_with_override(&initial, &a, &b, Side::B, &desc, &diff_override),
        Err(InvalidDiffOverride)
    );
    assert_eq!(calls.load(Relaxed), 2);
}

#[test]
fn native_window_preserves_utf8_and_crlf() {
    let pad = "😀\r\n".repeat(800);
    let at = pad.len();
    let a = Text::new(format!("{pad}é home\r\n{pad}"));
    let mut b = a.clone();
    let edit = ChangeSet::single(Change::new(at..at + 'é'.len_utf8(), "🍎")).unwrap();
    let desc = edit.desc(b.len()).unwrap();
    edit.apply_to(&mut b).unwrap();
    let diff_override = DiffOverride::new(|a, b| {
        // The byte margin starts inside an emoji before rounding to a line.
        for text in [a, b] {
            assert!(text.starts_with('😀') && text.ends_with("\r\n"));
        }
        let start = a.find("é home").unwrap();
        assert_eq!(b.find("🍎 home"), Some(start));
        vec![change([
            start,
            start + 'é'.len_utf8(),
            start,
            start + '🍎'.len_utf8(),
        ])]
    });
    let chunks = Chunk::update_with_override(&[], &a, &b, Side::B, &desc, &diff_override).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].range(Side::A), at..at + "é home\r\n".len());
    assert_eq!(chunks[0].range(Side::B), at..at + "🍎 home\r\n".len());
    assert_eq!(chunks[0].apply(&a, &b, Side::A).unwrap(), b);
    assert_eq!(chunks[0].apply(&a, &b, Side::B).unwrap(), a);
}

#[test]
fn override_precision_does_not_inherit_the_native_search_budget() {
    let diff_override = DiffOverride::new(|a, b| {
        let config = DiffConfig { scan_limit: 0 };
        assert!(!config.diff("é😀😀\r\n", "é😁😁\r\n").precise);
        vec![change([0, a.len(), 0, b.len()])]
    });
    assert!(diff_override.diff("a", "b").unwrap().precise);
    let chunks =
        Chunk::build_with_override(&Text::new("a"), &Text::new("b"), &diff_override).unwrap();
    assert!(chunks[0].precise());
}

fn fixture(name: &str) -> Value {
    cases().find(|row| row["name"] == name).unwrap()
}

fn replacement(doc: &Text, before: &str, insert: &str) -> ChangeSet {
    let start = doc.as_str().find(before).unwrap();
    ChangeSet::single(Change::new(start..start + before.len(), insert)).unwrap()
}

fn pair_chunks(pair: &MergeView) -> Arc<[Chunk]> {
    let [a, b] = [Side::A, Side::B].map(|side| {
        let (chunks, actual_side) = get_chunks(pair.state(side)).unwrap().unwrap();
        assert_eq!(actual_side, side);
        assert_eq!(&*chunks, pair.chunks());
        chunks
    });
    assert!(Arc::ptr_eq(&a, &b));
    a
}

type Calls = Arc<Mutex<Vec<(&'static str, String, String)>>>;

fn record_diff(
    calls: &Calls,
    id: &'static str,
    script: fn(&str, &str) -> Vec<DiffChange>,
) -> DiffOverride {
    let calls = calls.clone();
    DiffOverride::new(move |a, b| {
        calls.lock().unwrap().push((id, a.to_owned(), b.to_owned()));
        script(a, b)
    })
}

fn full_range(a: &str, b: &str) -> Vec<DiffChange> {
    vec![change([0, a.len(), 0, b.len()])]
}

fn native_ranges(a: &str, b: &str) -> Vec<DiffChange> {
    DiffConfig::default().diff(a, b).changes
}

fn assert_snapshot(step: &Value, a: &Text, b: &EditorState, chunks: &[Chunk], calls: &Calls) {
    let op = string(&step["op"]);
    assert_eq!(a.as_str(), string(&step["a"]), "{op}");
    assert_eq!(b.text().as_str(), string(&step["b"]), "{op}");
    assert_eq!(json!(b.selection().main().head()), step["head"], "{op}");
    assert_eq!(chunk_snapshot(chunks), step["chunks"], "{op}");
    assert_eq!(json!(take_calls(calls)), step["calls"], "{op}");
}

fn take_calls(calls: &Calls) -> Vec<(&'static str, String, String)> {
    std::mem::take(&mut *calls.lock().unwrap())
}

#[test]
fn side_by_side_override_reconfiguration_matches_pinned_lifecycle() {
    let row = fixture("side-by-side-reconfigure");
    let mut steps = items(&row["steps"]);
    let build = steps.next().unwrap();
    let calls = Calls::default();
    let b = EditorState::with_extensions(
        string(&build["b"]),
        change_filter(|_, tr| {
            if tr.is_user_event("reject") {
                ChangePermission::Reject
            } else {
                ChangePermission::Allow
            }
        }),
    )
    .unwrap();
    let mut pair = MergeView::new_with_override(
        EditorState::new(string(&build["a"])),
        b,
        MergeViewConfig::default(),
        record_diff(&calls, "old", full_range),
    )
    .unwrap();
    let mut chunks = pair_chunks(&pair);
    let check = |pair: &MergeView, step, chunks: &[Chunk]| {
        assert_snapshot(
            step,
            pair.state(Side::A).text(),
            pair.state(Side::B),
            chunks,
            &calls,
        );
    };
    check(&pair, build, &chunks);
    for step in steps {
        match string(&step["op"]) {
            "swap" => pair.reconfigure_diff_override(Some(record_diff(&calls, "new", |a, b| {
                if a == b {
                    full_range(a, b)
                } else {
                    native_ranges(a, b)
                }
            }))),
            "select" => pair
                .dispatch(
                    Side::B,
                    Transaction::default().with_selection(EditorSelection::single(1, 1)),
                )
                .unwrap(),
            op @ ("filtered" | "edit-b" | "edit-a" | "edit-native") => {
                let (side, before, insert) = match op {
                    "edit-native" => (Side::A, "cut", "cot"),
                    "edit-a" => (Side::A, "cat", "cut"),
                    _ => (Side::B, "cat", "cut"),
                };
                let mut edit =
                    Transaction::new(replacement(pair.state(side).text(), before, insert));
                if op == "filtered" {
                    edit = edit.with_user_event("reject");
                }
                pair.dispatch(side, edit).unwrap();
            }
            "clear" => pair.reconfigure_diff_override(None),
            op => panic!("unexpected step: {op}"),
        }
        let next = pair_chunks(&pair);
        if let Some(kept) = step["kept"].as_bool() {
            assert_eq!(Arc::ptr_eq(&chunks, &next), kept, "{}", step["op"]);
        }
        check(&pair, step, &next);
        chunks = next;
    }
}

#[test]
fn unified_override_matches_pinned_effect_order_and_compartment_lifecycle() {
    let row = fixture("unified-override-lifecycle");
    let mut steps = items(&row["steps"]);
    let build = steps.next().unwrap();
    let calls = Calls::default();
    let compartment = Compartment::new();
    let extension = unified_merge_view_with_override(
        UnifiedMergeConfig::new(Text::new(string(&build["a"]))),
        record_diff(&calls, "old", native_ranges),
    );
    let mut state =
        EditorState::with_extensions(string(&build["b"]), compartment.of(extension.clone()))
            .unwrap();
    let mut chunks = get_chunks(&state).unwrap().unwrap().0;
    let check = |state: &EditorState, step, chunks: &[Chunk]| {
        let merge = unified_merge_state(state).unwrap().unwrap();
        assert_snapshot(step, merge.original(), state, chunks, &calls);
    };
    check(&state, build, &chunks);
    for step in steps {
        let tr = match string(&step["op"]) {
            "edit-a-then-b" => {
                let merge = unified_merge_state(&state).unwrap().unwrap();
                let effect =
                    original_doc_change_effect(&state, replacement(merge.original(), "cat", "pet"))
                        .unwrap();
                Transaction::new(replacement(state.text(), "cut", "pit"))
                    .with_effects([effect])
                    .with_selection(EditorSelection::single(1, 1))
            }
            "same" => {
                Transaction::default().with_effects([compartment.reconfigure(extension.clone())])
            }
            "new-and-edit" => {
                let next = unified_merge_view_with_override(
                    UnifiedMergeConfig::new(Text::new(string(&step["a"]))),
                    record_diff(&calls, "new", full_range),
                );
                Transaction::new(replacement(state.text(), "pit", "pot"))
                    .with_effects([compartment.reconfigure(next)])
            }
            op => panic!("unexpected step: {op}"),
        };
        state.update(tr).unwrap();
        let (next, side) = get_chunks(&state).unwrap().unwrap();
        assert_eq!(side, Side::B);
        if let Some(kept) = step["kept"].as_bool() {
            assert_eq!(Arc::ptr_eq(&chunks, &next), kept, "{}", step["op"]);
        }
        check(&state, step, &next);
        chunks = next;
    }
}

const INVALID_OVERRIDE: EditError = EditError::Configuration(ConfigurationError::InvalidFieldData);

#[test]
fn invalid_overrides_reject_view_construction() {
    let invalid = DiffOverride::new(|_, _| vec![change([0, 99, 0, 99])]);
    assert_eq!(
        MergeView::new_with_override(
            EditorState::new("equal"),
            EditorState::new("equal"),
            MergeViewConfig::default(),
            invalid.clone()
        )
        .unwrap_err(),
        INVALID_OVERRIDE,
    );
    let extension =
        unified_merge_view_with_override(UnifiedMergeConfig::new(Text::new("equal")), invalid);
    assert_eq!(
        EditorState::with_extensions("equal", extension).unwrap_err(),
        ConfigurationError::InvalidFieldData
    );
}

#[test]
fn invalid_side_by_side_override_rejects_the_batch_and_keeps_projection() {
    let config = MergeViewConfig {
        collapse_unchanged: Some(CollapseConfig {
            margin: 0,
            min_size: 1,
        }),
        ..MergeViewConfig::default()
    };
    let mut pair = MergeView::new(
        EditorState::new("head\ncat\nfoot\n"),
        EditorState::new("head\ncut\nfoot\n"),
        config,
    )
    .unwrap();
    let before = pair.clone();
    let chunks = pair_chunks(&pair);
    assert!(!pair.projection().collapsed_ranges(Side::B).is_empty());
    let calls = Calls::default();
    pair.reconfigure_diff_override(Some(record_diff(&calls, "invalid", |_, _| Vec::new())));
    let first = Transaction::new(replacement(pair.state(Side::B).text(), "cut", "cot"));
    let second = Transaction::new(replacement(&Text::new("head\ncot\nfoot\n"), "cot", "pet"))
        .with_selection(EditorSelection::single(1, 1));
    assert_eq!(
        pair.dispatch_batch(Side::B, [first, second]),
        Err(INVALID_OVERRIDE)
    );
    assert_eq!(
        take_calls(&calls),
        [(
            "invalid",
            "head\ncat\nfoot\n".to_owned(),
            "head\npet\nfoot\n".to_owned()
        )]
    );
    assert!(Arc::ptr_eq(&chunks, &pair_chunks(&pair)));
    for side in [Side::A, Side::B] {
        assert_eq!(pair.state(side).text(), before.state(side).text());
        assert_eq!(pair.state(side).selection(), before.state(side).selection());
        assert_eq!(pair.projection().rows(side), before.projection().rows(side));
    }
}

#[test]
fn invalid_unified_override_keeps_both_documents_and_the_installed_snapshot() {
    let calls = Calls::default();
    let compartment = Compartment::new();
    let mut config = UnifiedMergeConfig::new(Text::new("head\ncat\nfoot\n"));
    config.collapse_unchanged = Some(CollapseConfig {
        margin: 0,
        min_size: 1,
    });
    let extension = unified_merge_view_with_override(
        config,
        record_diff(&calls, "old", |a, b| {
            if a.contains("bad") || b.contains("bad") {
                Vec::new()
            } else {
                native_ranges(a, b)
            }
        }),
    );
    let mut state =
        EditorState::with_extensions("head\ncut\nfoot\n", compartment.of(extension)).unwrap();
    take_calls(&calls);
    let before = state.clone();
    let snapshot = unified_merge_state(&before).unwrap().unwrap();
    let collapsed_at = snapshot.collapsed_ranges()[0].start;
    let assert_retained = |state: &EditorState| {
        assert_eq!(state.text(), before.text());
        assert_eq!(state.selection(), before.selection());
        assert!(Arc::ptr_eq(
            &snapshot,
            &unified_merge_state(state).unwrap().unwrap()
        ));
        assert!(std::ptr::eq(
            state.compartment(&compartment).unwrap(),
            before.compartment(&compartment).unwrap()
        ));
    };
    let original_edit = |to| {
        original_doc_change_effect(&before, replacement(snapshot.original(), "cat", to)).unwrap()
    };
    let both = |edit_a, edit_b| {
        Transaction::new(replacement(before.text(), "cut", edit_b))
            .with_effects([
                original_edit(edit_a),
                UnifiedMerge::expand_effect(collapsed_at),
            ])
            .with_selection(EditorSelection::single(1, 1))
    };
    for (a, b, expected) in [
        (
            "bad",
            "pit",
            json!([["old", "head\nbad\nfoot\n", "head\ncut\nfoot\n"]]),
        ),
        (
            "pet",
            "bad",
            json!([
                ["old", "head\npet\nfoot\n", "head\ncut\nfoot\n"],
                ["old", "head\npet\nfoot\n", "head\nbad\nfoot\n"]
            ]),
        ),
    ] {
        assert_eq!(state.update(both(a, b)), Err(INVALID_OVERRIDE));
        assert_eq!(json!(take_calls(&calls)), expected);
        assert_retained(&state);
    }
    let invalid_extension = unified_merge_view_with_override(
        UnifiedMergeConfig::new(Text::new("head\nnew\nfoot\n")),
        record_diff(&calls, "invalid", |_, _| Vec::new()),
    );
    let replace_extension = Transaction::new(replacement(state.text(), "cut", "new"))
        .with_effects([compartment.reconfigure(invalid_extension)]);
    assert_eq!(state.update(replace_extension), Err(INVALID_OVERRIDE));
    assert_eq!(
        json!(take_calls(&calls)),
        json!([["invalid", "head\nnew\nfoot\n", "head\ncut\nfoot\n"]])
    );
    assert_retained(&state);
    state.update(both("pet", "pet")).unwrap();
    let merge = unified_merge_state(&state).unwrap().unwrap();
    assert_eq!(merge.original(), state.text());
    assert!(merge.chunks().is_empty());
    assert_eq!(
        json!(take_calls(&calls)),
        json!([
            ["old", "head\npet\nfoot\n", "head\ncut\nfoot\n"],
            ["old", "head\npet\nfoot\n", "head\npet\nfoot\n"]
        ])
    );
}
