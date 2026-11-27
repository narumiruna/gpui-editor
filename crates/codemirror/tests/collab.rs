use std::{ops::Range, slice};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeDesc, ChangePermission, ChangeSet, ConfigurationError, EditError,
    EditorSelection, EditorState, MapMode, StateEffectType, StateField, Text, Transaction,
    change_filter,
    collab::{
        Collab, CollabError, Update, UpdateDescription, is_remote, rebase_updates,
        rebase_updates_described,
    },
};

#[test]
fn publishes_only_accepted_local_changes_and_does_not_filter_authoritative_edits() {
    let mut state = EditorState::with_extensions(
        "abcd",
        change_filter(|_, _| ChangePermission::Protect(std::iter::once(1..3).collect())),
    )
    .unwrap();
    let mut client = Collab::new("local", 0, &state);
    edit(&mut state, &mut client, 0..4, "!");
    assert_eq!(state.text().as_str(), "!bc");
    let accepted = client.sendable_updates().to_vec();
    assert_eq!(apply("abcd", &accepted).as_str(), "!bc");
    client.receive_updates(&mut state, 0, &accepted).unwrap();
    let remote = update("remote", 3, 1..2, "R");
    client.receive_updates(&mut state, 1, &[remote]).unwrap();
    assert_eq!(state.text().as_str(), "!Rc");
    assert!(client.sendable_updates().is_empty());
}

#[test]
fn rejects_out_of_session_mutation_even_if_the_document_length_matches() {
    let (mut state, mut client) = peer("abcd", "local", 0);
    edit(&mut state, &mut client, 0..1, "A");
    state.update(Transaction::new(changes(1..2, "B"))).unwrap();
    assert_eq!(
        client.apply_local(&mut state, Transaction::new(changes(2..3, "C"))),
        Err(CollabError::StateMismatch),
    );
    assert_eq!(state.text().as_str(), "ABcd");
    assert_eq!(client.sendable_updates().len(), 1);
}

fn peer(source: &str, client_id: &str, version: usize) -> (EditorState, Collab) {
    let state = EditorState::new(source);
    let collab = Collab::new(client_id, version, &state);
    (state, collab)
}

fn changes(range: Range<usize>, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(range, insert)).unwrap()
}

fn edit(state: &mut EditorState, collab: &mut Collab, range: Range<usize>, insert: &str) {
    collab
        .apply_local(state, Transaction::new(changes(range, insert)))
        .unwrap();
}

fn update(id: &str, length: usize, range: Range<usize>, insert: &str) -> Update {
    Update::new(id, changes(range, insert), length).unwrap()
}

fn describe(updates: &[Update]) -> Vec<UpdateDescription<'_>> {
    updates
        .iter()
        .map(|item| UpdateDescription {
            client_id: item.client_id(),
            changes: item.change_desc(),
        })
        .collect()
}

fn apply(source: &str, updates: &[Update]) -> Text {
    let mut text = Text::new(source);
    for update in updates {
        update.apply_to(&mut text).unwrap();
    }
    text
}

