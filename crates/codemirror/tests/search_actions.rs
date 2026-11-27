//! Results from the SHA-512-verified archives for @codemirror/search 6.7.3,
//! state 6.7.6, and view 6.44.0 in a scratch JSDOM EditorView. Offsets are
//! byte lengths of the corresponding UTF-16 document prefixes.

use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    EditError, EditorSelection, EditorState, Extension, READ_ONLY, SelectionRange, Transaction,
    search::{
        self, SearchAction, SearchActionResult, SearchOptions, SearchPanelInput, SearchQuery,
    },
    view::{ANNOUNCE, SCROLL_INTO_VIEW, Viewport},
};
use serde_json::{Value, json};

fn rows(kind: &str) -> impl Iterator<Item = Value> + '_ {
    include_str!("fixtures/search_actions.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn action(name: &str) -> SearchAction {
    match name {
        "openSearchPanel" => SearchAction::OpenSearchPanel,
        "closeSearchPanel" => SearchAction::CloseSearchPanel,
        "findNext" => SearchAction::FindNext,
        "findPrevious" => SearchAction::FindPrevious,
        "selectMatches" => SearchAction::SelectMatches,
        "selectSelectionMatches" => SearchAction::SelectSelectionMatches,
        "selectNextOccurrence" => SearchAction::SelectNextOccurrence,
        "replaceNext" => SearchAction::ReplaceNext,
        "replaceAll" => SearchAction::ReplaceAll,
        "gotoLine" => SearchAction::GotoLine,
        _ => panic!("unknown reference command: {name}"),
    }
}

fn offsets(value: &Value) -> [usize; 2] {
    serde_json::from_value(value.clone()).unwrap()
}

fn options(value: &Value) -> SearchOptions {
    SearchOptions {
        case_sensitive: value["caseSensitive"] == true,
        whole_word: value["wholeWord"] == true,
        regexp: value["regexp"] == true,
        literal: value["literal"] == true,
    }
}

fn query(value: &Value) -> SearchQuery {
    SearchQuery::from_input(value["search"].as_str().unwrap(), options(value))
        .with_replacement(value["replace"].as_str().unwrap_or_default())
}

fn document(row: &Value) -> String {
    row["docParts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| match part.as_str() {
            Some(text) => text.to_owned(),
            None => part[0]
                .as_str()
                .unwrap()
                .repeat(part[1].as_u64().unwrap().try_into().unwrap()),
        })
        .collect()
}

#[test]
fn panel_lifecycle_and_action_fallback_match_the_pinned_view() {
    for row in rows("panel") {
        let config = &row["config"];
        let mut extensions = vec![READ_ONLY.of(config["readOnly"] == true)];
        if !config["installed"].is_null() {
            extensions.push(search::extension(options(&config["installed"])));
        }
        let mut state = EditorState::with_extensions(
            row["doc"].as_str().unwrap(),
            Extension::group(extensions),
        )
        .unwrap();
        for step in row["steps"].as_array().unwrap() {
            if !step["select"].is_null() {
                let [a, h] = offsets(&step["select"]);
                state
                    .update(Transaction::default().with_selection(EditorSelection::single(a, h)))
                    .unwrap();
            }
            if !step["query"].is_null() {
                state
                    .update(
                        Transaction::default()
                            .with_effects([search::set_search_query(query(&step["query"]))]),
                    )
                    .unwrap();
            }
            let input = match step["input"].as_str() {
                Some("focused") => SearchPanelInput::Focused,
                Some("unfocused") => SearchPanelInput::Unfocused,
                _ => SearchPanelInput::Missing,
            };
            let outcome = action(step["action"].as_str().unwrap()).run(&state, input);
            let (handled, tx) = match outcome {
                SearchActionResult::Unhandled => (false, None),
                SearchActionResult::Update(tx) => (true, Some(tx)),
                SearchActionResult::OpenSearchPanel(tx) => (true, tx),
                other => panic!("unexpected outcome: {other:?}"),
            };
            let mut effects = Vec::new();
            if let Some(tx) = tx {
                effects.push(tx.effects().len());
                state.update(tx).unwrap();
            }
            let query = search::get_search_query(&state);
            let options = query.options();
            let selection: Vec<_> = state
                .selection()
                .ranges()
                .iter()
                .map(|range| [range.anchor(), range.head()])
                .collect();
            assert_eq!(
                json!({"result": handled, "effects": effects, "panel": search::search_panel_open(&state), "query": [query.search(), query.replacement_text(), query.is_valid(), options.case_sensitive, options.regexp, options.literal, options.whole_word], "selection": selection}),
                step["after"],
                "{}: {}",
                row["id"],
                step["action"]
            );
        }
    }
}

#[test]
fn query_marks_match_pinned_viewport_scans() {
    for row in rows("highlights") {
        let doc = document(&row);
        let mut state =
            EditorState::with_extensions(doc, search::extension(SearchOptions::default())).unwrap();
        if row["op"] != "closed" {
            state
                .update(search::open_search_panel(&state, SearchPanelInput::Missing).unwrap())
                .unwrap();
        }
        let ranges = row["ranges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|range| {
                let [a, h] = offsets(range);
                SelectionRange::new(a, h)
            })
            .collect();
        state
            .update(
                Transaction::default()
                    .with_selection(EditorSelection::new(ranges, 0).unwrap())
                    .with_effects([search::set_search_query(query(&row["query"]))]),
            )
            .unwrap();
        if row["op"] == "reclosed" {
            state
                .update(search::close_search_panel(&state).unwrap())
                .unwrap();
        }
        let viewport = match row["visible"].as_array() {
            Some(parts) => Viewport::around(
                state.text(),
                parts.iter().map(|part| {
                    let [start, end] = offsets(part);
                    start..end
                }),
            )
            .unwrap(),
            None => Viewport::complete(state.text()),
        };
        let marks = search::highlight_search_matches(&state, &viewport).unwrap();
        let marks: Vec<_> = marks
            .iter(0)
            .map(|mark| json!([mark.from, mark.to, mark.value.class()]))
            .collect();
        assert_eq!(json!(marks), row["marks"], "{}", row["id"]);
    }
}

