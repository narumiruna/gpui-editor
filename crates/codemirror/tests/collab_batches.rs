use gpui_editor_codemirror::{
    Change, ChangeSet, EditorState, Text, Transaction,
    collab::{
        Collab, CollabError, Update, UpdateDescription, rebase_updates, rebase_updates_described,
    },
};

#[test]
fn matching_batches_converge_and_other_batch_paths_never_silently_diverge() {
    let mut random = Random(0x52d9_a63b_80ef_147c);
    let mut mismatches = 0;
    for case in 0..512 {
        let source = ["a中🦊éz", "AAA語🙂", "e\u{301}\r\n犬", ""][case % 4];
        let start_version = 19;
        let mut state = EditorState::new(source);
        let mut client = Collab::new("local", start_version, &state);
        for _ in 0..3 {
            let change = random.edit(state.text());
            client
                .apply_local(&mut state, Transaction::new(change))
                .unwrap();
        }
        let push = client.sendable_updates().to_vec();
        let mut accepted = Vec::new();
        let mut authority = Text::new(source);
        for id in ["a", "b", "a"] {
            let remote = Update::new(id, random.edit(&authority), authority.len()).unwrap();
            remote.apply_to(&mut authority).unwrap();
            accepted.push(remote);
        }

        // When authority and client use the same partition, even overlapping edits
        // can be acknowledged. Every revision counts, including empty edits.
        for cuts in 0..4 {
            let mut matching_state = state.clone();
            let mut matching_client = client.clone();
            let mut version = start_version;
            let mut rebased = push.clone();
            for batch in batches(&accepted, cuts) {
                rebased = rebase_both(&rebased, batch);
                matching_client
                    .receive_updates(&mut matching_state, version, batch)
                    .unwrap();
                assert_eq!(
                    matching_client.sendable_updates(),
                    rebased,
                    "case {case}, cuts {cuts}"
                );
                version += batch.len();
            }
            let final_text = apply(authority.clone(), &rebased);
            for echo in rebased.chunks(1) {
                matching_client
                    .receive_updates(&mut matching_state, version, echo)
                    .unwrap();
                version += echo.len();
            }
            assert!(matching_client.sendable_updates().is_empty());
            assert_eq!(matching_client.synced_version(), version);
            assert_eq!(
                matching_state.text(),
                &final_text,
                "case {case}, cuts {cuts}"
            );
        }

        let mut log = accepted.clone();
        log.extend(rebase_both(&push, &accepted));
        assert!(rebase_both(&push, &log).is_empty());
        let mut other_state = state.clone();
        let mut other_client = client.clone();
        let mut synced = Text::new(source);
        let mut version = start_version;
        let mut rejected = false;
        for batch in batches(&log, case >> 2) {
            let previous = other_state.clone();
            let previous_pending = other_client.sendable_updates().to_vec();
            match other_client.receive_updates(&mut other_state, version, batch) {
                Ok(()) => {
                    synced = apply(synced, batch);
                    version += batch.len();
                    assert_eq!(other_client.synced_version(), version);
                    assert_eq!(
                        other_state.text(),
                        &apply(synced.clone(), other_client.sendable_updates()),
                        "case {case}"
                    );
                }
                Err(CollabError::MismatchedAcknowledgement) => {
                    mismatches += 1;
                    rejected = true;
                    assert_eq!(other_client.synced_version(), version);
                    assert_eq!(other_client.sendable_updates(), previous_pending);
                    assert_eq!(other_state.text(), previous.text());
                    assert_eq!(other_state.selection(), previous.selection());
                    break;
                }
                Err(error) => panic!("case {case}: {error:?}"),
            }
        }
        if !rejected {
            assert_eq!(version, start_version + log.len());
            assert!(other_client.sendable_updates().is_empty());
            assert_eq!(other_state.text(), &apply(Text::new(source), &log));
        }
    }
    assert!(
        mismatches > 0,
        "the generator must cover incompatible batch paths"
    );
}

fn rebase_both(push: &[Update], over: &[Update]) -> Vec<Update> {
    let full = rebase_updates(push, over).unwrap();
    let described = over
        .iter()
        .map(|update| UpdateDescription {
            client_id: update.client_id(),
            changes: update.change_desc(),
        })
        .collect::<Vec<_>>();
    assert_eq!(rebase_updates_described(push, &described).unwrap(), full);
    full
}

fn batches(updates: &[Update], cuts: usize) -> impl Iterator<Item = &[Update]> {
    let mut start = 0;
    (1..=updates.len()).filter_map(move |end| {
        if end != updates.len() && cuts & (1 << (end - 1)) == 0 {
            return None;
        }
        let batch = &updates[start..end];
        start = end;
        Some(batch)
    })
}

fn apply(mut text: Text, updates: &[Update]) -> Text {
    for update in updates {
        update.apply_to(&mut text).unwrap();
    }
    text
}

struct Random(u64);

impl Random {
    fn index(&mut self, end: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.0 >> 32) as usize % end
    }

    fn edit(&mut self, text: &Text) -> ChangeSet {
        let bounds: Vec<_> = text
            .as_str()
            .char_indices()
            .map(|(pos, _)| pos)
            .chain([text.len()])
            .collect();
        let mut positions = [(); 4].map(|()| bounds[self.index(bounds.len())]);
        positions.sort_unstable();
        let mut replacement = || ["", "A", "🦊", "語é", "\r\n"][self.index(5)];
        ChangeSet::new(vec![
            Change::new(positions[0]..positions[1], replacement()),
            Change::new(positions[2]..positions[3], replacement()),
        ])
        .unwrap()
    }
}
