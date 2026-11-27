//! Headless fixtures checked against the SRI-verified @codemirror/merge 6.13.0.
//! Oracle positions are converted to UTF-8 bytes and chunk ends clipped to EOF.

use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Change, ChangePermission, ChangeSet, Compartment, ConfigurationError, EditError,
    EditorSelection, EditorState, Extension, History, Text, Transaction, change_filter,
    merge::{
        Side, UnifiedMergeConfig, accept_chunk, get_chunks, get_original_doc, go_to_next_chunk,
        go_to_previous_chunk, original_doc_change_effect, reject_chunk, unified_merge_state,
        unified_merge_view,
    },
};

fn changes(range: Range<usize>, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(range, insert)).unwrap()
}

fn extension(original: &str) -> Extension {
    unified_merge_view(UnifiedMergeConfig::new(Text::new(original)))
}

fn state(original: &str, editable: &str) -> EditorState {
    EditorState::with_extensions(editable, extension(original)).unwrap()
}

fn original(state: &EditorState) -> Text {
    get_original_doc(state).unwrap().unwrap()
}

fn assert_chunks(state: &EditorState, expected: &[(Range<usize>, Range<usize>)]) {
    let (chunks, side) = get_chunks(state).unwrap().unwrap();
    assert_eq!(side, Side::B);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| (chunk.range(Side::A), chunk.range(Side::B)))
            .collect::<Vec<_>>(),
        expected,
    );
}

#[test]
fn absent_and_empty_providers_are_distinct() {
    let plain = EditorState::new("unchanged");
    assert!(get_chunks(&plain).unwrap().is_none());
    assert!(get_original_doc(&plain).unwrap().is_none());
    assert!(accept_chunk(&plain, None).unwrap().is_none());
    assert!(go_to_next_chunk(&plain).unwrap().is_none());
    let state = state("unchanged", "unchanged");
    assert_chunks(&state, &[]);
    assert_eq!(original(&state).as_str(), "unchanged");
    assert!(reject_chunk(&state, None).unwrap().is_none());
    assert!(go_to_previous_chunk(&state).unwrap().is_none());
}

#[test]
fn chunks_follow_ordinary_edits_and_simultaneous_original_effects() {
    let a = "intro\nold 🍎\nseparate lengthy keep text\nlast";
    let b = "intro\nnew 🍋\nseparate lengthy keep text\nother\n";
    let mut state = state(a, b);
    assert_chunks(&state, &[(6..15, 6..15), (42..46, 42..48)]);
    let initial = unified_merge_state(&state).unwrap().unwrap();
    state
        .update(Transaction::new(changes(0..0, "header\n")))
        .unwrap();
    assert_eq!(original(&state).as_str(), a);
    assert_chunks(&state, &[(0..0, 0..7), (6..15, 13..22), (42..46, 49..55)]);

    let effect = original_doc_change_effect(&state, changes(0..0, "header\n")).unwrap();
    let end = state.text().len();
    state
        .update(Transaction::new(changes(end..end, "trailing")).with_effects([effect]))
        .unwrap();
    assert_eq!(original(&state).as_str(), format!("header\n{a}"));
    assert_eq!(state.text().as_str(), format!("header\n{b}trailing"));
    assert_chunks(&state, &[(13..22, 13..22), (49..53, 49..63)]);
    assert_eq!(initial.original().as_str(), a);
    assert_eq!(initial.chunks()[1].range(Side::B), 42..48);
}

#[test]
fn chunks_use_accepted_changes_after_filtering() {
    let mut state = EditorState::with_extensions(
        "head\nnew\ntail",
        Extension::group([
            extension("head\nold\ntail"),
            change_filter(|_, tr| {
                if tr.is_user_event("revert") {
                    ChangePermission::Reject
                } else {
                    ChangePermission::Protect(std::iter::once(5..8).collect())
                }
            }),
        ]),
    )
    .unwrap();
    let before = get_chunks(&state).unwrap().unwrap().0;
    let reject = reject_chunk(&state, Some(5)).unwrap().unwrap();
    state.update(reject).unwrap();
    assert!(Arc::ptr_eq(
        &before,
        &get_chunks(&state).unwrap().unwrap().0
    ));
    assert_eq!(state.text().as_str(), "head\nnew\ntail");

    let edit = ChangeSet::new(vec![
        Change::new(0..0, "header\n"),
        Change::new(5..8, "old"),
    ]);
    let effect = original_doc_change_effect(&state, changes(0..0, "header\n")).unwrap();
    state
        .update(Transaction::new(edit.unwrap()).with_effects([effect]))
        .unwrap();
    assert_eq!(original(&state).as_str(), "header\nhead\nold\ntail");
    assert_eq!(state.text().as_str(), "header\nhead\nnew\ntail");
    assert_chunks(&state, &[(12..16, 12..16)]);
}

