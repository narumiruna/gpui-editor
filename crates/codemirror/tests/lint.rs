use std::{
    ops::Range,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Extension, Transaction,
    TransactionCoordinates,
    lint::{
        self, Action, DEFAULT_LINT_DELAY, Diagnostic, LintConfig, LintRun, LintScheduler,
        LintSession, LintSource, LintState, Severity, clear_diagnostics, diagnostic_count,
        for_each_diagnostic, force_linting, linter, next_diagnostic, open_lint_panel,
        previous_diagnostic, run_action, select_diagnostic, set_diagnostics,
        set_diagnostics_effect, set_diagnostics_for,
    },
};

fn state(text: &str) -> EditorState {
    EditorState::with_extensions(text, lint::extension()).unwrap()
}

fn diagnostic(range: Range<usize>, message: &str) -> Diagnostic {
    Diagnostic::new(range, Severity::Warning, message)
}

fn change(range: Range<usize>, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, insert)).unwrap())
}

fn positions(state: &EditorState) -> Vec<(String, Range<usize>, Range<usize>)> {
    let mut diagnostics = Vec::new();
    for_each_diagnostic(state, |entry| {
        diagnostics.push((
            entry.diagnostic.message.clone(),
            entry.range,
            entry.diagnostic.range.clone(),
        ));
    });
    diagnostics
}

#[test]
fn overlapping_diagnostics_follow_upstream_marker_count_and_order() {
    let mut state = state("abcdefghij");
    let mut inner = Diagnostic::new(3..6, Severity::Error, "inner");
    inner.source = Some("example".into());
    inner.mark_class = Some("important".into());
    state
        .update(set_diagnostics(vec![
            diagnostic(7..8, "tail"),
            inner,
            diagnostic(1..7, "outer"),
            Diagnostic::new(4..4, Severity::Info, "point"),
        ]))
        .unwrap();

    assert_eq!(diagnostic_count(&state), 6);
    assert_eq!(
        positions(&state),
        [
            ("point".into(), 4..4, 4..4),
            ("inner".into(), 3..6, 3..6),
            ("outer".into(), 1..7, 1..7),
            ("tail".into(), 7..8, 7..8),
        ]
    );
    let lint = LintState::get(&state).unwrap();
    let shared = lint.markers().iter(0).find(|m| m.from == 3).unwrap();
    assert_eq!(shared.value.severity(), Severity::Error);
    assert_eq!(
        shared.value.diagnostics()[1].source.as_deref(),
        Some("example")
    );
    assert_eq!(
        shared.value.diagnostics()[1].mark_class.as_deref(),
        Some("important")
    );
    assert_eq!(lint.selected().unwrap().range, 1..7);

    check_navigation(state.clone(), next_diagnostic, &[1..7, 7..8, 1..7]);
    check_navigation(
        state.clone(),
        previous_diagnostic,
        &[7..8, 6..7, 4..6, 4..4, 1..3, 7..8],
    );

    state.update(change(3..3, "XY")).unwrap();
    assert_eq!(
        positions(&state),
        [
            ("point".into(), 6..6, 4..4),
            ("inner".into(), 5..8, 3..6),
            ("outer".into(), 1..9, 1..7),
            ("tail".into(), 9..10, 7..8),
        ]
    );
    state.update(change(6..6, "!")).unwrap();
    assert_eq!(
        positions(&state),
        [
            ("point".into(), 6..6, 4..4),
            ("inner".into(), 5..9, 3..6),
            ("outer".into(), 1..10, 1..7),
            ("tail".into(), 10..11, 7..8),
        ]
    );
    state.update(change(0..11, "")).unwrap();
    assert_eq!(diagnostic_count(&state), 0);
    assert!(LintState::get(&state).unwrap().selected().is_none());
}

