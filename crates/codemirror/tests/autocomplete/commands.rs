use std::{slice::from_ref, time::Duration};

use gpui_editor_codemirror::{
    EditorSelection, EditorState, READ_ONLY, Transaction,
    autocomplete::{
        AutocompleteCommand::{self, *},
        Snippet, close_brackets_keymap, completion_keymap, has_next_snippet_field,
        has_previous_snippet_field, next_snippet_field,
        session::{
            CompletionConfig, CompletionItem, CompletionSession, CompletionStatus, CompletionStep,
            SourceId, SourceResult,
        },
        snippet_keymap, snippet_state_extension, start_snippet,
    },
    view::keymap::{KeyBinding, KeyEvent, KeyModifiers, KeyPlatform, KeySequence, Keymap},
};

const SOURCE: SourceId = SourceId(1);

fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

// Binding order and platform fields from the pinned 6.20.4 oracle.
#[test]
fn default_binding_catalogs_are_exact_and_ordered() {
    let fields = |b: KeyBinding<AutocompleteCommand>| {
        assert_eq!(b.scope, "editor");
        assert!(b.windows.is_none() && b.linux.is_none() && b.any.is_none());
        assert!(!b.prevent_default && !b.stop_propagation);
        (b.key, b.mac, b.run.unwrap(), b.shift)
    };
    assert_eq!(
        completion_keymap().map(fields),
        [
            (Some("Ctrl-Space".into()), None, StartCompletion, None),
            (None, Some("Alt-`".into()), StartCompletion, None),
            (None, Some("Alt-i".into()), StartCompletion, None),
            (Some("Escape".into()), None, CloseCompletion, None),
            (Some("ArrowDown".into()), None, NextCompletion, None),
            (Some("ArrowUp".into()), None, PreviousCompletion, None),
            (Some("PageDown".into()), None, NextCompletionPage, None),
            (Some("PageUp".into()), None, PreviousCompletionPage, None),
            (Some("Enter".into()), None, AcceptCompletion, None),
        ]
    );
    assert_eq!(
        close_brackets_keymap().map(fields),
        [(Some("Backspace".into()), None, DeleteBracketPair, None)]
    );
    assert_eq!(
        snippet_keymap().map(fields),
        [
            (
                Some("Tab".into()),
                None,
                NextSnippetField,
                Some(PreviousSnippetField)
            ),
            (Some("Escape".into()), None, ClearSnippet, None),
        ]
    );
}

#[test]
fn keymap_dispatch_selects_platform_and_shift_and_falls_through() {
    for platform in [KeyPlatform::Mac, KeyPlatform::Linux] {
        let keymap = Keymap::new(
            platform,
            snippet_keymap().into_iter().chain(completion_keymap()),
        )
        .unwrap();
        let dispatch = |key, modifiers, decline| {
            let mut commands = Vec::new();
            let result = keymap.dispatch(
                "editor",
                KeyEvent::new(key, modifiers),
                at(0),
                &mut KeySequence::default(),
                |&command| {
                    commands.push(command);
                    command != decline
                },
            );
            (commands, result.handled)
        };
        let (commands, handled) = dispatch(
            "i",
            KeyModifiers {
                alt: true,
                ..KeyModifiers::default()
            },
            AcceptCompletion,
        );
        assert_eq!(handled, platform == KeyPlatform::Mac);
        assert_eq!(
            commands,
            if handled {
                vec![StartCompletion]
            } else {
                vec![]
            }
        );
        assert_eq!(
            dispatch(
                "Tab",
                KeyModifiers {
                    shift: true,
                    ..KeyModifiers::default()
                },
                AcceptCompletion
            ),
            (vec![PreviousSnippetField], true)
        );
        assert_eq!(
            dispatch("Escape", KeyModifiers::default(), ClearSnippet),
            (vec![ClearSnippet, CloseCompletion], true)
        );
        assert_eq!(
            dispatch("Enter", KeyModifiers::default(), AcceptCompletion),
            (vec![AcceptCompletion], false)
        );
    }
}

