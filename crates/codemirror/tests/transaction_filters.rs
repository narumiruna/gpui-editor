use gpui_editor_codemirror::{
    AnnotationType, Assoc, Change, ChangePermission, ChangeSet, EditError, EditorSelection,
    EditorState, Extension, History, MapMode, StateEffectType, StateField, Transaction,
    TransactionCoordinates, TransactionExtension, change_filter, transaction_extender,
    transaction_filter,
};
use std::ops::Range;

fn replace(start: usize, end: usize, text: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(start..end, text)).unwrap())
}

#[test]
fn protected_intervals_clip_edits_and_keep_boundary_insertions() {
    fn change_set(changes: &[(Range<usize>, &str)]) -> ChangeSet {
        ChangeSet::new(
            changes
                .iter()
                .map(|(range, text)| Change::new(range.clone(), *text))
                .collect(),
        )
        .unwrap()
    }
    type Fixture<'a> = (
        &'a str,
        &'a [(Range<usize>, &'a str)],
        Range<usize>,
        usize,
        &'a str,
        usize,
    );
    let cases: &[Fixture<'_>] = &[
        ("abcdef", &[(0..6, "Q")], 2..4, 1, "Qcd", 1),
        ("abcdef", &[(2..5, "QR")], 2..4, 4, "abcdf", 4),
        (
            "abcdef",
            &[(2..2, "A"), (3..3, "B"), (4..4, "C")],
            2..4,
            7,
            "abAcdCef",
            6,
        ),
        ("a字bc", &[(0..5, "🦊")], 1..4, 4, "🦊字c", 4),
    ];
    for (source, changes, protect, head, expected, expected_head) in cases {
        let point = StateEffectType::with_mapping(|at: &usize, desc| {
            desc.map_pos(*at, Assoc::Before, MapMode::TrackDel)
        });
        let check_effect = StateField::new(|_| Ok(None), {
            let point = point.clone();
            move |_, update| {
                Ok(update
                    .transaction()
                    .effects()
                    .iter()
                    .find_map(|e| e.get(&point))
                    .copied())
            }
        });
        let config = Extension::group([
            change_filter({
                let protect = protect.clone();
                move |_, _| ChangePermission::Protect(vec![protect.clone()])
            }),
            check_effect.extension(),
        ]);
        let mut state = EditorState::with_extensions(*source, config).unwrap();
        state
            .update(
                Transaction::new(change_set(changes))
                    .with_selection(EditorSelection::single(*head, *head))
                    .with_effects([point.of(*head)]),
            )
            .unwrap();
        assert_eq!(state.text().as_str(), *expected);
        assert_eq!(state.selection().main().head(), *expected_head);
        assert_eq!(
            *state.context().field(&check_effect).unwrap().unwrap(),
            Some(*expected_head)
        );
    }
}

#[test]
fn transaction_filter_order_and_extender_order_are_independent() {
    let marker = AnnotationType::new();
    let observed = StateField::new(|_| Ok((None, false)), {
        let marker = marker.clone();
        move |_, update| {
            Ok((
                update.transaction().annotation(&marker).copied(),
                update.transaction().scroll_into_view(),
            ))
        }
    });
    let config = Extension::group([
        transaction_filter(|_, tx| {
            assert!(tx.is_user_event("input"));
            Ok(replace(0, 1, "Z").with_user_event("filtered"))
        }),
        transaction_filter(|_, _| Ok(Transaction::default().with_user_event("input.type"))),
        transaction_extender({
            let marker = marker.clone();
            move |_, tx| {
                assert!(tx.annotation(&marker).is_none());
                TransactionExtension {
                    annotations: vec![marker.of("first")],
                    ..Default::default()
                }
            }
        }),
        transaction_extender({
            let marker = marker.clone();
            move |_, tx| {
                assert!(!tx.scroll_into_view());
                TransactionExtension {
                    annotations: vec![marker.of("last")],
                    scroll_into_view: true,
                    ..Default::default()
                }
            }
        }),
        observed.extension(),
    ]);
    let mut state = EditorState::with_extensions("abc", config).unwrap();
    state.update(Transaction::default()).unwrap();
    assert_eq!(state.text().as_str(), "Zbc");
    assert_eq!(
        *state.context().field(&observed).unwrap().unwrap(),
        (Some("last"), true)
    );

    state
        .update(replace(0, 1, "X").without_filtering())
        .unwrap();
    assert_eq!(state.text().as_str(), "Xbc");
    assert_eq!(
        *state.context().field(&observed).unwrap().unwrap(),
        (Some("last"), true)
    );
}

