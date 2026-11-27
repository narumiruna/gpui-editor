use std::{sync::Arc, time::Duration};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, LINE_SEPARATOR,
    Transaction,
    autocomplete::brackets::close_brackets_enabled,
    commands::CommentTokens,
    language::{
        IndentOptions,
        brackets::{BracketMatchingOptions, bracket_matching},
        fold::foldable_in_state,
        get_indentation,
    },
    language_data::{CloseBracketData, NativeSyntax, attached_language_snapshot, lezer_extension},
    setup::{SetupEditor, basic_setup},
    view::keymap::{KeyEvent, KeyModifiers, KeyPlatform},
};
use serde_json::{Value, json};

fn rows(kind: &str) -> Vec<Value> {
    include_str!("fixtures/lezer_state.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|row| row["kind"] == kind)
        .collect()
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn source(row: &Value) -> &str {
    row["source"].as_str().unwrap()
}

fn select(state: &mut EditorState, at: usize) {
    state
        .update(Transaction::default().with_selection(EditorSelection::single(at, at)))
        .unwrap();
}

fn setup(row: &Value, language: Extension) -> SetupEditor<()> {
    let mut state = EditorState::with_extensions(source(row), language).unwrap();
    select(&mut state, byte(&row["head"]));
    basic_setup(state, KeyPlatform::Linux).unwrap()
}

fn assert_edit(editor: &SetupEditor<()>, row: &Value) {
    let state = editor.view().state();
    assert_eq!(
        state.text().as_str(),
        row["after"].as_str().unwrap(),
        "{}",
        row["name"]
    );
    assert_eq!(
        state.selection().main().head(),
        byte(&row["position"]),
        "{}",
        row["name"]
    );
}

fn assert_snapshot(state: &EditorState, row: &Value) {
    let name = &row["name"];
    let position = byte(&row["indentAt"]);
    let block = &row["block"];
    assert_eq!(state.text().as_str(), source(row), "{name}");
    assert_eq!(
        json!(get_indentation(state, position, IndentOptions::default(), None).unwrap()),
        row["indent"],
        "{name}"
    );
    let fold = foldable_in_state(state, byte(&block[0]), byte(&block[1]));
    assert_eq!(json!(fold.map(|f| [f.start, f.end])), row["fold"], "{name}");
    let snapshot = attached_language_snapshot(state);
    let root = match snapshot.as_deref().map(|snapshot| snapshot.syntax()) {
        Some(NativeSyntax::LezerGrammar(doc)) => doc.root().kind().name(),
        None => "",
        _ => unreachable!(),
    };
    assert_eq!(root, row["root"].as_str().unwrap(), "{name}");
    let comments =
        state.language_data_at::<CommentTokens>("commentTokens", position, Assoc::Before);
    let actual: Vec<_> = comments
        .iter()
        .map(|tokens| {
            let (open, close) = tokens.block.unwrap();
            json!({"line": tokens.line, "block": {"open": open, "close": close}})
        })
        .collect();
    assert_eq!(json!(actual), row["comments"], "{name}");
    let reindent =
        state.language_data_at::<fn(&str) -> bool>("indentOnInput", position, Assoc::Before);
    assert_eq!(
        json!(
            reindent
                .iter()
                .map(|check| [" }", " a}", " )"].map(check))
                .collect::<Vec<_>>()
        ),
        row["reindent"],
        "{name}"
    );
    assert!(row["brackets"].as_array().unwrap().is_empty());
    assert!(!close_brackets_enabled(state).unwrap());
    assert!(
        state
            .language_data_at::<CloseBracketData>("closeBrackets", position, Assoc::Before)
            .is_empty()
    );
}

#[test]
fn lezer_attachment_uses_one_current_snapshot_for_state_queries() {
    let cases = rows("snapshot");
    let slot = Compartment::new();
    let mut state =
        EditorState::with_extensions(source(&cases[0]), slot.of(lezer_extension())).unwrap();
    assert_snapshot(&state, &cases[0]);
    let original = state.clone();
    let syntax = attached_language_snapshot(&state).unwrap();
    select(&mut state, 1);
    assert_snapshot(&state, &cases[1]);
    assert!(Arc::ptr_eq(
        &syntax,
        &attached_language_snapshot(&state).unwrap()
    ));
    let at = source(&cases[0]).find('@').unwrap();
    state
        .update(Transaction::new(
            ChangeSet::single(Change::new(at..at, " ")).unwrap(),
        ))
        .unwrap();
    assert_snapshot(&state, &cases[2]);
    assert_snapshot(&original, &cases[3]);
    state
        .update(Transaction::default().with_effects([slot.reconfigure(Extension::default())]))
        .unwrap();
    assert_snapshot(&state, &cases[4]);
    state
        .update(Transaction::default().with_effects([slot.reconfigure(lezer_extension())]))
        .unwrap();
    assert_snapshot(&state, &cases[5]);
}

#[test]
fn lezer_setup_enter_uses_the_pinned_delimiter_gap() {
    for row in rows("enter") {
        let mut editor = setup(
            &row,
            Extension::group([lezer_extension(), LINE_SEPARATOR.of("\r\n".into())]),
        );
        let (key, _) = editor
            .key_event(
                KeyEvent::new("Enter", KeyModifiers::default()),
                Duration::ZERO,
                (),
            )
            .unwrap();
        assert!(key.handled, "{}", row["name"]);
        assert_edit(&editor, &row);
        assert_eq!(editor.view_mut().undo(), Some(()));
        assert_eq!(editor.view().state().text().as_str(), source(&row));
        assert_eq!(
            editor.view().state().selection().main().head(),
            byte(&row["head"])
        );
    }
}

#[test]
fn lezer_setup_uses_quote_syntax_from_the_attached_tree() {
    for row in rows("quote") {
        let mut editor = setup(&row, lezer_extension());
        assert!(
            editor
                .insert_text(row["input"].as_str().unwrap(), Duration::ZERO, ())
                .unwrap()
        );
        assert_edit(&editor, &row);
    }
}

#[test]
fn lezer_setup_projects_configured_syntax_bracket_marks() {
    for line in include_str!("fixtures/grammar_bracket_options.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        if row["mode"] != "lezer" {
            continue;
        }
        let options = &row["options"];
        let config = BracketMatchingOptions {
            after_cursor: options["afterCursor"].as_bool(),
            brackets: options["brackets"].as_str().map(Arc::from),
            max_scan_distance: serde_json::from_value(options["maxScanDistance"].clone()).unwrap(),
            ..Default::default()
        };
        let state = EditorState::with_extensions(
            source(&row),
            Extension::group([lezer_extension(), bracket_matching(config)]),
        )
        .unwrap();
        let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
        for check in row["checks"].as_array().unwrap() {
            let head = byte(&check[0]);
            editor
                .view_mut()
                .set_selection(EditorSelection::single(head, head))
                .unwrap();
            let hit = &check[1];
            let mut expected = if let Some(hit) = hit.as_array() {
                let label = if hit[2] == true {
                    "cm-matchingBracket"
                } else {
                    "cm-nonmatchingBracket"
                };
                hit[..2]
                    .iter()
                    .filter(|span| !span.is_null())
                    .map(|span| (byte(&span[0]), byte(&span[1]), label))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            expected.sort_unstable();
            let projected = editor.decorations().unwrap();
            let actual: Vec<_> = projected
                .bracket_matches
                .iter(0)
                .map(|mark| (mark.from, mark.to, mark.value.class()))
                .collect();
            assert_eq!(actual, expected, "{} at {head}", row["name"]);
        }
    }
}
