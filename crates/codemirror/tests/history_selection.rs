use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, EditorSelection, EditorState, History, HistoryIsolation,
    SelectionRange, Transaction, transaction_filter,
};

fn cursor(at: usize) -> EditorSelection {
    EditorSelection::single(at, at)
}

fn select(selection: EditorSelection, time: u64, event: &str) -> Transaction {
    Transaction::default()
        .with_selection(selection)
        .with_time(time)
        .with_user_event(event)
}

fn edit(from: usize, to: usize, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(from..to, insert)).unwrap())
        .isolate_history(HistoryIsolation::Full)
}

fn apply(history: &mut History<()>, transaction: Transaction) {
    history.update(transaction, (), |_| (None, ())).unwrap();
}

#[test]
fn groups_selection_moves_by_origin_shape_and_time_since_group_start() {
    let mut history = History::new(EditorState::new("abcde"));
    for (selection, time, event) in [
        (cursor(1), 1000, "select.keyboard"),
        (cursor(2), 1499, "select.keyboard"),
        (cursor(3), 1500, "select.keyboard"),
        (cursor(4), 1501, "select.pointer"),
        (EditorSelection::single(4, 5), 1502, "select.pointer"),
        (cursor(5), 1503, "select.pointer"),
    ] {
        apply(&mut history, select(selection, time, event));
    }
    assert!(!history.can_undo());
    assert!(history.can_undo_selection());
    assert!(!history.can_redo_selection());
    assert_eq!(history.undo(), None);
    for expected in [
        EditorSelection::single(4, 5),
        cursor(3),
        cursor(2),
        cursor(0),
    ] {
        assert_eq!(history.undo_selection(()), Some(()));
        assert_eq!(*history.state().selection(), expected);
    }
    assert_eq!(history.undo_selection(()), None);
    assert!(!history.can_undo_selection());
    for expected in [
        cursor(2),
        cursor(3),
        EditorSelection::single(4, 5),
        cursor(5),
    ] {
        assert_eq!(history.redo_selection(()), Some(()));
        assert_eq!(*history.state().selection(), expected);
    }
    assert_eq!(history.redo_selection(()), None);
}

#[test]
fn selection_shape_uses_prior_ranges_not_the_new_selection() {
    let mut history = History::new(EditorState::new("abcd"));
    let multiple = EditorSelection::new(
        vec![SelectionRange::new(1, 1), SelectionRange::new(3, 3)],
        1,
    )
    .unwrap();
    apply(&mut history, select(cursor(2), 1000, "select"));
    apply(&mut history, select(multiple.clone(), 1001, "select"));
    apply(&mut history, select(cursor(3), 1002, "select"));
    for expected in [multiple, cursor(0)] {
        assert_eq!(history.undo_selection(()), Some(()));
        assert_eq!(*history.state().selection(), expected);
    }
    assert_eq!(history.undo_selection(()), None);
}

#[test]
fn selection_groups_honor_isolation_and_unrecorded_transactions() {
    let mut history = History::new(EditorState::new("abcd"));
    history.set_selection(cursor(1)).unwrap();
    assert!(!history.can_undo_selection());
    apply(&mut history, select(cursor(2), 1000, "select"));
    apply(
        &mut history,
        select(cursor(3), 1001, "select").with_add_to_history(false),
    );
    apply(
        &mut history,
        select(cursor(4), 1002, "select").isolate_history(HistoryIsolation::Before),
    );
    for expected in [cursor(3), cursor(1)] {
        assert_eq!(history.undo_selection(()), Some(()));
        assert_eq!(*history.state().selection(), expected);
    }
    assert_eq!(history.undo_selection(()), None);
}

#[test]
fn plain_undo_discards_later_moves_but_selection_commands_can_fall_back_to_edits() {
    let mut history = History::new(EditorState::new("abc"));
    apply(&mut history, select(cursor(1), 1000, "select"));
    apply(&mut history, edit(1, 1, "X").with_selection(cursor(2)));
    apply(&mut history, select(cursor(4), 1001, "select"));
    apply(&mut history, select(cursor(3), 1002, "select.pointer"));
    assert_eq!(history.undo(), Some(()));
    assert_eq!(*history.state().selection(), cursor(1));
    assert_eq!(history.state().text().as_str(), "abc");
    assert!(!history.can_redo_selection());
    assert_eq!(history.redo_selection(()), Some(()));
    assert_eq!(*history.state().selection(), cursor(2));
    assert_eq!(history.state().text().as_str(), "aXbc");
    assert!(!history.can_undo_selection());
    assert_eq!(history.undo_selection(()), Some(()));
    assert_eq!(history.state().text().as_str(), "abc");
    assert_eq!(history.undo_selection(()), Some(()));
    assert_eq!(*history.state().selection(), cursor(0));
}

