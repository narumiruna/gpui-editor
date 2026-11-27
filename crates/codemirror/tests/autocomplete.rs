//! Core cases checked against the pinned `@codemirror/autocomplete` 6.20.4
//! distribution and its source tests. Rust ranges are measured in UTF-8 bytes.

#[path = "autocomplete/commands.rs"]
mod commands;
#[path = "autocomplete/sources.rs"]
mod sources;

use std::{ops::Range, slice::from_ref};

use gpui_editor_codemirror::{
    Change, ChangeDesc, ChangeSet, EditError, EditorSelection, EditorState, SelectionRange, Text,
    Transaction,
    autocomplete::{
        Completion, CompletionMatch, CompletionMatcher, CompletionResult, CompletionSection,
        MatchMode, SectionRank, Snippet, insert_completion_text, rank_completions,
        rank_completions_by,
    },
};

#[test]
fn fuzzy_scores_and_highlights_match_the_pinned_oracle() {
    type Case<'a> = (&'a str, &'a str, i64, &'a [(usize, usize)]);
    let cases: &[Case<'_>] = &[
        ("ott", "OneTwoThree", -311, &[(0, 1), (3, 4), (6, 7)]),
        ("ott", "onetwothree", -1311, &[(0, 1), (3, 4), (6, 7)]),
        ("abc", "aVeryBigCar", -2111, &[(0, 1), (5, 6), (8, 9)]),
        ("abc", "xabc", -704, &[(1, 4)]),
        ("eTw", "onetwothree", -911, &[(2, 5)]),
        ("foo", "FOO", -203, &[(0, 3)]),
        ("foo", "FOObar", -306, &[(0, 3)]),
        ("foo", "aFOO", -904, &[(1, 4)]),
        ("sel", "SCOPE_CATALOG", -1313, &[(0, 1), (4, 5), (10, 11)]),
        ("foo", "foo", 0, &[(0, 3)]),
        ("", "bulk", -100, &[]),
        ("é", "Éclair", -300, &[(0, 2)]),
        ("αβ", "ΑΒγ", -303, &[(0, 4)]),
        ("αβ", "α-Β", -303, &[(0, 2), (3, 5)]),
        ("😀B", "😀bTail", -307, &[(0, 5)]),
        ("😀B", "prefix😀b", -909, &[(6, 11)]),
        ("ß", "SS", -300, &[(0, 1)]),
    ];
    for &(query, label, score, highlights) in cases {
        let matcher = CompletionMatcher::new(query, MatchMode::Fuzzy);
        let ranges = highlights.iter().map(|&(start, end)| start..end).collect();
        assert_eq!(
            matcher.match_label(label),
            Some(CompletionMatch { score, ranges }),
            "{query:?}, {label:?}"
        );
    }
    for (query, label) in [("t", "onetwothree"), ("wr", "onetwothree"), ("a-b", "a_Bb")] {
        assert_eq!(
            CompletionMatcher::new(query, MatchMode::Fuzzy).match_label(label),
            None
        );
    }
    // Direct substrings can be found beyond the fuzzy scan window.
    let tail = format!("{}abc", "😀".repeat(110));
    assert_eq!(
        CompletionMatcher::new("abc", MatchMode::Fuzzy)
            .match_label(&tail)
            .unwrap()
            .ranges,
        from_ref(&(440..443))
    );
    assert_eq!(
        CompletionMatcher::new("ABC", MatchMode::Fuzzy).match_label(&tail),
        None
    );
}

#[test]
fn strict_mode_matches_only_prefixes() {
    let matcher = CompletionMatcher::new("😀B", MatchMode::Strict);
    let found = matcher.match_label("😀bTail").unwrap();
    assert_eq!(found.score, -300);
    assert_eq!(found.ranges, from_ref(&(0..5)));
    assert!(matcher.match_label("prefix😀b").is_none());
    let matcher = CompletionMatcher::new("foo", MatchMode::Strict);
    assert_eq!(matcher.match_label("foo").unwrap().score, 0);
    assert_eq!(matcher.match_label("FOO").unwrap().score, -200);
    assert_eq!(matcher.match_label("fooBar").unwrap().score, -100);
    assert!(matcher.match_label("f_o_o").is_none());
}

