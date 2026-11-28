//! Snapshots from the SHA-512-verified npm tarball for @codemirror/lint 6.9.8,
//! with @codemirror/state 6.7.6 and @codemirror/view 6.44.0. The scratch oracle
//! appends exports to inspect the original fields and drive the original plugin
//! with a fake clock and view. It converts reported UTF-16 positions to bytes.

use std::{
    future::pending,
    ops::Range,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Extension, Transaction,
    lint::{
        Diagnostic, LintBatch, LintCommand, LintConfig, LintGutterConfig, LintResult, LintSession,
        LintSource, LintState, Severity, clear_diagnostics, close_lint_panel, diagnostic_count,
        for_each_diagnostic, force_linting, lint_gutter, lint_gutter_markers, lint_keymap, linter,
        next_diagnostic, open_lint_panel, set_diagnostics_for,
    },
    view::{ViewUpdate, Viewport},
};
use serde_json::{Value, json};

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn fixture() -> impl Iterator<Item = Value> {
    include_str!("fixtures/lint_oracle.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn compare(group: &str, step: &str, mut actual: Value) {
    actual["group"] = json!(group);
    actual["step"] = json!(step);
    let expected = fixture()
        .find(|case| case["group"] == group && case["step"] == step)
        .unwrap_or_else(|| panic!("missing fixture: {group}: {step}"));
    assert_eq!(actual, expected, "{group}: {step}");
}

fn snapshot(state: &EditorState) -> Value {
    let lint = LintState::get(state);
    let mut iterate = Vec::new();
    for_each_diagnostic(state, |entry| {
        iterate.push(json!([
            entry.diagnostic.message,
            entry.range.start,
            entry.range.end
        ]));
    });
    let gutter = lint_gutter_markers(state).map_or_else(Vec::new, |markers| {
        markers
            .iter(0)
            .map(|marker| {
                let messages: Vec<_> = marker
                    .value
                    .diagnostics()
                    .iter()
                    .map(|d| &d.message)
                    .collect();
                json!([
                    marker.from,
                    format!("{:?}", marker.value.severity()).to_lowercase(),
                    messages
                ])
            })
            .collect()
    });
    let selection = state.selection().main();
    json!({
        "open": lint.as_ref().map(|lint| lint.panel_open()),
        "selected": lint.as_ref().and_then(|lint| lint.selected().map(|selected| selected.diagnostic.message.clone())),
        "selection": [selection.anchor(), selection.head()],
        "count": diagnostic_count(state),
        "entries": lint.as_ref().map_or_else(Vec::new, |lint| lint.panel_diagnostics().iter().map(|d| d.message.clone()).collect()),
        "iterate": iterate,
        "gutter": gutter,
    })
}

fn update(state: &mut EditorState, transaction: Option<Transaction>) -> bool {
    if let Some(transaction) = transaction {
        state.update(transaction).unwrap();
        true
    } else {
        false
    }
}

fn state_step(
    state: &mut EditorState,
    step: &str,
    command: impl FnOnce(&EditorState) -> Option<Transaction>,
) {
    let transaction = command(state);
    let handled = update(state, transaction);
    state_snapshot(state, step, Some(handled));
}

fn state_update(state: &mut EditorState, step: &str, transaction: Transaction) {
    state.update(transaction).unwrap();
    state_snapshot(state, step, None);
}

fn state_snapshot(state: &EditorState, step: &str, handled: Option<bool>) {
    let mut snapshot = snapshot(state);
    snapshot["handled"] = json!(handled);
    compare("state", step, snapshot);
}

fn change(range: Range<usize>, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, insert)).unwrap())
}

fn warning(message: &str) -> Diagnostic {
    Diagnostic::new(0..1, Severity::Warning, message)
}