#[test]
fn selection_moves_leave_redo_intact_but_recorded_edits_clear_it() {
    let mut history = History::new(EditorState::new("abc"));
    apply(&mut history, select(cursor(1), 1000, "select"));
    assert_eq!(history.undo_selection(()), Some(()));
    assert!(history.can_redo_selection());
    apply(&mut history, select(cursor(2), 1001, "select"));
    assert!(history.can_redo_selection());
    apply(&mut history, edit(2, 2, "X"));
    assert!(!history.can_redo_selection());
}

#[test]
fn remote_mapping_keeps_selection_snapshots_even_when_all_edits_drop() {
    let mut history = History::new(EditorState::new("abc"));
    apply(&mut history, select(cursor(1), 1000, "select"));
    apply(&mut history, edit(1, 1, "X").with_selection(cursor(2)));
    apply(&mut history, select(cursor(3), 1002, "select"));
    apply(&mut history, edit(2, 2, "YZ").with_selection(cursor(4)));
    apply(&mut history, select(cursor(6), 1003, "select"));
    apply(&mut history, edit(1, 4, "").with_add_to_history(false));
    assert!(!history.can_undo());
    for expected in [1, 1, 0] {
        assert_eq!(history.undo_selection(()), Some(()));
        assert_eq!(*history.state().selection(), cursor(expected));
        assert_eq!(history.state().text().as_str(), "abc");
    }
    assert_eq!(history.undo_selection(()), None);
}

#[test]
fn maps_selection_snapshots_in_both_branches_through_utf8_insertions() {
    let mut history = History::new(EditorState::new("abc"));
    history.set_selection(cursor(1)).unwrap();
    apply(&mut history, select(cursor(2), 1000, "select"));
    apply(&mut history, select(cursor(3), 1500, "select"));
    assert_eq!(history.undo_selection(()), Some(()));
    apply(&mut history, edit(0, 0, "🦊").with_add_to_history(false));
    assert_eq!(history.undo_selection(()), Some(()));
    assert_eq!(*history.state().selection(), cursor(5));
    for expected in [6, 7] {
        assert_eq!(history.redo_selection(()), Some(()));
        assert_eq!(*history.state().selection(), cursor(expected));
    }
}

#[test]
fn selection_replay_uses_accepted_transactions_and_keeps_host_metadata() {
    let filter = transaction_filter(|_, tx| {
        if tx.is_user_event("select.undo") {
            assert!(tx.scroll_into_view());
            Ok(tx.with_selection(cursor(1)))
        } else if tx.is_user_event("select.redo") {
            Err(EditError::InvalidSelection)
        } else {
            Ok(tx)
        }
    });
    let mut history = History::new(EditorState::with_extensions("abc", filter).unwrap());
    history
        .update(select(cursor(3), 1000, "select"), "old", |_| (None, "move"))
        .unwrap();
    assert_eq!(history.undo_selection("current"), Some("current"));
    assert_eq!(*history.state().selection(), cursor(1));
    assert_eq!(history.redo_selection("current"), None);
    assert!(history.can_redo_selection());
}

#[test]
fn explicit_edit_groups_keep_the_final_selection_and_metadata_after_a_noop() {
    let mut history = History::new(EditorState::new("abc"));
    history.start_group();
    let insert = Transaction::new(ChangeSet::single(Change::new(0..0, "X")).unwrap());
    history
        .update(insert.with_selection(cursor(1)), "before", |_| {
            (None, "first")
        })
        .unwrap();
    history
        .update(select(cursor(2), 1000, "select"), "first", |_| {
            (None, "last")
        })
        .unwrap();
    history.end_group();
    assert!(!history.can_undo_selection());
    assert_eq!(history.undo_selection("current"), Some("before"));
    assert_eq!(history.state().text().as_str(), "abc");
    assert_eq!(history.redo_selection("current"), Some("last"));
    assert_eq!(*history.state().selection(), cursor(2));
}

#[test]
fn selection_snapshots_are_bounded_per_edit() {
    let mut history = History::new(EditorState::new("abc"));
    for index in 1..=250 {
        apply(
            &mut history,
            select(cursor(index % 3), index as u64, "other"),
        );
    }
    for _ in 0..201 {
        assert_eq!(history.undo_selection(()), Some(()));
    }
    assert!(!history.can_undo_selection());
}
