use std::sync::{Arc, LazyLock, Mutex};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditError, EditorSelection, EditorState, Extension,
    MapMode, Precedence, StateEffect, StateEffectType, Transaction, TransactionCoordinates,
    TransactionExtension, inverted_effects, transaction_extender, transaction_filter,
    view::{
        EditorView, ViewPlugin, ViewUpdate, Viewport, focus_change_effect,
        focus_change_transaction, update_listener,
    },
};
use serde_json::{Value, json};

type Log = Arc<Mutex<Vec<Value>>>;

static FLAG: LazyLock<StateEffectType<bool>> = LazyLock::new(StateEffectType::new);
static TEXT: LazyLock<StateEffectType<String>> = LazyLock::new(StateEffectType::new);
static POINT: LazyLock<StateEffectType<usize>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|pos, changes| {
        changes.map_pos(*pos, Assoc::After, MapMode::Simple)
    })
});
static EXTRA: LazyLock<StateEffectType<&str>> = LazyLock::new(StateEffectType::new);

fn effects(effects: &[StateEffect]) -> Vec<Value> {
    effects
        .iter()
        .map(|effect| {
            if let Some(value) = effect.get(&FLAG) {
                json!(["flag", value])
            } else if let Some(value) = effect.get(&TEXT) {
                json!(["text", value])
            } else if let Some(value) = effect.get(&POINT) {
                json!(["point", value])
            } else if let Some(value) = effect.get(&EXTRA) {
                json!(["extra", value])
            } else {
                json!(["config"])
            }
        })
        .collect()
}

fn oracle(case: &str) -> Vec<Value> {
    include_str!("fixtures/view_focus.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|entry| entry["case"] == case)
        .unwrap()["log"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry[0] != "dispatch")
        .cloned()
        .map(|mut entry| {
            if entry[0] == "plugin" || entry[0] == "listener" {
                // The native ViewUpdate reports accepted effects, not transaction count.
                entry.as_array_mut().unwrap().remove(5);
            }
            entry
        })
        .collect()
}

fn provider(
    log: &Log,
    name: &'static str,
    effect: impl Fn(&EditorState, bool) -> Option<StateEffect> + Send + Sync + 'static,
) -> Extension {
    let log = log.clone();
    focus_change_effect(move |state, focused| {
        log.lock().unwrap().push(json!([
            "provider",
            name,
            state.text().as_str(),
            state.selection().main().head(),
            focused,
        ]));
        effect(state, focused)
    })
}

fn report(log: &Log, kind: &str, update: &ViewUpdate<'_>) {
    log.lock().unwrap().push(json!([
        kind,
        update.start_state.text().as_str(),
        update.state.text().as_str(),
        update.focus_changed,
        update.focused,
        effects(update.effects),
    ]));
}

struct Probe(Log);

impl ViewPlugin for Probe {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        report(&self.0, "plugin", update);
    }
}

fn view(log: &Log, extensions: impl IntoIterator<Item = Extension>) -> EditorView<&'static str> {
    let listener = log.clone();
    let state = EditorState::with_extensions(
        "ab",
        Extension::group(
            extensions
                .into_iter()
                .chain([update_listener(move |update| {
                    report(&listener, "listener", update);
                    Ok(())
                })]),
        ),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    view.add_plugin(Probe(log.clone()));
    view
}

#[test]
fn focus_and_blur_dispatch_one_ordered_typed_update_per_edge() {
    let log = Log::default();
    let text = provider(&log, "text", |state, focused| {
        Some(TEXT.of(format!("{}:{focused}", state.text().as_str())))
    });
    let mut view = view(
        &log,
        [
            text.clone(),
            provider(&log, "skip", |_, _| None),
            provider(&log, "flag", |_, focused| Some(FLAG.of(focused)))
                .with_precedence(Precedence::Highest),
            text,
            provider(&log, "point", |state, _| {
                Some(POINT.of(state.selection().main().head()))
            })
            .with_precedence(Precedence::Low),
        ],
    );
    let viewport = Viewport::new(view.state().text(), 1..2, std::iter::once(1..2)).unwrap();
    view.set_viewport_geometry(viewport.clone()).unwrap();
    assert!(!view.focused());
    log.lock().unwrap().clear();

    assert_eq!(view.set_focus(true, "focus").unwrap(), "focus");
    assert_eq!(view.set_focus(true, "repeat").unwrap(), "repeat");
    view.set_viewport_geometry(viewport).unwrap();
    assert!(view.focused());
    assert_eq!(view.set_focus(false, "blur").unwrap(), "blur");
    assert!(!view.focused());
    assert_eq!(view.revision(), 0);
    assert_eq!(view.undo_depth(), 0);
    assert_eq!(*log.lock().unwrap(), oracle("idle"));
}

#[test]
fn all_none_notifies_once_without_dispatch_or_timer_retry() {
    let log = Log::default();
    let mut view = view(
        &log,
        [
            provider(&log, "skip", |_, _| None),
            transaction_filter(|_, _| panic!("no transaction to filter")),
        ],
    );
    assert_eq!(view.set_focus(false, "settled").unwrap(), "settled");
    view.set_focus(true, "focus").unwrap();
    view.set_focus(false, "blur").unwrap();
    let mut expected = oracle("no_effect");
    // A browser timer first tries to build a transaction and then calls
    // update([]), which retries when the first attempt produced no effect.
    expected.dedup();
    assert_eq!(*log.lock().unwrap(), expected);
}