#[test]
fn ranks_labels_like_the_upstream_examples() {
    let words = "one onetwothree OneTwoThree two three";
    for (query, sources, expected) in [
        ("ott", vec![words], "OneTwoThree onetwothree"),
        (
            "one",
            vec![words, "onet bonae"],
            "one onet onetwothree OneTwoThree bonae",
        ),
        ("t", vec![words], "three two"),
        ("wr", vec![words], ""),
        ("eTw", vec![words], "OneTwoThree onetwothree"),
        ("", vec!["a b foo"], "a b foo"),
        ("a", vec!["ac ab acc"], "ab ac acc"),
        ("t", vec!["two", "two three"], "three two"),
        (
            "sel",
            vec!["SCOPE_CATALOG SELECT SELECTIVE"],
            "SELECT SELECTIVE SCOPE_CATALOG",
        ),
        ("abc", vec!["xabc aVeryBigCar"], "xabc aVeryBigCar"),
        (
            "hair",
            vec!["aVerySmallChair Hairstyle chair BigChair"],
            "Hairstyle chair BigChair aVerySmallChair",
        ),
    ] {
        let options: Vec<Vec<_>> = sources
            .iter()
            .map(|list| list.split_whitespace().map(Completion::new).collect())
            .collect();
        let sources: Vec<_> = options
            .iter()
            .map(|options| CompletionResult {
                range: 0..query.len(),
                options,
                filter: true,
            })
            .collect();
        let ranked = rank_completions(&Text::new(query), &sources, MatchMode::Fuzzy).unwrap();
        assert_eq!(
            ranked
                .iter()
                .map(|r| r.completion.label)
                .collect::<Vec<_>>()
                .join(" "),
            expected,
            "{query}"
        );
    }
}

#[test]
fn ranking_honors_metadata_and_separate_source_ranges() {
    let text = Text::new("af");
    let first = [
        Completion::new("aa"),
        Completion {
            boost: 8,
            ..Completion::new("alpha")
        },
        Completion {
            sort_text: Some("ab"),
            ..Completion::new("ac")
        },
        Completion {
            display_label: Some("Other"),
            ..Completion::new("abcd")
        },
        Completion {
            info: Some("Info"),
            ..Completion::new("aa")
        },
    ];
    let second = [Completion::new("foo")];
    let unfiltered = [Completion::new("provided"), Completion::new("unsorted")];
    let sources = [
        CompletionResult {
            range: 0..1,
            options: &first,
            filter: true,
        },
        CompletionResult {
            range: 1..2,
            options: &second,
            filter: true,
        },
        CompletionResult {
            range: 0..2,
            options: &unfiltered,
            filter: false,
        },
    ];
    let ranked = rank_completions(&text, &sources, MatchMode::Fuzzy).unwrap();
    let summary: Vec<_> = ranked
        .iter()
        .map(|r| (r.completion.label, r.score))
        .collect();
    assert_eq!(
        summary,
        [
            ("provided", 999999994),
            ("unsorted", 999999993),
            ("alpha", -92),
            ("aa", -100),
            ("ac", -100),
            ("abcd", -100),
            ("foo", -100)
        ]
    );
    assert_eq!(ranked[3].completion.info, Some("Info"));
    assert!(ranked[5].match_ranges.is_empty());
    assert_eq!(ranked[6].range, 1..2);
    let ranked_by = rank_completions_by(&text, &sources[..2], MatchMode::Strict, |a, b| {
        b.label.cmp(a.label)
    })
    .unwrap();
    assert_eq!(ranked_by[1].completion.label, "foo");
    assert_eq!(
        rank_completions(&Text::new("é"), &sources, MatchMode::Fuzzy),
        Err(EditError::InvalidRange)
    );
}

#[test]
fn ranks_fixed_and_dynamic_sections() {
    let option = |label, name, rank| Completion {
        section: Some(CompletionSection { name, rank }),
        ..Completion::new(label)
    };
    let options = [
        option("fool", "A", SectionRank::Dynamic),
        option("Foam", "C", SectionRank::Fixed(2)),
        option("fo", "B", SectionRank::Dynamic),
        Completion::new("Follow"),
        option("foam", "D", SectionRank::Fixed(1)),
    ];
    let source = CompletionResult {
        range: 0..2,
        options: &options,
        filter: true,
    };
    let ranked = rank_completions(&Text::new("fo"), &[source], MatchMode::Fuzzy).unwrap();
    let summary: Vec<_> = ranked
        .iter()
        .map(|r| (r.completion.label, r.score))
        .collect();
    assert_eq!(
        summary,
        [
            ("Follow", -306),
            ("foam", -100100),
            ("Foam", -200304),
            ("fo", -300000),
            ("fool", -400100)
        ]
    );
}

