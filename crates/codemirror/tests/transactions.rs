use gpui_editor_codemirror::{
    AnnotationType, Assoc, Change, ChangeDesc, ChangeSet, EditError, EditorSelection, EditorState,
    MapMode, RangeEdit, SelectionRange, StateEffect, StateEffectType, Transaction,
    TransactionCoordinates,
};

fn endpoints(range: SelectionRange) -> (usize, usize) {
    (range.anchor(), range.head())
}

#[test]
fn transaction_metadata_uses_typed_keys_and_event_prefixes() {
    let duration = AnnotationType::new();
    let other = AnnotationType::<u64>::new();
    let transaction = Transaction::default()
        .with_annotation(duration.of(42_u64))
        .with_annotation(duration.of(99))
        .with_user_event("input.type.compose");
    assert_eq!(transaction.annotation(&duration.clone()), Some(&42));
    assert_eq!(transaction.annotation(&other), None);
    for event in ["input", "input.type", "input.type.compose"] {
        assert!(transaction.is_user_event(event));
    }
    for event in ["in", "input.type.comp", "input.type.compose.more", "delete"] {
        assert!(!transaction.is_user_event(event));
    }
}

#[test]
fn built_in_annotations_use_shared_keys_and_keep_first_values_when_merged() {
    let first = Transaction::default()
        .with_annotation(Transaction::user_event_annotation().of("input.type".into()))
        .with_annotation(Transaction::time_annotation().of(123))
        .with_annotation(Transaction::remote_annotation().of(true))
        .with_annotation(Transaction::add_to_history_annotation().of(false));
    let other = Transaction::default()
        .with_user_event("delete")
        .with_time(456)
        .with_remote(false)
        .with_add_to_history(true);
    let merged = first
        .merge(other, 0, TransactionCoordinates::Original)
        .unwrap();
    assert!(merged.is_user_event("input"));
    assert_eq!(merged.time(), Some(123));
    assert!(merged.is_remote());
    assert!(!merged.add_to_history());
    assert!(Transaction::default().with_remote(true).add_to_history());
    assert!(!Transaction::default().is_remote());
}

#[test]
fn effect_mapping_keeps_order_and_discards_deleted_positions() {
    let point = StateEffectType::with_mapping(|offset: &usize, changes| {
        changes.map_pos(*offset, Assoc::After, MapMode::TrackDel)
    });
    let label = StateEffectType::new();
    let changes = ChangeDesc::new(7, vec![(0..0, 2), (2..5, 0)]).unwrap();
    let mapped = StateEffect::map_all(&[point.of(1), point.of(3), label.of("theme")], &changes);
    assert_eq!(mapped.len(), 2);
    assert_eq!(mapped[0].get(&point), Some(&3));
    assert_eq!(mapped[1].get(&label), Some(&"theme"));
    assert_eq!(mapped[1].get(&point), None);
}

#[test]
fn replaces_closed_and_collapsed_ranges_as_one_transaction() {
    let mut state = EditorState::new("a字c d🙂f");
    state
        .update(
            Transaction::default().with_selection(
                EditorSelection::new(
                    vec![
                        SelectionRange::new(4, 1),
                        SelectionRange::new(6, 6),
                        SelectionRange::new(7, 11),
                    ],
                    2,
                )
                .unwrap(),
            ),
        )
        .unwrap();

    let transaction = state.replace_selection("è").unwrap();
    assert_eq!(state.text().as_str(), "a字c d🙂f");
    state.update(transaction).unwrap();
    assert_eq!(state.text().as_str(), "aèc èdèf");
    assert_eq!(
        state
            .selection()
            .ranges()
            .iter()
            .copied()
            .map(endpoints)
            .collect::<Vec<_>>(),
        [(3, 3), (7, 7), (10, 10)]
    );
    assert_eq!(state.selection().main_index(), 2);
}

#[test]
fn each_range_edit_maps_its_effects_into_the_combined_document() {
    let point = StateEffectType::with_mapping(|offset: &usize, changes| {
        changes.map_pos(*offset, Assoc::After, MapMode::Simple)
    });
    let mut state = EditorState::new("abcdef");
    state
        .update(
            Transaction::default().with_selection(
                EditorSelection::new(
                    vec![SelectionRange::new(1, 1), SelectionRange::new(4, 4)],
                    1,
                )
                .unwrap(),
            ),
        )
        .unwrap();

    let tx = state
        .change_by_range(|range| {
            let at = range.head();
            let text = if at == 1 { "AA" } else { "Z" };
            let end = at + text.len();
            Ok(RangeEdit {
                changes: ChangeSet::single(Change::new(at..at, text))?,
                selection: SelectionRange::new(end, end),
                effects: vec![point.of(end)],
            })
        })
        .unwrap();
    assert_eq!(tx.effects()[0].get(&point), Some(&3));
    assert_eq!(tx.effects()[1].get(&point), Some(&7));
    state.update(tx).unwrap();
    assert_eq!(state.text().as_str(), "aAAbcdZef");
    assert_eq!(endpoints(state.selection().main()), (7, 7));
}

#[test]
fn rejects_a_locally_invalid_range_before_merging() {
    let state = EditorState::new("ab");
    let result = state.change_by_range(|_| {
        Ok(RangeEdit {
            changes: ChangeSet::single(Change::new(1..1, "字"))?,
            selection: SelectionRange::new(2, 2),
            effects: Vec::new(),
        })
    });
    assert!(matches!(result, Err(EditError::InvalidSelection)));
    assert_eq!(state.text().as_str(), "ab");
}
