use std::{sync::Arc, time::Duration};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Precedence,
    Text, Transaction,
    autocomplete::brackets::insert_bracket,
    language::brackets::{BracketMatchingOptions, bracket_matching, bracket_matching_options},
    language_data::{
        GoCompletionSource, NativeSyntax, attached_language_snapshot, go_extension,
        go_language_extension, json_extension,
    },
    setup::{SetupEditor, basic_setup},
    view::keymap::{KeyEvent, KeyModifiers, KeyPlatform},
};
use serde_json::{Value, json};

fn rows(fixture: &str) -> impl Iterator<Item = Value> + '_ {
    fixture
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn sources(state: &EditorState) -> Vec<GoCompletionSource> {
    state.language_data_at("autocomplete", 0, Assoc::After)
}

#[test]
fn go_base_and_bundle_share_only_the_active_parser() {
    let source = "package p\nfunc f(arg int) {\n  var local = 1\n  return local\n}\n";
    let slot = Compartment::new();
    let bare = go_language_extension();
    let bundle = go_extension();
    let retained = EditorState::with_extensions(source, slot.of(bare.clone())).unwrap();
    let mut state = retained.clone();
    let mut previous = None;
    let mut rows = rows(include_str!("fixtures/go_attachment.jsonl"));
    let check = |state: &EditorState, row: &Value, previous: &mut _| {
        let snapshot = attached_language_snapshot(state);
        let root = match snapshot.as_deref().map(|document| document.syntax()) {
            Some(NativeSyntax::Go(doc)) => doc.root().kind().name(),
            Some(NativeSyntax::Json(doc)) => doc.root().kind().name(),
            None => "",
            _ => unreachable!(),
        };
        let same = matches!((&*previous, &snapshot), (Some(a), Some(b)) if Arc::ptr_eq(a, b));
        assert_eq!(json!(same), row["sameTree"], "{}", row["name"]);
        assert_eq!(root, row["root"], "{}", row["name"]);
        assert_eq!(
            json!(root == "SourceFile"),
            row["active"],
            "{}",
            row["name"]
        );
        assert_eq!(
            json!(sources(state).len()),
            row["sources"],
            "{}",
            row["name"]
        );
        assert!(bracket_matching_options(state).unwrap().is_none());
        *previous = snapshot;
    };
    check(&state, &rows.next().unwrap(), &mut previous);
    let configure = |ext| Transaction::default().with_effects([slot.reconfigure(ext)]);
    let local = source.find("local").unwrap();
    for transaction in [
        configure(bundle.clone()),
        Transaction::default().with_selection(EditorSelection::single(12, 12)),
        Transaction::new(ChangeSet::single(Change::new(local..local + 5, "inner")).unwrap()),
        configure(go_extension()),
        configure(bare),
        configure(Extension::group([
            bundle.clone(),
            json_extension().with_precedence(Precedence::High),
        ])),
        configure(bundle),
        configure(Extension::default()),
    ] {
        state.update(transaction).unwrap();
        check(&state, &rows.next().unwrap(), &mut previous);
    }
    assert_eq!(retained.text().as_str(), source);
    assert!(sources(&retained).is_empty());
}

fn setup(source: &str, head: usize, extension: Extension) -> SetupEditor<()> {
    let state = EditorState::create_from_text(
        Text::new(source),
        Some(EditorSelection::single(head, head)),
        extension,
    )
    .unwrap();
    basic_setup(state, KeyPlatform::Linux).unwrap()
}

#[test]
fn basic_setup_consumes_attached_go_indentation() {
    for row in
        rows(include_str!("fixtures/go_attachment.jsonl")).filter(|row| row["text"].is_string())
    {
        let source = row["text"].as_str().unwrap();
        let mut editor = setup(source, number(&row["at"]), go_language_extension());
        let (result, _) = editor
            .key_event(
                KeyEvent::new("Enter", KeyModifiers::default()),
                Duration::ZERO,
                (),
            )
            .unwrap();
        assert!(result.handled, "{}", row["name"]);
        let state = editor.view().state();
        assert_eq!(state.text().as_str(), row["after"], "{}", row["name"]);
        assert_eq!(state.selection().main().head(), number(&row["head"]));
        assert_eq!(editor.view_mut().undo(), Some(()));
        assert_eq!(editor.view().state().text().as_str(), source);
    }
}

#[test]
fn basic_setup_consumes_attached_go_quote_syntax() {
    for row in rows(include_str!("fixtures/go_quotes.jsonl")) {
        let source = row["source"].as_str().unwrap();
        let key = row["key"].as_str().unwrap();
        let mut editor = setup(source, number(&row["head"]), go_language_extension());
        assert_eq!(
            insert_bracket(editor.view().state(), key)
                .unwrap()
                .is_some(),
            row["handled"],
            "{}",
            row["name"]
        );
        assert!(editor.insert_text(key, Duration::ZERO, ()).unwrap());
        let state = editor.view().state();
        assert_eq!(state.text().as_str(), row["after"], "{}", row["name"]);
        assert_eq!(state.selection().main().head(), number(&row["position"]));
    }
}

#[test]
fn basic_setup_consumes_attached_go_bracket_options() {
    for row in rows(include_str!("fixtures/go_rust_bracket_options.jsonl"))
        .filter(|row| row["mode"] == "go")
    {
        let source = row["source"].as_str().unwrap();
        let config = &row["options"];
        let matching = bracket_matching(BracketMatchingOptions {
            brackets: config["brackets"].as_str().map(Arc::from),
            after_cursor: config["afterCursor"].as_bool(),
            max_scan_distance: config["maxScanDistance"]
                .as_u64()
                .map(|n| usize::try_from(n).unwrap()),
            ..Default::default()
        });
        let mut editor = setup(
            source,
            0,
            Extension::group([go_language_extension(), matching]),
        );
        for probe in row["checks"].as_array().unwrap() {
            let head = number(&probe[0]);
            editor
                .view_mut()
                .set_selection(EditorSelection::single(head, head))
                .unwrap();
            let hits = editor.bracket_matches();
            assert!(hits.len() <= 1);
            let first = hits.first().map(|hit| {
                json!([
                    [hit.start.start, hit.start.end],
                    hit.end.as_ref().map(|end| [end.start, end.end]),
                    hit.matched
                ])
            });
            assert_eq!(json!(first), probe[1], "{} at {head}", row["name"]);
        }
    }
}
