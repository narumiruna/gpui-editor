use std::ops::Range;

use gpui_editor_codemirror::search::{
    CursorOptions, RegExpCursor, SearchCursor, SearchError, SearchMatch, SearchOptions,
    SearchQuery, extension, get_search_query, select_matches, set_search_query,
};
use gpui_editor_codemirror::{
    EditorSelection, EditorState, Extension, SelectionRange, Text, Transaction,
};

fn ranges<'a>(matches: impl Iterator<Item = SearchMatch<'a>>) -> Vec<Range<usize>> {
    matches.map(|result| result.range).collect()
}

fn assert_single<'a>(mut matches: impl Iterator<Item = SearchMatch<'a>>, expected: Range<usize>) {
    assert_eq!(matches.next().unwrap().range, expected);
    assert!(matches.next().is_none());
}

fn regex_query(pattern: &str) -> SearchQuery {
    SearchQuery::new(
        pattern,
        SearchOptions {
            regexp: true,
            ..SearchOptions::default()
        },
    )
    .unwrap()
}

#[test]
fn literal_cursor_can_switch_between_overlapping_and_non_overlapping_matches() {
    let text = Text::new("🦀aaaaa");
    assert_eq!(ranges(SearchCursor::new(&text, "aa")), [4..6, 6..8]);

    let mut cursor = SearchCursor::new(&text, "aa");
    assert_eq!(cursor.next().unwrap().range, 4..6);
    assert_eq!(cursor.next_overlapping().unwrap().range, 5..7);
    assert_eq!(cursor.next().unwrap().range, 7..9);
    assert!(cursor.next_overlapping().is_none());
    assert!(cursor.next().is_none());
}

#[test]
fn matching_uses_nfkd_but_returns_original_byte_ranges() {
    // Verified against @codemirror/search 6.7.3 before translating the offsets.
    let text = Text::new("é e\u{301} Ｅ ﬃ");
    let find = |pattern| {
        SearchCursor::new(&text, pattern)
            .map(|result| (result.range, result.text, result.precise))
            .collect::<Vec<_>>()
    };
    assert_eq!(find("é"), [(0..2, "é", true), (3..6, "e\u{301}", true)]);
    assert_eq!(find("e"), [(0..2, "é", false), (3..4, "e", true)]);
    assert_eq!(find("E"), [(7..10, "Ｅ", true)]);
    assert_eq!(find("f"), [(11..14, "ﬃ", false)]);
    assert_eq!(find("i"), [(11..14, "ﬃ", false)]);
    assert_eq!(find("ffi"), [(11..14, "ﬃ", true)]);

    // The pattern is normalized as a string, whereas the document uses
    // scalar-by-scalar normalization. It does not reorder adjacent marks.
    let text = Text::new("e\u{315}\u{300}");
    assert!(SearchCursor::new(&text, text.as_str()).next().is_none());
}

#[test]
fn literal_search_keeps_native_line_endings_and_checks_range_boundaries() {
    let text = Text::new("é\r\n🦀");
    assert_single(SearchCursor::new(&text, "\r\n🦀"), 2..8);
    assert_single(SearchCursor::new(&text, "\n"), 3..4);
    assert!(SearchCursor::new(&text, "").next().is_none());

    let options = CursorOptions::default();
    let cursor = SearchCursor::in_range(&text, "🦀", 4..8, options).unwrap();
    assert_single(cursor, 4..8);
    for range in [1..8, 4..5, 4..9, Range { start: 4, end: 0 }] {
        assert!(matches!(
            SearchCursor::in_range(&text, "é", range.clone(), options),
            Err(SearchError::InvalidRange)
        ));
        assert!(matches!(
            RegExpCursor::in_range(&text, ".", range, options),
            Err(SearchError::InvalidRange)
        ));
    }
}

