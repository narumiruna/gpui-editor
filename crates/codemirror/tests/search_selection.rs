//! Expected selections and marks checked with temporary Node fixtures against
//! @codemirror/search 6.7.3, state 6.7.6, and view 6.44.0. UTF-16 positions
//! from the reference are converted to byte positions.

use std::ops::Range;

use gpui_editor_codemirror::{
    EditError, EditorSelection, EditorState, READ_ONLY, SelectionRange, Transaction,
    search::{
        HighlightSelectionOptions, SCROLL_TO_OCCURRENCE, SearchOptions, SearchQuery, extension,
        highlight_selection_matches, select_matches, select_next_occurrence,
        select_selection_matches, set_search_query,
    },
    view::{SCROLL_INTO_VIEW, ScrollTarget, Viewport},
};

fn editor(source: &str, ranges: &[(usize, usize)], main: usize) -> EditorState {
    let mut editor = EditorState::with_extensions(source, READ_ONLY.of(true)).unwrap();
    let ranges = ranges
        .iter()
        .map(|&(anchor, head)| SelectionRange::new(anchor, head))
        .collect();
    let selection = EditorSelection::new(ranges, main).unwrap();
    editor
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    editor
}

fn assert_selection(editor: &EditorState, ranges: &[(usize, usize)], main: usize) {
    let selection = editor.selection();
    let actual: Vec<_> = selection
        .ranges()
        .iter()
        .map(|r| (r.anchor(), r.head()))
        .collect();
    assert_eq!(actual, ranges);
    assert_eq!(selection.main_index(), main);
}

fn apply_selection(editor: &mut EditorState, transaction: Transaction, event: Option<&str>) {
    assert!(editor.read_only());
    assert!(transaction.changes().is_empty());
    assert_eq!(transaction.user_event(), event);
    let before = editor.text().clone();
    editor.update(transaction).unwrap();
    assert_eq!(editor.text(), &before);
}

fn scroll_target(transaction: &Transaction) -> Option<usize> {
    let legacy = transaction
        .effects()
        .iter()
        .find_map(|effect| effect.get(&SCROLL_TO_OCCURRENCE).copied());
    let target = transaction
        .effects()
        .iter()
        .find_map(|effect| effect.get(&SCROLL_INTO_VIEW).copied());
    assert_eq!(target, legacy.map(ScrollTarget::at));
    legacy
}

#[test]
fn select_query_matches_selects_full_regex_groups_and_marks_the_first_primary() {
    let mut editor = EditorState::with_extensions(
        "🦀 Δ:1 β:23",
        gpui_editor_codemirror::Extension::group([
            extension(SearchOptions::default()),
            READ_ONLY.of(true),
        ]),
    )
    .unwrap();
    assert!(select_matches(&editor).is_none());
    let query = SearchQuery::new(
        r"(\p{Letter}+):(\d+)",
        SearchOptions {
            regexp: true,
            ..SearchOptions::default()
        },
    )
    .unwrap();
    editor
        .update(Transaction::default().with_effects([set_search_query(query)]))
        .unwrap();
    let transaction = select_matches(&editor).unwrap();
    apply_selection(&mut editor, transaction, Some("select.search.matches"));
    assert_selection(&editor, &[(5, 9), (10, 15)], 0);
}

#[test]
fn select_query_matches_includes_empty_and_imprecise_matches() {
    let options = SearchOptions {
        regexp: true,
        ..SearchOptions::default()
    };
    let mut state = editor("🦀\nx", &[(0, 0)], 0);
    let transaction = SearchQuery::new("^", options)
        .unwrap()
        .select_matches(&state)
        .unwrap();
    apply_selection(&mut state, transaction, Some("select.search.matches"));
    assert_selection(&state, &[(0, 0), (5, 5)], 0);

    let mut state = editor("🦀 ﬃ F f", &[(0, 0)], 0);
    let transaction = SearchQuery::new("f", SearchOptions::default())
        .unwrap()
        .select_matches(&state)
        .unwrap();
    apply_selection(&mut state, transaction, Some("select.search.matches"));
    assert_selection(&state, &[(5, 8), (9, 10), (11, 12)], 0);
    assert!(
        SearchQuery::new("not found", SearchOptions::default())
            .unwrap()
            .select_matches(&state)
            .is_none()
    );
}

#[test]
fn query_and_selection_commands_enforce_their_distinct_limits() {
    let query = SearchQuery::new("x", SearchOptions::default()).unwrap();
    for (count, query_limit, selection_limit) in [
        (1000, true, true),
        (1001, false, true),
        (1002, false, false),
    ] {
        let state = editor(&"x ".repeat(count), &[(0, 1)], 0);
        let selections = [
            query.select_matches(&state),
            select_selection_matches(&state),
        ];
        for (transaction, within_limit) in
            selections.into_iter().zip([query_limit, selection_limit])
        {
            assert_eq!(transaction.is_some(), within_limit);
            if let Some(transaction) = transaction {
                assert_eq!(transaction.selection().unwrap().ranges().len(), count);
            }
        }
    }
}

