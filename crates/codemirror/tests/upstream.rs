//! Parity fixtures from the pinned CodeMirror packages in `../README.md`.

use std::ops::Range;

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, EditorSelection, EditorState, History, SelectionRange, Transaction,
};

fn endpoints(range: SelectionRange) -> (usize, usize) {
    (range.anchor(), range.head())
}

fn state_with_selection(source: &str, anchor: usize, head: usize) -> EditorState {
    let mut state = EditorState::new(source);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(anchor, head)))
        .unwrap();
    state
}

fn replace(range: Range<usize>, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, insert)).unwrap())
}

fn mapped(changes: &ChangeSet, pos: usize) -> (usize, usize) {
    (
        changes.map_pos(pos, Assoc::Before),
        changes.map_pos(pos, Assoc::After),
    )
}

#[test]
fn maps_adjacent_replacements_and_deletions() {
    let cases = [
        (
            vec![Change::new(1..3, "UVW"), Change::new(3..6, "q")],
            "aUVWqg",
            [
                (0, 0),
                (1, 1),
                (1, 4),
                (4, 4),
                (4, 5),
                (4, 5),
                (5, 5),
                (6, 6),
            ],
        ),
        (
            vec![Change::new(1..3, ""), Change::new(3..6, "")],
            "ag",
            [
                (0, 0),
                (1, 1),
                (1, 1),
                (1, 1),
                (1, 1),
                (1, 1),
                (1, 1),
                (2, 2),
            ],
        ),
        (
            vec![
                Change::new(1..1, "UV"),
                Change::new(1..4, "K"),
                Change::new(4..4, "*!"),
            ],
            "aUVK*!efg",
            [
                (0, 0),
                (1, 3),
                (3, 4),
                (3, 4),
                (4, 6),
                (7, 7),
                (8, 8),
                (9, 9),
            ],
        ),
    ];

    for (edits, output, positions) in cases {
        let mut history = History::new(EditorState::new("abcdefg"));
        let changes = ChangeSet::new(edits).unwrap();
        for (pos, expected) in positions.into_iter().enumerate() {
            assert_eq!(mapped(&changes, pos), expected, "{output}, position {pos}");
        }
        history
            .update(Transaction::new(changes), (), |_| (None, ()))
            .unwrap();
        assert_eq!(history.state().text().as_str(), output);
        assert_eq!(history.undo(), Some(()));
        assert_eq!(history.state().text().as_str(), "abcdefg");
        assert_eq!(history.redo(), Some(()));
        assert_eq!(history.state().text().as_str(), output);
    }
}

#[test]
fn maps_empty_and_nonempty_selections_at_insertions() {
    // An empty range stays before the insertion. Nonempty ranges exclude both
    // insertions at their edges, in either direction.
    for ((anchor, head), expected) in [((1, 1), (1, 1)), ((1, 4), (3, 6)), ((4, 1), (6, 3))] {
        let mut state = state_with_selection("abcdefg", anchor, head);
        let changes =
            ChangeSet::new(vec![Change::new(1..1, "UV"), Change::new(4..4, "XYZ")]).unwrap();
        state.update(Transaction::new(changes)).unwrap();
        assert_eq!(endpoints(state.selection().main()), expected);
    }
}

#[test]
fn sorts_ranges_and_keeps_the_main_range_when_merging() {
    let ranges = [(11, 8), (2, 5), (7, 4), (7, 8), (8, 8), (8, 8)];
    // Test a main range that merges into a reversed range and a main cursor
    // that merges into a neighboring range. Touching nonempty ranges stay separate.
    for (main, expected_main) in [(1, (7, 2)), (5, (8, 11))] {
        let selection = EditorSelection::new(
            ranges
                .into_iter()
                .map(|(anchor, head)| SelectionRange::new(anchor, head))
                .collect(),
            main,
        )
        .unwrap();
        assert_eq!(
            selection
                .ranges()
                .iter()
                .copied()
                .map(endpoints)
                .collect::<Vec<_>>(),
            [(7, 2), (7, 8), (8, 11)]
        );
        assert_eq!(endpoints(selection.main()), expected_main);
    }
}

