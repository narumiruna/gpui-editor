//! Results from the SHA-512-verified @codemirror/state 6.7.6 tarball.

use std::{
    cell::Cell,
    rc::Rc,
    sync::{Arc, Mutex},
};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorState, Extension, Facet,
    Precedence, StateEffectType, StateField, StateFieldOptions, Transaction,
};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/extensions_oracle.json")).unwrap()
}

fn insert() -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(0..0, "a")).unwrap())
}

#[test]
fn fixture_uses_the_inventory_pin() {
    let inventory: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let state = inventory["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pin| pin["name"] == "@codemirror/state")
        .unwrap();
    let pin = &fixture()["pin"];
    for key in ["name", "version", "integrity"] {
        assert_eq!(state[key], pin[key]);
    }
}

#[test]
fn field_compares_the_intermediate_value_with_each_update() {
    let comparisons = Arc::new(Mutex::new(Vec::new()));
    let change = StateEffectType::<(u32, &str)>::new();
    let field = StateField::define(
        |_| Ok((0, "create")),
        {
            let change = change.clone();
            move |value: &(u32, &str), update| {
                Ok(update
                    .transaction()
                    .effects()
                    .iter()
                    .find_map(|effect| effect.get(&change).copied())
                    .unwrap_or((value.0, "update")))
            }
        },
        StateFieldOptions::<(u32, &str)>::default().compare({
            let comparisons = comparisons.clone();
            move |old, next| {
                comparisons.lock().unwrap().push([old.1, next.1]);
                old.0 == next.0
            }
        }),
    );
    let slot = Compartment::new();
    let init = field.init(|_| Ok((1, "init")));
    let mut state = EditorState::with_extensions("", slot.of(field.extension())).unwrap();
    let mut steps = Vec::new();
    let mut previous = None;
    let mut record = |state: &EditorState| {
        let value = state.context().field(&field).unwrap().unwrap();
        let reused = previous
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &value));
        steps.push(json!({"count": value.0, "source": value.1, "reused": reused}));
        previous = Some(value);
    };
    record(&state);
    for transaction in [
        Transaction::default(),
        Transaction::default().with_effects([change.of((1, "effect"))]),
        Transaction::default().with_effects([slot.reconfigure(init.clone())]),
        Transaction::default().with_effects([slot.reconfigure(init)]),
        Transaction::default().with_effects([slot.reconfigure(field.extension())]),
    ] {
        state.update(transaction).unwrap();
        record(&state);
    }
    assert_eq!(
        json!({"steps": steps, "comparisons": *comparisons.lock().unwrap()}),
        fixture()["comparison"],
    );
}

#[test]
fn a_shared_field_without_a_comparator_retains_only_the_returned_identity() {
    let field = StateField::define_shared(
        |_| Ok(Arc::new(0)),
        |value, update| {
            Ok(if update.changes().is_empty() {
                value.clone()
            } else {
                Arc::new(**value)
            })
        },
        StateFieldOptions::default(),
    );
    let mut state = EditorState::with_extensions("", field.extension()).unwrap();
    let read = |state: &EditorState| state.context().field(&field).unwrap().unwrap();
    let start = read(&state);
    state.update(Transaction::default()).unwrap();
    let noop = read(&state);
    state.update(insert()).unwrap();
    let edited = read(&state);
    assert_eq!(*noop, *edited);
    assert_eq!(
        json!([Arc::ptr_eq(&noop, &start), Arc::ptr_eq(&edited, &noop)]),
        fixture()["identity"],
    );
}