#[test]
fn select_selection_matches_keeps_original_start_as_primary_but_uses_forward_ranges() {
    for (source, selected, expected, main) in [
        (
            "🦀 é e\u{301} É é",
            (17, 15),
            &[(5, 7), (8, 11), (15, 17)][..],
            2,
        ),
        ("a\t a\\t a\\t", (10, 7), &[(3, 6), (7, 10)][..], 1),
        ("aaa", (3, 1), &[(0, 2)][..], 0),
    ] {
        let mut state = editor(source, &[selected], 0);
        let transaction = select_selection_matches(&state).unwrap();
        apply_selection(&mut state, transaction, Some("select.search.matches"));
        assert_selection(&state, expected, main);
    }
    assert!(select_selection_matches(&editor("a a a", &[(0, 1), (2, 3)], 1)).is_none());
    assert!(select_selection_matches(&editor("a a a", &[(0, 0)], 0)).is_none());
}

#[test]
fn next_occurrence_wraps_skips_substrings_for_whole_words_and_keeps_primary_direction() {
    let mut state = editor("🦀 cat scatter cat cat", &[(20, 17)], 0);
    let transaction = select_next_occurrence(&state).unwrap();
    assert_eq!(scroll_target(&transaction), Some(24));
    apply_selection(&mut state, transaction, None);
    assert_selection(&state, &[(20, 17), (21, 24)], 0);

    let transaction = select_next_occurrence(&state).unwrap();
    assert_eq!(scroll_target(&transaction), Some(8));
    apply_selection(&mut state, transaction, None);
    assert_selection(&state, &[(5, 8), (20, 17), (21, 24)], 1);
    assert!(select_next_occurrence(&state).is_none());
}

#[test]
fn next_occurrence_starts_after_last_range_and_can_match_partial_words() {
    let mut state = editor("a a a a", &[(1, 0), (4, 5)], 0);
    for (expected, scroll) in [
        (&[(1, 0), (4, 5), (6, 7)][..], 7),
        (&[(1, 0), (2, 3), (4, 5), (6, 7)][..], 3),
    ] {
        let transaction = select_next_occurrence(&state).unwrap();
        assert_eq!(scroll_target(&transaction), Some(scroll));
        apply_selection(&mut state, transaction, None);
        assert_selection(&state, expected, 0);
    }
    assert!(select_next_occurrence(&state).is_none());

    let mut state = editor("🦀 é xé é É _é é_", &[(11, 9)], 0);
    for scroll in [14, 21, 24, 7] {
        let transaction = select_next_occurrence(&state).unwrap();
        assert_eq!(scroll_target(&transaction), Some(scroll));
        apply_selection(&mut state, transaction, None);
    }
    assert_selection(&state, &[(5, 7), (11, 9), (12, 14), (19, 21), (22, 24)], 1);
    assert!(select_next_occurrence(&editor("Foo foo Foo foo", &[(0, 3), (7, 4)], 1)).is_none());
}

#[test]
fn next_occurrence_expands_all_heads_if_any_range_is_empty() {
    let mut state = editor("say βeta and 🦀 = βeta", &[(4, 0), (21, 21)], 1);
    let transaction = select_next_occurrence(&state).unwrap();
    assert_eq!(scroll_target(&transaction), None);
    apply_selection(&mut state, transaction, None);
    assert_selection(&state, &[(0, 3), (21, 26)], 1);
    assert!(select_next_occurrence(&state).is_none());

    let mut state = editor("a café a", &[(3, 3), (7, 7), (9, 9)], 1);
    let transaction = select_next_occurrence(&state).unwrap();
    apply_selection(&mut state, transaction, None);
    assert_selection(&state, &[(2, 7), (8, 9)], 0);
    assert!(select_next_occurrence(&editor("🦀 +", &[(0, 0)], 0)).is_none());
}

#[test]
fn next_occurrence_stops_wrapped_search_one_utf16_unit_before_the_last_range() {
    assert!(select_next_occurrence(&editor("aaaa", &[(4, 2)], 0)).is_none());
    for (source, last) in [("aaxaa", (5, 3)), ("aa🦀aa", (8, 6))] {
        let mut state = editor(source, &[last], 0);
        let transaction = select_next_occurrence(&state).unwrap();
        assert_eq!(scroll_target(&transaction), Some(2));
        apply_selection(&mut state, transaction, None);
        assert_selection(&state, &[(0, 2), last], 1);
        assert!(select_next_occurrence(&state).is_none());
    }
}

#[test]
fn wrapped_occurrence_can_merge_with_an_earlier_reversed_range() {
    let mut state = editor("aaa _aa", &[(3, 1), (5, 7)], 0);
    let transaction = select_next_occurrence(&state).unwrap();
    assert_eq!(scroll_target(&transaction), Some(2));
    apply_selection(&mut state, transaction, None);
    assert_selection(&state, &[(3, 0), (5, 7)], 0);
    assert!(select_next_occurrence(&state).is_none());
}