#[test]
fn invalid_protection_is_atomic_and_reject_filter_strips_changes() {
    let mut state = EditorState::with_extensions(
        "a字",
        change_filter(|_, _| ChangePermission::Protect(std::iter::once(2..4).collect())),
    )
    .unwrap();
    assert_eq!(
        state.update(replace(0, 0, "x")),
        Err(EditError::InvalidRange)
    );
    assert_eq!(state.text().as_str(), "a字");

    let mut state =
        EditorState::with_extensions("abc", change_filter(|_, _| ChangePermission::Reject))
            .unwrap();
    state
        .update(replace(0, 3, "字").with_selection(EditorSelection::single(3, 3)))
        .unwrap();
    assert_eq!(state.text().as_str(), "abc");
    assert_eq!(state.selection().main().head(), 3);
}

#[test]
fn history_replay_does_not_reapply_transaction_filters() {
    let mut history = History::new(
        EditorState::with_extensions("abc", change_filter(|_, _| ChangePermission::Reject))
            .unwrap(),
    );
    history
        .update(replace(1, 1, "字").without_filtering(), (), |_| (None, ()))
        .unwrap();
    assert_eq!(history.state().text().as_str(), "a字bc");
    assert_eq!(history.undo(), Some(()));
    assert_eq!(history.state().text().as_str(), "abc");
    assert_eq!(history.redo(), Some(()));
    assert_eq!(history.state().text().as_str(), "a字bc");
}

#[test]
fn direct_history_selection_updates_do_not_make_unrecorded_edits() {
    let mut history = History::<()>::new(
        EditorState::with_extensions("abc", transaction_filter(|_, _| Ok(replace(0, 1, "Z"))))
            .unwrap(),
    );
    history
        .set_selection(EditorSelection::single(1, 1))
        .unwrap();
    assert_eq!(history.state().text().as_str(), "abc");
    assert_eq!(history.state().selection().main().head(), 1);
    assert!(!history.can_undo());
}

#[test]
fn merged_transactions_map_metadata_in_both_coordinate_modes() {
    let point = StateEffectType::with_mapping(|at: &usize, desc| {
        desc.map_pos(*at, Assoc::After, MapMode::Simple)
    });
    let annotation = AnnotationType::new();
    let first = || {
        replace(1, 1, "AA")
            .with_selection(EditorSelection::single(3, 3))
            .with_effects([point.of(3)])
            .with_annotation(annotation.of("first"))
    };
    let cases = [
        (
            TransactionCoordinates::Original,
            replace(2, 4, "Q")
                .with_selection(EditorSelection::single(3, 3))
                .with_effects([point.of(3)]),
            "aAAbQef",
            5,
            5,
        ),
        (
            TransactionCoordinates::Sequential,
            replace(2, 3, "Z").with_effects([point.of(6)]),
            "aAZbcdef",
            3,
            6,
        ),
    ];
    for (coordinates, second, expected, selection, effect) in cases {
        let mut state = EditorState::new("abcdef");
        let tx = first()
            .merge(
                second
                    .with_annotation(annotation.of("second"))
                    .with_scroll_into_view(),
                6,
                coordinates,
            )
            .unwrap();
        assert_eq!(tx.annotation(&annotation), Some(&"first"));
        assert_eq!(tx.effects()[0].get(&point), Some(&3));
        assert_eq!(tx.effects()[1].get(&point), Some(&effect));
        assert!(tx.scroll_into_view());
        state.update(tx).unwrap();
        assert_eq!(state.text().as_str(), expected);
        assert_eq!(state.selection().main().head(), selection);
    }
}
