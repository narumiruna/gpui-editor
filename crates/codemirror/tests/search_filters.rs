//! Pinned oracle: actual @codemirror/search 6.7.3 and @codemirror/state 6.7.6
//! in Node, with SHA-512 matching the manifest and CRLF configured. The
//! expected match and callback offsets are UTF-8 bytes of the original slices.

use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    CharCategory, EditorSelection, EditorState, Extension, Text, Transaction,
    commands::{cursor_group_forward, delete_group_forward},
    search::{
        self, CursorOptions, HighlightSelectionOptions, SearchCursor, SearchOptions, SearchQuery,
        highlight_selection_matches, select_next_occurrence, word_characters,
    },
    view::Viewport,
};

const DOC: &str = "😀 foo-bar foo foo-bar\r\nFoo";

fn at(state: &mut EditorState, head: usize) {
    state
        .update(Transaction::default().with_selection(EditorSelection::single(head, head)))
        .unwrap();
}

#[test]
fn custom_tests_follow_whole_word_checks_and_see_the_editor_snapshot() {
    let extensions = Extension::group([
        word_characters(|_, pos| (pos < 12).then(|| Arc::from("-"))),
        word_characters(|_, _| Some(Arc::from(""))),
    ]);
    let mut state = EditorState::with_extensions(DOC, extensions).unwrap();
    for (regexp, head, expected, tested) in [
        (false, 7, &[[26, 29]][..], &[[13, 16], [26, 29]][..]),
        (true, 7, &[[26, 29]][..], &[[13, 16], [26, 29]][..]),
        (
            true,
            13,
            &[[5, 8], [26, 29]][..],
            &[[5, 8], [13, 16], [17, 20], [26, 29]][..],
        ),
    ] {
        at(&mut state, head);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&calls);
        let options = SearchOptions {
            regexp,
            whole_word: true,
            ..Default::default()
        };
        let query = SearchQuery::new(if regexp { "(foo)" } else { "foo" }, options)
            .unwrap()
            .with_test(move |found, state| {
                assert_eq!(state.text().as_str(), DOC);
                assert_eq!(state.selection().main().head(), head);
                if regexp {
                    assert_eq!(found.captures[1], Some(found.text));
                }
                log.lock()
                    .unwrap()
                    .push([found.range.start, found.range.end]);
                found.text == "Foo" || found.range.start < 10
            });
        let bound = query.in_state(&state);
        let positions: Vec<_> = bound
            .cursor(0..DOC.len())
            .unwrap()
            .map(|found| [found.range.start, found.range.end])
            .collect();
        assert_eq!(positions, expected);
        assert_eq!(*calls.lock().unwrap(), tested);
        assert_eq!(
            bound.find_next(0..0).unwrap().unwrap().range.start,
            positions[0][0]
        );
        assert_eq!(
            bound
                .find_previous(DOC.len()..DOC.len())
                .unwrap()
                .unwrap()
                .range,
            26..29
        );
        let changes = bound.replace_all("X").unwrap();
        assert_eq!(
            changes
                .changes()
                .map(|(range, _)| [range.start, range.end])
                .collect::<Vec<_>>(),
            expected
        );
        let selected = query.select_matches(&state).unwrap();
        assert_eq!(
            selected
                .selection()
                .unwrap()
                .ranges()
                .iter()
                .map(|range| [range.range().start, range.range().end])
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn occurrence_search_asks_for_word_characters_at_each_candidate() {
    for scoped in [true, false] {
        let extension =
            word_characters(move |_, pos| (!scoped || pos <= 12).then(|| Arc::from("-")));
        let mut state = EditorState::with_extensions(DOC, extension).unwrap();
        at(&mut state, 7);
        state
            .update(select_next_occurrence(&state).unwrap())
            .unwrap();
        assert_eq!(state.selection().main().range(), 5..12);
        let next = select_next_occurrence(&state);
        if scoped {
            assert!(next.is_none());
        } else {
            state.update(next.unwrap()).unwrap();
            assert_eq!(state.selection().ranges()[1].range(), 17..24);
        }
    }
}

#[test]
fn character_categories_drive_word_motion_deletion_and_marks() {
    let source = "😀 alpha-beta alpha alpha-beta";
    let mut state =
        EditorState::with_extensions(source, word_characters(|_, _| Some(Arc::from("- "))))
            .unwrap();
    let at = source.find("alpha").unwrap();
    let end = at + "alpha-beta".len();
    let other = source.rfind("alpha-beta").unwrap();
    let cat = state.char_categorizer(at);
    assert_eq!(cat.category("-"), CharCategory::Word);
    assert_eq!(cat.category(" \u{feff}"), CharCategory::Space);
    assert_eq!(cat.category("😀"), CharCategory::Other);
    assert_eq!(state.char_categorizer(1).category("-"), CharCategory::Other);
    assert_eq!(state.word_range_at(1), None);
    let start = EditorSelection::single(at, at);
    state
        .update(Transaction::default().with_selection(start))
        .unwrap();
    assert_eq!(state.word_range_at(at + "alpha-".len()), Some(at..end));
    let motion = cursor_group_forward(&state).unwrap().unwrap();
    assert_eq!(motion.selection().unwrap().main().head(), end);
    let erase = delete_group_forward(&state).unwrap().unwrap();
    assert_eq!(erase.changes().changes().next().unwrap().0, &(at..end));
    let whole = Viewport::complete(state.text());
    let marks = highlight_selection_matches(
        &state,
        &whole,
        HighlightSelectionOptions {
            highlight_word_around_cursor: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        marks
            .iter(0)
            .map(|mark| mark.from..mark.to)
            .collect::<Vec<_>>(),
        [at..end, other..source.len()]
    );
}

#[test]
fn query_equality_uses_callback_identity_and_ignores_the_literal_flag() {
    let options = SearchOptions::default();
    let first = SearchQuery::from_input("\\n", options);
    let literal = SearchQuery::from_input(
        "\\n",
        SearchOptions {
            literal: true,
            ..options
        },
    );
    assert_eq!(first, literal);
    assert_ne!(first, literal.with_replacement("text"));
    let is_later = |found: &gpui_editor_codemirror::search::SearchMatch<'_>, _: &EditorState| {
        found.range.start != 0
    };
    let query = first.clone().with_test(is_later);
    assert_eq!(query, query.clone());
    assert_ne!(query, first.with_test(is_later));
}

#[test]
fn rejecting_a_literal_candidate_does_not_hide_a_later_overlap() {
    let text = Text::new("aaa");
    let query = SearchQuery::new("aa", SearchOptions::default())
        .unwrap()
        .with_test(|found, state| {
            assert_eq!(state.text().as_str(), "aaa");
            assert_eq!(state.selection().main().head(), 0);
            found.range.start > 0
        });
    let mut matches = query.cursor(&text, 0..text.len()).unwrap();
    assert_eq!(matches.next().unwrap().range, 1..3);
    assert!(matches.next().is_none());
}

#[test]
fn literal_whole_word_uses_the_range_clipped_line_fragment() {
    let two_joiners = " a\u{200d}\u{200d}b a";
    for (doc, from, to, whole_word, expected) in [
        ("a\u{200d}b a", 0, None, true, &[[6, 7]][..]),
        ("a\u{200d}b a", 0, None, false, &[[0, 1], [6, 7]]),
        ("a\u{200d}! a", 0, None, true, &[[0, 1], [6, 7]]),
        ("ab a", 0, None, true, &[[3, 4]]),
        ("a\u{200d}\u{200d}b a", 0, None, true, &[[0, 1], [9, 10]]),
        (two_joiners, 0, None, true, &[[10, 11]]),
        (two_joiners, 1, None, true, &[[1, 2], [10, 11]]),
        (two_joiners, 0, Some(2), true, &[[1, 2]]),
        (
            "b\na\u{200d}\u{200d}b a",
            0,
            None,
            true,
            &[[2, 3], [11, 12]],
        ),
        ("a\u{200d}\nb a", 0, None, true, &[[0, 1], [7, 8]]),
        ("a\u{200d}\r\nb a", 0, None, true, &[[0, 1], [8, 9]]),
        ("a\u{345}a a", 0, None, true, &[[5, 6]]),
        ("b\u{200d}a a\u{200d}b a", 0, None, true, &[[12, 13]]),
    ] {
        let text = Text::new(doc);
        let options = CursorOptions {
            whole_word,
            ..Default::default()
        };
        let found: Vec<_> =
            SearchCursor::in_range(&text, "a", from..to.unwrap_or(doc.len()), options)
                .unwrap()
                .map(|found| [found.range.start, found.range.end])
                .collect();
        assert_eq!(found, expected, "{doc:?} at {from}..{to:?}");
    }
}

#[test]
fn literal_whole_word_counts_utf16_units_and_keeps_native_ranges() {
    let query = SearchQuery::new("a", whole_word()).unwrap();
    for (doc, from, extra, expected) in [
        ("a\u{200d}𐐀 a", 0, "", &[[0, 1], [9, 10]][..]),
        ("a\u{200d}𐐀 a", 0, "𐐁", &[[9, 10]]),
        ("a\u{200d}𐐀 a", 0, "\0", &[[0, 1], [9, 10]]),
        (" a\u{200d}𐐀 a", 0, "", &[[10, 11]]),
        ("𐐀\u{200d}a a", 7, "", &[[7, 8], [9, 10]]),
        ("𐐀\u{200d}a a", 7, "🐀", &[[9, 10]]),
        ("𐐀a a", 4, "", &[[6, 7]]),
        ("a\r\n\u{200d}b a", 0, "\u{200d}", &[[8, 9]]),
    ] {
        let state =
            EditorState::with_extensions(doc, word_characters(move |_, _| Some(Arc::from(extra))))
                .unwrap();
        let found: Vec<_> = query
            .in_state(&state)
            .cursor(from..doc.len())
            .unwrap()
            .map(|found| {
                assert_eq!(found.text, "a");
                assert!(found.precise);
                [found.range.start, found.range.end]
            })
            .collect();
        assert_eq!(found, expected, "{doc:?} with {extra:?}");
    }
}

#[test]
fn literal_whole_word_classifies_clusters_at_the_main_selection_head() {
    let doc = "a\u{200d}! a\u{200d}! a";
    let mut state = EditorState::with_extensions(
        doc,
        word_characters(|_, at| (at == 0).then(|| Arc::from("!"))),
    )
    .unwrap();
    let query = SearchQuery::new("a", whole_word()).unwrap();
    for (anchor, head, expected) in [(6, 0, &[[12, 13]][..]), (0, 6, &[[6, 7], [12, 13]])] {
        state
            .update(Transaction::default().with_selection(EditorSelection::single(anchor, head)))
            .unwrap();
        let found: Vec<_> = query
            .in_state(&state)
            .cursor(6..doc.len())
            .unwrap()
            .map(|found| [found.range.start, found.range.end])
            .collect();
        assert_eq!(found, expected);
    }
}

#[test]
fn literal_find_next_checks_whole_word_before_the_predicate_and_on_wrap() {
    for (doc, head, expected) in [("a\u{200d}b a", 0, 6..7), (" a\u{200d}\u{200d}b", 2, 1..2)] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&calls);
        let query = SearchQuery::new("a", whole_word())
            .unwrap()
            .with_test(move |found, _| {
                log.lock().unwrap().push(found.range.clone());
                true
            });
        let state = search_state(doc, query, head);
        let tx = search::find_next(&state).unwrap();
        assert_eq!(tx.selection().unwrap().main().range(), expected);
        assert_eq!(*calls.lock().unwrap(), [expected]);
        assert!(tx.changes().is_empty());
        assert_eq!(tx.user_event(), Some("select.search"));
    }
}

