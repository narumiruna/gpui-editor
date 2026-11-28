use std::time::Duration;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, EditorSelection, EditorState, READ_ONLY, Transaction,
    autocomplete::session::{
        CompletionConfig, CompletionItem, CompletionQuery, CompletionSession, CompletionStatus,
        CompletionStep, ListSource, SourceId, SourceResult,
    },
};

const A: SourceId = SourceId(1);
const B: SourceId = SourceId(2);

fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn candidates(from: usize, labels: &[&str]) -> SourceResult {
    SourceResult::new(
        from,
        labels
            .iter()
            .map(|label| CompletionItem::new(*label))
            .collect::<Vec<_>>(),
    )
}

fn edit(
    session: &mut CompletionSession,
    state: &mut EditorState,
    range: std::ops::Range<usize>,
    text: &str,
    head: Option<usize>,
    event: Option<&str>,
    time: u64,
) {
    let changes = ChangeSet::single(Change::new(range, text)).unwrap();
    let desc = changes.desc(state.text().len()).unwrap();
    let mut transaction = Transaction::new(changes);
    if let Some(head) = head {
        transaction = transaction.with_selection(EditorSelection::single(head, head));
    }
    if let Some(event) = event {
        transaction = transaction.with_user_event(event);
    }
    state.update(transaction).unwrap();
    session
        .update(state, &desc, head.is_some(), event, at(time))
        .unwrap();
}

#[test]
fn typing_debounces_and_reuses_valid_late_results_across_edits() {
    let mut state = EditorState::new("");
    let mut completion = CompletionSession::new(&state, CompletionConfig::default(), [A]);
    edit(
        &mut completion,
        &mut state,
        0..0,
        "f",
        Some(1),
        Some("input.type"),
        1,
    );
    assert_eq!(completion.deadline(), Some(at(101)));
    assert!(completion.take_queries(at(100)).is_empty());
    let [query] = completion.take_queries(at(101)).try_into().unwrap();
    assert_eq!(query.source(), A);
    assert!(!query.explicit);
    assert_eq!(query.position, 1);
    assert_eq!(query.state().text().as_str(), "f");
    edit(
        &mut completion,
        &mut state,
        1..1,
        "o",
        Some(2),
        Some("input.type.compose"),
        120,
    );
    let result = candidates(0, &["foo", "far", "word"])
        .with_valid_for_regex(r"f(?:o*)")
        .unwrap();
    assert!(
        completion
            .finish_query(&query, Some(result), at(130))
            .unwrap()
    );
    assert_eq!(completion.status(), CompletionStatus::Active);
    let options = completion.options().unwrap();
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].item.label, "foo");
    assert_eq!(options[0].range, 0..2);
    let id = options[0].id;
    assert!(completion.accept(id, at(204)).unwrap().is_none());
    let mut picked = state.clone();
    let tx = completion.accept(id, at(205)).unwrap().unwrap();
    assert!(tx.is_user_event("input.complete"));
    picked.update(tx).unwrap();
    assert_eq!(picked.text().as_str(), "foo");

    edit(
        &mut completion,
        &mut state,
        1..2,
        "",
        Some(1),
        Some("delete.backward"),
        210,
    );
    assert!(
        completion
            .options()
            .unwrap()
            .iter()
            .any(|option| option.id == id && option.range == (0..1))
    );
    edit(
        &mut completion,
        &mut state,
        0..1,
        "",
        Some(0),
        Some("delete.backward"),
        220,
    );
    assert_eq!(completion.status(), CompletionStatus::Inactive);
    assert!(completion.accept(id, at(500)).unwrap().is_none());
}

