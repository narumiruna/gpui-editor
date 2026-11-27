use std::{
    cell::Cell,
    collections::HashMap,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Change, ChangeSet, EditError, EditorSelection, EditorState,
    Extension, History, Precedence, SelectionRange, StateEffectType, StateField, Transaction,
    transaction_filter,
    view::input::{
        DefaultInput, InputChange, InputHandling, InputProposal, input_handler, plan_input,
        run_input_handlers,
    },
};
use serde_json::{Value, json};

static FLAG: LazyLock<StateEffectType<&str>> = LazyLock::new(StateEffectType::new);

#[derive(Default)]
struct Observed {
    calls: Vec<Value>,
    previews: Vec<Value>,
}

fn utf16(doc: &str, byte: usize) -> usize {
    doc[..byte].encode_utf16().count()
}

fn byte_at(doc: &str, position: &Value) -> usize {
    let position = usize::try_from(position.as_u64().unwrap()).unwrap();
    let mut units = 0;
    for (byte, character) in doc.char_indices() {
        if units == position {
            return byte;
        }
        units += character.len_utf16();
    }
    assert_eq!(units, position);
    doc.len()
}

fn range(doc: &str, pair: &Value) -> SelectionRange {
    SelectionRange::new(byte_at(doc, &pair[0]), byte_at(doc, &pair[1]))
}

fn ranges(doc: &str, selection: &EditorSelection) -> Vec<[usize; 2]> {
    selection
        .ranges()
        .iter()
        .map(|range| [utf16(doc, range.anchor()), utf16(doc, range.head())])
        .collect()
}

fn describe(state: &EditorState, tr: &Transaction) -> Value {
    let old = state.text().as_str();
    let changes: Vec<_> = tr
        .changes()
        .changes()
        .map(|(range, text)| json!([utf16(old, range.start), utf16(old, range.end), text]))
        .collect();
    let mut after = state.clone();
    after.update(tr.clone()).unwrap();
    let doc = after.text().as_str();
    let effects: Vec<_> = tr
        .effects()
        .iter()
        .map(|effect| effect.get(&FLAG))
        .collect();
    json!({
        "changes": changes, "selection": tr.selection().map(|selection| ranges(doc, selection)),
        "doc": doc, "ranges": ranges(doc, after.selection()), "main": after.selection().main_index(),
        "event": tr.user_event(), "scroll": tr.scroll_into_view(), "effects": effects,
    })
}

fn oracle_handler(observed: Arc<Mutex<Observed>>, name: String, action: String) -> Extension {
    input_handler(move |change, default| {
        let state = change.state();
        let doc = state.text().as_str();
        let range = change.range();
        let text = change.text();
        let mut log = observed.lock().unwrap();
        log.calls.push(json!([
            name,
            utf16(doc, range.start),
            utf16(doc, range.end),
            text,
            doc,
            ranges(doc, state.selection()),
        ]));
        match action.as_str() {
            "pass" => InputHandling::Pass,
            "peek" | "accept" => {
                let tr = default().unwrap();
                assert!(std::ptr::eq(tr, default().unwrap()));
                log.previews.push(describe(state, tr));
                if action == "accept" {
                    InputHandling::Handled(Some(tr.clone()))
                } else {
                    InputHandling::Pass
                }
            }
            "consume" => InputHandling::Handled(None),
            "custom" => {
                let text = text.to_uppercase();
                let end = range.start + text.len();
                let changes = ChangeSet::single(Change::new(range, text)).unwrap();
                InputHandling::Handled(Some(
                    Transaction::new(changes)
                        .with_selection(EditorSelection::single(end, end))
                        .with_effects([FLAG.of("accepted")])
                        .with_user_event("input.override"),
                ))
            }
            _ => panic!("unexpected handler: {name}"),
        }
    })
}

