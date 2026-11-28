use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, Facet, StateContext, StateEffectType, StateField, Transaction,
    collab::{
        Collab, CollabConfig, SharedEffectsContext, Update, collab, collab_state, receive_updates,
        sendable_updates,
    },
};
use serde_json::{Value, json};

#[test]
fn shared_effects_can_read_both_state_views_before_the_update_is_queued() {
    let pinned: Value =
        serde_json::from_str(include_str!("fixtures/collab_state_context.jsonl")).unwrap();
    for standalone in [false, true] {
        let bump = StateEffectType::<()>::new();
        let mark = StateEffectType::<Value>::new();
        let count = StateField::new(|_| Ok(0_u32), {
            let bump = bump.clone();
            move |count, update| {
                let bumps = update
                    .transaction()
                    .effects()
                    .iter()
                    .filter(|effect| effect.get(&bump).is_some())
                    .count();
                Ok(count + u32::try_from(bumps).unwrap())
            }
        });
        let label = Facet::new(|values: &[&&str]| values.first().map_or("", |value| **value));
        let total = Facet::new(|values: &[&u32]| values.first().map_or(0, |value| **value));
        let slot = Compartment::new();
        let describe = {
            let count = count.clone();
            let label = label.clone();
            let total = total.clone();
            move |state: &StateContext<'_>| {
                json!([
                    *state.field(&count).unwrap().unwrap(),
                    *state.facet(&label).unwrap(),
                    *state.facet(&total).unwrap(),
                    state.text().as_str(),
                    state.selection().main().head(),
                ])
            }
        };
        let share = {
            let describe = describe.clone();
            let mark = mark.clone();
            let bump = bump.clone();
            move |context: &SharedEffectsContext<'_>| {
                if !context
                    .transaction()
                    .effects()
                    .iter()
                    .any(|effect| effect.get(&bump).is_some())
                {
                    return Vec::new();
                }
                vec![mark.of(json!([
                    describe(context.start()),
                    describe(context.state()),
                    context.transaction().time()
                ]))]
            }
        };
        let extension = if standalone {
            Extension::default()
        } else {
            collab(CollabConfig::new("a", 7).with_shared_effects_state(share.clone()))
        };
        let initial = pinned["initial"].as_str().unwrap();
        let mut state = EditorState::with_extensions(
            initial,
            Extension::group([
                extension,
                total.from_field(count.clone(), |n| n * 2),
                count.extension(),
                slot.of(label.of("before")),
            ]),
        )
        .unwrap();
        let mut session =
            standalone.then(|| Collab::new("a", 7, &state).with_shared_effects_state(share));
        let at = usize::try_from(pinned["at"].as_u64().unwrap()).unwrap();
        let local = Transaction::default()
            .with_selection(EditorSelection::single(at, at))
            .with_time(19)
            .with_effects([bump.of(()), slot.reconfigure(label.of("after"))]);
        if let Some(session) = &mut session {
            session.apply_local(&mut state, local).unwrap();
        } else {
            state.update(local).unwrap();
        }
        let queued = |state: &EditorState, session: Option<&Collab>| {
            session.map_or_else(
                || sendable_updates(state).unwrap(),
                |session| session.sendable_updates().to_vec(),
            )
        };
        let verify = |state: &EditorState, session: Option<&Collab>, step| {
            let version = session.map_or_else(
                || collab_state(state).unwrap().unwrap().synced_version(),
                Collab::synced_version,
            );
            let updates = queued(state, session);
            let queue: Vec<_> = updates
                .iter()
                .map(|update| {
                    assert_eq!(update.client_id(), "a");
                    assert!(update.changes().is_empty());
                    assert_eq!(update.origin().unwrap().time(), Some(19));
                    assert_eq!(update.effects().len(), 1);
                    let captured = update.origin_context().unwrap();
                    let start = captured.start_collab_state().unwrap();
                    let result = captured.collab_state().unwrap();
                    assert_eq!(start.client_id(), "a");
                    assert_eq!(result.client_id(), "a");
                    assert_eq!(start.synced_version(), Ok(7));
                    assert_eq!(result.synced_version(), Ok(7));
                    assert!(start.sendable_updates().unwrap().is_empty());
                    let historical = result.sendable_updates().unwrap();
                    assert_eq!(historical.len(), 1);
                    assert!(std::ptr::eq(
                        historical[0].origin_context().unwrap(),
                        captured
                    ));
                    assert_eq!(historical[0].change_desc().length(), initial.len());
                    assert_eq!(update.change_desc().length(), state.text().len());
                    let expected = json!([
                        describe(&captured.start_state().unwrap()),
                        describe(&captured.state().unwrap()),
                        captured.transaction().time(),
                    ]);
                    let effect = update.effects()[0].get(&mark).unwrap();
                    assert_eq!(effect, &expected);
                    expected
                })
                .collect();
            assert_eq!(
                json!({"version": version, "state": describe(&state.context()), "queue": queue}),
                pinned[step]
            );
        };
        verify(&state, session.as_ref(), "before");
        let original = collab_state(&state)
            .unwrap()
            .map(|queue| Arc::downgrade(&queue));
        let peer = pinned["peer"].as_str().unwrap();
        let peer = Update::new(
            "b",
            ChangeSet::single(Change::new(0..0, peer)).unwrap(),
            initial.len(),
        )
        .unwrap();
        let dispatch =
            |state: &mut EditorState, session: Option<&mut Collab>, version, updates: &[Update]| {
                if let Some(session) = session {
                    session.receive_updates(state, version, updates).unwrap();
                } else {
                    state
                        .update(receive_updates(state, version, updates).unwrap())
                        .unwrap();
                }
            };
        dispatch(&mut state, session.as_mut(), 7, &[peer]);
        verify(&state, session.as_ref(), "received");
        let echoes = queued(&state, session.as_ref());
        dispatch(&mut state, session.as_mut(), 8, &echoes);
        verify(&state, session.as_ref(), "acknowledged");
        drop(state);
        drop(session);
        assert!(original.is_none_or(|queue| queue.upgrade().is_none()));
        let origin = echoes[0].origin_context().unwrap();
        assert_eq!(
            json!([
                describe(&origin.start_state().unwrap()),
                describe(&origin.state().unwrap()),
                origin.transaction().time()
            ]),
            pinned["before"]["queue"][0],
        );
    }
}

