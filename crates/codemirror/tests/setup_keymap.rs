//! Preset order and overlapping shortcuts checked with Chrome and npm `codemirror` 6.0.2.

use std::time::Duration;

use gpui_editor_codemirror::{
    EditorSelection, EditorState, Transaction,
    autocomplete::{
        AutocompleteCommand,
        session::{CompletionConfig, CompletionItem, CompletionSession, SourceId, SourceResult},
    },
    commands::{
        bindings::{CommandId, default_keymap, history_keymap},
        visual::{VerticalDistance, VisualCursor, VisualDirection, VisualGeometry},
    },
    lint::{self, Diagnostic, LintCommand, LintState, Severity},
    search::{SearchAction, search_panel_open},
    setup::{
        SetupCommand, SetupCompletion, SetupEditor, SetupGeometry, SetupHostAction,
        SetupKeyContext, SetupKeyResult, basic_setup, basic_setup_keymap, minimal_setup,
        minimal_setup_keymap,
    },
    view::keymap::{KeyBinding, KeyEvent, KeyModifiers, KeyPlatform},
};

fn state(source: &str, anchor: usize, head: usize) -> EditorState {
    let mut state = EditorState::new(source);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(anchor, head)))
        .unwrap();
    state
}

fn send(
    editor: &mut SetupEditor<()>,
    name: &str,
    modifiers: KeyModifiers,
    context: SetupKeyContext<'_>,
    ms: u64,
) -> SetupKeyResult<()> {
    editor
        .key_event_with(
            KeyEvent::new(name, modifiers),
            Duration::from_millis(ms),
            (),
            context,
        )
        .unwrap()
}

#[test]
fn catalogs_match_effective_order_in_the_pinned_keymap_facet() {
    use AutocompleteCommand::*;
    use SetupCommand::*;
    let commands = |keys: Vec<KeyBinding<SetupCommand>>| {
        keys.into_iter()
            .map(|key| key.run.unwrap())
            .collect::<Vec<_>>()
    };
    let minimal = commands(minimal_setup_keymap());
    let default: Vec<_> = default_keymap()
        .iter()
        .map(|key| Editor(key.run.unwrap()))
        .collect();
    let history: Vec<_> = history_keymap()
        .iter()
        .map(|key| Editor(key.run.unwrap()))
        .collect();
    assert_eq!(default.len(), 59);
    assert_eq!(history.len(), 5);
    assert_eq!(minimal, [default.clone(), history.clone()].concat());
    let completion = [
        StartCompletion,
        StartCompletion,
        StartCompletion,
        CloseCompletion,
        NextCompletion,
        PreviousCompletion,
        NextCompletionPage,
        PreviousCompletionPage,
        AcceptCompletion,
    ]
    .map(Completion);
    let search = [
        SearchAction::OpenSearchPanel,
        SearchAction::FindNext,
        SearchAction::FindNext,
        SearchAction::CloseSearchPanel,
        SearchAction::SelectSelectionMatches,
        SearchAction::GotoLine,
        SearchAction::SelectNextOccurrence,
    ]
    .map(Search);
    assert_eq!(
        commands(basic_setup_keymap()),
        [
            &completion[..],
            &[Completion(DeleteBracketPair)],
            &default,
            &search,
            &history,
            &[Fold, Unfold, FoldAll, UnfoldAll],
            &completion,
            &[
                Lint(LintCommand::OpenPanel),
                Lint(LintCommand::NextDiagnostic)
            ],
        ]
        .concat()
    );
    assert_eq!(minimal.last(), Some(&Editor(CommandId::RedoSelection)));
}

#[test]
fn search_and_lint_dispatch_state_and_request_host_focus() {
    let mut editor = basic_setup::<()>(state("one one one", 0, 3), KeyPlatform::Linux).unwrap();
    let mod_key = KeyModifiers {
        control: true,
        ..KeyModifiers::default()
    };
    let context = SetupKeyContext::default;
    assert!(send(&mut editor, "d", mod_key, context(), 1).key.handled);
    let ranges = editor.view().state().selection().ranges();
    assert_eq!(
        ranges.iter().map(|range| range.range()).collect::<Vec<_>>(),
        [0..3, 4..7]
    );
    let open = send(&mut editor, "f", mod_key, context(), 2);
    assert_eq!(open.host_action, Some(SetupHostAction::FocusSearchPanel));
    assert!(open.key.handled && search_panel_open(editor.view().state()));
    assert_eq!(
        editor.decorations().unwrap().search_matches.iter(0).count(),
        3
    );
    let no_mods = KeyModifiers::default();
    assert!(
        send(&mut editor, "Escape", no_mods, context(), 3)
            .key
            .handled
    );
    assert_eq!(editor.view().state().selection().ranges().len(), 1);
    assert!(search_panel_open(editor.view().state()));
    let panel = SetupKeyContext {
        scope: "search-panel",
        ..context()
    };
    assert!(send(&mut editor, "Escape", no_mods, panel, 4).key.handled);
    assert!(!search_panel_open(editor.view().state()));
    assert!(editor.decorations().unwrap().search_matches.is_empty());
    let goto = send(
        &mut editor,
        "g",
        KeyModifiers {
            alt: true,
            ..mod_key
        },
        context(),
        5,
    );
    assert_eq!(goto.host_action, Some(SetupHostAction::OpenGotoLine));
    assert!(goto.key.handled && goto.metadata.is_none());

    let diagnostic = lint::set_diagnostics_for(
        editor.view().state(),
        vec![Diagnostic::new(8..11, Severity::Warning, "example")],
    );
    editor
        .view_mut()
        .update(diagnostic, (), |_| (None, ()))
        .unwrap();
    assert!(send(&mut editor, "F8", no_mods, context(), 6).key.handled);
    assert_eq!(editor.view().state().selection().main().range(), 8..11);
    let mut lint_key = KeyEvent::new(
        "M",
        KeyModifiers {
            shift: true,
            ..mod_key
        },
    );
    lint_key.base_name = Some("m");
    let open = editor
        .key_event_with(lint_key, Duration::from_millis(7), (), context())
        .unwrap();
    assert!(open.key.handled);
    assert_eq!(open.host_action, Some(SetupHostAction::FocusLintPanel));
    assert!(LintState::get(editor.view().state()).unwrap().panel_open());
}

