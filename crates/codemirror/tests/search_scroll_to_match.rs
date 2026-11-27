//! Browser differential from SHA-512-verified search 6.7.3, state 6.7.6,
//! and view 6.44.0. See `fixtures/search_scroll_to_match.mjs`.

use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    ChangePermission, Compartment, EditorSelection, EditorState, Extension, Precedence, READ_ONLY,
    SelectionRange, StateEffectType, Transaction, change_filter,
    search::{
        self, SearchAction, SearchActionResult, SearchOptions, SearchPanelInput, SearchQuery,
    },
    transaction_filter,
    view::{ANNOUNCE, EditorView, SCROLL_INTO_VIEW, ScrollStrategy, ScrollTarget},
};
use serde_json::{Value, json};

fn offsets(range: SelectionRange) -> [usize; 2] {
    [range.anchor(), range.head()]
}

#[test]
fn scroll_to_match_effects_match_the_pinned_browser() {
    for line in include_str!("fixtures/search_scroll_to_match.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let id = row["id"].as_str().unwrap();
        let doc = row["doc"].as_str().unwrap();
        let mut extensions = vec![
            search::extension(SearchOptions::default()),
            READ_ONLY.of(row["readOnly"] == true),
        ];
        let calls = Arc::new(Mutex::new(Vec::new()));
        let marker = StateEffectType::<SelectionRange>::new();
        if let Some(callback) = row["callback"].as_str() {
            let (calls, marker) = (calls.clone(), marker.clone());
            let use_marker = callback == "custom";
            extensions.push(search::scroll_to_match(move |range, start| {
                calls.lock().unwrap().push(json!([
                    start.text().as_str(),
                    offsets(start.selection().main()),
                    offsets(range),
                ]));
                if use_marker {
                    marker.of(range)
                } else {
                    SCROLL_INTO_VIEW.of(ScrollTarget {
                        x: ScrollStrategy::Start,
                        y: ScrollStrategy::Center,
                        x_margin: 12.0,
                        y_margin: 24.0,
                        ..ScrollTarget::new(range)
                    })
                }
            }));
        }
        match row["filter"].as_str() {
            Some("reject") => extensions.push(change_filter(|_, tx| {
                if tx.changes().is_empty() {
                    ChangePermission::Allow
                } else {
                    ChangePermission::Reject
                }
            })),
            Some("drop") => extensions.push(transaction_filter(|_, tx| {
                if tx.is_user_event("select.search") {
                    Ok(Transaction::default()
                        .with_selection(tx.selection().unwrap().clone())
                        .with_user_event("select.search")
                        .with_effects(
                            tx.effects()
                                .iter()
                                .filter(|effect| effect.get(&ANNOUNCE).is_some())
                                .cloned(),
                        ))
                } else {
                    Ok(tx)
                }
            })),
            None => {}
            other => panic!("unknown filter {other:?}"),
        }
        let query = SearchQuery::new("hit", SearchOptions::default())
            .unwrap()
            .with_replacement(row["replacement"].as_str().unwrap());
        let mut state = EditorState::with_extensions(doc, Extension::group(extensions)).unwrap();
        let [anchor, head]: [usize; 2] = serde_json::from_value(row["before"].clone()).unwrap();
        state
            .update(
                Transaction::default()
                    .with_selection(EditorSelection::single(anchor, head))
                    .with_effects([search::set_search_query(query)]),
            )
            .unwrap();
        let mut view = EditorView::new(state);
        let action = match row["command"].as_str().unwrap() {
            "findNext" => SearchAction::FindNext,
            "findPrevious" => SearchAction::FindPrevious,
            "replaceNext" => SearchAction::ReplaceNext,
            "replaceAll" => SearchAction::ReplaceAll,
            other => panic!("unknown command {other}"),
        };
        let result = action.run(view.state(), SearchPanelInput::Missing);
        let expected = &row["expected"];
        if expected["handled"] == false {
            assert!(matches!(result, SearchActionResult::Unhandled), "{id}");
            assert!(view.take_scroll_request().is_none(), "{id}");
        } else {
            let SearchActionResult::Update(tx) = result else {
                panic!("{id}: {result:?}");
            };
            let update = &expected["update"];
            let mut scroll = None;
            view.update_checked(
                tx,
                (),
                |tx| {
                    let changes: Vec<_> = tx
                        .changes()
                        .changes()
                        .map(|(range, insert)| json!([range.start, range.end, insert]))
                        .collect();
                    assert_eq!(json!(changes), update["changes"], "{id}");
                    assert_eq!(
                        json!(tx.selection().map(|selection| offsets(selection.main()))),
                        update["selection"],
                        "{id}"
                    );
                    assert_eq!(json!(tx.user_event()), update["event"], "{id}");
                    assert_eq!(json!(tx.scroll_into_view()), update["scrollFlag"], "{id}");
                    let effects = update["effects"].as_array().unwrap();
                    assert_eq!(tx.effects().len(), effects.len(), "{id}");
                    for (actual, expected) in tx.effects().iter().zip(effects) {
                        match expected[0].as_str().unwrap() {
                            "announce" => {
                                assert_eq!(json!(actual.get(&ANNOUNCE)), expected[1], "{id}")
                            }
                            "custom" => assert_eq!(
                                json!(actual.get(&marker).copied().map(offsets)),
                                expected[1],
                                "{id}"
                            ),
                            "scroll" => {
                                let target = *actual.get(&SCROLL_INTO_VIEW).expect(id);
                                assert_eq!(json!(offsets(target.range)), expected[1], "{id}");
                                assert_eq!(
                                    format!("{:?}", target.x).to_lowercase(),
                                    expected[2],
                                    "{id}"
                                );
                                assert_eq!(
                                    format!("{:?}", target.y).to_lowercase(),
                                    expected[3],
                                    "{id}"
                                );
                                assert_eq!(target.x_margin, expected[4].as_f64().unwrap(), "{id}");
                                assert_eq!(target.y_margin, expected[5].as_f64().unwrap(), "{id}");
                                scroll = Some(target);
                            }
                            other => panic!("unknown effect {other}"),
                        }
                    }
                    Ok(())
                },
                |_| (None, ()),
            )
            .unwrap();
            assert_eq!(view.state().text().as_str(), update["doc"], "{id}");
            assert_eq!(view.take_scroll_target(), scroll, "{id}");
        }
        assert_eq!(json!(*calls.lock().unwrap()), expected["calls"], "{id}");
    }
}

