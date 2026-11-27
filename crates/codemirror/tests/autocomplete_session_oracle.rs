use std::time::Duration;

use gpui_editor_codemirror::{
    Change, ChangePermission, ChangeSet, EditError, EditorSelection, EditorState, Extension,
    READ_ONLY, StateEffectType, Transaction,
    autocomplete::{
        SectionRank, Snippet, insert_completion_text,
        session::{
            CompletionConfig, CompletionItem, CompletionQuery, CompletionSession, CompletionStatus,
            SourceId, SourceResult, activate_on_completion, compare_completion_ties,
            picked_completion,
        },
        start_snippet,
    },
    change_filter, transaction_filter,
    view::EditorView,
};
use serde_json::{Value, json};

const A: SourceId = SourceId(1);
const B: SourceId = SourceId(2);
const C: SourceId = SourceId(3);

#[test]
fn state_reactivation_options_allow_reusing_one_provider_but_conflict_atomically() {
    let provider = activate_on_completion(|_| true);
    let mut state =
        EditorState::with_extensions("a", Extension::group([provider.clone(), provider])).unwrap();
    let conflict = activate_on_completion(|_| false);
    assert_eq!(
        state.update(Transaction::default().with_effects([conflict.append()])),
        Err(EditError::Configuration(
            gpui_editor_codemirror::ConfigurationError::InvalidFieldData
        )),
    );
    assert_eq!(state.text().as_str(), "a");
}

fn ms(time: u64) -> Duration {
    Duration::from_millis(time)
}

fn row(name: &str) -> Value {
    include_str!("fixtures/autocomplete_session.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|row| row["case"] == name)
        .expect("case in pinned browser fixture")
}

fn snapshot(query: &CompletionQuery) -> Value {
    json!([
        query.state().text().as_str(),
        query.position,
        query.explicit
    ])
}

fn labels(session: &CompletionSession) -> Vec<String> {
    session
        .options()
        .unwrap()
        .into_iter()
        .map(|option| {
            option
                .item
                .display_label
                .as_ref()
                .unwrap_or(&option.item.label)
                .clone()
        })
        .collect()
}

fn read(session: &CompletionSession) -> [Value; 3] {
    [
        match session.status() {
            CompletionStatus::Inactive => Value::Null,
            CompletionStatus::Pending => json!("pending"),
            CompletionStatus::Active => json!("active"),
        },
        json!(labels(session)),
        json!(
            session
                .selected_completion()
                .unwrap()
                .map(|o| &o.item.label)
        ),
    ]
}

fn mount(extension: Extension) -> EditorView<()> {
    let mut state = EditorState::with_extensions("é foo", extension).unwrap();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(6, 6)))
        .unwrap();
    EditorView::new(state)
}

fn session(
    view: &EditorView<()>,
    sources: impl IntoIterator<Item = SourceId>,
) -> CompletionSession {
    let config = CompletionConfig {
        activate_on_typing: false,
        typing_delay: ms(25),
        update_sync_time: ms(10),
        interaction_delay: ms(75),
        select_on_open: true,
        match_mode: Default::default(),
    };
    let mut session = CompletionSession::new(view.state(), config, sources);
    session.start(true, ms(0));
    session
}

fn apply(
    view: &mut EditorView<()>,
    session: &mut CompletionSession,
    proposal: &Transaction,
    now: Duration,
) -> Result<(), EditError> {
    let mut accepted = None;
    view.update_checked(
        proposal.clone(),
        (),
        |resolved| {
            accepted = Some(resolved.clone());
            Ok(())
        },
        |_| (None, ()),
    )?;
    session
        .update_after_apply(view.state(), proposal, &accepted.unwrap(), now)
        .map(|_| ())
}

fn pick() -> CompletionItem {
    let mut item = CompletionItem::new("foo");
    item.insert = Some("foo.".into());
    item.commit_characters = Some(vec![';']);
    item
}

fn stamp(transaction: &Transaction) -> Value {
    json!([
        transaction.annotation(picked_completion()).unwrap().label,
        transaction.user_event().unwrap(),
    ])
}