#[test]
fn input_matches_trusted_chromium_oracle() {
    for line in include_str!("fixtures/view_input.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let doc = row["doc"].as_str().unwrap();
        let observed = Arc::new(Mutex::new(Observed::default()));
        let mut extensions = vec![
            ALLOW_MULTIPLE_SELECTIONS.of(true),
            EditorState::history_config(None, Some(0)),
        ];
        let mut handlers = HashMap::new();
        for handler in row["handlers"].as_array().unwrap() {
            let name = handler[0].as_str().unwrap();
            let extension = handlers.entry(name).or_insert_with(|| {
                let extension = oracle_handler(
                    observed.clone(),
                    name.to_owned(),
                    handler[1].as_str().unwrap().to_owned(),
                );
                match handler[2].as_str() {
                    Some("high") => extension.with_precedence(Precedence::High),
                    Some("low") => extension.with_precedence(Precedence::Low),
                    None => extension,
                    _ => panic!("invalid precedence"),
                }
            });
            extensions.push(extension.clone());
        }
        let selection = EditorSelection::new(
            row["ranges"]
                .as_array()
                .unwrap()
                .iter()
                .map(|pair| range(doc, pair))
                .collect(),
            usize::try_from(row["main"].as_u64().unwrap_or(0)).unwrap(),
        )
        .unwrap();
        let state =
            EditorState::create(doc, Some(selection), Extension::group(extensions)).unwrap();
        let mut host = History::<()>::new(state);
        let ops = row["ops"].as_array().unwrap();
        let results = row["results"].as_array().unwrap();
        assert_eq!(ops.len(), results.len());
        for (op, expected) in ops.iter().zip(results) {
            let state = host.state();
            let doc = state.text().as_str();
            let text = op["text"].as_str().unwrap();
            let input = match op["mode"].as_str().unwrap() {
                "selection" => InputProposal::ReplaceSelection(text),
                "explicit" => {
                    let span = range(doc, &op["range"]).range();
                    let mut replaced = doc.to_owned();
                    replaced.replace_range(span.clone(), text);
                    InputProposal::Replace {
                        range: span,
                        text,
                        selection: Some(range(&replaced, &op["selection"])),
                    }
                }
                _ => panic!("invalid input mode"),
            };
            let mut expected = expected.as_object().unwrap().clone();
            let trusted = expected.remove("trustedInput").unwrap();
            assert!(!trusted.as_array().unwrap().is_empty());
            let before = host.undo_depth();
            let tr = plan_input(host.state(), input).unwrap();
            assert_eq!(host.undo_depth(), before);
            let mut dispatches = Vec::new();
            if let Some(tr) = tr {
                dispatches.push(describe(host.state(), &tr));
                host.update(tr, (), |_| (None, ())).unwrap();
            }
            let state = host.state();
            let doc = state.text().as_str();
            let Observed { calls, previews } = std::mem::take(&mut *observed.lock().unwrap());
            let mut actual = json!({
                "calls": calls, "previews": previews, "dispatches": dispatches,
                "doc": doc, "ranges": ranges(doc, state.selection()),
                "main": state.selection().main_index(), "undoDepth": host.undo_depth(),
            });
            if op["undo"] == true {
                assert_eq!(host.undo(), Some(()));
                let doc = host.state().text().as_str();
                actual["undo"] = json!({"doc": doc, "ranges": ranges(doc, host.state().selection()), "depth": host.undo_depth()});
            }
            assert_eq!(actual, Value::Object(expected), "{}: {op}", row["name"]);
        }
    }
}