#[test]
fn provided_extensions_follow_field_precedence_and_facet_enables_use_default() {
    let labels = Facet::collect();
    let shared = labels.of("shared");
    let field_factory_calls = Rc::new(Cell::new(0));
    let mut facet_factory_calls = 0;
    let enabled = Facet::with_enables(
        |inputs: &[&i32]| inputs.iter().map(|value| **value).collect::<Vec<_>>(),
        |_| {
            facet_factory_calls += 1;
            Extension::group([
                labels.of("enabled"),
                shared.clone(),
                labels
                    .of("enabled-highest")
                    .with_precedence(Precedence::Highest),
            ])
        },
    );
    let factory_calls = |facet_calls| json!({"field_factory_calls": field_factory_calls.get(), "facet_factory_calls": facet_calls});
    let uninstalled = factory_calls(facet_factory_calls);
    let seed = Arc::new(5);
    let weak = Arc::downgrade(&seed);
    let field = StateField::define(
        move |_| Ok(*seed),
        |value, update| Ok(value + i32::from(!update.changes().is_empty())),
        StateFieldOptions::default().provide({
            let (labels, enabled, calls) =
                (labels.clone(), enabled.clone(), field_factory_calls.clone());
            move |field| {
                calls.set(calls.get() + 1);
                Extension::group([
                    labels.of("field"),
                    shared,
                    enabled.from_field(field, |n| *n),
                ])
            }
        }),
    );
    let defined = factory_calls(facet_factory_calls);
    let slot = Compartment::new();
    let low = field.extension().with_precedence(Precedence::Low);
    let mut state = EditorState::with_extensions(
        "",
        Extension::group([
            labels.of("default-before"),
            slot.of(Extension::group([
                low.clone(),
                field.extension().with_precedence(Precedence::High),
            ])),
            labels.of("default-after"),
        ]),
    )
    .unwrap();
    let read = |state: &EditorState| {
        let cx = state.context();
        json!({
            "labels": cx.facet(&labels).unwrap().as_slice(),
            "enabled": cx.facet(&enabled).unwrap().as_slice(),
            "field": cx.field(&field).unwrap().as_deref(),
        })
    };
    let mut steps = vec![read(&state)];
    for transaction in [
        insert().with_effects([slot.reconfigure(low)]),
        Transaction::default().with_effects([slot.reconfigure(enabled.of(9))]),
        Transaction::default().with_effects([slot.reconfigure(Extension::default())]),
        Transaction::default().with_effects([slot.reconfigure(field.init(|_| Ok(7)))]),
    ] {
        state.update(transaction).unwrap();
        steps.push(read(&state));
    }
    assert_eq!(
        json!({"uninstalled": uninstalled, "defined": defined, "steps": steps,
               "final": factory_calls(facet_factory_calls)}),
        fixture()["providers"],
    );
    drop(state);
    drop(field);
    assert!(weak.upgrade().is_none());
}

#[test]
fn a_cycle_through_a_provided_facet_rejects_the_complete_transaction() {
    let number = Facet::collect();
    let field = StateField::define(
        {
            let reader = number.reader();
            move |state| Ok(state.facet(&reader)?.len())
        },
        |value, _| Ok(*value),
        StateFieldOptions::default().provide(move |field| number.from_field(field, |n| *n)),
    );
    let mut state = EditorState::new("");
    let result = state.update(insert().with_effects([field.extension().append()]));
    assert_eq!(
        result
            == Err(EditError::Configuration(
                ConfigurationError::DependencyCycle
            )),
        fixture()["cycles"].as_bool().unwrap(),
    );
    assert!(state.text().is_empty());
    assert!(state.context().field(&field).unwrap().is_none());
}

#[test]
fn facet_enables_can_read_the_facet_without_owning_its_definition() {
    let count = Arc::new(0);
    let weak = Arc::downgrade(&count);
    let published = Facet::collect();
    {
        let facet = Facet::with_enables(
            move |inputs: &[&usize]| *count + inputs.iter().map(|value| **value).sum::<usize>(),
            |reader| {
                let field = StateField::new(
                    {
                        let reader = reader.clone();
                        move |state| Ok(*state.facet(&reader)?)
                    },
                    move |_, update| Ok(*update.state().facet(&reader)?),
                );
                Extension::group([
                    published.from_field(field.clone(), |value| *value),
                    field.extension(),
                ])
            },
        );
        let state = EditorState::with_extensions("", facet.of(5)).unwrap();
        assert_eq!(state.context().facet(&published).unwrap().as_slice(), [5]);
    }
    assert!(weak.upgrade().is_none());
}