#[test]
fn accepted_completion_resets_all_sources_without_reusing_results() {
    let expected = row("chain");
    assert_eq!(expected["reused"], json!([]));
    let mut view = mount(Extension::default());
    let mut session = session(&view, [A, B])
        .with_activate_on_completion(|item| item.insert.as_deref() == Some("foo."));
    let [first, slow] = session.take_queries(ms(50)).try_into().unwrap();
    let result = SourceResult::new(3, vec![pick()])
        .with_valid_for(|_, _, _| panic!("reactivation must not call validFor"))
        .with_map(|_, _| panic!("reactivation must not map the result"))
        .with_update(|_, _, _| panic!("reactivation must not update the result"));
    session.finish_query(&first, Some(result), ms(50)).unwrap();
    assert_eq!(session.deadline(), Some(ms(60)));
    assert!(session.take_queries(ms(60)).is_empty());
    let id = session.selected().unwrap();
    let early = session.accept_selected(ms(134)).unwrap().is_some();
    let tx = session.accept_selected(ms(135)).unwrap();
    assert_eq!(json!(tx.is_some()), expected["accepted"]);
    let tx = tx.unwrap();
    apply(&mut view, &mut session, &tx, ms(135)).unwrap();
    assert_eq!(json!(early), expected["early"]);
    assert_eq!(json!(read(&session)), expected["pending"]);
    assert_eq!(json!(slow.is_cancelled()), expected["aborted"]);
    assert_eq!(
        json!(session.accept(id, ms(135)).unwrap().is_some()),
        expected["acceptedWhilePending"]
    );
    assert!(session.take_queries(ms(159)).is_empty());
    assert_eq!(session.deadline(), Some(ms(160)));
    let [next, other] = session.take_queries(ms(160)).try_into().unwrap();
    assert_eq!([next.source(), other.source()], [A, B]);
    assert_eq!(
        json!([snapshot(&first), snapshot(&next)]),
        expected["primary"]
    );
    assert_eq!(
        json!([snapshot(&slow), snapshot(&other)]),
        expected["secondary"]
    );
    let late = SourceResult::new(3, vec![CompletionItem::new("foo late")]);
    assert!(!session.finish_query(&slow, Some(late), ms(160)).unwrap());
    let result = SourceResult::new(next.position, vec![CompletionItem::new("bar")]);
    session.finish_query(&next, Some(result), ms(160)).unwrap();
    session.finish_query(&other, None, ms(160)).unwrap();
    assert_eq!(json!(read(&session)), expected["ready"]);
    assert!(session.accept(id, ms(160)).unwrap().is_none());
    let follow = session.accept_selected(ms(160)).unwrap();
    assert_eq!(json!(follow.is_some()), expected["nextAccepted"]);
    let follow = follow.unwrap();
    assert_eq!(json!([stamp(&tx), stamp(&follow)]), expected["picks"]);
    apply(&mut view, &mut session, &follow, ms(160)).unwrap();
    let [status, labels, selected] = read(&session);
    assert_eq!(
        json!([view.state().text().as_str(), status, labels, selected]),
        expected["final"]
    );
    assert!(session.deadline().is_none());
}

#[test]
fn pick_annotations_and_input_gates_match_the_browser() {
    for mode in [
        "event_only",
        "annotation_only",
        "subtype",
        "snippet",
        "commit",
        "pointer",
        "no_dispatch",
        "read_only",
    ] {
        let expected = row(mode);
        let mut view = mount(READ_ONLY.of(mode == "read_only"));
        let mut session = session(&view, [A]).with_activate_on_completion(|_| true);
        let first = session.take_queries(ms(50)).pop().unwrap();
        session
            .finish_query(&first, Some(SourceResult::new(3, vec![pick()])), ms(50))
            .unwrap();
        let id = session.selected().unwrap();
        let tx = match mode {
            "event_only" | "annotation_only" | "subtype" => {
                let changes = ChangeSet::single(Change::new(6..6, ".")).unwrap();
                let tx = Transaction::new(changes)
                    .with_selection(EditorSelection::single(7, 7))
                    .with_user_event(if mode == "annotation_only" {
                        "input.custom"
                    } else {
                        "input.complete.subtype"
                    });
                Some(if mode == "event_only" {
                    tx
                } else {
                    tx.with_annotation(picked_completion().of(pick()))
                })
            }
            "snippet" => session
                .accept_with(id, ms(125), |state, option| {
                    let (mut tx, snippet) =
                        Snippet::parse("${field}").insert(state, option.range, "  ")?;
                    if let Some(snippet) = snippet {
                        tx = tx.with_effects([start_snippet(snippet)]);
                    }
                    Ok(tx
                        .with_user_event("input.complete")
                        .with_annotation(picked_completion().of(option.item.clone())))
                })
                .unwrap(),
            "commit" => session.commit(id, ';').unwrap(),
            "pointer" => {
                assert_eq!(
                    json!(session.accept(id, ms(50)).unwrap().is_some()),
                    expected["early"]
                );
                session
                    .accept_pointer_with(id, |state, option| {
                        let tx = insert_completion_text(
                            state,
                            option.range,
                            option.item.insert.as_deref().unwrap(),
                        )?;
                        Ok(tx.with_annotation(picked_completion().of(option.item.clone())))
                    })
                    .unwrap()
            }
            "no_dispatch" | "read_only" => session.accept_selected(ms(125)).unwrap(),
            _ => unreachable!(),
        };
        assert_eq!(tx.is_none(), mode == "read_only");
        let now = if matches!(mode, "commit" | "pointer") {
            ms(50)
        } else {
            ms(125)
        };
        if mode != "no_dispatch"
            && let Some(tx) = tx
        {
            apply(&mut view, &mut session, &tx, now).unwrap();
        }
        let pending = session.status() == CompletionStatus::Pending;
        assert_eq!(json!(pending), expected["pending"], "{mode}");
        assert!(session.take_queries(now + ms(24)).is_empty(), "{mode}");
        let mut calls = vec![snapshot(&first)];
        for next in session.take_queries(now + ms(25)) {
            calls.push(snapshot(&next));
            session.finish_query(&next, None, now + ms(25)).unwrap();
        }
        assert_eq!(json!(calls), expected["calls"], "{mode}");
        assert_eq!(view.state().text().as_str(), expected["doc"], "{mode}");
    }
}