fn check_navigation(
    mut state: EditorState,
    navigate: impl Fn(&EditorState) -> Option<Transaction>,
    expected: &[Range<usize>],
) {
    for range in expected {
        let transaction = navigate(&state).unwrap();
        assert!(transaction.scroll_into_view());
        state.update(transaction).unwrap();
        assert_eq!(&state.selection().main().range(), range);
    }
}

#[test]
fn newline_points_and_end_of_document_follow_upstream() {
    let mut state = state("abc\n\ndef");
    state
        .update(set_diagnostics(vec![
            diagnostic(3..5, "breaks"),
            diagnostic(8..8, "end"),
        ]))
        .unwrap();
    assert_eq!(
        positions(&state),
        [("breaks".into(), 3..3, 3..5), ("end".into(), 8..8, 8..8)]
    );

    let mut state = EditorState::with_extensions("abc", lint::extension()).unwrap();
    state
        .update(set_diagnostics(vec![
            diagnostic(0..3, "whole"),
            diagnostic(3..3, "end"),
        ]))
        .unwrap();
    assert_eq!(positions(&state), [("whole".into(), 0..3, 0..3)]);
    state.update(next_diagnostic(&state).unwrap()).unwrap();
    assert!(next_diagnostic(&state).is_none());

    let mut state = EditorState::with_extensions("a\r\nb\rc", lint::extension()).unwrap();
    state
        .update(set_diagnostics(vec![
            diagnostic(1..3, "CRLF"),
            diagnostic(4..5, "CR"),
        ]))
        .unwrap();
    assert_eq!(
        positions(&state),
        [("CRLF".into(), 1..1, 1..3), ("CR".into(), 4..5, 4..5)]
    );
}

#[test]
fn maps_utf8_edges_and_does_not_move_editor_selection_on_publication() {
    let mut state = state("a🌿bc");
    let selection = EditorSelection::single(6, 5);
    state
        .update(Transaction::default().with_selection(selection.clone()))
        .unwrap();
    state
        .update(set_diagnostics(vec![
            diagnostic(1..5, "leaf"),
            diagnostic(5..6, "b"),
        ]))
        .unwrap();
    assert_eq!(state.selection(), &selection);
    let insertions = ChangeSet::new(vec![Change::new(1..1, "é"), Change::new(5..5, "!")]).unwrap();
    state.update(Transaction::new(insertions)).unwrap();
    assert_eq!(
        positions(&state),
        [("leaf".into(), 3..7, 1..5), ("b".into(), 8..9, 5..6)]
    );
    assert_eq!(
        LintState::get(&state).unwrap().selected().unwrap().range,
        3..7
    );
    state.update(change(3..7, "")).unwrap();
    assert_eq!(
        LintState::get(&state).unwrap().selected().unwrap().range,
        4..5
    );
}

#[test]
fn typed_publication_uses_result_coordinates_and_skips_invalid_ranges() {
    let extension = Extension::group([
        lint::extension(),
        lint::marker_filter(|d, _| d.severity != Severity::Hint),
        lint::marker_filter(|d, state| state.text().slice(d.range.clone()) != Some("x")),
    ]);
    let mut state = EditorState::with_extensions("x", extension).unwrap();
    let publish = set_diagnostics_effect().of(vec![
        Diagnostic::new(0..4, Severity::Hint, "filtered"),
        diagnostic(0..4, "valid"),
        diagnostic(1..2, "split scalar"),
        diagnostic(4..5, "filtered by text"),
        diagnostic(4..8, "past document"),
        diagnostic(Range { start: 5, end: 4 }, "reversed"),
    ]);
    let transaction = change(0..0, "🌿").with_effects([publish]);
    state.update(transaction).unwrap();
    assert_eq!(positions(&state), [("valid".into(), 0..4, 0..4)]);

    let publish = set_diagnostics(vec![diagnostic(0..4, "not rebased")]);
    let merged = publish
        .merge(change(0..0, "!"), 5, TransactionCoordinates::Sequential)
        .unwrap();
    let mapped = merged.effects()[0].get(set_diagnostics_effect()).unwrap();
    assert_eq!(mapped[0].range, 0..4);
    state.update(merged).unwrap();
    assert_eq!(diagnostic_count(&state), 0);

    let mut disabled = EditorState::new("x");
    assert!(LintState::get(&disabled).is_none());
    assert!(next_diagnostic(&disabled).is_none());
    assert!(previous_diagnostic(&disabled).is_none());
    disabled
        .update(Transaction::default().with_effects([lint::extension().append()]))
        .unwrap();
    disabled
        .update(set_diagnostics(vec![diagnostic(0..1, "enabled")]))
        .unwrap();
    assert_eq!(diagnostic_count(&disabled), 1);
}