#[test]
fn converges_with_effect_only_updates_and_mapped_origins() {
    let effect = StateEffectType::with_mapping(|&(id, pos): &(char, usize), desc| {
        Some((id, desc.map_pos(pos, Assoc::After, MapMode::TrackDel)?))
    });
    let marks = StateField::new(|_| Ok(Vec::new()), {
        let effect = effect.clone();
        move |old: &Vec<(char, usize, bool)>, tr| {
            let mut mapped: Vec<_> = old
                .iter()
                .filter_map(|&(id, pos, remote)| {
                    Some((
                        id,
                        tr.changes().map_pos(pos, Assoc::After, MapMode::TrackDel)?,
                        remote,
                    ))
                })
                .collect();
            mapped.extend(tr.transaction().effects().iter().filter_map(|item| {
                let &(id, pos) = item.get(&effect)?;
                Some((id, pos, is_remote(tr.transaction())))
            }));
            mapped.sort_unstable();
            Ok(mapped)
        }
    });
    let peer = |id| {
        let state = EditorState::with_extensions("abcdef", marks.extension()).unwrap();
        let effect = effect.clone();
        let client = Collab::new(id, 0, &state).with_shared_effects(move |transaction| {
            transaction
                .effects()
                .iter()
                .filter(|item| item.get(&effect).is_some())
                .cloned()
                .collect()
        });
        (state, client)
    };
    let (mut a, mut alice) = peer("a");
    let (mut b, mut bob) = peer("b");
    alice
        .apply_local(
            &mut a,
            Transaction::new(changes(2..2, "X"))
                .with_user_event("input.type")
                .with_effects([effect.of(('A', 4))]),
        )
        .unwrap();
    bob.apply_local(
        &mut b,
        Transaction::default().with_effects([effect.of(('B', 5))]),
    )
    .unwrap();
    edit(&mut b, &mut bob, 0..0, "Y");
    let b_push = bob.sendable_updates().to_vec();
    assert!(b_push[0].changes().is_empty());
    assert_eq!(b_push[0].effects()[0].get(&effect), Some(&('B', 5)));
    let a_push = rebase_updates(alice.sendable_updates(), &b_push).unwrap();
    assert_eq!(a_push[0].effects()[0].get(&effect), Some(&('A', 5)));
    let described = rebase_updates_described(alice.sendable_updates(), &describe(&b_push)).unwrap();
    assert_eq!(described[0].changes(), a_push[0].changes());
    assert_eq!(described[0].effects()[0].get(&effect), Some(&('A', 5)));
    alice.receive_updates(&mut a, 0, &b_push).unwrap();
    let rebased = &alice.sendable_updates()[0];
    assert_eq!(rebased.effects()[0].get(&effect), Some(&('A', 5)));
    let origin = rebased.origin().unwrap();
    assert!(origin.is_user_event("input"));
    assert_eq!(origin.effects()[0].get(&effect), Some(&('A', 4)));
    alice.receive_updates(&mut a, 2, &a_push).unwrap();
    let mut authority = b_push;
    authority.extend(a_push);
    bob.receive_updates(&mut b, 0, &authority).unwrap();
    assert_eq!(a.text().as_str(), "YabXcdef");
    assert_eq!(a.text(), b.text());
    assert_eq!(
        *a.context().field(&marks).unwrap().unwrap(),
        [('A', 5, false), ('B', 7, true)]
    );
    assert_eq!(
        *b.context().field(&marks).unwrap().unwrap(),
        [('A', 5, true), ('B', 7, false)]
    );
    assert!(alice.sendable_updates().is_empty());
    assert!(bob.sendable_updates().is_empty());
    assert_eq!(bob.synced_version(), 3);
}

#[test]
fn rebase_discards_shared_effects_at_deleted_positions() {
    let mark = StateEffectType::with_mapping(|pos, changes| {
        changes.map_pos(*pos, Assoc::Before, MapMode::TrackDel)
    });
    let local = Update::new("a", ChangeSet::default(), 6)
        .unwrap()
        .with_effects([mark.of(3)]);
    let deletion = update("b", 6, 2..4, "");
    let rebased = rebase_updates(&[local], &[deletion]).unwrap();
    assert_eq!(rebased.len(), 1);
    assert!(rebased[0].changes().is_empty());
    assert!(rebased[0].effects().is_empty());
}

