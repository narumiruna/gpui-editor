//! Oracle: published @codemirror/search 6.7.3, state 6.7.6, and view 6.44.0.
//! The npm archives were checked against docs/codemirror-upstream.json before
//! running Node 26.10.0 in scratch. The fixtures convert UTF-16 offsets to
//! lengths of UTF-8 prefixes. Command fixtures use an EditorView in JSDOM.

use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    EditorSelection, EditorState, Text, Transaction,
    search::{self, RegExpCursor, SearchError, SearchOptions, SearchQuery},
    view::{ANNOUNCE, SCROLL_INTO_VIEW},
};
use serde_json::{Value, json};

fn fixtures(kind: &'static str) -> impl Iterator<Item = Value> {
    include_str!("fixtures/search_regexp.jsonl")
        .lines()
        .chain(include_str!("fixtures/search_regexp_hyphen.jsonl").lines())
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn source(row: &Value) -> String {
    row["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| match part.as_str() {
            Some(text) => text.to_owned(),
            None => part[0]
                .as_str()
                .unwrap()
                .repeat(serde_json::from_value(part[1].clone()).unwrap()),
        })
        .collect()
}

#[test]
fn cursor_ranges_captures_and_filter_order_match_the_pinned_search() {
    for row in fixtures("cursor") {
        let id = row["id"].as_str().unwrap();
        let text = Text::new(source(&row));
        let [from, to]: [usize; 2] = serde_json::from_value(row["range"].clone()).unwrap();
        let reject: Vec<usize> = serde_json::from_value(row["reject"].clone()).unwrap();
        let visits = Arc::new(Mutex::new(Vec::new()));
        let track = visits.clone();
        let query = SearchQuery::new(
            row["pattern"].as_str().unwrap(),
            SearchOptions {
                regexp: true,
                case_sensitive: row["ignoreCase"] != true,
                ..SearchOptions::default()
            },
        )
        .unwrap_or_else(|error| panic!("{id}: {error}"))
        .with_test(move |found, _| {
            track.lock().unwrap().push(found.range.start);
            !reject.contains(&found.range.start)
        });
        let matches: Vec<_> = query
            .cursor(&text, from..to)
            .unwrap()
            .map(|found| {
                assert!(found.precise, "{id}");
                assert_eq!(found.text, &text.as_str()[found.range.clone()], "{id}");
                let mut result = vec![json!(found.range.start), json!(found.range.end)];
                result.extend(found.captures.into_iter().map(|capture| json!(capture)));
                result
            })
            .collect();
        assert_eq!(json!(matches), row["matches"], "{id}");
        assert_eq!(json!(*visits.lock().unwrap()), row["visits"], "{id}");
    }
}

#[test]
fn regex_syntax_does_not_accept_rust_only_constructs() {
    let text = Text::new("aa{2}α");
    for row in fixtures("invalid") {
        for pattern in row["patterns"].as_array().unwrap() {
            let pattern = pattern.as_str().unwrap();
            assert!(
                matches!(
                    RegExpCursor::new(&text, pattern),
                    Err(SearchError::InvalidRegExp(_))
                ),
                "{pattern}"
            );
        }
    }
}

#[test]
fn unsupported_patterns_do_not_enable_backtracking() {
    let text = Text::new("aaaa!");
    for pattern in [
        r"(a+)\1",
        r"(?<word>a+)\k<word>",
        r"(x)(\1\1)\2",
        r"(x{16}x{16}x{16}x{16}x)\1",
        "(?!(a+)+$)",
        "^(?:(?=(x))){80000}x$",
        "(?=(a{0,})b)",
        "(?:a?){16}b(?=c)",
    ] {
        assert!(matches!(
            RegExpCursor::new(&text, pattern),
            Err(SearchError::InvalidRegExp(_))
        ));
        let query = SearchQuery::from_input(
            pattern,
            SearchOptions {
                regexp: true,
                ..SearchOptions::default()
            },
        );
        assert!(!query.is_valid(), "{pattern}");
    }
}

#[test]
fn unbounded_pinned_backreference_and_lookaround_cases_are_rejected() {
    // `gap` rows record pinned matches and test visits for patterns without
    // a small proven bound. Do not fall back to unlimited backtracking.
    for row in fixtures("gap") {
        let id = row["id"].as_str().unwrap();
        let pattern = row["pattern"].as_str().unwrap();
        let text = Text::new(source(&row));
        let [from, to]: [usize; 2] = serde_json::from_value(row["range"].clone()).unwrap();
        let ignore_case = row["ignoreCase"] == true;
        assert!(
            matches!(
                RegExpCursor::in_range(
                    &text,
                    pattern,
                    from..to,
                    search::CursorOptions {
                        ignore_case,
                        ..search::CursorOptions::default()
                    },
                ),
                Err(SearchError::InvalidRegExp(_))
            ),
            "{id}"
        );
        let query = SearchQuery::from_input(
            pattern,
            SearchOptions {
                regexp: true,
                case_sensitive: !ignore_case,
                ..SearchOptions::default()
            },
        );
        assert!(!query.is_valid(), "{id}");
    }
}