#[test]
fn host_factory_is_lazy_and_shared_by_handlers_even_when_it_fails() {
    let builds = Cell::new(0);
    let state = EditorState::new("é");
    let change = InputChange::new(&state, 0..2, "x").unwrap();
    assert!(matches!(
        run_input_handlers(change, || panic!("unused factory")),
        Ok(InputHandling::Pass)
    ));
    let prior = Arc::new(AtomicUsize::new(0));
    let first = prior.clone();
    let inspect = input_handler(move |_, default| {
        let tr = default().unwrap();
        assert!(std::ptr::eq(tr, default().unwrap()));
        first.store(std::ptr::from_ref(tr).addr(), Ordering::Relaxed);
        InputHandling::Pass
    });
    let accept = input_handler(move |_, default| {
        let tr = default().unwrap();
        assert_eq!(std::ptr::from_ref(tr).addr(), prior.load(Ordering::Relaxed));
        InputHandling::Handled(Some(tr.clone()))
    });
    let state = EditorState::with_extensions("é", Extension::group([inspect, accept])).unwrap();
    for expected in 1..=2 {
        let change = InputChange::new(&state, 0..2, "a").unwrap();
        let result = run_input_handlers(change, || {
            builds.set(builds.get() + 1);
            Ok(Transaction::default())
        });
        assert!(matches!(result, Ok(InputHandling::Handled(Some(_)))));
        assert_eq!(builds.get(), expected);
    }
    let inspect_error = |_: InputChange<'_>, default: &DefaultInput<'_>| {
        assert_eq!(default().unwrap_err(), EditError::InvalidSelection);
        InputHandling::Pass
    };
    let state = EditorState::with_extensions(
        "é",
        Extension::group([input_handler(inspect_error), input_handler(inspect_error)]),
    )
    .unwrap();
    let result = run_input_handlers(InputChange::new(&state, 0..2, "x").unwrap(), || {
        builds.set(builds.get() + 1);
        Ok(Transaction::default().with_selection(EditorSelection::single(1, 1)))
    });
    assert!(matches!(result, Ok(InputHandling::Pass)));
    assert_eq!(builds.get(), 3);
}

#[test]
fn invalid_ranges_and_selections_are_rejected_before_a_handler_can_consume() {
    let state =
        EditorState::with_extensions("é", input_handler(|_, _| panic!("invalid input"))).unwrap();
    let reversed = std::ops::Range { start: 2, end: 0 };
    for range in [1..1, 0..3, reversed] {
        assert_eq!(
            InputChange::new(&state, range, "").unwrap_err(),
            EditError::InvalidRange
        );
    }
    for position in [1, 5] {
        let proposal = InputProposal::Replace {
            range: 0..2,
            text: "🦀",
            selection: Some(SelectionRange::new(position, position)),
        };
        assert_eq!(
            plan_input(&state, proposal).unwrap_err(),
            EditError::InvalidSelection
        );
    }
    let handler = input_handler(|_, _| {
        let changes = ChangeSet::single(Change::new(1..2, "!")).unwrap();
        InputHandling::Handled(Some(Transaction::new(changes)))
    });
    let state = EditorState::with_extensions("é", handler).unwrap();
    let change = InputChange::new(&state, 0..2, "x").unwrap();
    assert_eq!(
        run_input_handlers(change, || panic!("unused default")).unwrap_err(),
        EditError::InvalidRange
    );
}

#[test]
fn only_host_dispatch_runs_filters_applies_effects_and_records_history() {
    let checks = Arc::new(AtomicUsize::new(0));
    let called = checks.clone();
    let filter = transaction_filter(move |_, tr| {
        called.fetch_add(1, Ordering::Relaxed);
        Ok(tr)
    });
    let field = StateField::new(
        |_| Ok(false),
        |previous, update| {
            Ok(*previous
                || update
                    .transaction()
                    .effects()
                    .iter()
                    .any(|effect| effect.get(&FLAG).is_some()))
        },
    );
    let custom = input_handler(|_, default| {
        InputHandling::Handled(Some(
            default()
                .unwrap()
                .clone()
                .with_effects([FLAG.of("accepted")]),
        ))
    });
    let state =
        EditorState::with_extensions("é", Extension::group([filter, field.extension(), custom]))
            .unwrap();
    let mut host = History::<()>::new(state);
    let proposal = InputProposal::Replace {
        range: 0..0,
        text: "a\r\n",
        selection: None,
    };
    let tr = plan_input(host.state(), proposal).unwrap().unwrap();
    assert_eq!(checks.load(Ordering::Relaxed), 0);
    assert_eq!(host.undo_depth(), 0);
    assert!(!*host.state().context().field(&field).unwrap().unwrap());
    assert_eq!(host.state().text().as_str(), "é");
    assert!(tr.selection().is_none());
    host.update(tr, (), |_| (None, ())).unwrap();
    assert_eq!(checks.load(Ordering::Relaxed), 1);
    assert_eq!(host.undo_depth(), 1);
    assert!(*host.state().context().field(&field).unwrap().unwrap());
    assert_eq!(host.state().text().as_str(), "a\r\né");
    assert_eq!(host.state().selection().main().head(), 0);
}