// Pinned upstream drops a TrackDel mark at the interior of a composed
// deletion even when each step would keep the mark on a deletion boundary.
#[test]
fn mapping_effects_uses_the_same_batch_boundaries_as_changes() {
    let mark = StateEffectType::with_mapping(|pos, changes| {
        changes.map_pos(*pos, Assoc::After, MapMode::TrackDel)
    });
    let (mut state, client) = peer("abc", "a", 0);
    let mut client = client.with_shared_effects(|tr| tr.effects().to_vec());
    client
        .apply_local(
            &mut state,
            Transaction::default().with_effects([mark.of(1)]),
        )
        .unwrap();
    let stale = client.sendable_updates().to_vec();
    let remote = [update("b", 3, 0..1, ""), update("c", 2, 0..1, "")];
    for batches in [&[remote.as_slice()][..], &[&remote[..1], &remote[1..]][..]] {
        let mut state = state.clone();
        let mut client = client.clone();
        let mut full = stale.clone();
        let mut described = stale.clone();
        let mut version = 0;
        for batch in batches {
            full = rebase_updates(&full, batch).unwrap();
            described = rebase_updates_described(&described, &describe(batch)).unwrap();
            client.receive_updates(&mut state, version, batch).unwrap();
            version += batch.len();
        }
        assert_eq!(state.text().as_str(), "c");
        let expected = (batches.len() == 2).then_some(&0);
        for queue in [
            full.as_slice(),
            described.as_slice(),
            client.sendable_updates(),
        ] {
            let [pending] = queue else {
                panic!("expected one effect-only update")
            };
            assert!(pending.changes().is_empty());
            assert_eq!(pending.effects().len(), usize::from(expected.is_some()));
            assert_eq!(
                pending
                    .effects()
                    .first()
                    .and_then(|effect| effect.get(&mark)),
                expected
            );
            assert_eq!(pending.origin().unwrap().effects()[0].get(&mark), Some(&1));
        }
    }
}

#[test]
fn descriptor_only_authority_rebases_and_deduplicates_by_geometry() {
    let a_one = update("a", 4, 2..2, "X");
    let a_two = update("a", 5, 4..5, "!");
    let submitted = [a_one, a_two];
    let mut authority = vec![update("b", 4, 0..1, "Bb")];
    let expected = rebase_updates(&submitted, &authority).unwrap();
    assert_eq!(
        rebase_updates_described(&submitted, &describe(&authority)).unwrap(),
        expected
    );
    authority.push(expected[0].clone());
    assert_eq!(
        rebase_updates_described(&submitted, &describe(&authority)).unwrap(),
        &expected[1..]
    );
    authority[1] = update("a", 5, 0..0, "X");
    assert_eq!(
        rebase_updates_described(&submitted, &describe(&authority)),
        Err(CollabError::MismatchedAcknowledgement)
    );
    authority[1] = update("a", 6, 0..0, "X");
    assert_eq!(
        rebase_updates_described(&submitted, &describe(&authority)),
        Err(CollabError::Edit(EditError::MismatchedLength))
    );
}

#[test]
fn described_echo_can_split_a_gap_or_insertion_but_not_a_replacement() {
    let local = update("a", 4, 4..4, "語!");
    let remote = update("b", 4, 0..0, "🦊");
    let rebased = rebase_updates(slice::from_ref(&local), slice::from_ref(&remote)).unwrap();
    assert_eq!(rebased[0].changes(), &changes(8..8, "語!"));
    let split = ChangeDesc::from_json(&serde_json::json!([4, -1, 4, -1, 0, 3, 0, 1])).unwrap();
    let accepted = [
        UpdateDescription {
            client_id: "b",
            changes: remote.change_desc(),
        },
        UpdateDescription {
            client_id: "a",
            changes: &split,
        },
    ];
    assert!(
        rebase_updates_described(&[local], &accepted)
            .unwrap()
            .is_empty()
    );

    let replacement = update("a", 4, 0..4, "é🙂");
    let split = ChangeDesc::from_json(&serde_json::json!([2, 2, 2, 4])).unwrap();
    assert_eq!(
        rebase_updates_described(
            &[replacement],
            &[UpdateDescription {
                client_id: "a",
                changes: &split
            }]
        ),
        Err(CollabError::MismatchedAcknowledgement),
    );
}