#[test]
fn native_callback_precedence_and_reconfiguration_preserve_query_state() {
    let scroll = Compartment::new();
    let custom = |y| {
        search::scroll_to_match(move |range, _| {
            SCROLL_INTO_VIEW.of(ScrollTarget {
                y,
                ..ScrollTarget::new(range)
            })
        })
    };
    let mut state = EditorState::with_extensions(
        "hit hit",
        Extension::group([
            search::extension(SearchOptions::default()),
            scroll.of(Extension::group([
                custom(ScrollStrategy::Start),
                custom(ScrollStrategy::End).with_precedence(Precedence::High),
            ])),
        ]),
    )
    .unwrap();
    let query = SearchQuery::new("hit", SearchOptions::default()).unwrap();
    state
        .update(Transaction::default().with_effects([search::set_search_query(query.clone())]))
        .unwrap();
    let strategy = |state: &EditorState| {
        search::find_next(state).unwrap().effects()[1]
            .get(&SCROLL_INTO_VIEW)
            .unwrap()
            .y
    };
    assert_eq!(strategy(&state), ScrollStrategy::End);
    for (extension, expected) in [
        (custom(ScrollStrategy::Center), ScrollStrategy::Center),
        (Extension::default(), ScrollStrategy::Nearest),
    ] {
        state
            .update(Transaction::default().with_effects([scroll.reconfigure(extension)]))
            .unwrap();
        assert_eq!(strategy(&state), expected);
        assert_eq!(search::get_search_query(&state), query);
    }
}