#[test]
fn explicit_focus_after_a_composed_edit_reads_its_result_and_new_configuration() {
    let log = Log::default();
    let compartment = Compartment::new();
    let mut view = view(
        &log,
        [compartment.of(provider(&log, "old", |_, _| Some(TEXT.of("old".into()))))],
    );
    let next = provider(&log, "new", |state, _| {
        Some(TEXT.of(state.text().as_str().to_owned()))
    });
    let first = insert(1, "X").with_effects([POINT.of(2), compartment.reconfigure(next)]);
    let second = insert(0, "!").with_selection(EditorSelection::single(3, 3));
    let composed = first
        .merge(
            second,
            view.state().text().len(),
            TransactionCoordinates::Sequential,
        )
        .unwrap();
    assert!(!composed.is_focus_change());
    view.update(composed, "before", |_| (None, "after"))
        .unwrap();
    view.set_focus(true, "after").unwrap();

    let mut expected = oracle("composed");
    // This host reports focus after the edit. It does not sample an ambient
    // browser focus flag or prepare a transaction before edit listeners run.
    let provider = expected.remove(0);
    expected[0][4] = json!(false);
    expected[1][4] = json!(false);
    expected.insert(2, provider);
    assert_eq!(*log.lock().unwrap(), expected);
}

fn insert(pos: usize, value: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(pos..pos, value)).unwrap())
}

#[test]
fn proposal_marks_the_transaction_without_applying_filters_or_mutating_state() {
    let state = EditorState::with_extensions(
        "ab",
        Extension::group([
            focus_change_effect(|state, focused| {
                assert_eq!(state.text().as_str(), "ab");
                assert!(!focused);
                Some(FLAG.of(focused))
            }),
            transaction_filter(|_, _| panic!("a proposal must not run filters")),
        ]),
    )
    .unwrap();
    assert!(
        focus_change_transaction(&state, true, true)
            .unwrap()
            .is_none()
    );
    let proposal = focus_change_transaction(&state, true, false)
        .unwrap()
        .unwrap();
    assert!(proposal.is_focus_change());
    assert!(proposal.changes().is_empty());
    assert!(proposal.selection().is_none());
    assert!(proposal.user_event().is_none());
    assert!(!proposal.scroll_into_view());
    assert!(proposal.add_to_history());
    assert_eq!(proposal.effects()[0].get(&FLAG), Some(&false));
}

#[test]
fn filters_then_extenders_see_the_focus_proposal_once() {
    let log = Log::default();
    let filter = log.clone();
    let extender = log.clone();
    let mut view = view(
        &log,
        [
            provider(&log, "flag", |_, focused| Some(FLAG.of(focused))),
            transaction_filter(move |_, transaction| {
                assert!(transaction.is_focus_change());
                filter
                    .lock()
                    .unwrap()
                    .push(json!(["filter", effects(transaction.effects())]));
                Ok(transaction)
            }),
            transaction_extender(move |_, transaction| {
                extender
                    .lock()
                    .unwrap()
                    .push(json!(["extender", effects(transaction.effects())]));
                TransactionExtension {
                    effects: vec![EXTRA.of("accepted")],
                    ..Default::default()
                }
            }),
        ],
    );
    view.set_focus(true, "focus").unwrap();
    assert_eq!(*log.lock().unwrap(), oracle("filtered"));
}

#[test]
fn filter_rejection_does_not_consume_focus_and_replacement_does_not_repeat_it() {
    let log = Log::default();
    let mut view = view(
        &log,
        [
            provider(&log, "flag", |_, focused| Some(FLAG.of(focused))),
            transaction_filter(|_, transaction| {
                assert!(transaction.is_focus_change());
                if transaction.effects()[0].get(&FLAG) == Some(&false) {
                    Err(EditError::InvalidRange)
                } else {
                    Ok(Transaction::default())
                }
            }),
        ],
    );
    view.set_focus(true, "focus").unwrap();
    view.set_focus(true, "repeat").unwrap();
    assert_eq!(*log.lock().unwrap(), oracle("removed_effect"));
    assert_eq!(view.set_focus(false, "blur"), Err(EditError::InvalidRange));
    assert!(view.focused());
    let mut expected = oracle("removed_effect");
    expected.push(json!(["provider", "flag", "ab", 0, false]));
    assert_eq!(*log.lock().unwrap(), expected);
}

#[test]
fn focus_obeys_inverted_effects_history_filtering_and_host_metadata() {
    let log = Log::default();
    let mut view = view(
        &log,
        [
            provider(&log, "flag", |_, focused| Some(FLAG.of(focused))),
            inverted_effects(|_, transaction, _| {
                transaction
                    .effects()
                    .iter()
                    .filter_map(|effect| effect.get(&FLAG).map(|value| FLAG.of(!value)))
                    .collect()
            }),
            transaction_filter(|_, transaction| {
                if transaction
                    .effects()
                    .iter()
                    .any(|effect| effect.get(&FLAG) == Some(&false))
                {
                    Ok(transaction.with_add_to_history(false))
                } else {
                    Ok(transaction)
                }
            }),
        ],
    );
    let record_depth = |depth| log.lock().unwrap().push(json!(["undo_depth", depth]));
    view.set_focus(true, "focus host snapshot").unwrap();
    record_depth(view.undo_depth());
    view.set_focus(false, "blur host snapshot").unwrap();
    record_depth(view.undo_depth());
    assert_eq!(view.undo(), Some("focus host snapshot"));
    record_depth(view.undo_depth());
    assert_eq!(*log.lock().unwrap(), oracle("history"));
}