#[test]
fn literal_whole_word_replaces_only_precise_matches() {
    let query = SearchQuery::new("f", whole_word())
        .unwrap()
        .with_replacement("X");
    let mut state = search_state("ﬃ f", query, 0);
    let all = search::replace_all(&state).unwrap();
    assert_eq!(
        all.changes().changes().collect::<Vec<_>>(),
        [(&(4..5), "X")]
    );

    let first = search::find_next(&state).unwrap();
    assert_eq!(first.selection().unwrap().main().range(), 0..3);
    state.update(first).unwrap();
    let skip = search::replace_next(&state).unwrap();
    assert!(skip.changes().is_empty());
    assert_eq!(skip.selection().unwrap().main().range(), 4..5);
    state.update(skip).unwrap();
    let replace = search::replace_next(&state).unwrap();
    state.update(replace).unwrap();
    assert_eq!(state.text().as_str(), "ﬃ X");
}

fn whole_word() -> SearchOptions {
    SearchOptions {
        case_sensitive: true,
        whole_word: true,
        ..Default::default()
    }
}

fn search_state(doc: &str, query: SearchQuery, at: usize) -> EditorState {
    let mut state =
        EditorState::with_extensions(doc, search::extension(SearchOptions::default())).unwrap();
    state
        .update(
            Transaction::default()
                .with_selection(EditorSelection::single(at, at))
                .with_effects([search::set_search_query(query)]),
        )
        .unwrap();
    state
}