#[test]
fn maps_unicode_selections_in_utf8_bytes() {
    for (selection, expected) in [(None, (12, 4)), (Some((11, 7)), (11, 7))] {
        // Upstream starts with (6, 3) in UTF-16. Its implicit result is (6, 2).
        let mut state = state_with_selection("A🦊字e\u{301}Z", 11, 5);
        let changes = ChangeSet::new(vec![
            Change::new(1..5, "猫"),
            Change::new(8..8, "🌱"),
            Change::new(9..11, ""),
        ])
        .unwrap();
        // At UTF-16 position 4, upstream maps to (3, 5) on each side of 🌱.
        assert_eq!(mapped(&changes, 8), (7, 11));
        let mut transaction = Transaction::new(changes);
        if let Some((anchor, head)) = selection {
            // The upstream explicit selection is (5, 3) in the new document.
            transaction = transaction.with_selection(EditorSelection::single(anchor, head));
        }
        state.update(transaction).unwrap();
        assert_eq!(state.text().as_str(), "A猫字🌱eZ");
        assert_eq!(endpoints(state.selection().main()), expected);
    }
}

#[test]
fn rejects_out_of_bounds_changes_and_post_edit_selections() {
    let mut state = EditorState::new("abcdefg");
    assert!(state.update(replace(2..8, "p")).is_err());
    assert_eq!(state.text().as_str(), "abcdefg");

    let transaction = replace(1..6, "").with_selection(EditorSelection::single(3, 3));
    assert!(state.update(transaction).is_err());
    assert_eq!(state.text().as_str(), "abcdefg");
    assert_eq!(endpoints(state.selection().main()), (0, 0));
}

#[test]
fn undoes_and_redoes_two_separate_edits() {
    // Upstream keeps these adjacent edits separate when they occur at 1000 ms
    // and 1500 ms. The Rust transactions are outside any explicit group.
    let mut history = History::new(state_with_selection("mango", 2, 2));
    for (pos, insert) in [(2, "X"), (3, "Y")] {
        let transaction =
            replace(pos..pos, insert).with_selection(EditorSelection::single(pos + 1, pos + 1));
        history.update(transaction, (), |_| (None, ())).unwrap();
    }
    assert_eq!(history.state().text().as_str(), "maXYngo");
    for (text, cursor) in [("maXngo", 3), ("mango", 2)] {
        assert_eq!(history.undo(), Some(()));
        assert_eq!(history.state().text().as_str(), text);
        assert_eq!(
            endpoints(history.state().selection().main()),
            (cursor, cursor)
        );
    }
    assert!(!history.can_undo());
    for (text, cursor) in [("maXngo", 3), ("maXYngo", 4)] {
        assert_eq!(history.redo(), Some(()));
        assert_eq!(history.state().text().as_str(), text);
        assert_eq!(
            endpoints(history.state().selection().main()),
            (cursor, cursor)
        );
    }
    assert!(!history.can_redo());
}

mod rust_contract {
    use super::*;
    use gpui_editor_codemirror::{EditError, Text};

    #[test]
    fn collapses_selections_strictly_inside_a_replacement() {
        // Pinned upstream maps these endpoints across one another. Rust
        // collapses them to the start of the replacement in either direction.
        for (anchor, head) in [(2, 3), (3, 2)] {
            let mut state = state_with_selection("abcdefg", anchor, head);
            state.update(replace(1..4, "XYZ")).unwrap();
            assert_eq!(endpoints(state.selection().main()), (1, 1));
        }
    }

    #[test]
    fn rejects_overlapping_edits_and_split_code_points() {
        assert!(matches!(
            ChangeSet::new(vec![Change::new(1..3, "X"), Change::new(2..4, "Y")]),
            Err(EditError::OverlappingChanges)
        ));

        let mut state = state_with_selection("a🦊z", 5, 1);
        assert_eq!(
            state.update(replace(2..3, "?")),
            Err(EditError::InvalidRange)
        );
        assert_eq!(state.text().as_str(), "a🦊z");
        assert_eq!(endpoints(state.selection().main()), (5, 1));
    }

