use std::{ops::Range, slice::from_ref};

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, History, SelectionRange, Transaction,
    TransactionCoordinates,
    autocomplete::{
        Snippet, SnippetSession, active_snippet, clear_snippet, next_snippet_field,
        previous_snippet_field, snippet_range_in_active_field, snippet_state_extension,
        start_snippet,
    },
};

fn editor(source: &str) -> EditorState {
    EditorState::with_extensions(source, snippet_state_extension()).unwrap()
}

fn insertion(state: &EditorState, template: &str, at: usize) -> Transaction {
    let (transaction, session) = Snippet::parse(template)
        .insert(state, at..at, "  ")
        .unwrap();
    transaction.with_effects([start_snippet(session.unwrap())])
}

fn edit(changes: Vec<Change>) -> Transaction {
    Transaction::new(ChangeSet::new(changes).unwrap())
}

fn assert_selection(state: &EditorState, expected: &[Range<usize>]) {
    let ranges: Vec<_> = state
        .selection()
        .ranges()
        .iter()
        .map(|range| range.range())
        .collect();
    assert_eq!(ranges, expected);
}

fn assert_fields(session: &SnippetSession, expected: &[&[Range<usize>]]) {
    let fields: Vec<_> = session.fields().iter().map(Vec::as_slice).collect();
    assert_eq!(fields, expected);
}

// The state and selection expectations use byte offsets converted from the
// scratch oracle for @codemirror/autocomplete 6.20.4.
#[test]
fn tracks_linked_fields_and_clears_after_selecting_the_last_field() {
    let mut state = editor(" pre ");
    state
        .update(insertion(&state, "${1:λ}+#{1:β}+${2:x}${0}", 1))
        .unwrap();
    assert_eq!(state.text().as_str(), " λ+β+xpre ");
    assert_selection(&state, &[1..3, 4..6]);
    let session = active_snippet(&state).unwrap();
    assert_eq!(session.active_field(), Some(0));
    assert!(snippet_range_in_active_field(&session, 1..1));
    assert!(snippet_range_in_active_field(&session, 6..6));
    assert!(!snippet_range_in_active_field(&session, 2..5));
    assert!(!snippet_range_in_active_field(&session, 7..8));
    assert!(previous_snippet_field(&state).is_none());

    state
        .update(state.replace_selection("é😀").unwrap())
        .unwrap();
    assert_eq!(state.text().as_str(), " é😀+é😀+xpre ");
    assert_selection(&state, &[7..7, 14..14]);
    assert_fields(
        &active_snippet(&state).unwrap(),
        &[&[1..7, 8..14], from_ref(&(15..16)), from_ref(&(16..16))],
    );

    state.update(next_snippet_field(&state).unwrap()).unwrap();
    assert_selection(&state, from_ref(&(15..16)));
    assert_eq!(active_snippet(&state).unwrap().active_field(), Some(1));

    state
        .update(previous_snippet_field(&state).unwrap())
        .unwrap();
    assert_selection(&state, &[1..7, 8..14]);
    assert_eq!(active_snippet(&state).unwrap().active_field(), Some(0));

    state.update(next_snippet_field(&state).unwrap()).unwrap();
    state.update(next_snippet_field(&state).unwrap()).unwrap();
    assert_selection(&state, from_ref(&(16..16)));
    assert!(active_snippet(&state).is_none());
    assert!(next_snippet_field(&state).is_none());
    assert!(previous_snippet_field(&state).is_none());
    assert!(clear_snippet(&state).is_none());
}

#[test]
fn maps_both_field_edges_and_cancels_when_any_field_edge_is_deleted() {
    let mut state = editor("");
    state
        .update(insertion(&state, "a${1:xy}b${2:zw}c${0}d", 0))
        .unwrap();
    state
        .update(edit(vec![
            Change::new(1..1, "é"),
            Change::new(3..3, "💡"),
            Change::new(8..8, "!"),
        ]))
        .unwrap();
    assert_eq!(state.text().as_str(), "aéxy💡bzwcd!");
    assert_selection(&state, from_ref(&(3..5)));
    assert_fields(
        &active_snippet(&state).unwrap(),
        &[from_ref(&(1..9)), from_ref(&(10..12)), from_ref(&(13..13))],
    );

    let mut changed_selection = state.clone();
    let selection = EditorSelection::new(
        vec![SelectionRange::new(1, 1), SelectionRange::new(13, 13)],
        0,
    )
    .unwrap();
    changed_selection
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    assert_selection(&changed_selection, &[1..1, 13..13]);
    assert!(active_snippet(&changed_selection).is_none());

    state.update(edit(vec![Change::new(9..11, "")])).unwrap();
    assert_eq!(state.text().as_str(), "aéxy💡wcd!");
    assert_selection(&state, from_ref(&(3..5)));
    assert!(active_snippet(&state).is_none());
}

