//! State and command cases checked against `@codemirror/language` 6.13.1
//! with state 6.7.6 and view 6.44.0. Positions use native bytes.

use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeDesc, ChangeSet, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, SelectionRange, StateEffect, Transaction,
    language::{
        fold::{
            code_folding, fold_all, fold_code, fold_effect, fold_json, fold_state, folded_ranges,
            toggle_fold, unfold_all, unfold_code, unfold_effect,
        },
        json::JsonDocument,
    },
};
use serde_json::json;

fn folds(state: &EditorState) -> Vec<Range<usize>> {
    folded_ranges(state)
        .iter(0)
        .map(|range| range.from..range.to)
        .collect()
}

fn apply(state: &mut EditorState, effects: impl IntoIterator<Item = StateEffect>) {
    state
        .update(Transaction::default().with_effects(effects))
        .unwrap();
}

fn fold_ranges(state: &mut EditorState, ranges: impl IntoIterator<Item = Range<usize>>) {
    apply(
        state,
        ranges.into_iter().map(|range| fold_effect().of(range)),
    );
}

fn state_with_folds(doc: &str, ranges: impl IntoIterator<Item = Range<usize>>) -> EditorState {
    let mut state = EditorState::with_extensions(doc, code_folding()).unwrap();
    fold_ranges(&mut state, ranges);
    state
}

fn select(state: &mut EditorState, cursors: &[usize], main: usize) {
    let ranges = cursors
        .iter()
        .map(|&head| SelectionRange::new(head, head))
        .collect();
    let selection = EditorSelection::new(ranges, main).unwrap();
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
}

#[test]
fn field_is_optional_and_fold_effects_use_resulting_coordinates() {
    let mut state = EditorState::new("abcd");
    assert!(state.context().field(fold_state()).unwrap().is_none());
    apply(&mut state, [fold_effect().of(1..3)]);
    assert!(folds(&state).is_empty());

    state
        .update(
            Transaction::new(ChangeSet::single(Change::new(0..0, "😀")).unwrap())
                .with_effects([fold_effect().of(5..8), code_folding().append()]),
        )
        .unwrap();
    assert_eq!(state.text().as_str(), "😀abcd");
    assert_eq!(folds(&state), vec![5..8]);
    assert_eq!(
        state
            .context()
            .field(fold_state())
            .unwrap()
            .unwrap()
            .ranges()
            .len(),
        1
    );
}

#[test]
fn effect_mappers_exclude_insertions_at_both_edges() {
    let changes = ChangeDesc::new(10, vec![(2..2, 4), (6..6, 2)]).unwrap();
    for kind in [fold_effect(), unfold_effect()] {
        let effect = kind.of(2..6);
        assert_eq!(effect.map(&changes).unwrap().get(kind), Some(&(6..10)));
        let replace_exact = ChangeDesc::new(10, vec![(2..6, 3)]).unwrap();
        assert_eq!(effect.map(&replace_exact).unwrap().get(kind), Some(&(2..5)));
        let replace_cover = ChangeDesc::new(10, vec![(1..7, 3)]).unwrap();
        assert!(effect.map(&replace_cover).is_none());
    }
}

#[test]
fn stored_folds_map_through_insertions_replacements_and_deletions() {
    let mut state = state_with_folds("0123456789abcdefghijklmn", [1..12, 3..7, 14..20]);
    let insert = ChangeSet::new(vec![Change::new(1..1, "😀"), Change::new(12..12, "é")]).unwrap();
    state.update(Transaction::new(insert)).unwrap();
    assert_eq!(folds(&state), vec![5..16, 7..11, 20..26]);

    let replace = ChangeSet::single(Change::new(5..16, "🌿")).unwrap();
    state.update(Transaction::new(replace)).unwrap();
    assert_eq!(folds(&state), vec![5..9, 13..19]);

    let remove = ChangeSet::single(Change::new(5..9, "")).unwrap();
    state.update(Transaction::new(remove)).unwrap();
    assert_eq!(folds(&state), vec![9..15]);
}

#[test]
fn delete_events_clear_overlapping_old_ranges_but_not_touching_edges() {
    let original = state_with_folds("0123456789abcdefghijklmn", [1..12, 3..7, 14..20]);
    let remove = || Transaction::new(ChangeSet::single(Change::new(4..5, "")).unwrap());
    let mut state = original.clone();
    state.update(remove()).unwrap();
    assert_eq!(folds(&state), vec![1..11, 3..6, 13..19]);

    state = original.clone();
    state
        .update(remove().with_user_event("delete.forward"))
        .unwrap();
    assert_eq!(folds(&state), vec![13..19]);

    state = original;
    state
        .update(
            Transaction::new(ChangeSet::single(Change::new(1..3, "")).unwrap())
                .with_user_event("delete.backward"),
        )
        .unwrap();
    assert_eq!(folds(&state), vec![1..5, 12..18]);
}