#[test]
fn explicit_start_partial_results_and_commit_chars_follow_provider_order() {
    let mut state = EditorState::new("");
    let mut completion = CompletionSession::new(&state, CompletionConfig::default(), [A, B, A]);
    completion.start(true, at(1));
    assert!(completion.take_queries(at(50)).is_empty());
    let [first, second] = completion.take_queries(at(51)).try_into().unwrap();
    assert_eq!([first.source(), second.source()], [A, B]);
    assert!(first.explicit);
    let mut on = CompletionItem::new("foo");
    on.commit_characters = Some(vec![';']);
    let mut response = SourceResult::new(0, vec![on]);
    response.commit_characters = vec!['('];
    response.filter = false;
    assert!(
        completion
            .finish_query(&first, Some(response), at(55))
            .unwrap()
    );
    assert_eq!(completion.deadline(), Some(at(155)));
    assert!(completion.options().unwrap().is_empty());
    completion.take_queries(at(154));
    assert!(completion.options().unwrap().is_empty());
    completion.take_queries(at(155));
    assert_eq!(completion.status(), CompletionStatus::Pending);
    let id = completion.options().unwrap()[0].id;
    assert!(completion.commit(id, '(').unwrap().is_none());
    assert!(completion.commit(id, '💻').unwrap().is_none());
    let tx = completion.commit(id, ';').unwrap().unwrap();
    state.update(tx).unwrap();
    assert_eq!(state.text().as_str(), "foo");
    assert!(
        completion
            .finish_query(&second, Some(candidates(0, &["bar"])), at(156))
            .unwrap()
    );
    assert_eq!(completion.status(), CompletionStatus::Active);
    assert_eq!(
        completion
            .options()
            .unwrap()
            .iter()
            .map(|option| option.item.label.as_str())
            .collect::<Vec<_>>(),
        ["foo", "bar"]
    );
}

#[test]
fn change_abort_selection_motion_close_and_reconfiguration_drop_old_responses() {
    let mut state = EditorState::new("a");
    let mut session = CompletionSession::new(&state, CompletionConfig::default(), [A]);
    session.start(true, at(0));
    let first = session.take_queries(at(50)).pop().unwrap();
    first.abort_on_document_change();
    edit(
        &mut session,
        &mut state,
        1..1,
        "b",
        Some(2),
        Some("input.type"),
        80,
    );
    assert!(first.is_cancelled());
    assert!(
        !session
            .finish_query(&first, Some(candidates(0, &["abc"])), at(90))
            .unwrap()
    );
    let second = session.take_queries(at(130)).pop().unwrap();
    edit(
        &mut session,
        &mut state,
        0..0,
        "",
        Some(1),
        Some("select"),
        140,
    );
    assert!(second.is_cancelled());
    assert_eq!(session.status(), CompletionStatus::Inactive);
    session.start(true, at(140));
    let third = session.take_queries(at(190)).pop().unwrap();
    session.set_sources([B], at(192));
    assert!(third.is_cancelled());
    assert!(!session.finish_query(&third, None, at(194)).unwrap());
    let fourth = session.take_queries(at(292)).pop().unwrap();
    assert_eq!(fourth.source(), B);
    session.close();
    assert!(fourth.is_cancelled());
    assert!(session.deadline().is_none());
}

#[test]
fn result_callbacks_remap_and_update_options_without_reusing_old_option_ids() {
    let mut state = EditorState::new("./Ä");
    let mut session = CompletionSession::new(&state, CompletionConfig::default(), [A]);
    edit(
        &mut session,
        &mut state,
        4..4,
        "",
        Some(4),
        Some("select"),
        0,
    );
    session.start(true, at(0));
    let query = session.take_queries(at(50)).pop().unwrap();
    let original = candidates(2, &["Äther"])
        .with_map(|result, change| {
            let mut mapped = result.clone();
            mapped.from = change.map_pos(
                mapped.from,
                gpui_editor_codemirror::Assoc::Before,
                gpui_editor_codemirror::MapMode::Simple,
            )?;
            Some(mapped)
        })
        .with_update(|result, range, state| {
            assert_eq!(result.from, range.start);
            assert_eq!(state.text().slice(range.clone()), Some("Äb"));
            Some(candidates(range.start, &["Äbc"]))
        });
    session
        .finish_query(&query, Some(original), at(60))
        .unwrap();
    let id = session.options().unwrap()[0].id;
    edit(
        &mut session,
        &mut state,
        0..0,
        "pré",
        None,
        Some("input.paste"),
        80,
    );
    assert_eq!(session.options().unwrap()[0].range, 6..8);
    assert_eq!(session.options().unwrap()[0].id, id);
    edit(
        &mut session,
        &mut state,
        8..8,
        "b",
        Some(9),
        Some("input.type"),
        90,
    );
    let option = &session.options().unwrap()[0];
    assert_eq!(option.item.label, "Äbc");
    assert_ne!(option.id, id);
    assert!(session.accept(id, at(500)).unwrap().is_none());
}

#[test]
fn malformed_or_foreign_results_cannot_target_the_current_state() {
    let state = EditorState::new("字");
    let mut session = CompletionSession::new(&state, CompletionConfig::default(), [A]);
    let mut foreign = CompletionSession::new(&state, CompletionConfig::default(), [A]);
    session.start(true, at(0));
    foreign.start(true, at(0));
    let query = session.take_queries(at(50)).pop().unwrap();
    let other = foreign.take_queries(at(50)).pop().unwrap();
    assert_eq!(
        session.finish_query(&other, Some(candidates(0, &["x"])), at(55)),
        Ok(false)
    );
    assert_eq!(
        session.finish_query(&query, Some(candidates(1, &["x"])), at(60)),
        Err(EditError::InvalidRange)
    );
    assert_eq!(session.status(), CompletionStatus::Inactive);
}