fn selection(ranges: &[(usize, usize)]) -> EditorSelection {
    EditorSelection::new(
        ranges
            .iter()
            .map(|&(a, b)| SelectionRange::new(a, b))
            .collect(),
        0,
    )
    .unwrap()
}

fn ranges(selection: &EditorSelection) -> Vec<Range<usize>> {
    selection
        .ranges()
        .iter()
        .map(|range| range.range())
        .collect()
}

#[test]
fn applies_completion_to_matching_cursors_and_to_the_main_selection() {
    let mut state = EditorState::new("o\na\no");
    state
        .update(Transaction::default().with_selection(selection(&[(1, 1), (3, 3), (5, 5)])))
        .unwrap();
    let transaction = insert_completion_text(&state, 0..1, "okay").unwrap();
    assert!(transaction.is_user_event("input.complete"));
    state.update(transaction).unwrap();
    assert_eq!(state.text().as_str(), "okay\na\nokay");
    assert_eq!(ranges(state.selection()), [4..4, 6..6, 11..11]);

    let mut state = EditorState::new("hello world");
    state
        .update(Transaction::default().with_selection(EditorSelection::single(5, 1)))
        .unwrap();
    let transaction = insert_completion_text(&state, 0..1, "hey").unwrap();
    state.update(transaction).unwrap();
    assert_eq!(state.text().as_str(), "hey world");
}

#[test]
fn expands_numbered_fields_in_numeric_order_then_exits_at_zero() {
    let mut state = EditorState::new("");
    let snippet = Snippet::parse("😀${2:β} + ${01:å} + #{1:文}${0}");
    let (insert, session) = snippet.insert(&state, 0..0, "  ").unwrap();
    let mut session = session.unwrap();
    state.update(insert).unwrap();
    assert_eq!(state.text().as_str(), "😀β + å + 文");
    assert_eq!(ranges(state.selection()), [9..11, 14..17]);
    assert!(session.previous_field().is_none());
    assert_eq!(ranges(&session.next_field().unwrap()), from_ref(&(4..6)));
    assert_eq!(ranges(&session.next_field().unwrap()), from_ref(&(17..17)));
    assert_eq!(session.active_field(), None);
    assert!(session.next_field().is_none());
    assert!(session.previous_field().is_none());
}

#[test]
fn expands_indentation_and_named_mirrors() {
    let mut state = EditorState::new("  prefix");
    let template = "for (let ${index} = 0; #{index} < ${end}; ${index}++) {\r\n\t${}\r}";
    let (insert, session) = Snippet::parse(template).insert(&state, 2..8, "  ").unwrap();
    let mut session = session.unwrap();
    state.update(insert).unwrap();
    assert_eq!(
        state.text().as_str(),
        "  for (let index = 0; index < end; index++) {\n    \n  }"
    );
    assert_eq!(ranges(state.selection()), [11..16, 22..27, 35..40]);
    assert_eq!(ranges(&session.next_field().unwrap()), from_ref(&(30..33)));
    assert_eq!(ranges(&session.next_field().unwrap()), from_ref(&(50..50)));
}

#[test]
fn linked_edit_maps_sites_and_preserves_reverse_traversal() {
    let mut state = EditorState::new(" pre ");
    let (insert, session) = Snippet::parse("${1:λ}+#{1:β}+${2:x}${0}")
        .insert(&state, 1..1, "  ")
        .unwrap();
    let mut session = session.unwrap();
    state.update(insert).unwrap();
    assert_eq!(ranges(state.selection()), [1..3, 4..6]);
    let edit = state.replace_selection("é😀").unwrap();
    let desc = edit.changes().desc(state.text().len()).unwrap();
    state.update(edit).unwrap();
    assert_eq!(state.text().as_str(), " é😀+é😀+xpre ");
    assert!(session.update(&desc, Some(state.selection())).unwrap());
    assert_eq!(ranges(&session.next_field().unwrap()), from_ref(&(15..16)));
    assert_eq!(ranges(&session.previous_field().unwrap()), [1..7, 8..14]);
    assert_eq!(ranges(&session.next_field().unwrap()), from_ref(&(15..16)));
    assert_eq!(ranges(&session.next_field().unwrap()), from_ref(&(16..16)));
    assert!(session.fields().is_empty());
}

