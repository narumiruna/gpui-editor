use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, History, HistoryIsolation, Transaction,
    transaction_filter,
};

type TextHistory = History<()>;

fn insert(position: usize, text: &str, time: u64) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(position..position, text)).unwrap())
        .with_selection(EditorSelection::single(
            position + text.len(),
            position + text.len(),
        ))
        .with_user_event("input.type")
        .with_time(time)
}

fn apply(history: &mut TextHistory, transaction: Transaction) {
    history.update(transaction, (), |_| (None, ())).unwrap();
}

fn source(history: &TextHistory) -> &str {
    history.state().text().as_str()
}

#[test]
fn timed_history_groups_adjacent_edits_before_the_500_ms_boundary() {
    for (delay, remaining) in [(499, ""), (500, "a")] {
        let mut history = History::new(EditorState::new(""));
        apply(&mut history, insert(0, "a", 1000));
        apply(&mut history, insert(1, "字", 1000 + delay));
        assert_eq!(source(&history), "a字");
        assert_eq!(history.undo(), Some(()));
        assert_eq!(source(&history), remaining);
        assert_eq!(history.redo(), Some(()));
        assert_eq!(source(&history), "a字");
    }
}

#[test]
fn nonadjacent_edits_and_selection_transactions_break_timed_groups() {
    let mut prefix = History::new(EditorState::new("abcd"));
    apply(&mut prefix, insert(1, "X", 1000));
    apply(&mut prefix, insert(5, "Y", 1001));
    assert_eq!(prefix.undo(), Some(()));
    assert_eq!(source(&prefix), "aXbcd");

    let mut selected = History::new(EditorState::new(""));
    apply(&mut selected, insert(0, "X", 1000));
    apply(
        &mut selected,
        Transaction::default()
            .with_selection(EditorSelection::single(0, 0))
            .with_time(1001),
    );
    apply(&mut selected, insert(1, "Y", 1002));
    assert_eq!(selected.undo(), Some(()));
    assert_eq!(source(&selected), "X");
}

#[test]
fn isolation_breaks_only_the_requested_history_boundaries() {
    for (isolation, after_one, after_two) in [
        (HistoryIsolation::Before, "a", ""),
        (HistoryIsolation::After, "ab", ""),
        (HistoryIsolation::Full, "ab", "a"),
    ] {
        let mut history = History::new(EditorState::new(""));
        apply(&mut history, insert(0, "a", 1000));
        apply(
            &mut history,
            insert(1, "b", 1001).isolate_history(isolation),
        );
        apply(&mut history, insert(2, "c", 1002));
        assert_eq!(history.undo(), Some(()));
        assert_eq!(source(&history), after_one);
        assert_eq!(history.undo(), Some(()));
        assert_eq!(source(&history), after_two);
    }
}

#[test]
fn composition_updates_join_but_a_new_composition_starts_its_own_step() {
    for (event, remaining) in [
        ("input.type.compose", ""),
        ("input.type.compose.start", "字"),
    ] {
        let mut history = History::new(EditorState::new(""));
        apply(&mut history, insert(0, "字", 1000));
        apply(
            &mut history,
            Transaction::new(ChangeSet::single(Change::new(3..3, "🌿")).unwrap())
                .with_time(3000)
                .with_user_event(event),
        );
        assert_eq!(history.undo(), Some(()));
        assert_eq!(source(&history), remaining);
    }
}

#[test]
fn grouping_uses_accepted_transaction_metadata_and_explicit_groups_stay_separate() {
    let mut history = History::new(EditorState::new(""));
    history.start_group();
    apply(&mut history, insert(0, "A", 1000));
    apply(&mut history, insert(1, "B", 1001));
    history.end_group();
    apply(&mut history, insert(2, "C", 1002));
    assert_eq!(history.undo(), Some(()));
    assert_eq!(source(&history), "AB");
    assert_eq!(history.undo(), Some(()));
    assert!(source(&history).is_empty());

    let config = transaction_filter(|_, tx| Ok(tx.with_time(1000).with_user_event("input.type")));
    let mut filtered = History::new(EditorState::with_extensions("", config).unwrap());
    for (pos, text) in [(0, "A"), (1, "B")] {
        apply(
            &mut filtered,
            Transaction::new(ChangeSet::single(Change::new(pos..pos, text)).unwrap()),
        );
    }
    assert_eq!(filtered.undo(), Some(()));
    assert!(source(&filtered).is_empty());
}

#[test]
fn mapped_history_still_groups_adjacent_typing_across_an_unrecorded_edit() {
    let mut history = History::new(EditorState::new(""));
    apply(&mut history, insert(0, "a", 1000));
    apply(
        &mut history,
        insert(1, "b", 9000).with_add_to_history(false),
    );
    apply(&mut history, insert(1, "c", 1002));
    assert_eq!(source(&history), "acb");
    assert_eq!(history.undo(), Some(()));
    assert_eq!(source(&history), "b");
    assert_eq!(history.undo(), None);
    assert_eq!(history.redo(), Some(()));
    assert_eq!(source(&history), "acb");
}

#[test]
fn unrecorded_isolation_before_or_full_breaks_timed_groups_but_after_does_not() {
    for (isolation, after_undo) in [
        (HistoryIsolation::Before, "ab"),
        (HistoryIsolation::After, "b"),
        (HistoryIsolation::Full, "ab"),
    ] {
        let mut history = History::new(EditorState::new(""));
        apply(&mut history, insert(0, "a", 1000));
        apply(
            &mut history,
            insert(1, "b", 1001)
                .with_add_to_history(false)
                .isolate_history(isolation),
        );
        apply(&mut history, insert(1, "c", 1002));
        assert_eq!(history.undo(), Some(()));
        assert_eq!(source(&history), after_undo);
    }
}