#[test]
fn native_marks_skip_empty_matches_and_reject_out_of_bounds_viewports() {
    let mut state = EditorState::new("x\ny");
    state
        .update(search::open_search_panel(&state, SearchPanelInput::Missing).unwrap())
        .unwrap();
    state
        .update(
            Transaction::default().with_effects([search::set_search_query(
                SearchQuery::new(
                    "^",
                    SearchOptions {
                        regexp: true,
                        ..SearchOptions::default()
                    },
                )
                .unwrap(),
            )]),
        )
        .unwrap();
    let viewport = Viewport::complete(state.text());
    assert!(
        search::highlight_search_matches(&state, &viewport)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        search::highlight_search_matches(&EditorState::new("x"), &viewport),
        Err(EditError::InvalidRange)
    ));
}

#[test]
fn command_windows_edits_and_scroll_targets_match_the_pinned_dispatches() {
    for row in rows("command") {
        let doc = document(&row);
        let mut query = query(&row["query"]);
        let visits = Arc::new(Mutex::new(Vec::new()));
        if row["predicate"] == true {
            let visits = visits.clone();
            query = query.with_test(move |matched, _| {
                visits
                    .lock()
                    .unwrap()
                    .push([matched.range.start, matched.range.end]);
                true
            });
        }
        let mut state =
            EditorState::with_extensions(doc, search::extension(SearchOptions::default())).unwrap();
        let [a, h] = offsets(&row["selection"]);
        state
            .update(
                Transaction::default()
                    .with_selection(EditorSelection::single(a, h))
                    .with_effects([search::set_search_query(query)]),
            )
            .unwrap();
        let outcome =
            action(row["action"].as_str().unwrap()).run(&state, SearchPanelInput::Missing);
        let SearchActionResult::Update(tx) = outcome else {
            panic!("{}: {outcome:?}", row["id"])
        };
        assert_eq!(row["handled"], true);
        let out = &row["out"];
        let selection = tx
            .selection()
            .map(|selection| [selection.main().anchor(), selection.main().head()]);
        let changes: Vec<_> = tx
            .changes()
            .changes()
            .map(|(range, insert)| json!([range.start, range.end, insert]))
            .collect();
        let speak: Vec<_> = tx
            .effects()
            .iter()
            .filter_map(|effect| effect.get(&ANNOUNCE))
            .collect();
        let scroll = tx
            .effects()
            .iter()
            .find_map(|effect| effect.get(&SCROLL_INTO_VIEW));
        let scroll = scroll.map(|s| {
            let x = format!("{:?}", s.x).to_lowercase();
            let y = format!("{:?}", s.y).to_lowercase();
            json!([
                s.range.anchor(),
                s.range.head(),
                x,
                y,
                s.x_margin,
                s.y_margin
            ])
        });
        assert_eq!(
            json!(*visits.lock().unwrap()),
            row["visits"],
            "{}",
            row["id"]
        );
        assert_eq!(json!(selection), out["selection"], "{}", row["id"]);
        assert_eq!(json!(changes), out["changes"], "{}", row["id"]);
        assert_eq!(json!(speak), out["speak"], "{}", row["id"]);
        if let Some(scroll) = scroll {
            for i in 0..6 {
                if i < 4 {
                    assert_eq!(scroll[i], out["scroll"][i], "{}", row["id"]);
                } else {
                    assert_eq!(
                        scroll[i].as_f64(),
                        out["scroll"][i].as_f64(),
                        "{}",
                        row["id"]
                    );
                }
            }
        } else {
            assert!(out["scroll"].is_null(), "{}", row["id"]);
        }
        assert_eq!(
            json!(tx.scroll_into_view()),
            out["scrollFlag"],
            "{}",
            row["id"]
        );
        assert_eq!(json!(tx.user_event()), out["event"], "{}", row["id"]);
        state.update(tx).unwrap();
        if let Some(text) = out["text"].as_str() {
            assert_eq!(state.text().as_str(), text, "{}", row["id"]);
        }
    }
}

#[test]
fn default_search_keymap_matches_pinned_scopes_and_actions() {
    let row = rows("keymap").next().unwrap();
    let expected = row["bindings"].as_array().unwrap();
    let bindings = search::search_keymap();
    assert_eq!(bindings.len(), expected.len());
    for (binding, expected) in bindings.iter().zip(expected) {
        assert_eq!(binding.key.as_deref(), expected[0].as_str());
        assert_eq!(binding.run, expected[1].as_str().map(action));
        assert_eq!(binding.shift, expected[2].as_str().map(action));
        assert_eq!(binding.scope, expected[3].as_str().unwrap());
        assert_eq!(binding.prevent_default, expected[4] == true);
        assert_eq!(binding.stop_propagation, expected[5] == true);
    }
}
