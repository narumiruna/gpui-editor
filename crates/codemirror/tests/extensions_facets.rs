//! Facet callbacks and identities from the verified @codemirror/state 6.7.6 tarball.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, Facet, StateEffectType, StateField, StateFieldOptions, Transaction,
};
use serde_json::{Value, json};

fn fixture() -> Value {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/extensions_facets.json")).unwrap();
    let upstream: Value =
        serde_json::from_str(include_str!("fixtures/extensions_oracle.json")).unwrap();
    assert_eq!(fixture["pin"], upstream["pin"]);
    fixture
}

fn edit(range: std::ops::Range<usize>, text: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, text)).unwrap())
}

#[test]
fn declared_dependencies_follow_transactions_and_configuration() {
    let calls = Arc::new(std::array::from_fn::<_, 6, _>(|_| AtomicUsize::new(0)));
    let (doc, selection, constant, projected, setting, derived) = (
        Facet::collect(),
        Facet::collect(),
        Facet::collect(),
        Facet::collect(),
        Facet::collect(),
        Facet::collect(),
    );
    let set = StateEffectType::<usize>::new();
    let field = StateField::define(
        |_| Ok(0),
        {
            let set = set.clone();
            move |old, tr| {
                Ok(tr
                    .transaction()
                    .effects()
                    .iter()
                    .find_map(|e| e.get(&set).copied())
                    .unwrap_or(*old))
            }
        },
        StateFieldOptions::default().compare(usize::eq),
    );
    let (field_slot, setting_slot, extra) =
        (Compartment::new(), Compartment::new(), Compartment::new());
    let (setting_a, setting_b) = (setting.of(10), setting.of(20));
    let additional_doc = doc.compute_with_dependencies(|deps| deps.document(), {
        let calls = calls.clone();
        move |s| Ok(100 * s.text().len() + calls[5].fetch_add(1, Relaxed) + 1)
    });
    let mut state = EditorState::with_extensions(
        "ab",
        Extension::group([
            derived.compute_with_dependencies(
                |deps| {
                    deps.facet(&projected);
                    deps.facet(&setting);
                },
                {
                    let (projected, setting, calls) =
                        (projected.reader(), setting.reader(), calls.clone());
                    move |s| {
                        calls[4].fetch_add(1, Relaxed);
                        Ok(s.facet(&projected)?[0] + s.facet(&setting)?[0])
                    }
                },
            ),
            field_slot.of(field.extension()),
            setting_slot.of(setting_a.clone()),
            extra.of(Extension::default()),
            doc.compute_with_dependencies(|deps| deps.document(), {
                let calls = calls.clone();
                move |s| {
                    calls[0].fetch_add(1, Relaxed);
                    Ok(s.text().len())
                }
            }),
            selection.compute_with_dependencies(|deps| deps.selection(), {
                let calls = calls.clone();
                move |s| {
                    calls[1].fetch_add(1, Relaxed);
                    Ok(s.selection().main().head())
                }
            }),
            constant.compute_with_dependencies(|_| {}, {
                let calls = calls.clone();
                move |_| {
                    calls[2].fetch_add(1, Relaxed);
                    Ok(99)
                }
            }),
            projected.from_field(field.reader(), {
                let calls = calls.clone();
                move |value| {
                    calls[3].fetch_add(1, Relaxed);
                    *value
                }
            }),
        ]),
    )
    .unwrap();
    let facets = [&doc, &selection, &constant, &projected, &setting, &derived];
    let mut old = None;
    let mut read = |state: &EditorState| {
        let outputs = facets.map(|f| state.context().facet(f).unwrap());
        let reused: [bool; 6] = std::array::from_fn(|i| {
            old.as_ref()
                .is_some_and(|old: &[Arc<Vec<usize>>; 6]| Arc::ptr_eq(&old[i], &outputs[i]))
        });
        let step = json!({ "calls": calls.each_ref().map(|n| n.load(Relaxed)),
                          "values": outputs.each_ref().map(|items| items.as_slice()), "reused": reused });
        old = Some(outputs);
        step
    };
    let mut steps = vec![read(&state)];
    for tr in [
        Transaction::default(),
        Transaction::default().with_selection(EditorSelection::single(0, 0)),
        Transaction::default().with_selection(EditorSelection::single(1, 1)),
        edit(2..2, "c"),
        Transaction::default().with_effects([set.of(1)]),
        Transaction::default().with_effects([set.of(1)]),
        Transaction::default().with_effects([setting_slot.reconfigure(setting_a)]),
        Transaction::default().with_effects([setting_slot.reconfigure(setting_b.clone())]),
        Transaction::default().with_effects([setting_slot.reconfigure(setting_b)]),
        edit(3..3, "d").with_effects([field_slot.reconfigure(field.init(|_| Ok(2)))]),
        edit(4..4, "e").with_effects([extra.reconfigure(additional_doc.clone())]),
        Transaction::default().with_effects([extra.reconfigure(Extension::default())]),
        Transaction::default().with_effects([extra.reconfigure(additional_doc)]),
    ] {
        state.update(tr).unwrap();
        steps.push(read(&state));
    }
    assert_eq!(json!(steps), fixture()["dependencies"]);
}