#[test]
fn whole_word_uses_context_at_both_ends_of_a_match() {
    let text = Text::new("É e\u{301} xé é _é é_ é-É");
    let query = SearchQuery::new(
        "é",
        SearchOptions {
            whole_word: true,
            ..SearchOptions::default()
        },
    )
    .unwrap();
    let matches: Vec<_> = query
        .cursor(&text, 0..text.len())
        .unwrap()
        .map(|result| result.text)
        .collect();
    assert_eq!(matches, ["É", "e\u{301}", "é", "é", "É"]);
    assert!(query.cursor(&text, 8..10).unwrap().next().is_none());

    let text = Text::new("cat-cat scat catapult _cat cat_");
    let query = SearchQuery::new(
        "cat",
        SearchOptions {
            regexp: true,
            whole_word: true,
            ..SearchOptions::default()
        },
    )
    .unwrap();
    assert_eq!(
        ranges(query.cursor(&text, 0..text.len()).unwrap()),
        [0..3, 4..7]
    );
    assert!(query.cursor(&text, 9..12).unwrap().next().is_none());

    let query = SearchQuery::new(
        "-cat-",
        SearchOptions {
            whole_word: true,
            ..Default::default()
        },
    )
    .unwrap();
    let text = Text::new("a-cat-z");
    assert_single(query.cursor(&text, 0..text.len()).unwrap(), 1..6);
}

#[test]
fn regex_cursor_captures_multiline_text_with_original_byte_offsets() {
    let text = Text::new("🦀\nαβ\nZ");
    let mut cursor = RegExpCursor::new(&text, r"(?<lead>\p{Script=Greek}+)\s+(Z)").unwrap();
    let found = cursor.next().unwrap();
    assert_eq!(found.range, 5..11);
    assert_eq!(found.text, "αβ\nZ");
    assert_eq!(found.captures, [Some("αβ\nZ"), Some("αβ"), Some("Z")]);
    assert!(found.precise);
    assert!(cursor.next().is_none());

    let text = Text::new("🦀\r\nZ");
    assert_single(
        RegExpCursor::new(&text, r"\uD83E\uDD80\r\nZ").unwrap(),
        0..7,
    );
    assert_eq!(
        ranges(RegExpCursor::new(&text, ".$").unwrap()),
        [0..4, 6..7]
    );
}

#[test]
fn regex_uses_common_ecmascript_character_class_rules() {
    let text = Text::new("é ա K ſ _Az9 ٤ 0\u{85}\u{feff}\u{2028}\n");
    let matches = |pattern, ignore_case| {
        RegExpCursor::in_range(
            &text,
            pattern,
            0..text.len(),
            CursorOptions {
                ignore_case,
                ..CursorOptions::default()
            },
        )
        .unwrap()
        .map(|result| result.text)
        .collect::<Vec<_>>()
    };
    assert_eq!(matches(r"[\w]+", false), ["_Az9", "0"]);
    assert_eq!(matches(r"\w+", true), ["K", "ſ", "_Az9", "0"]);
    assert_eq!(matches(r"\d", false), ["9", "0"]);
    assert_eq!(matches(r"\s+$", false), ["\u{feff}\u{2028}\n"]);
    assert_eq!(matches(r"\bé\b", false), [] as [&str; 0]);
    assert_eq!(matches(r"[]", false), [] as [&str; 0]);
    assert_eq!(matches(r"[^]", false).len(), text.as_str().chars().count());
}

