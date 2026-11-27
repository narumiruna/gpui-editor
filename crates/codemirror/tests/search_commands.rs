//! Captured actual @codemirror/search 6.7.3 commands, state 6.7.6, and
//! view 6.44.0 in a scratch JSDOM EditorView. All archive SHA-512 values
//! match the manifest. Positions use UTF-8 lengths of the reference slices.

use std::sync::Arc;

use gpui_editor_codemirror::{
    EditorSelection, EditorState, Extension, PHRASES, PhraseMap, READ_ONLY, Transaction,
    search::{self, SearchOptions, SearchQuery},
    view::{ANNOUNCE, SCROLL_INTO_VIEW, ScrollTarget},
};
use serde_json::{Value, json};

#[test]
fn native_find_and_replace_transactions_match_the_pinned_dispatches() {
    let fixture = include_str!("fixtures/search_commands.jsonl");
    for line in fixture.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["id"].as_str().unwrap();
        let doc = row["doc"].as_str().unwrap();
        let query = &row["query"];
        let query = SearchQuery::new(
            query["search"].as_str().unwrap(),
            SearchOptions {
                regexp: query["regexp"] == true,
                ..Default::default()
            },
        )
        .unwrap()
        .with_replacement(query["replace"].as_str().unwrap_or(""));
        let phrases: PhraseMap = serde_json::from_value(row["phrases"].clone()).unwrap();
        let mut state = EditorState::with_extensions(
            doc,
            Extension::group([
                search::extension(SearchOptions::default()),
                PHRASES.of(Arc::new(phrases)),
                READ_ONLY.of(row["readOnly"] == true),
            ]),
        )
        .unwrap();
        let [a, h]: [usize; 2] = serde_json::from_value(row["before"].clone()).unwrap();
        state
            .update(
                Transaction::default()
                    .with_selection(EditorSelection::single(a, h))
                    .with_effects([search::set_search_query(query)]),
            )
            .unwrap();
        let result = match name {
            "previous" => search::find_previous(&state),
            "replace-all" | "replace-all-readonly" => search::replace_all(&state),
            name if name.starts_with("replace-") => search::replace_next(&state),
            _ => search::find_next(&state),
        };
        assert_eq!(result.is_some(), row["handled"] == true, "{name}");
        let Some(tx) = result else { continue };
        let selections = tx
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
            .filter_map(|effect| effect.get(&ANNOUNCE).cloned())
            .collect();
        let scroll = tx
            .effects()
            .iter()
            .find_map(|effect| effect.get(&SCROLL_INTO_VIEW));
        assert!(!tx.scroll_into_view(), "{name}");
        if let Some(scroll) = scroll {
            assert_eq!(
                *scroll,
                ScrollTarget::new(tx.selection().unwrap().main()),
                "{name}"
            );
        }
        let scroll = scroll.is_some();
        let event = tx.user_event().map(str::to_owned);
        state.update(tx).unwrap();
        assert_eq!(
            json!({"text": state.text().as_str(), "selected": selections, "changes": changes, "speak": speak, "scroll": scroll, "event": event}),
            row["result"],
            "{name}"
        );
    }
}

#[test]
fn unavailable_queries_yield_no_headless_command() {
    for state in [
        EditorState::new("x"),
        EditorState::with_extensions("x", search::extension(SearchOptions::default())).unwrap(),
    ] {
        assert!(search::find_next(&state).is_none());
        assert!(search::find_previous(&state).is_none());
        assert!(search::replace_next(&state).is_none());
        assert!(search::replace_all(&state).is_none());
    }
}