type Events = Arc<Mutex<Vec<Vec<String>>>>;

fn event(events: &Events, parts: &[&str]) {
    events
        .lock()
        .unwrap()
        .push(parts.iter().map(|s| (*s).to_owned()).collect());
}

#[derive(Clone)]
struct Token {
    n: usize,
    label: String,
}

fn token(n: usize, label: impl Into<String>) -> Token {
    Token {
        n,
        label: label.into(),
    }
}

#[test]
fn input_and_output_comparisons_preserve_the_correct_cached_values() {
    let events = Events::default();
    let tokens = Facet::define(
        {
            let events = events.clone();
            move |inputs: &[&Token]| {
                let labels = inputs
                    .iter()
                    .map(|t| t.label.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                event(&events, &["combine", &labels]);
                token(inputs.iter().map(|t| t.n).sum(), labels)
            }
        },
        |options| {
            let input_events = events.clone();
            options.compare_input(move |next, old| {
                event(&input_events, &["input", &next.label, &old.label]);
                next.n == old.n
            });
            let output_events = events.clone();
            options.compare(move |next, old| {
                event(&output_events, &["output", &next.label, &old.label]);
                next.n == old.n
            });
        },
    );
    let derived = Facet::collect();
    let single = || {
        tokens.compute_with_dependencies(|deps| deps.document(), {
            let events = events.clone();
            move |state| {
                let name = state.text().as_str();
                event(&events, &["single", name]);
                Ok(token(
                    if matches!(name, "a" | "b") { 1 } else { 2 },
                    format!("s:{name}"),
                ))
            }
        })
    };
    let first = single();
    let multi = tokens.compute_many_with_dependencies(|deps| deps.document(), {
        let events = events.clone();
        move |state| {
            let name = state.text().as_str();
            event(&events, &["multi", name]);
            let values: &[usize] = match name {
                "a" | "b" => &[1, 2],
                "c" => &[1, 1],
                "f" => &[],
                _ => &[3],
            };
            Ok(values
                .iter()
                .enumerate()
                .map(|(i, &n)| token(n, format!("m:{name}:{i}")))
                .collect())
        }
    });
    let fixed = tokens.of(token(10, "fixed"));
    let project = derived.compute_with_dependencies(|deps| deps.facet(&tokens), {
        let (tokens, events) = (tokens.reader(), events.clone());
        move |state| {
            let value = state.facet(&tokens)?;
            event(&events, &["dependent", &value.label]);
            Ok(value)
        }
    });
    let slot = Compartment::new();
    let original = Extension::group([first.clone(), fixed.clone(), multi.clone()]);
    let mut state =
        EditorState::with_extensions("a", Extension::group([slot.of(original.clone()), project]))
            .unwrap();
    let mut prev = None;
    let mut prev_derived = None;
    let mut read = |state: &EditorState| {
        let value = state.context().facet(&tokens).unwrap();
        let dependent = state.context().facet(&derived).unwrap();
        let step = json!({
            "value": {"sum": value.n, "labels": value.label},
            "reused": prev.as_ref().is_some_and(|old| Arc::ptr_eq(old, &value)),
            "derivedReused": prev_derived.as_ref().is_some_and(|old| Arc::ptr_eq(old, &dependent)),
            "events": std::mem::take(&mut *events.lock().unwrap()),
        });
        prev = Some(value);
        prev_derived = Some(dependent);
        step
    };
    let mut steps = vec![read(&state)];
    for tr in ["b", "c", "d", "e", "f"]
        .map(|text| edit(0..1, text))
        .into_iter()
        .chain(
            [
                Extension::group([multi.clone(), first, fixed.clone()]),
                Extension::group([multi, single(), fixed.clone()]),
                Extension::group([tokens.of(token(2, "static")), fixed]),
                original,
            ]
            .map(|extension| Transaction::default().with_effects([slot.reconfigure(extension)])),
        )
    {
        state.update(tr).unwrap();
        steps.push(read(&state));
    }
    assert_eq!(json!(steps), fixture()["comparisons"]);
}

#[test]
fn reconfiguration_compares_static_outputs_and_inactive_defaults() {
    let calls = Events::default();
    let facet = Facet::define(
        {
            let calls = calls.clone();
            move |inputs: &[&Token]| {
                let tag = inputs
                    .iter()
                    .map(|x| x.label.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                event(&calls, &["combine", &tag]);
                token(inputs.iter().map(|x| x.n).sum(), tag)
            }
        },
        |options| {
            options.compare_input(|_, _| panic!("static inputs are never compared"));
            let calls = calls.clone();
            options.compare(move |next, old| {
                event(&calls, &["compare", &next.label, &old.label]);
                next.n == old.n
            });
        },
    );
    let collected = Facet::collect();
    let collect_empty = collected.compute_many_with_dependencies(|_| {}, |_| Ok(vec![]));
    let elem = collected.of(1);
    let equivalent = collected.of(1);
    let zero = facet.of(token(0, "zero"));
    let one = facet.of(token(1, "one"));
    let mut state = EditorState::new("");
    let mut prev = None;
    let mut prev_collect = None;
    let mut read = |state: &EditorState| {
        let value = state.context().facet(&facet).unwrap();
        let list = state.context().facet(&collected).unwrap();
        let step = json!({
            "value": { "n": value.n, "tag": value.label },
            "reused": prev.as_ref().is_some_and(|old| Arc::ptr_eq(old, &value)),
            "list": list.as_slice(),
            "listReused": prev_collect.as_ref().is_some_and(|old| Arc::ptr_eq(old, &list)),
            "calls": std::mem::take(&mut *calls.lock().unwrap()),
        });
        prev = Some(value);
        prev_collect = Some(list);
        step
    };
    let mut steps = vec![read(&state)];
    for extensions in [
        vec![zero.clone(), collect_empty.clone()],
        vec![zero.clone(), elem.clone(), collect_empty],
        vec![one.clone(), elem],
        vec![one, equivalent.clone()],
        vec![zero, equivalent],
        vec![],
    ] {
        state
            .update(
                Transaction::default().with_effects([Extension::group(extensions).reconfigure()]),
            )
            .unwrap();
        steps.push(read(&state));
    }
    assert_eq!(json!(steps), fixture()["staticAndDefault"]);
}

#[test]
fn reconfigured_dependency_can_produce_an_equal_input() {
    let setting = Facet::new(|inputs: &[&usize]| inputs.first().map_or(0, |n| **n));
    let comparisons = Arc::new(AtomicUsize::new(0));
    let facet = Facet::collect_with(|options| {
        let comparisons = comparisons.clone();
        options.compare_input(move |next: &Token, old| {
            comparisons.fetch_add(1, Relaxed);
            next.n == old.n
        });
    });
    let provider = facet.compute_with_dependencies(|deps| deps.facet(&setting), {
        let setting = setting.reader();
        move |state| {
            let number = *state.facet(&setting)?;
            Ok(token(number % 2, number.to_string()))
        }
    });
    let slot = Compartment::new();
    let mut state =
        EditorState::with_extensions("", Extension::group([slot.of(setting.of(1)), provider]))
            .unwrap();
    let old = state.context().facet(&facet).unwrap();
    state
        .update(Transaction::default().with_effects([slot.reconfigure(setting.of(3))]))
        .unwrap();
    assert!(Arc::ptr_eq(&old, &state.context().facet(&facet).unwrap()));
    assert_eq!(comparisons.load(Relaxed), 1);
    state
        .update(Transaction::default().with_effects([slot.reconfigure(Extension::default())]))
        .unwrap();
    assert_eq!(state.context().facet(&facet).unwrap()[0].label, "0");
    assert_eq!(comparisons.load(Relaxed), 2);
}

#[test]
fn field_comparison_can_invalidate_without_changing_identity() {
    let field = StateField::define_shared(
        |_| Ok(Arc::new(0)),
        |old, _| Ok(old.clone()),
        StateFieldOptions::default().compare(|_, _| false),
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let facet = Facet::collect();
    let provider = facet.from_field(field.reader(), {
        let calls = calls.clone();
        move |n| {
            calls.fetch_add(1, Relaxed);
            *n
        }
    });
    let mut state =
        EditorState::with_extensions("", Extension::group([field.extension(), provider])).unwrap();
    let old = state.context().field(&field).unwrap().unwrap();
    state.update(Transaction::default()).unwrap();
    assert!(Arc::ptr_eq(
        &old,
        &state.context().field(&field).unwrap().unwrap()
    ));
    assert_eq!(calls.load(Relaxed), 2);
}

#[test]
fn removing_a_required_field_does_not_publish_a_partial_cache() {
    let field = StateField::new(|_| Ok(3), |v, _| Ok(*v));
    let facet = Facet::collect();
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(
        "a",
        Extension::group([
            slot.of(field.extension()),
            facet.from_field(field.reader(), |n| *n),
        ]),
    )
    .unwrap();
    let old = state.context().facet(&facet).unwrap();
    let result =
        state.update(edit(0..1, "b").with_effects([slot.reconfigure(Extension::default())]));
    assert_eq!(
        result,
        Err(EditError::Configuration(ConfigurationError::MissingField))
    );
    assert_eq!(state.text().as_str(), "a");
    assert!(Arc::ptr_eq(&old, &state.context().facet(&facet).unwrap()));
}
