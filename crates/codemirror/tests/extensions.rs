use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, Facet, History, Precedence, StateEffectType, StateField, Transaction,
};

fn edit(start: usize, end: usize, text: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(start..end, text)).unwrap())
}

#[test]
fn collects_facet_inputs_in_precedence_order_and_deduplicates_reused_inputs() {
    let names = Facet::collect();
    let repeated = names.of("repeat");
    let config = Extension::group([
        Extension::group([names.of("low"), repeated.clone()]).with_precedence(Precedence::Low),
        names.of("default-1"),
        repeated.with_precedence(Precedence::Highest),
        names.of("default-2"),
        names.of("high").with_precedence(Precedence::High),
    ]);
    let state = EditorState::with_extensions("", config).unwrap();
    assert_eq!(
        state.context().facet(&names).unwrap().as_slice(),
        ["repeat", "high", "default-1", "default-2", "low"],
    );
    assert!(
        state
            .context()
            .facet(&Facet::<u32, Vec<_>>::collect())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn resolves_fields_and_dynamic_facets_with_both_document_snapshots() {
    let increment = StateEffectType::<usize>::new();
    let counter = StateField::new(|state| Ok(state.text().len()), {
        let increment = increment.clone();
        move |count, update| {
            assert_eq!(update.changes().length(), update.start().text().len());
            assert_eq!(update.changes().new_length(), update.state().text().len());
            Ok(count
                + update
                    .transaction()
                    .effects()
                    .iter()
                    .filter_map(|effect| effect.get(&increment))
                    .sum::<usize>())
        }
    });
    let numbers = Facet::collect();
    let sum = Facet::new(|inputs: &[&usize]| inputs.iter().map(|n| **n).sum::<usize>());
    let config = Extension::group([
        sum.compute({
            let numbers = numbers.clone();
            move |state| Ok(state.facet(&numbers)?.iter().sum())
        }),
        numbers.of(2),
        numbers.from_field(counter.clone(), |count| *count),
        numbers.compute_many(|state| Ok(vec![state.text().len(), state.selection().main().head()])),
        counter.extension(),
    ]);
    let mut state = EditorState::with_extensions("abc", config).unwrap();
    assert_eq!(*state.context().facet(&sum).unwrap(), 8);
    let snapshot = state.clone();
    state
        .update(
            edit(1, 1, "字")
                .with_effects([increment.of(5)])
                .with_selection(EditorSelection::single(4, 4)),
        )
        .unwrap();
    assert_eq!(state.text().as_str(), "a字bc");
    assert_eq!(*state.context().field(&counter).unwrap().unwrap(), 8);
    assert_eq!(
        state.context().facet(&numbers).unwrap().as_slice(),
        [2, 8, 6, 4]
    );
    assert_eq!(*state.context().facet(&sum).unwrap(), 20);
    assert_eq!(snapshot.text().as_str(), "abc");
    assert_eq!(*snapshot.context().facet(&sum).unwrap(), 8);
    let disabled_field = StateField::new(|_| Ok(false), |_, _| Ok(false));
    assert!(state.context().field(&disabled_field).unwrap().is_none());
}

#[test]
fn reconfigures_compartments_and_retains_fields_with_the_same_identity() {
    let config_value = Facet::collect();
    let compartment = Compartment::new();
    let retained = StateField::new(|_| Ok(0_usize), |previous, _| Ok(previous + 1));
    let config = Extension::group([
        retained.extension(),
        compartment.of(config_value.of(1)),
        config_value.of(2),
    ]);
    let mut state = EditorState::with_extensions("", config).unwrap();
    assert!(state.compartment(&compartment).is_some());
    state
        .update(Transaction::default().with_effects([
            compartment.reconfigure(config_value.of(3)),
            config_value.of(4).append(),
        ]))
        .unwrap();
    assert_eq!(
        state.context().facet(&config_value).unwrap().as_slice(),
        [3, 2, 4]
    );
    assert_eq!(*state.context().field(&retained).unwrap().unwrap(), 1);

    state
        .update(Transaction::default().with_effects([retained.extension().reconfigure()]))
        .unwrap();
    assert!(state.compartment(&compartment).is_none());
    assert!(state.context().facet(&config_value).unwrap().is_empty());
    assert_eq!(*state.context().field(&retained).unwrap().unwrap(), 2);

    state
        .update(Transaction::default().with_effects([compartment.of(config_value.of(5)).append()]))
        .unwrap();
    assert_eq!(
        state.context().facet(&config_value).unwrap().as_slice(),
        [5]
    );
}

#[test]
fn invalid_configuration_leaves_text_selection_and_extension_values_unchanged() {
    let first = Facet::<usize, Vec<_>>::collect();
    let other = Facet::<usize, Vec<_>>::collect();
    let field = StateField::new(|_| Ok(0), |n, _| Ok(n + 1));
    let context = Compartment::new();
    let config = Extension::group([context.of(first.of(1)), field.extension()]);
    let mut state = EditorState::with_extensions("字", config).unwrap();

    let duplicate = edit(0, 0, "a")
        .with_effects([context.of(first.of(2)).append()])
        .with_selection(EditorSelection::single(1, 1));
    assert_eq!(
        state.update(duplicate),
        Err(EditError::Configuration(
            ConfigurationError::DuplicateCompartment
        ))
    );

    let cycle = Extension::group([
        first.compute({
            let other = other.clone();
            move |state| Ok(state.facet(&other)?.len())
        }),
        other.compute({
            let first = first.clone();
            move |state| Ok(state.facet(&first)?.len())
        }),
    ]);
    assert_eq!(
        state.update(edit(0, 0, "a").with_effects([cycle.append()])),
        Err(EditError::Configuration(
            ConfigurationError::DependencyCycle
        )),
    );
    assert_eq!(state.text().as_str(), "字");
    assert_eq!(state.selection().main().head(), 0);
    assert_eq!(state.context().facet(&first).unwrap().as_slice(), [1]);
    assert_eq!(*state.context().field(&field).unwrap().unwrap(), 0);
}

#[test]
fn pinned_field_initializers_follow_precedence_and_reconfiguration_identity() {
    let number = StateField::new(|_| Ok(0), |n, tr| Ok(n + tr.state().text().len()));
    let compartment = Compartment::new();
    let initial = Extension::group([
        number.init(|_| Ok(20)),
        number.init(|_| Ok(40)).with_precedence(Precedence::High),
    ]);
    let mut state = EditorState::with_extensions(
        "abc",
        Extension::group([compartment.of(initial.clone()), number.extension()]),
    )
    .unwrap();
    let read = |state: &EditorState| *state.context().field(&number).unwrap().unwrap();
    assert_eq!(read(&state), 40);
    state
        .update(Transaction::default().with_effects([compartment.reconfigure(initial)]))
        .unwrap();
    assert_eq!(read(&state), 43);
    let custom = number.init(|start| Ok(start.text().len() * 10 + start.selection().main().head()));
    state
        .update(
            edit(3, 3, "de")
                .with_effects([compartment.reconfigure(custom)])
                .with_selection(EditorSelection::single(5, 5)),
        )
        .unwrap();
    assert_eq!(read(&state), 35);
    state
        .update(Transaction::default().with_effects([compartment.reconfigure(number.extension())]))
        .unwrap();
    assert_eq!(read(&state), 40);
}

#[test]
fn pinned_field_added_with_edit_initializes_on_the_old_document_then_updates() {
    let field = StateField::new(
        |state| Ok(state.text().len() * 10),
        |value, tr| {
            assert_eq!(tr.start().text().as_str(), "abcde");
            Ok(value + tr.state().text().len())
        },
    );
    let mut state = EditorState::new("abcde");
    state
        .update(edit(5, 5, "f").with_effects([field.extension().append()]))
        .unwrap();
    assert_eq!(*state.context().field(&field).unwrap().unwrap(), 56);
}

#[test]
fn invalid_initializers_do_not_commit_an_edit_or_configuration() {
    let field = StateField::new(|_| Ok(7), |value, _| Ok(*value));
    let mut state = EditorState::with_extensions("abc", field.extension()).unwrap();
    let bad = field.init(|_| Err(ConfigurationError::InvalidFieldData));
    let result = state.update(edit(0, 3, "new").with_effects([bad.reconfigure()]));
    assert_eq!(
        result,
        Err(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        ))
    );
    assert_eq!(state.text().as_str(), "abc");
    assert_eq!(*state.context().field(&field).unwrap().unwrap(), 7);
}

#[test]
fn history_replays_field_updates_and_rolls_back_a_failed_finish() {
    let observed = StateField::new(
        |state| Ok(state.text().as_str().to_owned()),
        |_, update| Ok(update.state().text().as_str().to_owned()),
    );
    let state = EditorState::with_extensions("a", observed.extension()).unwrap();
    let mut history = History::new(state);
    let value =
        |history: &History<()>| history.state().context().field(&observed).unwrap().unwrap();

    assert_eq!(
        history.update(edit(1, 1, "字"), (), |_| (
            Some(EditorSelection::single(2, 2)),
            ()
        )),
        Err(EditError::InvalidSelection),
    );
    assert_eq!(&**value(&history), "a");
    assert_eq!(history.state().text().as_str(), "a");
    assert!(!history.can_undo());

    history.start_group();
    history
        .update(edit(1, 1, "字"), (), |_| (None, ()))
        .unwrap();
    history.update(edit(4, 4, "!"), (), |_| (None, ())).unwrap();
    history.end_group();
    assert_eq!(&**value(&history), "a字!");
    assert_eq!(history.undo(), Some(()));
    assert_eq!(&**value(&history), "a");
    assert_eq!(history.redo(), Some(()));
    assert_eq!(&**value(&history), "a字!");
}