#[test]
fn regex_cursors_respect_ranges_anchors_and_empty_match_progress() {
    let text = Text::new("ab cd");
    let cursor =
        |pattern| RegExpCursor::in_range(&text, pattern, 2..4, CursorOptions::default()).unwrap();
    assert!(cursor("^").next().is_none());
    assert_single(cursor(".$"), 3..4);
    assert_single(cursor(r"^|\s?"), 3..3);

    let text = Text::new("x\na\n");
    assert_eq!(
        ranges(RegExpCursor::new(&text, "^").unwrap()),
        [0..0, 2..2, 4..4]
    );
    assert_eq!(
        ranges(RegExpCursor::new(&text, "$").unwrap()),
        [1..1, 3..3, 4..4]
    );

    let text = Text::new("a🦀a");
    assert_eq!(
        ranges(RegExpCursor::new(&text, "").unwrap()),
        [0..0, 1..1, 5..5, 6..6]
    );
    assert_eq!(
        ranges(RegExpCursor::new(&text, "a|$").unwrap()),
        [0..1, 5..6]
    );
    let text = Text::new("abc");
    assert_eq!(
        ranges(RegExpCursor::new(&text, r"\s*").unwrap()),
        [1..1, 3..3]
    );
    let text = Text::new("🦀x");
    assert_eq!(
        ranges(RegExpCursor::new(&text, r"\s*").unwrap()),
        [0..0, 5..5]
    );
}

#[test]
fn invalid_or_unsupported_regex_queries_return_errors() {
    let text = Text::new("test");
    for pattern in [
        "[", r"\xQF", r"\uD800", r"(a+)\1", "(?=a*)", "(?i:a)", r"\z",
    ] {
        assert!(
            matches!(
                RegExpCursor::new(&text, pattern),
                Err(SearchError::InvalidRegExp(_))
            ),
            "{pattern}"
        );
    }
    assert!(matches!(
        SearchQuery::new("", SearchOptions::default()),
        Err(SearchError::EmptyQuery)
    ));
}

#[test]
fn navigation_wraps_in_both_directions() {
    let text = Text::new("🦀 É é");
    let query = SearchQuery::new("é", SearchOptions::default()).unwrap();
    assert_eq!(query.find_next(&text, 8..10).unwrap().unwrap().range, 5..7);
    assert_eq!(
        query.find_previous(&text, 5..7).unwrap().unwrap().range,
        8..10
    );
    assert!(matches!(
        query.find_next(&text, 1..5),
        Err(SearchError::InvalidRange)
    ));

    let text = Text::new("aaa");
    let query = SearchQuery::new("aa", SearchOptions::default()).unwrap();
    assert_eq!(query.find_next(&text, 1..3).unwrap().unwrap().range, 0..2);
    assert_eq!(
        query.find_previous(&text, 0..2).unwrap().unwrap().range,
        1..3
    );
    assert!(query.find_next(&text, 0..2).unwrap().is_none());

    let text = Text::new("a\na");
    let query = regex_query("a");
    assert_eq!(query.find_next(&text, 2..3).unwrap().unwrap().range, 0..1);
    assert_eq!(
        query.find_previous(&text, 0..1).unwrap().unwrap().range,
        2..3
    );
}

#[test]
fn literal_replacement_interprets_escapes_and_skips_imprecise_matches() {
    let mut text = Text::new("f ﬃ Ｆ");
    let query = SearchQuery::new("f", SearchOptions::default()).unwrap();
    query
        .replace_all(&text, r"\t$1")
        .unwrap()
        .apply_to(&mut text)
        .unwrap();
    assert_eq!(text.as_str(), "\t$1 ﬃ \t$1");

    let text = Text::new("a\tb a\\tb");
    let query = SearchQuery::new(r"a\tb", SearchOptions::default()).unwrap();
    assert_single(query.cursor(&text, 0..text.len()).unwrap(), 0..3);
    let literal = SearchOptions {
        literal: true,
        ..SearchOptions::default()
    };
    let query = SearchQuery::new(r"a\tb", literal).unwrap();
    let found = query.cursor(&text, 0..text.len()).unwrap().next().unwrap();
    assert_eq!(found.range, 4..8);
    assert_eq!(query.replacement(&found, r"\t\\"), r"\t\\");
}