#[test]
fn accept_and_reject_choose_the_chunk_under_the_position_or_cursor() {
    let original_text = "same\nligne 🍋\nseparator has content\nold";
    let edited_text = "same\nligne 🍊\nseparator has content\nnew";
    let mut state = state(original_text, edited_text);
    assert_chunks(&state, &[(5..16, 5..16), (38..41, 38..41)]);
    assert!(accept_chunk(&state, None).unwrap().is_none());
    assert!(reject_chunk(&state, Some(16)).unwrap().is_none());
    assert!(reject_chunk(&state, Some(13)).unwrap().is_none()); // Inside an emoji.
    state
        .update(Transaction::default().with_selection(EditorSelection::single(15, 15)))
        .unwrap();
    let mut accept_state = state.clone();
    let accept = accept_chunk(&accept_state, None).unwrap().unwrap();
    assert_eq!(accept.user_event(), Some("accept"));
    assert!(accept.changes().is_empty());
    accept_state.update(accept).unwrap();
    assert_eq!(accept_state.text().as_str(), edited_text);
    assert_eq!(accept_state.selection().main().head(), 15);
    assert_eq!(
        original(&accept_state).as_str(),
        "same\nligne 🍊\nseparator has content\nold"
    );
    assert_chunks(&accept_state, &[(38..41, 38..41)]);

    let reject = reject_chunk(&state, Some(5)).unwrap().unwrap();
    assert_eq!(reject.user_event(), Some("revert"));
    assert_eq!(reject.changes(), &changes(5..16, "ligne 🍋\n"));
    state.update(reject).unwrap();
    assert_eq!(original(&state).as_str(), original_text);
    assert_eq!(
        state.text().as_str(),
        "same\nligne 🍋\nseparator has content\nnew"
    );
    assert_chunks(&state, &[(38..41, 38..41)]);
}

#[test]
fn actions_handle_blank_lines_empty_sides_and_trailing_newlines() {
    for (a, b, at, after_chunk) in [
        ("a\nb", "a\n\nb", 2, Some(3)),
        ("a\n\nb", "a\nb", 2, Some(3)),
        ("", "🍊\n", 4, Some(5)),
        ("new", "", 0, None),
        ("same\n🍋", "same\n🍊\n", 10, None),
        // Native actions preserve CRLF rather than normalizing the documents.
        ("same\r\nold\r\ntail", "same\r\nnew\r\ntail", 9, Some(10)),
        // Unlike the upstream zero-width chunk here, the native chunk covers the edit.
        ("\na", "\n\na", 0, Some(1)),
    ] {
        let mut accept_state = state(a, b);
        let mut reject_state = accept_state.clone();
        if let Some(after) = after_chunk {
            assert!(
                accept_chunk(&accept_state, Some(after)).unwrap().is_none(),
                "{a:?} -> {b:?}"
            );
        }
        let accept = accept_chunk(&accept_state, Some(at))
            .unwrap()
            .unwrap_or_else(|| panic!("no chunk at {at}: {a:?} -> {b:?}"));
        accept_state.update(accept).unwrap();
        assert_eq!(original(&accept_state).as_str(), b);
        assert_chunks(&accept_state, &[]);

        let reject = reject_chunk(&reject_state, Some(at)).unwrap().unwrap();
        reject_state.update(reject).unwrap();
        assert_eq!(reject_state.text().as_str(), a);
        assert_chunks(&reject_state, &[]);
    }
}