#[test]
fn internal_wast_publication_captures_effect_only_origins() {
    use gpui_editor_codemirror::{language_data::wast_language, view::EditorView};

    let effect = StateEffectType::<()>::new();
    let counter = StateField::new(|_| Ok(0_u32), |count, _| Ok(count + 1));
    let state = EditorState::with_extensions(
        "(module)\n".repeat(800),
        Extension::group([
            wast_language().viewport_extension().unwrap(),
            collab(CollabConfig::new("a", 0).with_shared_effects(move |_| vec![effect.of(())])),
            counter.extension(),
        ]),
    )
    .unwrap();
    let mut view: EditorView<()> = EditorView::new(state);
    let before = sendable_updates(view.state()).unwrap().len();
    let count = *view.state().context().field(&counter).unwrap().unwrap();
    assert!(view.force_wast_parsing(4500, 100_000).unwrap());
    let queue = sendable_updates(view.state()).unwrap();
    assert_eq!(queue.len(), before + 1);
    let publication = queue.last().unwrap();
    assert!(publication.changes().is_empty());
    let origin = publication.origin_context().unwrap();
    assert_eq!(
        *origin
            .start_state()
            .unwrap()
            .field(&counter)
            .unwrap()
            .unwrap(),
        count
    );
    assert_eq!(
        *origin.state().unwrap().field(&counter).unwrap().unwrap(),
        count + 1
    );
}

#[test]
fn a_rejected_field_update_cannot_publish_a_collaboration_origin() {
    let calls = Arc::new(AtomicUsize::new(0));
    let share = {
        let calls = calls.clone();
        move |_: &SharedEffectsContext<'_>| {
            calls.fetch_add(1, Ordering::Relaxed);
            Vec::new()
        }
    };
    let veto = StateField::new(
        |_| Ok(0),
        |count, update| {
            if update.state().text().as_str() == "bad" {
                Err(ConfigurationError::InvalidFieldData)
            } else {
                Ok(count + 1)
            }
        },
    );
    let mut state = EditorState::with_extensions(
        "abc",
        Extension::group([
            collab(CollabConfig::new("a", 2).with_shared_effects_state(share)),
            veto.extension(),
        ]),
    )
    .unwrap();
    let snapshot = collab_state(&state).unwrap().unwrap();
    let replace = |text| Transaction::new(ChangeSet::single(Change::new(0..3, text)).unwrap());
    assert_eq!(
        state.update(replace("bad")),
        Err(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        ))
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(Arc::ptr_eq(
        &collab_state(&state).unwrap().unwrap(),
        &snapshot
    ));
    assert!(sendable_updates(&state).unwrap().is_empty());
    assert_eq!(state.text().as_str(), "abc");
    state.update(replace("good")).unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    let updates = sendable_updates(&state).unwrap();
    let [update] = updates.as_slice() else {
        panic!("expected only the accepted origin")
    };
    let origin = update.origin_context().unwrap();
    assert_eq!(
        *origin.start_state().unwrap().field(&veto).unwrap().unwrap(),
        0
    );
    assert_eq!(*origin.state().unwrap().field(&veto).unwrap().unwrap(), 1);
}