#[test]
fn regex_replacement_expands_captures_in_one_change_set() {
    let mut text = Text::new("é b ab");
    let query = regex_query(r"(?<lead>a)?(b)");
    let matched = query.cursor(&text, 0..text.len()).unwrap().next().unwrap();
    assert_eq!(matched.captures, [Some("b"), None, Some("b")]);
    let template = r"$0$1$2$3|$&|$$|$21|$01|$<lead>|$2١\n";
    assert_eq!(
        query.replacement(&matched, template),
        "$0undefinedb$3|b|$|b1|undefined|$<lead>|b١\n"
    );
    let changes = query.replace_all(&text, "$2:$1:$$").unwrap();
    let inverse = changes.inverted(&text).unwrap();
    changes.apply_to(&mut text).unwrap();
    assert_eq!(text.as_str(), "é b:undefined:$ b:a:$");
    inverse.apply_to(&mut text).unwrap();
    assert_eq!(text.as_str(), "é b ab");
}

#[test]
fn regex_replacement_can_insert_at_empty_match_boundaries() {
    let mut text = Text::new("🦀\r\nx\n");
    let query = regex_query("^");
    query
        .replace_all(&text, ">")
        .unwrap()
        .apply_to(&mut text)
        .unwrap();
    assert_eq!(text.as_str(), ">🦀\r\n>x\n>");
}

#[test]
fn search_query_uses_main_selection_until_the_search_extension_is_installed() {
    // Checked against the pinned JS search extension before translating offsets.
    let mut state = EditorState::new("x\né a");
    let selection = EditorSelection::new(
        vec![SelectionRange::new(6, 5), SelectionRange::new(4, 1)],
        1,
    )
    .unwrap();
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    let query = get_search_query(&state);
    assert_eq!(query.search(), r"\né");
    assert!(query.is_valid());
    assert!(select_matches(&state).is_none());
    state
        .update(
            Transaction::default().with_effects([set_search_query(SearchQuery::from_input(
                "a",
                SearchOptions::default(),
            ))]),
        )
        .unwrap();
    assert_eq!(get_search_query(&state).search(), r"\né");

    let options = SearchOptions {
        literal: true,
        case_sensitive: true,
        whole_word: true,
        ..SearchOptions::default()
    };
    state
        .update(Transaction::default().with_effects([Extension::append(extension(options))]))
        .unwrap();
    assert_eq!(get_search_query(&state).search(), "\né");
    assert_eq!(get_search_query(&state).options(), options);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(0, 1)))
        .unwrap();
    assert_eq!(get_search_query(&state).search(), "\né");
}

#[test]
fn search_query_retains_the_latest_input_even_if_invalid() {
    let mut state = EditorState::with_extensions("a", extension(SearchOptions::default())).unwrap();
    let query = get_search_query(&state);
    assert_eq!(query.search(), "");
    assert!(!query.is_valid());
    let valid = SearchQuery::from_input("a", SearchOptions::default());
    let invalid = SearchQuery::from_input(
        "[",
        SearchOptions {
            regexp: true,
            ..SearchOptions::default()
        },
    )
    .with_replacement("x");
    state
        .update(
            Transaction::default()
                .with_effects([set_search_query(valid), set_search_query(invalid)]),
        )
        .unwrap();
    let query = get_search_query(&state);
    assert_eq!(query.search(), "[");
    assert_eq!(query.replacement_text(), "x");
    assert!(!query.is_valid());
    assert!(matches!(
        query.cursor(state.text(), 0..1),
        Err(SearchError::InvalidRegExp(_))
    ));
    assert!(
        query
            .replace_all(state.text(), query.replacement_text())
            .unwrap()
            .is_empty()
    );
    assert!(select_matches(&state).is_none());
}

#[test]
fn default_search_query_limits_the_selection_to_100_utf16_units() {
    for count in [50, 51] {
        let source = "🦀".repeat(count);
        let mut state = EditorState::new(&source);
        state
            .update(Transaction::default().with_selection(EditorSelection::single(source.len(), 0)))
            .unwrap();
        assert_eq!(
            get_search_query(&state).search(),
            if count == 50 { &source } else { "" }
        );
    }
}