#[test]
fn effect_order_preserves_queued_folds_and_exact_unfolds() {
    let mut state = state_with_folds("0123456789ab", []);
    apply(
        &mut state,
        [fold_effect().of(1..11), unfold_effect().of(1..11)],
    );
    assert_eq!(folds(&state), vec![1..11]);
    apply(
        &mut state,
        [fold_effect().of(1..11), unfold_effect().of(1..11)],
    );
    assert!(folds(&state).is_empty());

    fold_ranges(&mut state, [1..11, 3..7, 3..7]);
    assert_eq!(folds(&state), vec![1..11, 3..7, 3..7]);
    apply(&mut state, [unfold_effect().of(3..6)]);
    assert_eq!(folds(&state), vec![1..11, 3..7, 3..7]);
    apply(
        &mut state,
        [unfold_effect().of(3..7), fold_effect().of(3..7)],
    );
    assert_eq!(folds(&state), vec![1..11, 3..7]);
}

#[test]
fn explicit_primary_head_reveals_folds_after_effects_and_mapping() {
    let original = state_with_folds("0123456789abcdefghijklmn", [1..12, 3..7, 14..20]);
    let mut state = original.clone();
    select(&mut state, &[4, 15], 0);
    assert_eq!(folds(&state), vec![14..20]);

    state = original;
    state
        .update(Transaction::default().with_selection(EditorSelection::single(6, 3)))
        .unwrap();
    assert_eq!(folds(&state), vec![3..7, 14..20]);
    select(&mut state, &[7], 0);
    assert_eq!(folds(&state), vec![3..7, 14..20]);

    select(&mut state, &[4], 0);
    fold_ranges(&mut state, [1..12, 3..7]);
    assert_eq!(folds(&state), vec![1..12, 3..7, 14..20]);
    state
        .update(Transaction::new(
            ChangeSet::single(Change::new(0..0, "é")).unwrap(),
        ))
        .unwrap();
    assert_eq!(state.selection().main().head(), 6);
    assert_eq!(folds(&state), vec![3..14, 5..9, 16..22]);
    state
        .update(
            Transaction::default()
                .with_selection(EditorSelection::single(6, 6))
                .with_effects([fold_effect().of(4..8)]),
        )
        .unwrap();
    assert_eq!(folds(&state), vec![16..22]);
}

#[test]
fn fold_field_round_trips_nested_byte_ranges_in_native_order() {
    let source = "é😀0123456789";
    let state = state_with_folds(source, [2..10, 6..8, 11..14]);
    let mut encoded = state.to_json_with_fields(&[fold_json("folds")]).unwrap();
    assert_eq!(encoded["folds"], json!([2, 10, 6, 8, 11, 14]));
    // The upstream between() traversal can write the outer and disjoint
    // ranges before a nested range. This order also restores the same folds.
    encoded["folds"] = json!([2, 10, 11, 14, 6, 8]);
    let mut restored =
        EditorState::from_json_with_fields(&encoded, Extension::default(), &[fold_json("folds")])
            .unwrap();
    assert_eq!(restored.text(), state.text());
    assert_eq!(folds(&restored), folds(&state));
    apply(&mut restored, [unfold_effect().of(2..10)]);
    assert_eq!(folds(&restored), vec![6..8, 11..14]);

    let without = EditorState::new(source);
    let json = without.to_json_with_fields(&[fold_json("folds")]).unwrap();
    assert!(json.get("folds").is_none());
    let restored =
        EditorState::from_json_with_fields(&json, code_folding(), &[fold_json("folds")]).unwrap();
    assert!(restored.context().field(fold_state()).unwrap().is_some());
    assert!(folds(&restored).is_empty());
}

#[test]
fn invalid_ranges_cannot_enter_fold_state() {
    let mut state = state_with_folds("a🌿bc", std::iter::once(1..5));
    for range in [2..5, 1..99, 1..1, Range { start: 5, end: 1 }] {
        let transaction = Transaction::default()
            .with_effects([unfold_effect().of(1..5), fold_effect().of(range)]);
        assert_eq!(
            state.update(transaction),
            Err(EditError::Configuration(
                ConfigurationError::InvalidFieldData
            ))
        );
        assert_eq!(folds(&state), vec![1..5]);
    }
    for invalid in [
        json!({}),
        json!([1]),
        json!([1, 2]),
        json!([-1, 1]),
        json!([1.5, 5]),
        json!([7, 5]),
        json!([0, 8]),
    ] {
        let mut data = state.to_json();
        data["folds"] = invalid;
        assert!(
            EditorState::from_json_with_fields(&data, Extension::default(), &[fold_json("folds")])
                .is_err()
        );
    }
}

const DOC: &str = "head\nalpha\nbeta\ntail C\nchild\nend\nafter\n";

