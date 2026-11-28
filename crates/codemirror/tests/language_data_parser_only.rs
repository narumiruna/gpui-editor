use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Precedence,
    Transaction,
    autocomplete::brackets::close_brackets_enabled,
    commands::CommentTokens,
    language::{
        IndentOptions, LanguageData, brackets::bracket_matching_options, fold::foldable_in_state,
        get_indentation, language_data_provider,
    },
    language_data::{
        CloseBracketData, GoCompletionSource, LANGUAGES, LanguageDescription, NativeLanguage,
        NativeLanguageSupport, NativeSyntax, attached_language_snapshot,
    },
    lint::LintConfiguration,
};
use serde_json::{Value, json};

fn load(name: &str) -> Arc<NativeLanguageSupport> {
    let entry = LanguageDescription::match_name(LANGUAGES, name, false).unwrap();
    let mut future = entry.catalog_support_load().unwrap().load();
    let Poll::Ready(Ok(support)) =
        Pin::new(&mut future).poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("expected native catalog support for {name}");
    };
    support
}

fn extension(name: &str) -> Extension {
    if name == "outside" {
        return language_data_provider(|_, _, _| {
            vec![Arc::new(LanguageData::default().with(
                "commentTokens",
                CommentTokens {
                    line: Some("outside"),
                    block: None,
                },
            ))]
        });
    }
    if let Some(name) = name.strip_prefix("high ") {
        return extension(name).with_precedence(Precedence::High);
    }
    load(name).extension().unwrap()
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn transaction(row: &Value, slot: &Compartment) -> Transaction {
    let mut tr = Transaction::default();
    if let Some(edit) = row["edit"].as_array() {
        let change = Change::new(byte(&edit[0])..byte(&edit[1]), edit[2].as_str().unwrap());
        tr = Transaction::new(ChangeSet::single(change).unwrap());
    }
    if !row["select"].is_null() {
        let at = byte(&row["select"]);
        tr = tr.with_selection(EditorSelection::single(at, at));
    }
    if let Some(event) = row["userEvent"].as_str() {
        tr = tr.with_user_event(event);
    }
    if let Some(extensions) = row["extensions"].as_array() {
        tr = tr.with_effects([slot.reconfigure(Extension::group(
            extensions
                .iter()
                .map(|entry| extension(entry.as_str().unwrap())),
        ))]);
    }
    tr
}

fn parser(syntax: &NativeSyntax) -> (&str, &[&str], NativeLanguage) {
    match syntax {
        NativeSyntax::Cpp(doc) => (doc.root().kind().name(), &["C", "C++"], NativeLanguage::Cpp),
        NativeSyntax::Java(doc) => (doc.root().name(), &["Java"], NativeLanguage::Java),
        NativeSyntax::Rust(doc) => (doc.root().kind().name(), &["Rust"], NativeLanguage::Rust),
        NativeSyntax::Yaml(doc) => (doc.root().kind().name(), &["YAML"], NativeLanguage::Yaml),
        _ => panic!("unexpected parser-only catalog syntax"),
    }
}

fn data_at(state: &EditorState, at: usize, side: Assoc, triggers: &[Value]) -> Value {
    let comments: Vec<_> = state
        .language_data_at::<CommentTokens>("commentTokens", at, side)
        .iter()
        .map(|tokens| json!([tokens.line, tokens.block]))
        .collect();
    let brackets: Vec<_> = state
        .language_data_at::<CloseBracketData>("closeBrackets", at, side)
        .iter()
        .map(|data| json!([data.brackets, data.before, data.string_prefixes]))
        .collect();
    let indent_on_input: Vec<Vec<_>> = state
        .language_data_at::<fn(&str) -> bool>("indentOnInput", at, side)
        .iter()
        .map(|rule| {
            triggers
                .iter()
                .map(|text| rule(text.as_str().unwrap()))
                .collect()
        })
        .collect();
    json!({
        "comments": comments,
        "brackets": brackets,
        "indentOnInput": indent_on_input,
        "autocomplete": state.language_data_at::<GoCompletionSource>("autocomplete", at, side).len(),
        "wordChars": state.language_data_at::<&str>("wordChars", at, side),
    })
}

#[test]
fn catalog_loads_remain_separate_from_parser_identity() {
    let c = load("C");
    let cpp = load("C++");
    assert!(Arc::ptr_eq(&c, &load("C")));
    assert!(Arc::ptr_eq(&cpp, &load("C++")));
    assert!(!Arc::ptr_eq(&c, &cpp));

    for name in ["C", "C++", "Java", "Rust", "YAML"] {
        let support = *load(name);
        let language = support.language();
        assert_eq!(support.completion_source_capacity(), 0);
        assert!(support.extension().is_some());
        assert!(language.support().extension().is_none());
    }
}

#[test]
fn parser_only_catalog_state_matches_the_pinned_oracle() {
    let oracle: Value =
        serde_json::from_str(include_str!("fixtures/language_data_parser_only.json")).unwrap();
    let triggers = oracle["triggers"].as_array().unwrap();
    for scenario in oracle["scenarios"].as_array().unwrap() {
        let records = scenario["records"].as_array().unwrap();
        let initial = records[0]["source"].as_str().unwrap();
        let slot = Compartment::new();
        let mut state =
            EditorState::with_extensions(initial, slot.of(Extension::default())).unwrap();
        let mut retained = None;
        for row in records {
            let label = format!("{}, {}", scenario["name"], row["step"]);
            let before = attached_language_snapshot(&state);
            state.update(transaction(row, &slot)).unwrap();
            let snapshot = attached_language_snapshot(&state);
            let reused = match (&before, &snapshot) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            let (root, active, language) =
                snapshot.as_ref().map_or(("", [].as_slice(), None), |doc| {
                    assert_eq!(doc.text(), state.text(), "{label}");
                    let (root, active, language) = parser(doc.syntax());
                    (root, active, Some(language))
                });
            assert_eq!(json!(state.text().as_str()), row["source"], "{label}");
            assert_eq!(
                json!(state.selection().main().head()),
                row["head"],
                "{label}"
            );
            assert_eq!(json!(root), row["root"], "{label}");
            assert_eq!(json!(active), row["active"], "{label}");
            assert_eq!(json!(reused), row["reused"], "{label}");

            for at in row["positions"].as_array().unwrap().iter().map(byte) {
                for side in [Assoc::Before, Assoc::After] {
                    let active = snapshot.as_ref().and_then(|doc| doc.language_at(at, side));
                    assert_eq!(active, language, "{label}, byte {at}, {side:?}");
                    let data = data_at(&state, at, side, triggers);
                    assert_eq!(data, row["data"], "{label}, byte {at}, {side:?}");
                    // Each installed provider in this oracle has exactly one comment value.
                    assert_eq!(
                        json!(data["comments"].as_array().unwrap().len()),
                        row["providers"],
                        "{label}"
                    );
                }
                assert!(
                    snapshot
                        .as_ref()
                        .is_none_or(|doc| doc.completions(at, true).is_empty()),
                    "{label}"
                );
            }
            assert!(
                bracket_matching_options(&state).unwrap().is_none(),
                "{label}"
            );
            assert!(!close_brackets_enabled(&state).unwrap(), "{label}");
            assert!(!LintConfiguration::get(&state).has_sources(), "{label}");

            for result in row["services"].as_array().unwrap() {
                let start = byte(&result["lineRange"][0]);
                let end = byte(&result["lineRange"][1]);
                let indent =
                    get_indentation(&state, start, IndentOptions::default(), None).unwrap();
                let fold =
                    foldable_in_state(&state, start, end).map(|range| [range.start, range.end]);
                assert_eq!(
                    json!(indent),
                    result["indent"],
                    "{label}, indent at {start}"
                );
                assert_eq!(json!(fold), result["fold"], "{label}, fold at {start}");
            }
            if retained.is_none() {
                retained = snapshot;
            }
        }
        assert_eq!(retained.unwrap().text().as_str(), initial);
    }
}