    #[test]
    fn retains_mixed_line_endings_and_indexes_lines_by_byte() {
        let source = "字\r\n🦊\ne\u{301}\r\n";
        let text = Text::new(source);
        assert_eq!(text.as_str(), source);
        for (number, range, content) in [
            (1, 0..3, "字"),
            (2, 5..9, "🦊"),
            (3, 10..13, "e\u{301}"),
            (4, 15..15, ""),
        ] {
            let line = text.line(number).unwrap();
            assert_eq!(line.number, number);
            assert_eq!(line.text, content);
            assert_eq!(line.range, range);
        }
        for (byte, number) in [(3, 1), (4, 1), (5, 2), (9, 2), (15, 4)] {
            assert_eq!(text.line_at(byte).unwrap().number, number);
        }
        assert!(text.line_at(6).is_none());
        assert!(text.line(5).is_none());
        assert_eq!(Text::new("").line(1).unwrap().range, 0..0);
    }

    #[test]
    fn undoes_a_unicode_group_and_restores_selection_and_metadata() {
        let mut history = History::new(state_with_selection("az", 1, 1));
        history.start_group();
        let marked = history
            .update(replace(1..1, "字"), "initial", |state| {
                assert_eq!(state.text().as_str(), "a字z");
                assert_eq!(endpoints(state.selection().main()), (1, 1));
                (Some(EditorSelection::single(4, 4)), "marked")
            })
            .unwrap();
        assert_eq!(marked, "marked");
        let committed = history
            .update(replace(1..4, "漢🦊"), marked, |state| {
                assert_eq!(state.text().as_str(), "a漢🦊z");
                assert_eq!(endpoints(state.selection().main()), (8, 8));
                (Some(EditorSelection::single(8, 4)), "committed")
            })
            .unwrap();
        assert_eq!(committed, "committed");
        assert_eq!(endpoints(history.state().selection().main()), (8, 4));
        history.end_group();

        assert_eq!(history.undo(), Some("initial"));
        assert_eq!(history.state().text().as_str(), "az");
        assert_eq!(endpoints(history.state().selection().main()), (1, 1));
        assert_eq!(history.undo(), None);

        assert_eq!(history.redo(), Some("committed"));
        assert_eq!(history.state().text().as_str(), "a漢🦊z");
        assert_eq!(endpoints(history.state().selection().main()), (8, 4));
        assert_eq!(history.redo(), None);
    }

    #[test]
    fn preserves_history_on_selection_only_or_rejected_updates() {
        let mut history = History::new(EditorState::new("a0z"));
        history
            .set_selection(EditorSelection::single(2, 1))
            .unwrap();
        assert!(!history.can_undo());
        history
            .update(replace(1..2, "!"), (), |_| (None, ()))
            .unwrap();
        assert_eq!(history.undo(), Some(()));
        assert!(history.can_redo());
        history
            .update(
                Transaction::default().with_selection(EditorSelection::single(3, 2)),
                (),
                |_| (None, ()),
            )
            .unwrap();
        assert!(!history.can_undo());
        assert!(history.can_redo());

        let error = history.update(replace(1..2, "字"), (), |state| {
            assert_eq!(state.text().as_str(), "a字z");
            (Some(EditorSelection::single(2, 2)), ())
        });
        assert_eq!(error, Err(EditError::InvalidSelection));
        assert_eq!(history.state().text().as_str(), "a0z");
        assert_eq!(endpoints(history.state().selection().main()), (3, 2));
        assert!(!history.can_undo());
        assert!(history.can_redo());

        history
            .update(replace(1..2, "?"), (), |_| (None, ()))
            .unwrap();
        assert!(history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn keeps_the_latest_one_hundred_history_groups() {
        let mut history = History::new(EditorState::new(""));
        for pos in 0..103 {
            history
                .update(replace(pos..pos, "x"), (), |_| (None, ()))
                .unwrap();
        }
        for _ in 0..100 {
            assert_eq!(history.undo(), Some(()));
        }
        assert_eq!(history.undo(), None);
        assert_eq!(history.state().text().as_str(), "xxx");
        for _ in 0..100 {
            assert_eq!(history.redo(), Some(()));
        }
        assert_eq!(history.redo(), None);
        assert_eq!(history.state().text().as_str().len(), 103);
    }
}