#[test]
fn panel_publication_and_gutter_match_pinned_state() {
    let expected = fixture().next().unwrap();
    let bindings = lint_keymap().map(|binding| {
        let command = match binding.run.unwrap() {
            LintCommand::OpenPanel => "open",
            LintCommand::NextDiagnostic => "next",
        };
        json!([binding.key, command, binding.prevent_default])
    });
    assert_eq!(json!(bindings), expected["keymap"]);

    let gutter = lint_gutter(LintGutterConfig {
        marker_filter: Some(Arc::new(|mut ds, _| {
            ds.retain(|d| d.message != "content only");
            ds
        })),
        ..Default::default()
    });
    let mut state = EditorState::with_extensions("a🌿\nbb\nc", gutter).unwrap();
    state_step(&mut state, "no lint state", close_lint_panel);
    let open = open_lint_panel(&state).unwrap();
    state.update(open).unwrap();
    state_snapshot(&state, "open installs lint", Some(true));
    assert!(open_lint_panel(&state).is_none());
    state_snapshot(&state, "repeated open", Some(true));
    state_step(&mut state, "close open panel", close_lint_panel);
    state_step(&mut state, "close closed panel", close_lint_panel);
    let publish = set_diagnostics_for(
        &state,
        vec![Diagnostic::new(1..5, Severity::Hint, "manual")],
    );
    state_update(&mut state, "manual publication remains closed", publish);
    let settings = Extension::group([
        linter(
            None,
            LintConfig {
                auto_panel: true,
                marker_filter: Some(Arc::new(|mut ds, _| {
                    ds.retain(|d| d.message != "gutter only");
                    ds
                })),
                ..LintConfig::default()
            },
        ),
        linter(
            None,
            LintConfig {
                marker_filter: Some(Arc::new(|mut ds, _| {
                    for d in &mut ds {
                        d.message.make_ascii_uppercase();
                    }
                    ds
                })),
                ..LintConfig::default()
            },
        ),
    ]);
    state_update(
        &mut state,
        "add filters and auto panel",
        Transaction::default().with_effects([settings.append()]),
    );
    let publish = set_diagnostics_for(
        &state,
        vec![Diagnostic::new(7..8, Severity::Warning, "gutter only")],
    );
    state_update(&mut state, "filtered raw publication still opens", publish);
    let clear = clear_diagnostics(&state);
    state_update(&mut state, "clear closes", clear);
    let publish = set_diagnostics_for(
        &state,
        vec![
            Diagnostic::new(9..10, Severity::Info, "tail"),
            Diagnostic::new(7..8, Severity::Warning, "b"),
            Diagnostic::new(1..5, Severity::Error, "leaf"),
            Diagnostic::new(0..1, Severity::Hint, "first"),
            Diagnostic::new(6..7, Severity::Warning, "content only"),
        ],
    );
    state_update(
        &mut state,
        "publish then sort entries and group gutter",
        publish,
    );
    state_step(
        &mut state,
        "navigate without changing panel selection",
        next_diagnostic,
    );
    state_update(
        &mut state,
        "map through prefix line insert",
        change(0..0, "x\n"),
    );
    let end = state.text().len();
    state_update(
        &mut state,
        "remove all markers by editing",
        change(0..end, ""),
    );
    state_step(
        &mut state,
        "explicit close when already autoclosed",
        close_lint_panel,
    );
    let mut editor = EditorState::new("a");
    let publish = set_diagnostics_for(&editor, vec![warning("new field")]);
    state_update(&mut editor, "publish installs lint", publish);
}

struct Trace {
    state: EditorState,
    session: LintSession,
    batches: Vec<LintBatch>,
    clock: Arc<Mutex<Clock>>,
    start: Instant,
    errors: Vec<String>,
}

#[derive(Default)]
struct Clock {
    time: u64,
    starts: Vec<Value>,
}

impl Trace {
    fn new() -> Self {
        let clock = Arc::new(Mutex::new(Clock::default()));
        let providers = [30, 50, 80].into_iter().enumerate().map(|(id, delay)| {
            let clock = clock.clone();
            linter(
                Some(LintSource::asynchronous(move |state| {
                    let mut clock = clock.lock().unwrap();
                    let time = clock.time;
                    clock.starts.push(json!([
                        time,
                        id,
                        state.text().as_str(),
                        state.selection().main().head()
                    ]));
                    pending::<LintResult>()
                })),
                LintConfig {
                    delay: Some(ms(delay)),
                    needs_refresh: Some(Arc::new(move |update| id == 1 && update.selection_set)),
                    auto_panel: id == 2,
                    ..LintConfig::default()
                },
            )
        });
        let state = EditorState::with_extensions(
            "xy",
            Extension::group(providers.chain([lint_gutter(LintGutterConfig::default())])),
        )
        .unwrap();
        let start = Instant::now();
        let session = LintSession::new(&state, start);
        Self {
            state,
            session,
            batches: Vec::new(),
            clock,
            start,
            errors: Vec::new(),
        }
    }

    fn now(&self) -> Instant {
        self.start + ms(self.clock.lock().unwrap().time)
    }