#[test]
fn filtered_and_failed_completion_application_does_not_activate() {
    let replace = transaction_filter(|_, tx| {
        let Some(item) = tx.annotation(picked_completion()) else {
            return Ok(tx);
        };
        Ok(
            Transaction::new(ChangeSet::single(Change::new(3..6, "foo!"))?)
                .with_selection(tx.selection().unwrap().clone())
                .with_effects(tx.effects().iter().cloned())
                .with_user_event("input.complete")
                .with_annotation(picked_completion().of(item.clone())),
        )
    });
    let remove_effect = transaction_filter(|_, tx| {
        let Some(item) = tx.annotation(picked_completion()) else {
            return Ok(tx);
        };
        Ok(Transaction::new(tx.changes().clone())
            .with_selection(tx.selection().unwrap().clone())
            .with_user_event("input.complete")
            .with_annotation(picked_completion().of(item.clone())))
    });
    let reject = change_filter(|_, tx| {
        if tx.is_user_event("input.complete") {
            ChangePermission::Reject
        } else {
            ChangePermission::Allow
        }
    });
    let fail = transaction_filter(|_, tx| {
        if tx.is_user_event("input.complete") {
            Err(EditError::InvalidRange)
        } else {
            Ok(tx)
        }
    });
    for (extension, expected_doc, expected_result) in [
        (replace, "é foo!", Ok(())),
        (remove_effect, "é foo.", Ok(())),
        (reject, "é foo", Ok(())),
        (fail, "é foo", Err(EditError::InvalidRange)),
    ] {
        let mut view = mount(extension);
        let mut session =
            session(&view, [A]).with_activate_on_completion(|_| panic!("pick was not accepted"));
        let first = session.take_queries(ms(50)).pop().unwrap();
        session
            .finish_query(&first, Some(SourceResult::new(3, vec![pick()])), ms(50))
            .unwrap();
        let tx = session
            .accept_selected(ms(125))
            .unwrap()
            .unwrap()
            .with_effects([StateEffectType::new().of(())]);
        assert_eq!(
            apply(&mut view, &mut session, &tx, ms(125)),
            expected_result
        );
        assert_eq!(view.state().text().as_str(), expected_doc);
        assert!(session.deadline().is_none());
    }
}

#[test]
fn host_comparison_breaks_only_final_score_ties_before_adjacent_deduplication() {
    let compare = |a: &CompletionItem, b: &CompletionItem| b.sort_text.cmp(&a.sort_text);
    let configured = EditorState::with_extensions("", compare_completion_ties(compare)).unwrap();
    let default = EditorState::new("");
    for session in [
        CompletionSession::new(&default, CompletionConfig::default(), [A, B, C])
            .with_compare_completions(compare),
        CompletionSession::new(&configured, CompletionConfig::default(), [A, B, C]),
    ] {
        check_tie_order(session);
    }
}

fn check_tie_order(mut session: CompletionSession) {
    session.start(true, ms(0));
    let [first, second, third] = session.take_queries(ms(50)).try_into().unwrap();
    let item = |label, key: &str, display: Option<&str>| {
        let mut item = CompletionItem::new(label);
        item.sort_text = Some(key.into());
        item.display_label = display.map(str::to_owned);
        item
    };
    let mut boosted = item("boosted", "a", None);
    boosted.boost = 5;
    let mut fixed = item("fixed", "z", None);
    fixed.section = Some(("Section".into(), SectionRank::Fixed(1)));
    let mut last_same = item("same", "a", Some("last same"));
    last_same.info = Some("separate".into());
    let mut rich_value = item("value", "z", Some("rich value"));
    rich_value.info = Some("preferred".into());
    let mut unfiltered =
        SourceResult::new(0, vec![item("pin1", "a", None), item("pin2", "z", None)]);
    unfiltered.filter = false;
    let first_result = SourceResult::new(
        0,
        vec![
            item("same", "z", Some("first same")),
            item("alpha", "m", None),
            boosted,
            fixed,
        ],
    );
    let second_result = SourceResult::new(
        0,
        vec![
            last_same,
            item("value", "z", Some("plain value")),
            rich_value,
            item("beta", "m", None),
        ],
    )
    .with_match_ranges(|option, matches| {
        assert!(matches.is_some());
        if option.display_label.as_deref() == Some("rich value") {
            std::iter::once(0..4).collect()
        } else {
            Vec::new()
        }
    });
    for (query, result) in [
        (&first, first_result),
        (&second, second_result),
        (&third, unfiltered),
    ] {
        session.finish_query(query, Some(result), ms(60)).unwrap();
    }
    assert_eq!(json!(labels(&session)), row("compare")["labels"]);
    let rows = session.options().unwrap();
    assert_eq!(rows[4].id.source(), B);
    assert_eq!(
        rows[4].match_ranges,
        std::iter::once(0..4).collect::<Vec<_>>()
    );
    assert_eq!(
        session.selected_completion().unwrap().unwrap().item.label,
        "pin1"
    );
}