#[test]
fn next_occurrence_uses_word_clusters_and_keeps_crlf_bytes() {
    let mut state = editor("e\u{301}\r\ne\u{301} xe\u{301}", &[(3, 3)], 0);
    let transaction = select_next_occurrence(&state).unwrap();
    apply_selection(&mut state, transaction, None);
    assert_selection(&state, &[(0, 3)], 0);
    let transaction = select_next_occurrence(&state).unwrap();
    assert_eq!(scroll_target(&transaction), Some(8));
    apply_selection(&mut state, transaction, None);
    assert_selection(&state, &[(0, 3), (5, 8)], 0);
    assert!(select_next_occurrence(&state).is_none());
}

const MARK: &str = "cm-selectionMatch";
const MAIN_MARK: &str = "cm-selectionMatch cm-selectionMatch-main";

fn marks(
    state: &EditorState,
    viewport: &Viewport,
    options: HighlightSelectionOptions,
) -> Vec<(Range<usize>, String)> {
    highlight_selection_matches(state, viewport, options)
        .unwrap()
        .iter(0)
        .map(|mark| (mark.from..mark.to, mark.value.class().to_owned()))
        .collect()
}

fn assert_marks(
    state: &EditorState,
    options: HighlightSelectionOptions,
    expected: &[(Range<usize>, &str)],
) {
    let expected: Vec<_> = expected
        .iter()
        .map(|(range, class)| (range.clone(), (*class).to_owned()))
        .collect();
    assert_eq!(
        marks(state, &Viewport::complete(state.text()), options),
        expected
    );
}

#[test]
fn selection_marks_skip_the_primary_and_check_boundaries_when_requested() {
    let source = "🦀 cat scatter cat cat_ cat Cat";
    let state = editor(source, &[(8, 5)], 0);
    assert_marks(
        &state,
        HighlightSelectionOptions::default(),
        &[
            (10..13, MARK),
            (17..20, MARK),
            (21..24, MARK),
            (26..29, MARK),
        ],
    );
    let whole = HighlightSelectionOptions {
        whole_words: true,
        ..Default::default()
    };
    assert_marks(&state, whole, &[(17..20, MARK), (26..29, MARK)]);
    assert_marks(&editor(source, &[(11, 14)], 0), whole, &[]);
    assert_marks(
        &editor("𐐀a a a𐐀", &[(6, 7)], 0),
        whole,
        &[(4..5, MARK), (8..9, MARK)],
    );
    assert_marks(&editor("𐐀 𐐀", &[(0, 4)], 0), whole, &[]);
}

#[test]
fn cursor_word_marks_the_main_match_and_respects_viewport_and_match_cap() {
    let source = "🦀 cat scatter cat cat_ cat Cat";
    let state = editor(source, &[(8, 8)], 0);
    let enabled = HighlightSelectionOptions {
        highlight_word_around_cursor: true,
        ..Default::default()
    };
    assert_marks(&state, HighlightSelectionOptions::default(), &[]);
    assert_marks(
        &state,
        enabled,
        &[(5..8, MAIN_MARK), (17..20, MARK), (26..29, MARK)],
    );
    assert_marks(
        &state,
        HighlightSelectionOptions {
            max_matches: 2,
            ..enabled
        },
        &[],
    );
    assert_marks(
        &state,
        HighlightSelectionOptions {
            max_matches: 3,
            ..enabled
        },
        &[(5..8, MAIN_MARK), (17..20, MARK), (26..29, MARK)],
    );
    assert_marks(&editor(source, &[(8, 5), (17, 17)], 0), enabled, &[]);

    let viewport = Viewport::around(state.text(), [23..31, 17..19, 10..13, 0..8]).unwrap();
    assert_eq!(
        marks(&state, &viewport, enabled),
        [(5..8, MAIN_MARK.into()), (26..29, MARK.into())]
    );
    let changed = editor("🦀", &[(0, 0)], 0);
    assert!(matches!(
        highlight_selection_matches(&changed, &viewport, enabled),
        Err(EditError::InvalidRange)
    ));
}

#[test]
fn highlight_length_limits_use_utf16_units_not_bytes() {
    let state = editor("🦀 🦀", &[(4, 0)], 0);
    assert_marks(
        &state,
        HighlightSelectionOptions {
            min_selection_length: 3,
            ..Default::default()
        },
        &[],
    );
    assert_marks(
        &state,
        HighlightSelectionOptions {
            min_selection_length: 2,
            ..Default::default()
        },
        &[(5..9, MARK)],
    );
    for count in [100, 101] {
        let word = "🦀".repeat(count);
        let source = format!("{word} {word}");
        let state = editor(&source, &[(0, word.len())], 0);
        let expected = [(word.len() + 1..source.len(), MARK)];
        assert_marks(
            &state,
            HighlightSelectionOptions::default(),
            if count == 100 { &expected } else { &[] },
        );
    }
}
