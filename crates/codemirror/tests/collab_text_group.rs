use gpui_editor_codemirror::{
    Change, ChangeSet, ConfigurationError, EditError, EditorSelection, EditorState,
    StateEffectType, Transaction,
    collab::{
        CollabConfig, CollabError, Update, collab, get_synced_version, prepare_text_update_group,
        receive_updates, sendable_updates,
    },
    view::EditorView,
};

fn edit(range: std::ops::Range<usize>, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, insert)).unwrap())
}

#[test]
fn composition_receipt_replaces_only_the_locally_held_suffix() {
    let mut view: EditorView<()> = EditorView::new(
        EditorState::with_extensions("名😀", collab(CollabConfig::new("a", 3))).unwrap(),
    );
    let at = view.state().text().len();
    view.update(edit(at..at, "a"), (), |_| (None, ())).unwrap();
    let checkpoint = view.state().clone();
    assert!(
        prepare_text_update_group(&checkpoint, &checkpoint)
            .unwrap()
            .is_none()
    );
    let prefix = sendable_updates(&checkpoint).unwrap();
    let at = checkpoint.text().len();
    view.start_group();
    view.update(edit(at..at, "ni"), (), |_| (None, ())).unwrap();
    view.update(edit(at..at + 2, "你好"), (), |_| (None, ()))
        .unwrap();
    view.end_group();
    let (receipt, grouped) = prepare_text_update_group(&checkpoint, view.state())
        .unwrap()
        .unwrap();
    assert!(grouped.changes().eq([(&(at..at), "你好")]));
    assert!(receipt.changes().is_empty());
    let selection = view.state().selection().clone();
    let old_pending = sendable_updates(view.state()).unwrap();
    assert_eq!(old_pending.len(), 3);
    let revision = view.revision();
    view.update(receipt, (), |_| (None, ())).unwrap();
    assert_eq!(view.revision(), revision);
    assert_eq!(view.undo_depth(), 2);
    assert_eq!(view.state().selection(), &selection);
    let grouped_updates = sendable_updates(view.state()).unwrap();
    assert_eq!(grouped_updates.len(), 2);
    assert_eq!(grouped_updates[0], prefix[0]);
    assert!(std::ptr::eq(
        grouped_updates[0].origin_context().unwrap(),
        prefix[0].origin_context().unwrap()
    ));
    let origin = grouped_updates[1].origin_context().unwrap();
    assert_eq!(origin.start_state().unwrap().text().as_str(), "名😀a");
    assert_eq!(origin.state().unwrap().text().as_str(), "名😀a你好");
    let start = origin.start_collab_state().unwrap();
    let result = origin.collab_state().unwrap();
    assert_eq!(start.sendable_updates().unwrap(), prefix);
    assert_eq!(result.sendable_updates().unwrap(), old_pending);
    assert_eq!(start.synced_version(), Ok(3));
    assert_eq!(result.synced_version(), Ok(3));
    assert_eq!(
        origin.transaction().user_event(),
        Some("input.type.compose")
    );
    assert_eq!(get_synced_version(view.state()).unwrap(), 3);
    let acknowledge = receive_updates(view.state(), 3, &grouped_updates).unwrap();
    view.update(acknowledge, (), |_| (None, ())).unwrap();
    assert_eq!(get_synced_version(view.state()).unwrap(), 5);
    assert!(sendable_updates(view.state()).unwrap().is_empty());
    assert_eq!(view.undo(), Some(()));
    assert_eq!(view.state().text().as_str(), "名😀a");
    assert_eq!(view.redo(), Some(()));
    assert_eq!(view.state().text().as_str(), "名😀a你好");
}

#[test]
fn canceled_composition_drops_its_suffix_without_advancing_the_authority_version() {
    let mut state =
        EditorState::with_extensions("test", collab(CollabConfig::new("a", 3))).unwrap();
    let checkpoint = state.clone();
    state.update(edit(0..0, "👀")).unwrap();
    state.update(edit(0.."👀".len(), "")).unwrap();
    let (receipt, grouped) = prepare_text_update_group(&checkpoint, &state)
        .unwrap()
        .unwrap();
    assert!(grouped.is_empty());
    assert_eq!(sendable_updates(&state).unwrap().len(), 2);
    state.update(receipt).unwrap();
    assert_eq!(state.text(), checkpoint.text());
    assert!(sendable_updates(&state).unwrap().is_empty());
    assert_eq!(get_synced_version(&state).unwrap(), 3);
}

#[test]
fn text_group_rejects_a_changed_base_stale_receipt_and_shared_effects() {
    let mut state = EditorState::with_extensions("abc", collab(CollabConfig::new("a", 3))).unwrap();
    let checkpoint = state.clone();
    state.update(edit(1..1, "字")).unwrap();
    let (receipt, _) = prepare_text_update_group(&checkpoint, &state)
        .unwrap()
        .unwrap();
    state.update(edit(0..0, "x")).unwrap();
    assert_eq!(
        state.update(receipt),
        Err(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        ))
    );
    let accepted =
        Update::new("b", ChangeSet::single(Change::new(3..3, "🙂")).unwrap(), 3).unwrap();
    state
        .update(receive_updates(&state, 3, &[accepted]).unwrap())
        .unwrap();
    assert!(matches!(
        prepare_text_update_group(&checkpoint, &state),
        Err(CollabError::StateMismatch)
    ));

    let mut reset = EditorState::with_extensions("abc", collab(CollabConfig::new("a", 3))).unwrap();
    reset.update(edit(0..0, "new session")).unwrap();
    assert!(matches!(
        prepare_text_update_group(&checkpoint, &reset),
        Err(CollabError::StateMismatch)
    ));

    let shared = StateEffectType::<bool>::new();
    let install = collab(CollabConfig::new("a", 3).with_shared_effects({
        let shared = shared.clone();
        move |transaction| {
            transaction
                .effects()
                .iter()
                .filter(|effect| effect.get(&shared).is_some())
                .cloned()
                .collect()
        }
    }));
    let mut state = EditorState::with_extensions("abc", install).unwrap();
    let checkpoint = state.clone();
    state
        .update(
            Transaction::default()
                .with_selection(EditorSelection::single(1, 1))
                .with_effects([shared.of(true)]),
        )
        .unwrap();
    assert!(matches!(
        prepare_text_update_group(&checkpoint, &state),
        Err(CollabError::SharedEffectsInTextGroup)
    ));
    assert_eq!(sendable_updates(&state).unwrap().len(), 1);
}