#[test]
fn session_commands_report_pending_active_and_inactive_state() {
    let mut state = EditorState::new("f");
    state
        .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
        .unwrap();
    let mut empty = CompletionSession::new(&state, CompletionConfig::default(), []);
    assert!(!empty.start_completion(at(0)));
    assert!(!empty.close_completion());
    let mut session = CompletionSession::new(&state, CompletionConfig::default(), [SOURCE]);
    assert_eq!(session.status(), CompletionStatus::Inactive);
    assert!(session.start_completion(at(0)));
    assert_eq!(session.status(), CompletionStatus::Pending);
    assert!(session.options().unwrap().is_empty());
    assert_eq!(session.selected_completion_index().unwrap(), None);
    assert!(!session.set_selected_completion(Some(0)).unwrap());
    let cancelled = session.take_queries(at(50)).pop().unwrap();
    assert!(session.close_completion());
    assert!(cancelled.is_cancelled());
    assert!(!session.close_completion());
    assert!(session.start_completion(at(60)));
    let query = session.take_queries(at(110)).pop().unwrap();
    assert!(query.explicit);
    let options = ["faa", "far", "foo"].map(CompletionItem::new);
    session
        .finish_query(&query, Some(SourceResult::new(0, options)), at(110))
        .unwrap();
    assert_eq!(session.status(), CompletionStatus::Active);
    assert_eq!(session.selected_completion_index().unwrap(), Some(0));
    assert!(session.set_selected_completion(Some(2)).unwrap());
    assert!(!session.set_selected_completion(Some(2)).unwrap());
    assert!(!session.set_selected_completion(Some(99)).unwrap());
    assert_eq!(
        session.selected_completion().unwrap().unwrap().item.label,
        "foo"
    );
    assert!(
        !session
            .move_selection(true, CompletionStep::Option, at(184))
            .unwrap()
    );
    assert!(
        session
            .move_selection(true, CompletionStep::Option, at(185))
            .unwrap()
    );
    assert_eq!(session.selected_completion_index().unwrap(), Some(0));
    assert!(
        session
            .move_selection(false, CompletionStep::Option, at(185))
            .unwrap()
    );
    assert_eq!(session.selected_completion_index().unwrap(), Some(2));
    assert!(
        session
            .move_selection(true, CompletionStep::Page(5), at(185))
            .unwrap()
    );
    assert_eq!(session.selected_completion_index().unwrap(), Some(2));
    assert!(session.set_selected_completion(None).unwrap());
    assert!(session.selected_completion().unwrap().is_none());
    assert!(session.accept_selected(at(185)).unwrap().is_none());
    assert!(
        session
            .move_selection(false, CompletionStep::Page(5), at(185))
            .unwrap()
    );
    let edit = session.accept_selected(at(185)).unwrap().unwrap();
    state.update(edit).unwrap();
    assert_eq!(state.text().as_str(), "foo");
    assert!(session.close_completion());
    assert!(session.selected_completion().unwrap().is_none());
}

#[test]
fn selected_completion_queries_do_not_require_a_writable_document() {
    let state = EditorState::with_extensions("", READ_ONLY.of(true)).unwrap();
    let mut session = CompletionSession::new(&state, CompletionConfig::default(), [SOURCE]);
    assert!(session.start_completion(at(0)));
    let query = session.take_queries(at(50)).pop().unwrap();
    session
        .finish_query(
            &query,
            Some(SourceResult::new(0, [CompletionItem::new("foo")])),
            at(50),
        )
        .unwrap();
    assert_eq!(
        session.selected_completion().unwrap().unwrap().item.label,
        "foo"
    );
    assert!(session.accept_selected(at(125)).unwrap().is_none());
}

#[test]
fn snippet_field_queries_follow_navigation_and_exit() {
    let mut state = EditorState::with_extensions("", snippet_state_extension()).unwrap();
    let navigation = |state: &EditorState| {
        (
            has_next_snippet_field(state),
            has_previous_snippet_field(state),
        )
    };
    assert_eq!(navigation(&state), (false, false));
    let (insert, snippet) = Snippet::parse("${first} ${second}${0}")
        .insert(&state, 0..0, "  ")
        .unwrap();
    state
        .update(insert.with_effects([start_snippet(snippet.unwrap())]))
        .unwrap();
    assert_eq!(navigation(&state), (true, false));
    state.update(next_snippet_field(&state).unwrap()).unwrap();
    assert_eq!(navigation(&state), (true, true));
    assert_eq!(state.selection().main().range(), 6..12);
    state.update(next_snippet_field(&state).unwrap()).unwrap();
    assert_eq!(navigation(&state), (false, false));
}

#[test]
fn source_highlights_apply_only_to_display_labels_and_unfiltered_options() {
    let mut state = EditorState::new("f");
    state
        .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
        .unwrap();
    let mut session =
        CompletionSession::new(&state, CompletionConfig::default(), [SOURCE, SourceId(2)]);
    assert!(session.start_completion(at(0)));
    let queries = session.take_queries(at(50));
    let mut display = CompletionItem::new("fa");
    display.display_label = Some("é/fa".into());
    let mut unmatched = CompletionItem::new("none");
    unmatched.display_label = Some("filtered out".into());
    let filtered = SourceResult::new(0, [display, CompletionItem::new("foo"), unmatched])
        .with_match_ranges(|item, matched| {
            assert_eq!(item.label, "fa");
            assert_eq!(matched.unwrap(), from_ref(&(0..1)));
            // Offset 2 in the JS fixture is byte 3 here.
            std::iter::once(3..4).collect()
        });
    let mut provided =
        SourceResult::new(0, [CompletionItem::new("given")]).with_match_ranges(|item, matched| {
            assert_eq!(item.label, "given");
            assert!(matched.is_none());
            std::iter::once(1..4).collect()
        });
    provided.filter = false;
    session
        .finish_query(&queries[0], Some(filtered), at(50))
        .unwrap();
    session
        .finish_query(&queries[1], Some(provided), at(50))
        .unwrap();
    let options = session.options().unwrap();
    let rows: Vec<_> = options
        .iter()
        .map(|option| (option.item.label.as_str(), option.match_ranges.as_slice()))
        .collect();
    assert_eq!(
        rows,
        [
            ("given", from_ref(&(1..4))),
            ("fa", from_ref(&(3..4))),
            ("foo", from_ref(&(0..1)))
        ]
    );
}