#[test]
fn shared_effect_policy_and_origin_keep_same_text_change_geometry() {
    let (mut state, client) = peer("aé🦊", "a", 0);
    let mark = StateEffectType::new();
    let mut client = client.with_shared_effects({
        let mark = mark.clone();
        move |tr| vec![mark.of(tr.changes().changes().count())]
    });
    let proposed = ChangeSet::new(vec![Change::new(1..3, "é"), Change::new(3..7, "語")]).unwrap();
    client
        .apply_local(
            &mut state,
            Transaction::new(proposed.clone()).with_user_event("input.type"),
        )
        .unwrap();
    assert_eq!(state.text().as_str(), "aé語");
    let pending = &client.sendable_updates()[0];
    let origin = pending.origin().unwrap();
    assert_eq!(origin.changes(), &proposed);
    assert_eq!(origin.changes(), pending.changes());
    assert_eq!(origin.user_event(), Some("input.type"));
    assert_eq!(pending.effects()[0].get(&mark), Some(&2));
}

#[test]
fn failed_remote_effect_does_not_commit_the_edit_or_version() {
    let reject = StateEffectType::new();
    let field = StateField::new(|_| Ok(()), {
        let reject = reject.clone();
        move |_, tr| {
            if tr
                .transaction()
                .effects()
                .iter()
                .any(|item| item.get(&reject) == Some(&true))
            {
                return Err(ConfigurationError::InvalidFieldData);
            }
            Ok(())
        }
    });
    let mut state = EditorState::with_extensions("abc", field.extension()).unwrap();
    let mut collab = Collab::new("a", 0, &state);
    let remote = update("b", 3, 1..2, "X").with_effects([reject.of(true)]);
    assert_eq!(
        collab.receive_updates(&mut state, 0, &[remote]),
        Err(CollabError::Edit(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        )))
    );
    assert_eq!(state.text().as_str(), "abc");
    assert_eq!(collab.synced_version(), 0);
}

// The convergence fixtures match @codemirror/collab@6.1.1 with state 6.7.6.
#[test]
fn three_peers_converge_with_stale_pushes_partial_acknowledgements_and_retries() {
    let source = "abcdef";
    let (mut a, mut alice) = peer(source, "a", 7);
    let (mut b, mut bob) = peer(source, "b", 7);
    let (mut c, mut carol) = peer(source, "c", 7);
    edit(&mut a, &mut alice, 1..1, "A");
    edit(&mut a, &mut alice, 4..6, "Z");
    edit(&mut b, &mut bob, 2..4, "B");
    edit(&mut c, &mut carol, 6..6, "!");
    assert_eq!(alice.client_id(), "a");
    assert_eq!(alice.synced_version(), 7);
    let a_push = alice.sendable_updates().to_vec();
    assert_eq!(a_push[0].client_id(), "a");
    assert_eq!(a_push[1].change_desc().length(), 7);

    let mut accepted = bob.sendable_updates().to_vec();
    accepted.extend(rebase_updates(&a_push[..1], &accepted).unwrap());
    accepted.extend(rebase_updates(carol.sendable_updates(), &accepted).unwrap());
    alice.receive_updates(&mut a, 7, &accepted).unwrap();
    assert_eq!(alice.synced_version(), 10);
    assert_eq!(alice.sendable_updates().len(), 1);

    let remaining = rebase_updates(&a_push, &accepted).unwrap();
    assert_eq!(remaining, alice.sendable_updates());
    accepted.extend_from_slice(&remaining);
    assert!(rebase_updates(&a_push, &accepted).unwrap().is_empty());

    alice.receive_updates(&mut a, 10, &remaining).unwrap();
    bob.receive_updates(&mut b, 7, &accepted[..1]).unwrap();
    bob.receive_updates(&mut b, 8, &accepted[1..]).unwrap();
    carol.receive_updates(&mut c, 7, &accepted).unwrap();
    let authority = apply(source, &accepted);
    assert_eq!(authority.as_str(), "aAbBZf!");
    for (state, collab) in [(&a, &alice), (&b, &bob), (&c, &carol)] {
        assert_eq!(state.text(), &authority);
        assert!(collab.sendable_updates().is_empty());
        assert_eq!(collab.synced_version(), 11);
    }
    assert_eq!(
        alice.receive_updates(&mut a, 10, &remaining),
        Err(CollabError::UnexpectedVersion {
            expected: 11,
            received: 10,
        })
    );
    assert_eq!(a.text(), &authority);
}