    fn record(&self, step: &str) {
        let mut snap = snapshot(&self.state);
        snap["starts"] = json!(self.clock.lock().unwrap().starts);
        snap["errors"] = json!(self.errors);
        compare("providers", step, snap);
    }

    fn advance(&mut self, time: u64) {
        self.clock.lock().unwrap().time = time;
        let now = self.now();
        if let Some((batch, futures)) = self.session.take_due(&self.state, now) {
            assert_eq!(futures.len(), 3);
            self.batches.push(batch);
        }
        for batch in &mut self.batches {
            update(&mut self.state, batch.take_due(now));
        }
    }

    fn force(&mut self) -> bool {
        let now = self.now();
        if let Some((batch, futures)) = force_linting(&mut self.session, &self.state, now) {
            assert_eq!(futures.len(), 3);
            self.batches.push(batch);
            true
        } else {
            false
        }
    }

    fn deliver(&mut self, index: usize, result: LintResult) {
        let now = self.now();
        match self.batches[index / 3].complete(index % 3, result, now) {
            Ok(transaction) => {
                update(&mut self.state, transaction);
            }
            Err(error) => self.errors.push(error),
        }
    }

    fn update(&mut self, transaction: Transaction) {
        let start_state = self.state.clone();
        let selection_set = transaction.selection().is_some();
        let changes = transaction
            .changes()
            .desc(start_state.text().len())
            .unwrap();
        let effects = transaction.effects().to_vec();
        self.state.update(transaction).unwrap();
        let now = self.now();
        self.session.observe_update(
            &ViewUpdate {
                start_state: &start_state,
                state: &self.state,
                changes: &changes,
                effects: &effects,
                input_steps: &[],
                start_viewport: &Viewport::complete(start_state.text()),
                viewport: &Viewport::complete(self.state.text()),
                selection_set,
                viewport_changed: false,
                focus_changed: false,
                focused: false,
            },
            now,
        );
    }

    fn select(&mut self, at: usize) {
        self.update(Transaction::default().with_selection(EditorSelection::single(at, at)));
    }
}

#[test]
fn delayed_multi_provider_batches_match_pinned_plugin() {
    let mut trace = Trace::new();
    trace.advance(35);
    trace.update(change(2..2, "z"));
    trace.advance(80);
    trace.record("first wake resets to debounced due time");
    trace.advance(114);
    trace.record("not yet due");
    trace.advance(115);
    trace.record("all configured providers launched");
    assert!(!trace.force());
    trace.record("force during flight does nothing");
    trace.advance(116);
    trace.deliver(2, Ok(vec![warning("two")]));
    trace.advance(215);
    trace.deliver(0, Ok(vec![warning("zero")]));
    trace.advance(316);
    trace.record("first quiet timer cancelled");
    trace.advance(414);
    trace.deliver(1, Err("provider one failed".into()));
    trace.record("failure does not publish or reset timer");
    trace.advance(415);
    trace.record("partial batch in completion order");
    trace.advance(416);
    trace.select(1);
    assert!(trace.force());
    trace.record("selection refresh forced");
    trace.advance(420);
    trace.deliver(4, Ok(vec![warning("late final")]));
    trace.advance(620);
    trace.record("first partial publication");
    trace.advance(621);
    trace.update(clear_diagnostics(&trace.state));
    trace.record("manual clear keeps batch");
    trace.advance(622);
    trace.deliver(5, Ok(Vec::new()));
    trace.advance(721);
    trace.deliver(3, Ok(vec![warning("full")]));
    trace.record("all successes publish without waiting");
    trace.advance(722);
    trace.select(2);
    assert!(trace.force());
    trace.advance(723);
    trace.deliver(6, Ok(vec![warning("stale")]));
    trace.advance(724);
    trace.update(change(0..0, "!"));
    trace.advance(725);
    trace.update(change(0..1, ""));
    trace.advance(804);
    trace.record("old timer fires within ten milliseconds");
    trace.advance(923);
    trace.record("stale partial after edit restore not published");
    trace.advance(924);
    trace.deliver(7, Ok(vec![warning("still stale")]));
    trace.deliver(8, Ok(Vec::new()));
    trace.record("stale final after edit restore not published");
    trace.advance(925);
    for index in 9..12 {
        trace.deliver(index, Ok(Vec::new()));
    }
    trace.record("empty successful batch clears panel and gutter");
}
