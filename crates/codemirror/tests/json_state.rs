use std::{collections::HashMap, ops::Range, sync::Arc, time::Duration};

use gpui_editor_codemirror::{
    Assoc, Change, ChangePermission, ChangeSet, Compartment, EditorSelection, EditorState,
    Extension, HistoryIsolation, Text, Transaction,
    autocomplete::brackets::close_brackets_enabled,
    change_filter,
    language::{
        IndentOptions,
        brackets::{BracketMatch, BracketMatchingOptions, bracket_matching},
        fold::foldable_in_state,
        get_indentation,
    },
    language_data::{CloseBracketData, NativeLanguage, NativeSyntax, json_extension},
    lint::{LintState, diagnostic_count},
    setup::{SetupEditor, SetupKeyContext, basic_setup},
    view::{
        EditorView,
        keymap::{KeyEvent, KeyModifiers, KeyPlatform},
    },
};
use serde_json::{Value, json};

fn oracle() -> HashMap<String, Value> {
    include_str!("fixtures/json_state.jsonl")
        .lines()
        .map(|line| {
            let row: Value = serde_json::from_str(line).unwrap();
            (row["name"].as_str().unwrap().into(), row)
        })
        .collect()
}

fn number(value: &Value) -> usize {
    serde_json::from_value(value.clone()).unwrap()
}

fn source(row: &Value) -> &str {
    row["source"].as_str().unwrap()
}

fn setup(row: &Value, extensions: Extension) -> SetupEditor<()> {
    let head = number(&row["head"]);
    let state = EditorState::create_from_text(
        Text::new(source(row)),
        Some(EditorSelection::single(head, head)),
        extensions,
    )
    .unwrap();
    basic_setup(state, KeyPlatform::Linux).unwrap()
}

fn indentation(state: &EditorState, row: &Value, syntax: Option<&NativeSyntax>) -> Value {
    json!(
        get_indentation(
            state,
            number(&row["indentAt"]),
            IndentOptions::default(),
            syntax
        )
        .unwrap()
    )
}

fn fold(state: &EditorState, row: &Value) -> Value {
    json!(
        foldable_in_state(state, 0, number(&row["lineEnd"])).map(|range| [range.start, range.end])
    )
}

#[test]
fn json_state_only_indentation() {
    let rows = oracle();
    let initial = &rows["initial"];
    let state = EditorState::with_extensions(source(initial), json_extension()).unwrap();
    assert_eq!(indentation(&state, initial, None), initial["indent"]);
}

#[test]
fn json_state_only_fold() {
    let rows = oracle();
    let initial = &rows["initial"];
    let state = EditorState::with_extensions(source(initial), json_extension()).unwrap();
    assert_eq!(fold(&state, initial), initial["fold"]);
}

#[test]
fn json_setup_enter_uses_syntax_indentation() {
    let rows = oracle();
    let row = &rows["enter"];
    let mut editor = setup(row, json_extension());
    assert!(press_enter(&mut editor, None));
    assert_input(&editor, row);
}

#[test]
fn json_setup_brackets_skip_the_string_literal() {
    let rows = oracle();
    let row = &rows["match"];
    let editor = setup(row, json_extension());
    let [hit] = editor.bracket_matches().try_into().unwrap();
    let BracketMatch {
        start,
        end,
        matched,
    } = hit;
    assert!(matched);
    assert_eq!(
        json!([
            [start.start, start.end],
            end.map(|range| [range.start, range.end])
        ]),
        row["marks"]
    );
}

#[test]
fn json_setup_quote_uses_the_parsed_opener() {
    let rows = oracle();
    for name in ["string opener", "property opener", "inside a string"] {
        let row = &rows[name];
        let mut editor = setup(row, json_extension());
        assert!(
            editor
                .insert_text(row["input"].as_str().unwrap(), Duration::ZERO, ())
                .unwrap()
        );
        assert_input(&editor, row);
    }
}

fn press_enter(editor: &mut SetupEditor<()>, syntax: Option<&NativeSyntax>) -> bool {
    editor
        .key_event_with(
            KeyEvent::new("Enter", KeyModifiers::default()),
            Duration::ZERO,
            (),
            SetupKeyContext {
                syntax,
                ..Default::default()
            },
        )
        .unwrap()
        .key
        .handled
}

fn assert_input(editor: &SetupEditor<()>, row: &Value) {
    let state = editor.view().state();
    assert_eq!(state.text().as_str(), row["result"], "{}", row["name"]);
    assert_eq!(
        json!(state.selection().main().head()),
        row["selection"],
        "{}",
        row["name"]
    );
}

fn assert_snapshot(state: &EditorState, row: &Value) {
    let name = &row["name"];
    assert_eq!(state.text().as_str(), source(row), "{name}");
    assert_eq!(
        json!(state.selection().main().head()),
        row["head"],
        "{name}"
    );
    assert_eq!(indentation(state, row, None), row["indent"], "{name}");
    assert_eq!(fold(state, row), row["fold"], "{name}");
    let brackets = state.language_data_at::<CloseBracketData>("closeBrackets", 0, Assoc::After);
    assert_eq!(
        json!(
            brackets
                .iter()
                .map(|data| data.brackets)
                .collect::<Vec<_>>()
        ),
        row["brackets"],
        "{name}"
    );
    assert_eq!(json!(diagnostic_count(state)), row["diagnostics"], "{name}");
    assert!(LintState::get(state).is_none(), "{name}");
    assert!(
        state
            .language_data_at::<Value>("autocomplete", 0, Assoc::After)
            .is_empty(),
        "{name}"
    );
    assert!(!close_brackets_enabled(state).unwrap(), "{name}");
}