#[test]
fn maps_start_effect_over_a_concurrent_edit_before_applying_it() {
    let mut state = editor("<> tail");
    let transaction = insertion(&state, "${1:é}/${2:文}${0}", 2)
        .merge(
            edit(vec![Change::new(0..0, "🧪")]).with_selection(EditorSelection::single(0, 0)),
            state.text().len(),
            TransactionCoordinates::Original,
        )
        .unwrap();
    state.update(transaction).unwrap();
    assert_eq!(state.text().as_str(), "🧪<>é/文 tail");
    assert_selection(&state, from_ref(&(0..0)));
    assert_fields(
        &active_snippet(&state).unwrap(),
        &[from_ref(&(6..8)), from_ref(&(9..12)), from_ref(&(12..12))],
    );

    let next = next_snippet_field(&state).unwrap();
    assert!(next.scroll_into_view());
    state.update(next).unwrap();
    assert_selection(&state, from_ref(&(9..12)));
    assert_eq!(active_snippet(&state).unwrap().active_field(), Some(1));

    let clear = clear_snippet(&state).unwrap();
    assert!(!clear.scroll_into_view());
    assert!(clear.selection().is_none());
    state.update(clear).unwrap();
    assert_selection(&state, from_ref(&(9..12)));
    assert!(active_snippet(&state).is_none());
}

#[test]
fn invalidated_start_effect_clears_the_previous_snippet() {
    let mut state = editor("-----");
    state.update(insertion(&state, "${1:a} ${2:b}", 0)).unwrap();
    let transaction = insertion(&state, "x${1:foo}:${2:bar}${0}z", 8)
        .merge(
            edit(vec![Change::new(8..10, "")]).with_selection(EditorSelection::single(0, 0)),
            state.text().len(),
            TransactionCoordinates::Sequential,
        )
        .unwrap();
    state.update(transaction).unwrap();
    assert_eq!(state.text().as_str(), "a b-----oo:barz");
    assert_selection(&state, from_ref(&(0..0)));
    // The old active field still contains the caret. Discarding the mapped
    // start effect instead of converting it to a clear would retain that field.
    assert!(active_snippet(&state).is_none());
}

#[test]
fn maps_navigation_effect_and_uses_the_first_snippet_effect() {
    let mut state = editor("");
    state
        .update(insertion(&state, "a${1:é}b${2:文}${0}", 0))
        .unwrap();
    let next = next_snippet_field(&state)
        .unwrap()
        .merge(
            edit(vec![Change::new(0..0, "😀")]),
            state.text().len(),
            TransactionCoordinates::Original,
        )
        .unwrap();
    state.update(next).unwrap();
    assert_eq!(state.text().as_str(), "😀aéb文");
    assert_selection(&state, from_ref(&(8..11)));
    let active = active_snippet(&state).unwrap();
    assert_eq!(active.active_field(), Some(1));
    assert_fields(
        &active,
        &[from_ref(&(5..7)), from_ref(&(8..11)), from_ref(&(11..11))],
    );

    let clear = clear_snippet(&state).unwrap();
    state
        .update(clear.with_effects([start_snippet((*active).clone())]))
        .unwrap();
    assert_selection(&state, from_ref(&(8..11)));
    assert!(active_snippet(&state).is_none());
}

#[test]
fn does_not_track_snippets_without_the_extension() {
    let mut state = EditorState::new("");
    state.update(insertion(&state, "${1:a}${0}", 0)).unwrap();
    assert_selection(&state, from_ref(&(0..1)));
    assert!(active_snippet(&state).is_none());
    assert!(next_snippet_field(&state).is_none());
    assert!(previous_snippet_field(&state).is_none());
    assert!(clear_snippet(&state).is_none());
}

#[test]
fn navigation_and_clear_do_not_make_undo_entries() {
    let state = editor("");
    let start = insertion(&state, "a${1:é}b${2:文}${0}", 0);
    let mut history = History::new(state);
    history.update(start, (), |_| (None, ())).unwrap();
    for command in [next_snippet_field, previous_snippet_field, clear_snippet] {
        let transaction = command(history.state()).unwrap();
        assert!(!transaction.add_to_history());
        history.update(transaction, (), |_| (None, ())).unwrap();
        assert!(!history.can_undo_selection());
    }
    assert!(active_snippet(history.state()).is_none());
    assert_eq!(history.undo(), Some(()));
    assert_eq!(history.state().text().as_str(), "");
    assert!(!history.can_undo());
    assert_eq!(history.redo(), Some(()));
    assert_eq!(history.state().text().as_str(), "aéb文");
    assert!(active_snippet(history.state()).is_none());
}