fn query_for(document: &str, explicit: bool) -> (CompletionSession, CompletionQuery) {
    query_at(document, document.len(), explicit)
}

fn query_at(
    document: &str,
    position: usize,
    explicit: bool,
) -> (CompletionSession, CompletionQuery) {
    let mut state = EditorState::new(document);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(position, position)))
        .unwrap();
    let mut session = CompletionSession::new(&state, CompletionConfig::default(), [A]);
    session.start(explicit, at(0));
    let query = session.take_queries(at(100)).pop().unwrap();
    (session, query)
}

#[test]
fn query_match_before_is_confined_to_the_last_250_units_of_one_line() {
    let document = format!("prior\r\n{}tail", "💻".repeat(130));
    let (_session, query) = query_for(&document, true);
    let suffix = query.match_before(".*").unwrap().unwrap();
    assert_eq!(suffix.range.end, document.len());
    assert_eq!(suffix.text.encode_utf16().count(), 250);
    assert_eq!(suffix.text, &document[suffix.range]);
    let word = query.match_before("[a-z]+").unwrap().unwrap();
    assert_eq!(word.text, "tail");
}

#[test]
fn fixed_list_uses_ascii_words_and_punctuation_prefixes() {
    let word = ListSource::new(vec![
        CompletionItem::new("alpha"),
        CompletionItem::new("allow"),
    ])
    .unwrap();
    let (_session, query) = query_for("é al", false);
    let response = word.complete(&query).unwrap();
    assert_eq!(response.from, 3);
    assert_eq!(response.options.len(), 2);
    let (_session, query) = query_for("é", false);
    assert!(word.complete(&query).is_none());
    let (_session, query) = query_for("é", true);
    assert_eq!(word.complete(&query).unwrap().from, 2);
    let symbolic = ListSource::new(vec![
        CompletionItem::new("@block"),
        CompletionItem::new("!note"),
    ])
    .unwrap();
    let (_session, query) = query_for("a = @bl", false);
    assert_eq!(symbolic.complete(&query).unwrap().from, 4);
}

#[test]
fn fixed_list_replays_empty_label_prefixes_and_picks() {
    use serde_json::Value;

    for row in include_str!("fixtures/autocomplete_list_prefix.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let items = |value: &Value| {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|option| {
                    let label = option
                        .as_str()
                        .or_else(|| option["label"].as_str())
                        .unwrap();
                    let mut item = CompletionItem::new(label);
                    item.display_label = option["displayLabel"].as_str().map(str::to_owned);
                    item.insert = option["apply"].as_str().map(str::to_owned);
                    item
                })
                .collect::<Vec<_>>()
        };
        let byte = |value: &Value| value.as_u64().map(|at| usize::try_from(at).unwrap());
        let (mut session, query) = query_at(
            row["source"].as_str().unwrap(),
            byte(&row["at"]).unwrap(),
            row["explicit"].as_bool().unwrap(),
        );
        let actual = ListSource::new(items(&row["options"]))
            .unwrap()
            .complete(&query);
        let expected = &row["result"];
        let Some(result) = actual else {
            assert!(expected.is_null(), "{row}");
            continue;
        };
        assert_eq!(Some(result.from), byte(&expected["from"]), "{row}");
        assert_eq!(result.to, byte(&expected["to"]), "{row}");
        assert_eq!(&*result.options, items(&expected["options"]), "{row}");
        if let Some(pick) = row["pick"].as_str() {
            let mut state = query.state().clone();
            assert!(session.finish_query(&query, Some(result), at(100)).unwrap());
            let id = session
                .options()
                .unwrap()
                .into_iter()
                .find(|entry| entry.item.label == pick)
                .unwrap()
                .id;
            let tx = session.accept(id, at(200)).unwrap().unwrap();
            state.update(tx).unwrap();
            assert_eq!(
                state.text().as_str(),
                row["edit"].as_str().unwrap(),
                "{row}"
            );

            let mut state = query.state().clone();
            edit(
                &mut session,
                &mut state,
                query.position..query.position,
                "t",
                None,
                Some("input.type"),
                201,
            );
            assert_eq!(session.status(), CompletionStatus::Active);
            assert_eq!(session.deadline(), None);
            let options = session.options().unwrap();
            assert_eq!(options[0].item.label, pick);
            assert_eq!(
                options[0].range,
                byte(&expected["from"]).unwrap()..query.position + 1
            );
        }
    }
}