#[test]
fn revert_is_undoable_but_accept_only_changes_the_baseline() {
    let mut history = History::new(state("old", "new"));
    let tx = reject_chunk(history.state(), None).unwrap().unwrap();
    history.update(tx, (), |_| (None, ())).unwrap();
    assert_chunks(history.state(), &[]);
    history.undo().unwrap();
    assert_chunks(history.state(), &[(0..3, 0..3)]);
    history.redo().unwrap();
    assert_chunks(history.state(), &[]);

    let mut history = History::new(state("old", "new"));
    let tx = accept_chunk(history.state(), None).unwrap().unwrap();
    history.update(tx, (), |_| (None, ())).unwrap();
    assert_eq!(original(history.state()).as_str(), "new");
    assert!(!history.can_undo());
    history
        .update(Transaction::new(changes(0..3, "old")), (), |_| (None, ()))
        .unwrap();
    assert_chunks(history.state(), &[(0..3, 0..3)]);
    history.undo().unwrap();
    assert_chunks(history.state(), &[]);
}

#[test]
fn navigation_wraps_and_compares_the_full_line_range() {
    let single = state("old", "new");
    assert!(go_to_next_chunk(&single).unwrap().is_none());
    assert!(go_to_previous_chunk(&single).unwrap().is_none());

    let mut state = state(
        "same\nold\nlong bridge\nvery strong separator\nold again\nend",
        "same\nnew\nlong bridge\nvery strong separator\nnew again\nend",
    );
    for (head, next, previous) in [
        (0, 5, 43),
        (5, 43, 43),
        (9, 43, 43),
        (10, 43, 5),
        (56, 5, 43),
    ] {
        state
            .update(Transaction::default().with_selection(EditorSelection::single(head, head)))
            .unwrap();
        let forward = go_to_next_chunk(&state).unwrap().unwrap();
        let backward = go_to_previous_chunk(&state).unwrap().unwrap();
        for (tr, expected) in [(forward, next), (backward, previous)] {
            assert!(tr.is_user_event("select.byChunk"));
            assert!(tr.scroll_into_view());
            assert!(tr.changes().is_empty());
            assert_eq!(tr.selection().unwrap().main().head(), expected);
        }
    }
}

#[test]
fn original_effects_are_validated_and_last_effect_wins() {
    let mut state = state("same\nold\ntail", "same\nnew\ntail");
    assert_eq!(
        original_doc_change_effect(&state, changes(0..14, "invalid")).unwrap_err(),
        EditError::InvalidRange,
    );
    let first = original_doc_change_effect(&state, changes(5..8, "one")).unwrap();
    let last = original_doc_change_effect(&state, changes(5..8, "two words")).unwrap();
    state
        .update(Transaction::default().with_effects([first.clone(), last]))
        .unwrap();
    assert_eq!(original(&state).as_str(), "same\ntwo words\ntail");
    assert_chunks(&state, &[(5..15, 5..9)]);

    let bad = Transaction::new(changes(0..4, "oops")).with_effects([first]);
    assert_eq!(
        state.update(bad),
        Err(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        )),
    );
    assert_eq!(state.text().as_str(), "same\nnew\ntail");
    assert_chunks(&state, &[(5..15, 5..9)]);
}

#[test]
fn last_original_effect_keeps_equal_documents_in_sync() {
    let mut state = state("starting", "current");
    let first = original_doc_change_effect(&state, changes(0..8, "ONE")).unwrap();
    let last = original_doc_change_effect(&state, changes(0..8, "current")).unwrap();
    state
        .update(Transaction::default().with_effects([first, last]))
        .unwrap();
    // The pinned oracle reports a chunk for ONE -> current even though it
    // reports current for both documents. Use the last effect for both.
    assert_eq!(&original(&state), state.text());
    assert_chunks(&state, &[]);
}

#[test]
fn reconfiguration_retains_the_same_extension_and_installs_a_new_baseline() {
    let compartment = Compartment::new();
    let view = extension("starting");
    let mut state = EditorState::with_extensions("current", compartment.of(view.clone())).unwrap();
    let accept = accept_chunk(&state, None).unwrap().unwrap();
    state.update(accept).unwrap();
    state
        .update(Transaction::default().with_effects([compartment.reconfigure(view)]))
        .unwrap();
    assert_eq!(original(&state).as_str(), "current");
    assert_chunks(&state, &[]);

    let replacement = compartment.reconfigure(extension("another"));
    state
        .update(Transaction::new(changes(0..7, "another")).with_effects([replacement]))
        .unwrap();
    assert_eq!(original(&state).as_str(), "another");
    assert_chunks(&state, &[]);

    state
        .update(
            Transaction::default().with_effects([compartment.reconfigure(Extension::default())]),
        )
        .unwrap();
    assert!(get_chunks(&state).unwrap().is_none());
}