#[test]
fn scoped_runs_constrain_both_marker_layers_and_discard_does_not_run_filters() {
    let filter: lint::DiagnosticFilter = Arc::new(|mut diagnostics, _| {
        assert_eq!(diagnostics.len(), 1);
        diagnostics.push(diagnostic(0..1, "outside from filter"));
        diagnostics.push(diagnostic(2..3, "bad UTF-8 from filter"));
        diagnostics
    });
    let extension = Extension::group([
        linter(
            None,
            LintConfig {
                marker_filter: Some(filter.clone()),
                auto_panel: true,
                ..LintConfig::default()
            },
        ),
        lint::lint_gutter(lint::LintGutterConfig {
            marker_filter: Some(filter),
            ..Default::default()
        }),
    ]);
    let mut state = EditorState::with_extensions("a字b", extension).unwrap();
    let mut run = LintRun::start(&state).unwrap();
    run.restrict_to(1..4);
    let publish = run.clone().finish(vec![
        diagnostic(1..4, "safe"),
        diagnostic(0..1, "outside"),
        diagnostic(2..3, "bad UTF-8"),
        diagnostic(Range { start: 4, end: 1 }, "reversed"),
    ]);
    state.update(publish.clone()).unwrap();
    assert_eq!(positions(&state), [("safe".into(), 1..4, 1..4)]);
    let gutter = lint::lint_gutter_markers(&state).unwrap();
    assert_eq!(gutter.iter(0).next().unwrap().value.diagnostics().len(), 1);
    assert!(LintState::get(&state).unwrap().panel_open());
    state.update(run.discard()).unwrap();
    assert!(!LintState::get(&state).unwrap().panel_open());
    assert!(lint::lint_gutter_markers(&state).unwrap().is_empty());
    state.update(change(0..1, "A")).unwrap();
    state.update(publish).unwrap();
    assert_eq!(diagnostic_count(&state), 0);
}

#[test]
fn explicit_selection_and_action_use_the_full_current_diagnostic_range() {
    let mut state = state("text after");
    let mut fix = diagnostic(0..4, "word");
    let mut action = Action::new("Replace word", |state, range| {
        assert_eq!(state.text().slice(range.clone()), Some("text"));
        Ok(change(range, "new"))
    });
    action.mark_class = Some("fix".into());
    fix.actions.push(action);
    state
        .update(set_diagnostics(vec![
            diagnostic(2..2, "interior point"),
            fix,
            diagnostic(5..10, "another"),
        ]))
        .unwrap();
    state.update(next_diagnostic(&state).unwrap()).unwrap();
    state.update(next_diagnostic(&state).unwrap()).unwrap();
    assert_eq!(state.selection().main().range(), 5..10);

    let fix = LintState::get(&state)
        .unwrap()
        .selected()
        .unwrap()
        .diagnostic
        .clone();
    assert_eq!(fix.actions[0].name, "Replace word");
    assert_eq!(fix.actions[0].mark_class.as_deref(), Some("fix"));
    state.update(change(0..0, "🌿")).unwrap();
    assert!(run_action(&state, &fix, 1).unwrap().is_none());
    state
        .update(select_diagnostic(&state, &fix).unwrap())
        .unwrap();
    assert_eq!(state.selection().main().range(), 4..8);
    assert!(Arc::ptr_eq(
        &LintState::get(&state)
            .unwrap()
            .selected()
            .unwrap()
            .diagnostic,
        &fix
    ));
    state
        .update(run_action(&state, &fix, 0).unwrap().unwrap())
        .unwrap();
    assert_eq!(state.text().as_str(), "🌿new after");
    state
        .update(set_diagnostics(vec![diagnostic(4..7, "word")]))
        .unwrap();
    assert!(run_action(&state, &fix, 0).unwrap().is_none());
    assert!(select_diagnostic(&state, &fix).is_none());
}