#[test]
fn same_position_insertions_follow_authority_order_not_client_id() {
    let source = "x";
    for (first_id, first_text, second_id, second_text, expected) in
        [("z", "Z", "a", "A", "ZAx"), ("a", "A", "z", "Z", "AZx")]
    {
        let (mut first, mut first_client) = peer(source, first_id, 0);
        let (mut second, mut second_client) = peer(source, second_id, 0);
        edit(&mut first, &mut first_client, 0..0, first_text);
        edit(&mut second, &mut second_client, 0..0, second_text);
        let mut accepted = first_client.sendable_updates().to_vec();
        accepted.extend(rebase_updates(second_client.sendable_updates(), &accepted).unwrap());
        assert_eq!(apply(source, &accepted).as_str(), expected);
        first_client
            .receive_updates(&mut first, 0, &accepted)
            .unwrap();
        second_client
            .receive_updates(&mut second, 0, &accepted)
            .unwrap();
        assert_eq!(first.text().as_str(), expected);
        assert_eq!(second.text().as_str(), expected);
    }
}

#[test]
fn unicode_changes_use_utf8_byte_coordinates() {
    let source = "a中🦊z";
    let (mut a, mut east) = peer(source, "east", 0);
    let (mut b, mut west) = peer(source, "west", 0);
    edit(&mut a, &mut east, 1..4, "語🙂");
    edit(&mut b, &mut west, 4..4, "?");
    edit(&mut b, &mut west, 5..9, "");
    let mut accepted = east.sendable_updates().to_vec();
    accepted.extend(rebase_updates(west.sendable_updates(), &accepted).unwrap());
    let authority = apply(source, &accepted);
    assert_eq!(authority.as_str(), "a語🙂?z");
    east.receive_updates(&mut a, 0, &accepted).unwrap();
    west.receive_updates(&mut b, 0, &accepted).unwrap();
    assert_eq!(a.text(), &authority);
    assert_eq!(b.text(), &authority);
}

#[test]
fn a_concurrent_deletion_rebased_to_empty_still_needs_acknowledgement() {
    let (mut state, mut client) = peer("x", "a", 0);
    edit(&mut state, &mut client, 0..1, "");
    let first = update("b", 1, 0..1, "");
    let rebased = rebase_updates(client.sendable_updates(), slice::from_ref(&first)).unwrap();
    assert_eq!(rebased.len(), 1);
    assert!(rebased[0].changes().is_empty());
    client.receive_updates(&mut state, 0, &[first]).unwrap();
    assert_eq!(client.synced_version(), 1);
    assert_eq!(client.sendable_updates(), rebased);
    client.receive_updates(&mut state, 1, &rebased).unwrap();
    assert_eq!(client.synced_version(), 2);
    assert!(client.sendable_updates().is_empty());
    assert!(state.text().is_empty());
}

#[test]
fn a_contradictory_echo_leaves_editor_and_session_unchanged() {
    let (mut state, mut client) = peer("abc", "a", 5);
    let selection = EditorSelection::single(4, 4);
    client
        .apply_local(
            &mut state,
            Transaction::new(changes(3..3, "!")).with_selection(selection.clone()),
        )
        .unwrap();
    let pending = client.sendable_updates().to_vec();
    let remote = update("b", 3, 0..0, "B");
    let bad_echo = update("a", 4, 4..4, "?");
    let accepted = [remote.clone(), bad_echo];
    assert_eq!(
        client.receive_updates(&mut state, 5, &accepted),
        Err(CollabError::MismatchedAcknowledgement)
    );
    assert_eq!(
        rebase_updates(&pending, &accepted),
        Err(CollabError::MismatchedAcknowledgement)
    );
    assert_eq!(state.text().as_str(), "abc!");
    assert_eq!(state.selection(), &selection);
    assert_eq!(client.sendable_updates(), pending);
    assert_eq!(client.synced_version(), 5);

    let good_echo = update("a", 4, 4..4, "!");
    client
        .receive_updates(&mut state, 5, &[remote, good_echo])
        .unwrap();
    assert_eq!(state.text().as_str(), "Babc!");
    assert_eq!(state.selection(), &EditorSelection::single(5, 5));
    assert_eq!(client.synced_version(), 7);
    assert!(client.sendable_updates().is_empty());
    assert_eq!(
        client.receive_updates(&mut state, 7, &[update("a", 5, 0..0, "?")]),
        Err(CollabError::UnexpectedAcknowledgement)
    );
    assert_eq!(state.text().as_str(), "Babc!");
}