#[test]
fn session_tracks_boundary_insertions_and_ends_on_invalidated_field_or_selection() {
    let state = EditorState::new("");
    let (_, session) = Snippet::parse("<${1:x}>${}${0}")
        .insert(&state, 0..0, "  ")
        .unwrap();
    let mut session = session.unwrap();
    let prior = session.clone();
    assert_eq!(
        session.update(&ChangeDesc::new(0, vec![]).unwrap(), None),
        Err(EditError::MismatchedLength)
    );
    assert_eq!(session, prior);

    let desc = ChangeSet::single(Change::new(2..2, "é"))
        .unwrap()
        .desc(3)
        .unwrap();
    let caret = EditorSelection::single(4, 4);
    assert!(session.update(&desc, Some(&caret)).unwrap());
    assert_eq!(
        session.fields(),
        [from_ref(&(1..4)), from_ref(&(5..5)), from_ref(&(5..5))]
    );
    let mut other = session.clone();
    let outside = EditorSelection::single(0, 0);
    assert!(
        !other
            .update(&ChangeDesc::new(5, vec![]).unwrap(), Some(&outside))
            .unwrap()
    );
    // The deletion crosses the right edge of the first field.
    assert!(
        !session
            .update(&ChangeDesc::new(5, vec![(2..5, 0)]).unwrap(), None)
            .unwrap()
    );
    assert_eq!(session.active_field(), None);
}

#[test]
fn keeps_escaped_braces_and_uses_valid_byte_ranges_for_later_fields() {
    let mut state = EditorState::new("");
    let (insert, session) = Snippet::parse(r"\{${name\{x\}}\} ${2:next}${0}")
        .insert(&state, 0..0, "  ")
        .unwrap();
    let mut session = session.unwrap();
    state.update(insert).unwrap();
    assert_eq!(state.text().as_str(), "{name{x}} next");
    assert_eq!(ranges(state.selection()), from_ref(&(10..14)));
    // This follows the documented final-stop behavior. The pinned JS build
    // puts zero before unnumbered fields and can misplace later escaped sites.
    assert_eq!(ranges(&session.next_field().unwrap()), from_ref(&(1..8)));
    assert_eq!(ranges(&session.next_field().unwrap()), from_ref(&(14..14)));
}

#[test]
fn snippet_line_endings_keep_field_byte_ranges() {
    let state = EditorState::new("  pré\r\n");
    let template = Snippet::parse("${word}\n\t${}.");
    let (transaction, session) = template
        .insert_with_line_ending(&state, 2..6, "  ", "\r\n")
        .unwrap();
    let mut result = state.clone();
    result.update(transaction).unwrap();
    assert_eq!(result.text().as_str(), "  word\r\n    .\r\n");
    assert_eq!(result.selection().main().range(), 2..6);
    assert_eq!(
        session.unwrap().next_field().unwrap().main().range(),
        12..12
    );
    assert!(
        template
            .insert_with_line_ending(&state, 2..6, "  ", "\r")
            .is_err()
    );
}

#[test]
fn snippet_validates_offsets_and_does_not_open_a_single_field_session() {
    let mut state = EditorState::new("😀selected");
    state
        .update(Transaction::default().with_selection(EditorSelection::single(4, 12)))
        .unwrap();
    let snippet = Snippet::parse("${hello}");
    assert!(matches!(
        snippet.insert(&state, 1..4, "  "),
        Err(EditError::InvalidRange)
    ));
    let (insert, session) = snippet.insert(&state, 4..4, "  ").unwrap();
    assert!(session.is_none());
    state.update(insert).unwrap();
    assert_eq!(state.text().as_str(), "😀hello");
    assert_eq!(ranges(state.selection()), from_ref(&(4..9)));
}