#[test]
fn scheduler_debounces_changes_and_can_force_pending_work() {
    let mut state = state("old");
    let now = Instant::now();
    let mut default = LintScheduler::new(&state, now);
    assert_eq!(DEFAULT_LINT_DELAY, Duration::from_millis(750));
    assert!(
        default
            .take_due(&state, now + Duration::from_millis(749))
            .is_none()
    );
    assert!(default.take_due(&state, now + DEFAULT_LINT_DELAY).is_some());

    let delay = Duration::from_millis(20);
    let mut lint = LintScheduler::with_delay(&state, now, delay);
    let later = now + delay / 2;
    state.update(change(0..3, "new")).unwrap();
    lint.observe(&state, later);
    assert!(lint.take_due(&state, now + delay).is_none());
    let run = lint.take_due(&state, later + delay).unwrap();
    assert_eq!(run.text().as_str(), "new");
    assert!(lint.force(&state, later + delay).is_none());
    lint.refresh(later + delay);
    assert_eq!(
        lint.force(&state, later + delay).unwrap().text().as_str(),
        "new"
    );
    assert!(lint.force(&state, later + delay).is_none());

    state
        .update(Transaction::default().with_effects([Extension::default().reconfigure()]))
        .unwrap();
    assert!(lint.force(&state, later + delay).is_none());
    state
        .update(Transaction::default().with_effects([lint::extension().append()]))
        .unwrap();
    assert!(lint.force(&state, later + delay).is_some());
}