#[test]
fn highest_completion_bindings_win_before_arrow_escape_and_enter_fallbacks() {
    let mut editor = basic_setup::<()>(state("al\nrow", 2, 2), KeyPlatform::Linux).unwrap();
    let mut session = CompletionSession::new(
        editor.view().state(),
        CompletionConfig {
            interaction_delay: Duration::ZERO,
            ..CompletionConfig::default()
        },
        [SourceId(1)],
    );
    fn with(session: &mut CompletionSession) -> SetupKeyContext<'_> {
        SetupKeyContext {
            completion: Some(SetupCompletion {
                session,
                visible_options: 2,
            }),
            ..SetupKeyContext::default()
        }
    }
    let ctrl = KeyModifiers {
        control: true,
        ..KeyModifiers::default()
    };
    let start = send(&mut editor, " ", ctrl, with(&mut session), 1);
    assert!(start.key.handled && start.metadata.is_none());
    let due = session.deadline().unwrap();
    let [query] = session.take_queries(due).try_into().unwrap();
    assert!(query.explicit);
    assert!(
        session
            .finish_query(
                &query,
                Some(SourceResult::new(
                    0,
                    vec![CompletionItem::new("alpha"), CompletionItem::new("alpine")]
                )),
                due
            )
            .unwrap()
    );
    assert_eq!(session.selected_completion_index().unwrap(), Some(0));
    let no_mods = KeyModifiers::default();
    assert!(
        send(&mut editor, "ArrowDown", no_mods, with(&mut session), 1500)
            .key
            .handled
    );
    assert_eq!(session.selected_completion_index().unwrap(), Some(1));
    assert_eq!(editor.view().state().selection().main().head(), 2);
    let open = send(&mut editor, "f", ctrl, with(&mut session), 1501);
    assert_eq!(open.host_action, Some(SetupHostAction::FocusSearchPanel));
    assert!(
        send(&mut editor, "Escape", no_mods, with(&mut session), 1502)
            .key
            .handled
    );
    assert!(search_panel_open(editor.view().state()));
    assert!(
        send(&mut editor, "Escape", no_mods, with(&mut session), 1503)
            .key
            .handled
    );
    assert!(!search_panel_open(editor.view().state()));

    assert!(
        send(&mut editor, " ", ctrl, with(&mut session), 1504)
            .key
            .handled
    );
    let due = session.deadline().unwrap();
    let [query] = session.take_queries(due).try_into().unwrap();
    assert!(
        session
            .finish_query(
                &query,
                Some(SourceResult::new(0, vec![CompletionItem::new("alpine")])),
                due
            )
            .unwrap()
    );
    assert!(
        send(&mut editor, "Enter", no_mods, with(&mut session), 2200)
            .key
            .handled
    );
    assert_eq!(editor.view().state().text().as_str(), "alpine\nrow");
    assert!(
        send(
            &mut editor,
            "Enter",
            no_mods,
            SetupKeyContext::default(),
            2201
        )
        .key
        .handled
    );
    assert_eq!(editor.view().state().text().as_str(), "alpine\n\nrow");
}

struct WrappedRow;

impl VisualGeometry for WrappedRow {
    fn move_vertical(
        &self,
        start: VisualCursor,
        direction: VisualDirection,
        distance: VerticalDistance,
    ) -> VisualCursor {
        assert_eq!(
            (start.head, direction, distance),
            (0, VisualDirection::Forward, VerticalDistance::Line)
        );
        VisualCursor {
            head: 2,
            goal_column: Some(10.),
            ..start
        }
    }

    fn line_boundary(&self, start: VisualCursor, _: VisualDirection) -> VisualCursor {
        start
    }
}

#[test]
fn geometry_routes_arrow_keys_to_visual_commands_without_replacing_legacy_motion() {
    let state = state("abc\nrow", 0, 0);
    let mut minimal = minimal_setup::<()>(state.clone(), KeyPlatform::Linux);
    let mut logical = minimal_setup::<()>(state, KeyPlatform::Linux);
    let geometry = SetupGeometry {
        geometry: &WrappedRow,
        selection: None,
    };
    let visual = send(
        &mut minimal,
        "ArrowDown",
        KeyModifiers::default(),
        SetupKeyContext {
            geometry: Some(geometry),
            ..SetupKeyContext::default()
        },
        1,
    );
    assert!(visual.key.handled);
    assert_eq!(minimal.view().state().selection().main().head(), 2);
    assert_eq!(
        visual.visual_selection.unwrap().cursors()[0].goal_column,
        Some(10.)
    );
    let fallback = send(
        &mut logical,
        "ArrowDown",
        KeyModifiers::default(),
        SetupKeyContext::default(),
        1,
    );
    assert!(fallback.key.handled);
    assert!(fallback.visual_selection.is_none());
    assert_eq!(logical.view().state().selection().main().head(), 4);
}