#[test]
fn an_echo_can_join_insertions_at_the_same_position() {
    let (mut state, mut client) = peer("x", "a", 0);
    let split = ChangeSet::new(vec![Change::new(0..0, "A"), Change::new(0..0, "B")]).unwrap();
    client
        .apply_local(&mut state, Transaction::new(split))
        .unwrap();
    let joined = update("a", 1, 0..0, "AB");
    assert!(
        rebase_updates(client.sendable_updates(), slice::from_ref(&joined))
            .unwrap()
            .is_empty()
    );
    client.receive_updates(&mut state, 0, &[joined]).unwrap();
    assert_eq!(state.text().as_str(), "ABx");
    assert_eq!(client.synced_version(), 1);
    assert!(client.sendable_updates().is_empty());
}

#[test]
fn invalid_utf8_boundaries_and_broken_authority_chains_are_atomic() {
    let (mut state, mut client) = peer("éx", "a", 13);
    edit(&mut state, &mut client, 3..3, "!");
    let pending = client.sendable_updates().to_vec();
    let first = update("b", 3, 3..3, "?");
    let invalid = update("c", 4, 0..1, "");
    let masks_invalid_range = update("c", 3, 0..1, "");
    assert_eq!(
        client.receive_updates(
            &mut state,
            13,
            &[first.clone(), invalid, masks_invalid_range]
        ),
        Err(CollabError::Edit(EditError::InvalidRange))
    );
    let wrong_length = update("c", 3, 0..0, "z");
    let broken = [first, wrong_length];
    assert_eq!(
        client.receive_updates(&mut state, 13, &broken),
        Err(CollabError::Edit(EditError::MismatchedLength))
    );
    assert_eq!(
        rebase_updates(&pending, &broken),
        Err(CollabError::Edit(EditError::MismatchedLength))
    );
    assert_eq!(state.text().as_str(), "éx!");
    assert_eq!(client.synced_version(), 13);
    assert_eq!(client.sendable_updates(), pending);
}

#[test]
fn selection_only_edits_and_failed_local_transactions_do_not_queue_updates() {
    let (mut state, mut client) = peer("éx", "a", 1);
    let selection = EditorSelection::single(2, 2);
    client
        .apply_local(
            &mut state,
            Transaction::default().with_selection(selection.clone()),
        )
        .unwrap();
    assert_eq!(state.selection(), &selection);
    assert_eq!(
        client.apply_local(
            &mut state,
            Transaction::new(changes(2..3, "y")).with_selection(EditorSelection::single(1, 1)),
        ),
        Err(CollabError::Edit(EditError::InvalidSelection))
    );
    assert_eq!(
        client.apply_local(&mut state, Transaction::new(changes(1..2, ""))),
        Err(CollabError::Edit(EditError::InvalidRange))
    );
    assert_eq!(state.text().as_str(), "éx");
    assert_eq!(state.selection(), &selection);
    assert_eq!(client.synced_version(), 1);
    assert!(client.sendable_updates().is_empty());
}