#[test]
fn worker_publication_rejects_stale_results_at_transaction_application() {
    let mut editor = state("text");
    let run = LintRun::start(&editor).unwrap();
    let stale = run.finish(vec![diagnostic(0..1, "stale")]);
    editor.update(change(0..0, "!")).unwrap();
    editor.update(change(0..1, "")).unwrap();
    editor
        .update(set_diagnostics(vec![diagnostic(0..4, "keep")]))
        .unwrap();
    editor.update(stale).unwrap();
    assert_eq!(positions(&editor), [("keep".into(), 0..4, 0..4)]);

    let run = LintRun::start(&editor).unwrap();
    let mut other = state("text");
    other
        .update(run.clone().finish(vec![diagnostic(0..1, "wrong editor")]))
        .unwrap();
    assert_eq!(diagnostic_count(&other), 0);
    editor
        .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
        .unwrap();
    editor
        .update(run.finish(vec![diagnostic(0..4, "current")]))
        .unwrap();
    assert_eq!(positions(&editor), [("current".into(), 0..4, 0..4)]);

    let result = LintRun::start(&editor).unwrap().finish(Vec::new());
    editor
        .update(
            change(0..0, "!")
                .merge(result, 4, TransactionCoordinates::Sequential)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(positions(&editor), [("current".into(), 1..5, 0..4)]);
}

#[test]
fn same_document_worker_runs_publish_in_completion_order() {
    let mut editor = state("text");
    let older = LintRun::start(&editor).unwrap();
    // A native no-op edit does not change the document revision.
    editor.update(change(0..4, "text")).unwrap();
    let newer = LintRun::start(&editor).unwrap();
    editor
        .update(newer.finish(vec![diagnostic(0..1, "newer")]))
        .unwrap();
    assert_eq!(positions(&editor), [("newer".into(), 0..1, 0..1)]);
    editor
        .update(older.finish(vec![diagnostic(4..4, "older")]))
        .unwrap();
    assert_eq!(positions(&editor), [("older".into(), 4..4, 4..4)]);
}

#[test]
fn session_runs_installed_provider_and_reschedules_after_configuration_changes() {
    let current_text =
        LintSource::new(|state| vec![diagnostic(0..state.text().len(), state.text().as_str())]);
    let initial = linter(Some(current_text), LintConfig::default());
    let mut editor = EditorState::with_extensions("test", initial.clone()).unwrap();
    let start = Instant::now();
    let mut session = LintSession::new(&editor, start);
    assert_eq!(session.wait_time(start), Some(DEFAULT_LINT_DELAY));
    assert!(
        session
            .take_due(
                &editor,
                start + DEFAULT_LINT_DELAY - Duration::from_millis(1)
            )
            .is_none()
    );
    let first = start + DEFAULT_LINT_DELAY;
    let (mut batch, mut futures) = session.take_due(&editor, first).unwrap();
    let mut context = Context::from_waker(Waker::noop());
    let Poll::Ready(result) = futures[0].as_mut().poll(&mut context) else {
        panic!("the synchronous provider must be ready");
    };
    editor
        .update(batch.complete(0, result, first).unwrap().unwrap())
        .unwrap();
    assert_eq!(positions(&editor), [("test".into(), 0..4, 0..4)]);
    assert!(batch.complete(0, Ok(Vec::new()), first).unwrap().is_none());
    assert!(batch.take_due(first + Duration::from_secs(1)).is_none());

    editor
        .update(Transaction::default().with_effects([initial.reconfigure()]))
        .unwrap();
    assert!(force_linting(&mut session, &editor, first).is_none());
    let settings = linter(None, LintConfig::default());
    editor
        .update(Transaction::default().with_effects([settings.reconfigure()]))
        .unwrap();
    assert!(force_linting(&mut session, &editor, first).is_none());
    assert!(session.wait_time(first).is_none());
    editor
        .update(Transaction::default().with_effects([lint::extension().reconfigure()]))
        .unwrap();
    assert!(force_linting(&mut session, &editor, first).is_none());

    let replacement = linter(
        Some(LintSource::asynchronous(|_| async { Ok(Vec::new()) })),
        LintConfig {
            delay: Some(Duration::from_millis(5)),
            ..LintConfig::default()
        },
    );
    editor
        .update(Transaction::default().with_effects([replacement.reconfigure()]))
        .unwrap();
    session.observe(&editor, first);
    assert_eq!(session.wait_time(first), Some(Duration::from_millis(5)));
    let (mut batch, mut futures) = force_linting(&mut session, &editor, first).unwrap();
    let Poll::Ready(result) = futures[0].as_mut().poll(&mut context) else {
        panic!("the async provider completes without waiting");
    };
    editor
        .update(batch.complete(0, result, first).unwrap().unwrap())
        .unwrap();
    assert_eq!(diagnostic_count(&editor), 0);
}

#[test]
fn lazy_publication_uses_result_coordinates_and_manual_clear_keeps_panel_open() {
    let mut editor = EditorState::new("x");
    let publish = set_diagnostics_for(&editor, vec![diagnostic(0..4, "leaf")]);
    let transaction = change(0..0, "🌿")
        .merge(publish, 1, TransactionCoordinates::Sequential)
        .unwrap();
    editor.update(transaction).unwrap();
    assert_eq!(positions(&editor), [("leaf".into(), 0..4, 0..4)]);
    editor.update(open_lint_panel(&editor).unwrap()).unwrap();
    editor.update(clear_diagnostics(&editor)).unwrap();
    let lint = LintState::get(&editor).unwrap();
    assert!(lint.panel_open());
    assert!(lint.selected().is_none());
    assert!(lint.markers().is_empty());
}