fn provider(_: &EditorState, start: usize, end: usize) -> Option<Range<usize>> {
    match (start, end) {
        (0, 4) => Some(4..16),
        (5, 10) => Some(10..13),
        (16, 22) => Some(22..33),
        _ => None,
    }
}

#[test]
fn fold_code_uses_first_selected_head_and_handles_existing_folds_like_the_provider() {
    let mut state = EditorState::new(DOC);
    select(&mut state, &[0, 5], 1);
    assert!(unfold_code(&state).is_none());
    assert!(unfold_all(&state).is_none());
    let transaction = fold_code(&state, provider).unwrap();
    state.update(transaction).unwrap();
    assert_eq!(state.selection().main().head(), 5);
    assert_eq!(folds(&state), vec![4..16]);
    assert!(fold_code(&state, provider).is_none());

    let mut calls = Vec::new();
    let duplicate = fold_code(&state, |_, start, end| {
        calls.push(start..end);
        Some(4..16)
    })
    .unwrap();
    assert_eq!(calls, vec![0..22]);
    state.update(duplicate).unwrap();
    assert_eq!(folds(&state), vec![4..16]);

    let mut state = EditorState::new(DOC);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(0, 5)))
        .unwrap();
    state.update(fold_code(&state, provider).unwrap()).unwrap();
    assert_eq!(folds(&state), vec![10..13]);
}

#[test]
fn fold_all_skips_nested_and_terminal_lines_even_with_existing_folds() {
    let mut state = EditorState::new(DOC);
    let mut calls = Vec::new();
    let transaction = fold_all(&state, |state, start, end| {
        calls.push(start..end);
        provider(state, start, end)
    })
    .unwrap();
    assert_eq!(calls, vec![0..4, 23..28, 29..32, 33..38]);
    state.update(transaction).unwrap();
    assert_eq!(folds(&state), vec![4..16]);
    assert!(fold_all(&state, provider).is_none());
    assert!(fold_all(&EditorState::new(""), |_, _, _| panic!("empty doc")).is_none());

    let mut state = state_with_folds(DOC, [4..16, 8..13, 22..33]);
    let mut calls = Vec::new();
    let duplicate = fold_all(&state, |_, start, end| {
        calls.push(start..end);
        Some(4..16)
    })
    .unwrap();
    assert_eq!(calls, vec![0..38]);
    state.update(duplicate).unwrap();
    assert_eq!(folds(&state), vec![4..16, 8..13, 22..33]);
    state.update(unfold_all(&state).unwrap()).unwrap();
    assert!(unfold_all(&state).is_none());
}

#[test]
fn toggle_finds_the_nearest_container_and_unfold_keeps_nested_ranges() {
    let mut state = EditorState::new(DOC);
    select(&mut state, &[12], 0);
    let mut calls = Vec::new();
    let transaction = toggle_fold(&state, |state, start, end| {
        calls.push(start..end);
        provider(state, start, end)
    })
    .unwrap();
    assert_eq!(calls, vec![11..15, 5..10]);
    state.update(transaction).unwrap();
    assert_eq!(folds(&state), vec![10..13]);
    assert_eq!(state.selection().main().head(), 12);
    state
        .update(toggle_fold(&state, |_, _, _| panic!("unfold before querying provider")).unwrap())
        .unwrap();
    assert!(folds(&state).is_empty());

    select(&mut state, &[5, 16, 23], 0);
    fold_ranges(&mut state, [4..16, 8..13, 22..33]);
    let transaction = unfold_code(&state).unwrap();
    assert_eq!(transaction.effects().len(), 2);
    assert_eq!(
        transaction.effects()[0].get(unfold_effect()),
        Some(&(4..16))
    );
    assert!(
        transaction.effects()[1]
            .get(&gpui_editor_codemirror::view::ANNOUNCE)
            .is_some()
    );
    state.update(transaction).unwrap();
    assert_eq!(folds(&state), vec![8..13, 22..33]);
    let transaction = unfold_code(&state).unwrap();
    assert_eq!(transaction.effects().len(), 4);
    state.update(transaction).unwrap();
    assert!(folds(&state).is_empty());
}

#[test]
fn parser_provider_uses_native_byte_offsets_and_crlf_bounds() {
    let source = "{\r\n  \"😀\": [\r\n    0\r\n  ]\r\n}\r\n";
    let mut state = EditorState::new(source);
    let parsed = JsonDocument::parse(state.text().clone());
    let transaction = fold_all(&state, |_, start, end| parsed.foldable(start, end)).unwrap();
    state.update(transaction).unwrap();
    let closing = source.find('}').unwrap();
    assert_eq!(folds(&state), vec![1..closing]);
    let mut calls = Vec::new();
    assert!(
        fold_code(&state, |_, start, end| {
            calls.push(start..end);
            None
        })
        .is_none()
    );
    assert_eq!(calls, vec![0..closing + 1]);
}