#[test]
fn versions_reject_gaps_and_overflow_but_allow_empty_batches() {
    let (mut state, mut client) = peer("x", "a", usize::MAX);
    assert_eq!(
        client.receive_updates(&mut state, 0, &[]),
        Err(CollabError::UnexpectedVersion {
            expected: usize::MAX,
            received: 0,
        })
    );
    client.receive_updates(&mut state, usize::MAX, &[]).unwrap();
    assert_eq!(
        client.receive_updates(&mut state, usize::MAX, &[update("b", 1, 0..0, "B")]),
        Err(CollabError::VersionOverflow)
    );
    assert_eq!(client.synced_version(), usize::MAX);
    assert_eq!(state.text().as_str(), "x");

    let (mut state, mut client) = peer("x", "a", 0);
    assert_eq!(
        client.receive_updates(&mut state, 1, &[]),
        Err(CollabError::UnexpectedVersion {
            expected: 0,
            received: 1,
        })
    );
}

// Verified against collab 6.1.1 and state 6.7.6. Upstream clears the queue
// and advances the version even when the echo uses the opposite batch path.
#[test]
fn incompatible_batch_paths_are_rejected_even_if_the_resulting_text_matches() {
    // YY yields different documents. X yields XXbc on both paths but
    // the local insertion maps to a different position.
    for (replacement, want_batch, want_sequential) in
        [("YY", "XYYbc", "YYXbc"), ("X", "XXbc", "XXbc")]
    {
        let (mut state, mut client) = peer("abc", "local", 0);
        edit(&mut state, &mut client, 0..0, "X");
        let stale_push = client.sendable_updates().to_vec();
        let remote = [update("b", 3, 0..0, "X"), update("c", 4, 0..2, replacement)];
        let base = apply("abc", &remote);
        let batched_echo = rebase_updates(&stale_push, &remote).unwrap();
        let first = rebase_updates(&stale_push, &remote[..1]).unwrap();
        let sequential_echo = rebase_updates(&first, &remote[1..]).unwrap();
        assert_eq!(batched_echo[0].changes(), &changes(0..0, "X"));
        assert_eq!(
            sequential_echo[0].changes(),
            &changes(replacement.len()..replacement.len(), "X")
        );
        assert_eq!(apply(base.as_str(), &batched_echo).as_str(), want_batch);
        assert_eq!(
            apply(base.as_str(), &sequential_echo).as_str(),
            want_sequential
        );

        let mut wrong_log = remote.to_vec();
        wrong_log.extend_from_slice(&sequential_echo);
        assert_eq!(
            rebase_updates(&stale_push, &wrong_log),
            Err(CollabError::MismatchedAcknowledgement)
        );
        assert_eq!(
            rebase_updates_described(&stale_push, &describe(&wrong_log)),
            Err(CollabError::MismatchedAcknowledgement)
        );
        assert_eq!(
            client.receive_updates(&mut state, 0, &wrong_log),
            Err(CollabError::MismatchedAcknowledgement)
        );
        assert_eq!(client.synced_version(), 0);
        assert_eq!(client.sendable_updates(), stale_push);
        assert_eq!(state.text().as_str(), "Xabc");

        let mut batch_state = state.clone();
        let mut batch_client = client.clone();
        batch_client
            .receive_updates(&mut batch_state, 0, &remote)
            .unwrap();
        client.receive_updates(&mut state, 0, &remote[..1]).unwrap();
        client.receive_updates(&mut state, 1, &remote[1..]).unwrap();
        for (state, client, wrong, right, expected) in [
            (
                &mut batch_state,
                &mut batch_client,
                &sequential_echo,
                &batched_echo,
                want_batch,
            ),
            (
                &mut state,
                &mut client,
                &batched_echo,
                &sequential_echo,
                want_sequential,
            ),
        ] {
            assert_eq!(client.sendable_updates(), right);
            assert_eq!(
                client.receive_updates(state, 2, wrong),
                Err(CollabError::MismatchedAcknowledgement)
            );
            assert_eq!(client.synced_version(), 2);
            assert_eq!(client.sendable_updates(), right);
            assert_eq!(state.text().as_str(), expected);
            client.receive_updates(state, 2, right).unwrap();
            assert_eq!(client.synced_version(), 3);
            assert!(client.sendable_updates().is_empty());
            assert_eq!(state.text().as_str(), expected);
        }
    }
}