#[test]
fn a_current_provider_error_closes_the_batch_but_late_errors_do_not() {
    let state = EditorState::new("");
    let mut session = CompletionSession::new(&state, CompletionConfig::default(), [A, B]);
    session.start(true, at(0));
    let [first, second] = session.take_queries(at(50)).try_into().unwrap();
    assert!(session.fail_query(&first));
    assert!(second.is_cancelled());
    assert_eq!(session.status(), CompletionStatus::Inactive);
    session.start(true, at(55));
    assert!(!session.fail_query(&second));
    assert_eq!(session.take_queries(at(105)).len(), 2);
}

#[test]
fn explicit_list_selection_waits_wraps_and_clamps_pages() {
    let state = EditorState::new("");
    let config = CompletionConfig {
        select_on_open: false,
        ..Default::default()
    };
    let mut completion = CompletionSession::new(&state, config, [A]);
    completion.start(true, at(0));
    let query = completion.take_queries(at(50)).pop().unwrap();
    completion
        .finish_query(
            &query,
            Some(candidates(0, &["alpha", "beta", "gamma"])),
            at(60),
        )
        .unwrap();
    assert_eq!(completion.selected(), None);
    assert!(
        !completion
            .move_selection(false, CompletionStep::Option, at(134))
            .unwrap()
    );
    assert!(
        completion
            .move_selection(false, CompletionStep::Option, at(135))
            .unwrap()
    );
    let id = |session: &CompletionSession, at: usize| session.options().unwrap()[at].id;
    assert_eq!(completion.selected(), Some(id(&completion, 2)));
    completion
        .move_selection(true, CompletionStep::Option, at(136))
        .unwrap();
    assert_eq!(completion.selected(), Some(id(&completion, 0)));
    completion
        .move_selection(false, CompletionStep::Page(10), at(137))
        .unwrap();
    assert_eq!(completion.selected(), Some(id(&completion, 0)));
    completion
        .move_selection(true, CompletionStep::Page(10), at(138))
        .unwrap();
    assert_eq!(completion.selected(), Some(id(&completion, 2)));
    let selected = completion.accept_selected(at(139)).unwrap().unwrap();
    let mut result = state;
    result.update(selected).unwrap();
    assert_eq!(result.text().as_str(), "gamma");
}

#[test]
fn custom_accept_uses_only_current_options_after_the_interaction_delay() {
    let mut state = EditorState::new("a");
    state
        .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
        .unwrap();
    let mut session = CompletionSession::new(&state, CompletionConfig::default(), [A]);
    session.start(true, at(0));
    let query = session.take_queries(at(50)).pop().unwrap();
    session
        .finish_query(&query, Some(candidates(0, &["alpha"])), at(50))
        .unwrap();
    let id = session.options().unwrap()[0].id;
    assert_eq!(
        session
            .accept_with(id, at(124), |_, _| -> Result<(), _> { panic!("too soon") })
            .unwrap(),
        None
    );
    let result = session
        .accept_with(id, at(125), |state, option| {
            assert_eq!(state.text().as_str(), "a");
            assert_eq!(id.source(), A);
            Ok((option.range, option.item.label.clone()))
        })
        .unwrap();
    assert_eq!(result, Some((0..1, "alpha".into())));
    session.close();
    assert_eq!(
        session
            .accept_with(id, at(600), |_, _| -> Result<(), _> {
                panic!("closed selection")
            })
            .unwrap(),
        None
    );
}

#[test]
fn read_only_state_never_accepts_or_commits() {
    let locked = EditorState::with_extensions("", READ_ONLY.of(true)).unwrap();
    let mut session = CompletionSession::new(&locked, CompletionConfig::default(), [A]);
    session.start(true, at(0));
    let query = session.take_queries(at(50)).pop().unwrap();
    let mut result = candidates(0, &["foo"]);
    result.commit_characters = vec!['('];
    session.finish_query(&query, Some(result), at(60)).unwrap();
    let id = session.options().unwrap()[0].id;
    assert!(session.accept(id, at(500)).unwrap().is_none());
    assert!(session.commit(id, '(').unwrap().is_none());
    assert_eq!(
        session
            .accept_with(id, at(500), |_, _| -> Result<(), _> { panic!("read-only") })
            .unwrap(),
        None
    );
}