#[test]
fn json_attachment_follows_accepted_edits_recovery_history_and_reconfiguration() {
    let rows = oracle();
    let initial = &rows["initial"];
    let [start, end] =
        serde_json::from_value::<[usize; 2]>(rows["protected"]["range"].clone()).unwrap();
    let protected = start..end;
    let policy = protected.clone();
    let language = Compartment::new();
    let old = EditorState::with_extensions(
        source(initial),
        Extension::group([
            language.of(json_extension()),
            change_filter(move |_, _| ChangePermission::Protect(vec![policy.clone()])),
        ]),
    )
    .unwrap();
    let mut view = EditorView::new(old.clone());
    assert_snapshot(view.state(), initial);
    apply(
        &mut view,
        Transaction::default().with_selection(EditorSelection::single(1, 1)),
    );
    assert_snapshot(view.state(), &rows["selection"]);
    edit(
        &mut view,
        vec![Change::new(0..0, " "), Change::new(protected, "ignored")],
    );
    assert_snapshot(view.state(), &rows["edited"]);
    assert_snapshot(&old, &rows["retained"]);
    let end = view.state().text().len();
    edit(&mut view, vec![Change::new(end - 1..end, "")]);
    assert_snapshot(view.state(), &rows["broken"]);
    edit(&mut view, vec![Change::new(end - 1..end - 1, "}")]);
    assert_snapshot(view.state(), &rows["repaired"]);
    assert_eq!(view.undo(), Some(()));
    assert_snapshot(view.state(), &rows["undo"]);
    assert_eq!(view.redo(), Some(()));
    assert_snapshot(view.state(), &rows["redo"]);
    apply(
        &mut view,
        Transaction::default().with_effects([language.reconfigure(Extension::default())]),
    );
    assert_snapshot(view.state(), &rows["detached"]);
    edit(&mut view, vec![Change::new(0..0, " ")]);
    assert_snapshot(view.state(), &rows["detached edit"]);
    apply(
        &mut view,
        Transaction::default().with_effects([language.reconfigure(json_extension())]),
    );
    assert_snapshot(view.state(), &rows["reattached"]);
}

fn apply(view: &mut EditorView<()>, transaction: Transaction) {
    view.update(transaction, (), |_| (None, ())).unwrap();
}

fn edit(view: &mut EditorView<()>, changes: Vec<Change>) {
    apply(
        view,
        Transaction::new(ChangeSet::new(changes).unwrap())
            .with_user_event("input.type")
            .isolate_history(HistoryIsolation::Full),
    );
}

#[test]
fn current_explicit_syntax_wins_and_stale_syntax_is_ignored() {
    let rows = oracle();
    let initial = &rows["initial"];
    let state = EditorState::with_extensions(source(initial), json_extension()).unwrap();
    let current = NativeLanguage::Wast.parse(state.text().clone()).unwrap();
    let stale = NativeLanguage::Markdown
        .parse(Text::new("# Heading"))
        .unwrap();
    assert_ne!(
        indentation(&state, initial, Some(&current)),
        initial["indent"]
    );
    assert_eq!(
        indentation(&state, initial, Some(&stale)),
        initial["indent"]
    );
    let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
    assert!(
        !editor
            .toggle_line_fold(1, Some(&current), Duration::ZERO, ())
            .unwrap()
    );
    assert!(
        editor
            .toggle_line_fold(1, Some(&stale), Duration::ZERO, ())
            .unwrap()
    );

    let row = &rows["enter"];
    let current = NativeLanguage::Wast.parse(Text::new(source(row))).unwrap();
    let mut editor = setup(row, json_extension());
    assert!(press_enter(&mut editor, Some(&current)));
    assert_ne!(editor.view().state().text().as_str(), row["result"]);
    let mut editor = setup(row, json_extension());
    assert!(press_enter(&mut editor, Some(&stale)));
    assert_input(&editor, row);
}

#[test]
fn json_setup_honors_installed_bracket_options() {
    for line in include_str!("fixtures/json_bracket_options.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let options = &row["options"];
        let extension = Extension::group([
            json_extension(),
            bracket_matching(BracketMatchingOptions {
                after_cursor: options["afterCursor"].as_bool(),
                brackets: options["brackets"].as_str().map(Arc::from),
                max_scan_distance: serde_json::from_value(options["maxScanDistance"].clone())
                    .unwrap(),
                ..Default::default()
            }),
        ]);
        let state = EditorState::with_extensions(source(&row), extension).unwrap();
        let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
        for check in row["checks"].as_array().unwrap() {
            let head = number(&check[0]);
            editor
                .view_mut()
                .set_selection(EditorSelection::single(head, head))
                .unwrap();
            let mut marks = Vec::new();
            for hit in editor.bracket_matches() {
                let mark = |range: Range<usize>| (range.start, range.end, hit.matched);
                marks.push(mark(hit.start));
                marks.extend(hit.end.map(mark));
            }
            marks.sort_unstable();
            assert_eq!(json!(marks), check[1], "{} at {head}", row["name"]);
        }
    }
}