#[test]
fn hyphen_ranges_preserve_utf8_and_native_crlf_when_replacing() {
    let source = "🦀-./0\r\né+,-\r\n";
    let mut text = Text::new(source);
    let pattern = "([--0]+)$";
    assert!(matches!(
        RegExpCursor::in_range(
            &text,
            pattern,
            1..text.len(),
            search::CursorOptions::default()
        ),
        Err(SearchError::InvalidRange)
    ));
    assert!(
        RegExpCursor::new(&text, r"[--0]+\x0D$")
            .unwrap()
            .next()
            .is_none()
    );
    let multiline: Vec<_> = RegExpCursor::new(&text, r"([--0]+)(\r\n)")
        .unwrap()
        .map(|found| (found.range, found.captures))
        .collect();
    assert_eq!(
        multiline,
        [
            (4..10, vec![Some("-./0\r\n"), Some("-./0"), Some("\r\n")]),
            (14..17, vec![Some("-\r\n"), Some("-"), Some("\r\n")]),
        ]
    );

    let query = SearchQuery::new(
        pattern,
        SearchOptions {
            regexp: true,
            ..Default::default()
        },
    )
    .unwrap();
    let changes = query.replace_all(&text, "<$1>").unwrap();
    assert_eq!(
        changes.changes().collect::<Vec<_>>(),
        [(&(4..8), "<-./0>"), (&(14..15), "<->")]
    );
    let inverse = changes.inverted(&text).unwrap();
    changes.apply_to(&mut text).unwrap();
    assert_eq!(text.as_str(), "🦀<-./0>\r\né+,<->\r\n");
    inverse.apply_to(&mut text).unwrap();
    assert_eq!(text.as_str(), source);
}

#[test]
fn bounded_assertion_captures_can_be_used_by_replace_all() {
    let options = SearchOptions {
        regexp: true,
        ..SearchOptions::default()
    };
    let query = SearchQuery::new(r"(?<=([ab]{1,2}))c", options)
        .unwrap()
        .with_replacement("[$1]");
    let mut state =
        EditorState::with_extensions("🦀abc ac dc", search::extension(options)).unwrap();
    state
        .update(Transaction::default().with_effects([search::set_search_query(query)]))
        .unwrap();
    let edit = search::replace_all(&state).unwrap();
    assert_eq!(
        edit.changes().changes().collect::<Vec<_>>(),
        [(&(6..7), "[ab]"), (&(9..10), "[a]")]
    );
    state.update(edit).unwrap();
    assert_eq!(state.text().as_str(), "🦀ab[ab] a[a] dc");
}

#[test]
fn regexp_command_transactions_match_the_pinned_search() {
    for row in fixtures("command") {
        let id = row["id"].as_str().unwrap();
        let spec = &row["query"];
        let query = SearchQuery::new(
            spec["search"].as_str().unwrap(),
            SearchOptions {
                regexp: true,
                ..SearchOptions::default()
            },
        )
        .unwrap()
        .with_replacement(spec["replace"].as_str().unwrap_or(""));
        let mut state =
            EditorState::with_extensions(source(&row), search::extension(SearchOptions::default()))
                .unwrap();
        let [anchor, head]: [usize; 2] = serde_json::from_value(row["before"].clone()).unwrap();
        state
            .update(
                Transaction::default()
                    .with_selection(EditorSelection::single(anchor, head))
                    .with_effects([search::set_search_query(query)]),
            )
            .unwrap();
        let tx = match row["action"].as_str().unwrap() {
            "find-next" => search::find_next(&state),
            "find-previous" => search::find_previous(&state),
            "replace-next" => search::replace_next(&state),
            "replace-all" => search::replace_all(&state),
            action => panic!("unknown action: {action}"),
        }
        .unwrap_or_else(|| panic!("no transaction: {id}"));
        let selected = tx
            .selection()
            .map(|selection| [selection.main().anchor(), selection.main().head()]);
        let changes: Vec<_> = tx
            .changes()
            .changes()
            .map(|(range, text)| json!([range.start, range.end, text]))
            .collect();
        let speak: Vec<_> = tx
            .effects()
            .iter()
            .filter_map(|e| e.get(&ANNOUNCE))
            .collect();
        let scroll: Vec<_> = tx
            .effects()
            .iter()
            .filter_map(|e| e.get(&SCROLL_INTO_VIEW))
            .map(|target| [target.range.anchor(), target.range.head()])
            .collect();
        assert_eq!(
            json!({"changes": changes, "selected": selected, "speak": speak, "scroll": scroll, "event": tx.user_event()}),
            row["expected"],
            "{id}"
        );
    }
}
