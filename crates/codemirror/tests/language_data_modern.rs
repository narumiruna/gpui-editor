use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    time::Duration,
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Precedence,
    Text, Transaction,
    autocomplete::brackets::close_brackets_enabled,
    commands::CommentTokens,
    language::{
        IndentOptions, LanguageData, brackets::bracket_matching_options, fold::foldable_in_state,
        get_indentation, language_data_provider,
    },
    language_data::{
        CloseBracketData, GoCompletionSource, LANGUAGES, LanguageDescription, NativeLanguage,
        NativeLanguageSupport, NativeSyntax, attached_language_snapshot, go_extension,
        go_language_extension, json_language, wast_language,
    },
    lint::LintConfiguration,
    setup::basic_setup,
    view::keymap::{KeyEvent, KeyModifiers, KeyPlatform},
};
use serde_json::{Value, json};

#[path = "language_data_modern/jinja_liquid.rs"]
mod jinja_liquid;
#[path = "language_data_modern/markdown.rs"]
mod markdown;
#[path = "language_data_modern/php.rs"]
mod php;
#[path = "language_data_modern/script.rs"]
mod script;
#[path = "language_data_modern/vue_angular.rs"]
mod vue_angular;
#[path = "language_data_modern/xml.rs"]
mod xml;

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/language_data_modern.json")).unwrap()
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn load(entry: &LanguageDescription) -> Arc<NativeLanguageSupport> {
    // A concurrent caller can join the shared load before the factory returns.
    static LOAD: Mutex<()> = Mutex::new(());
    let _guard = LOAD.lock().unwrap();
    let mut future = entry.catalog_support_load().unwrap().load();
    let Poll::Ready(Ok(support)) =
        Pin::new(&mut future).poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("expected native support for {}", entry.name);
    };
    support
}

fn bundle(name: &str) -> Extension {
    let entry = LanguageDescription::match_name(LANGUAGES, name, false).unwrap();
    load(entry).extension().unwrap()
}

fn extension(name: &str) -> Extension {
    match name {
        "bare Go" => go_language_extension(),
        "bare JSON" => json_language().extension(),
        "bare WebAssembly" => wast_language().extension(),
        "fresh Go" => go_extension(),
        "high JSON" => bundle("JSON").with_precedence(Precedence::High),
        "outside" => language_data_provider(|_, _, _| {
            let source: GoCompletionSource = |_, _, _| None;
            let script: gpui_editor_codemirror::language_data::ScriptCompletionSource =
                Arc::new(|_, _, _| None);
            vec![
                Arc::new(LanguageData::default().with("autocomplete", source)),
                Arc::new(LanguageData::default().with("autocomplete", script)),
            ]
        }),
        _ => bundle(name),
    }
}

#[test]
fn all_modern_catalog_entries_install() {
    let mut modern = Vec::new();
    let mut available = Vec::new();
    for entry in LANGUAGES {
        let support = load(entry);
        if matches!(
            support.language(),
            NativeLanguage::Legacy(_) | NativeLanguage::Asn1
        ) {
            continue;
        }
        modern.push(entry.name);
        if support.extension().is_some() {
            available.push(entry.name);
        }
        assert!(support.language().support().extension().is_none());
    }
    assert_eq!(json!(modern), oracle()["modernEntries"]);
    assert_eq!(
        available,
        [
            "C",
            "C++",
            "CQL",
            "CSS",
            "Go",
            "HTML",
            "Java",
            "JavaScript",
            "Jinja",
            "JSON",
            "JSX",
            "LESS",
            "Liquid",
            "MariaDB SQL",
            "Markdown",
            "MS SQL",
            "MySQL",
            "PHP",
            "PLSQL",
            "PostgreSQL",
            "Python",
            "Rust",
            "Sass",
            "SCSS",
            "SQL",
            "SQLite",
            "TSX",
            "TypeScript",
            "WebAssembly",
            "XML",
            "YAML",
            "Vue",
            "Angular Template"
        ]
    );
}

#[test]
fn modern_recipes_preserve_active_parser_and_positional_provider_order() {
    for scenario in oracle()["scenarios"].as_array().unwrap() {
        let name = &scenario["name"];
        let records = scenario["records"].as_array().unwrap();
        let triggers = scenario["triggers"].as_array().unwrap();
        let slot = Compartment::new();
        let initial = records[0]["source"].as_str().unwrap();
        let mut state =
            EditorState::with_extensions(initial, slot.of(Extension::default())).unwrap();
        let mut retained = None;
        for row in records {
            let step = &row["step"];
            let before = attached_language_snapshot(&state);
            let mut tr = Transaction::default();
            if let Some(change) = row["edit"].as_array() {
                let range = byte(&change[0])..byte(&change[1]);
                tr = Transaction::new(
                    ChangeSet::single(Change::new(range, change[2].as_str().unwrap())).unwrap(),
                );
            }
            if !row["select"].is_null() {
                let at = byte(&row["select"]);
                tr = tr.with_selection(EditorSelection::single(at, at));
            }
            if let Some(extensions) = row["extensions"].as_array() {
                tr = tr.with_effects([slot.reconfigure(Extension::group(
                    extensions
                        .iter()
                        .map(|item| extension(item.as_str().unwrap())),
                ))]);
            }
            state.update(tr).unwrap();
            let at = byte(&row["at"]);
            let side = Assoc::Before;
            let snapshot = attached_language_snapshot(&state);
            let reused = matches!((&before, &snapshot), (Some(a), Some(b)) if Arc::ptr_eq(a, b));
            let mut active = Vec::new();
            let root = match snapshot.as_deref().map(|doc| doc.syntax()) {
                Some(NativeSyntax::Go(doc)) => {
                    assert_eq!(
                        snapshot.as_ref().unwrap().language_at(at, side),
                        Some(NativeLanguage::Go)
                    );
                    active.push("Go");
                    doc.root().kind().name()
                }
                Some(NativeSyntax::Json(doc)) => doc.root().kind().name(),
                Some(NativeSyntax::Wast(doc)) => doc.root().kind().name(),
                None => "",
                _ => unreachable!("unexpected parser for {name}: {step}"),
            };
            if json_language().is_active_at(&state, at, side) {
                active.push("JSON");
            }
            if wast_language().is_active_at(&state, at, side) {
                active.push("WebAssembly");
            }
            let sources = state.language_data_at::<GoCompletionSource>("autocomplete", at, side);
            let completion: Vec<_> = sources
                .iter()
                .map(|source| {
                    source(&state, at, true).map(|result| {
                        let has = |label| result.options.iter().any(|option| option.label == label);
                        json!({"from": result.from, "func": has("func"), "arg": has("arg"),
                    "local": has("local"), "inner": has("inner")})
                    })
                })
                .collect();
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
            let indent = get_indentation(
                &state,
                byte(&row["indentAt"]),
                IndentOptions::default(),
                None,
            )
            .unwrap();
            let fold = foldable_in_state(
                &state,
                byte(&row["lineRange"][0]),
                byte(&row["lineRange"][1]),
            )
            .map(|range| [range.start, range.end]);
            let actual = json!({"source": state.text().as_str(), "head": state.selection().main().head(),
                "root": root, "active": active, "reused": reused, "sources": completion, "comments": comments,
                "brackets": brackets, "indentOnInput": indent_on_input, "indent": indent, "fold": fold});
            for (key, actual) in actual.as_object().unwrap() {
                assert_eq!(actual, &row[key], "{name}, {step}: {key}");
            }
            assert!(bracket_matching_options(&state).unwrap().is_none());
            assert!(!close_brackets_enabled(&state).unwrap());
            assert!(!LintConfiguration::get(&state).has_sources());
            if retained.is_none() {
                retained = snapshot;
            }
        }
        assert_eq!(retained.unwrap().text().as_str(), initial);
    }
}

#[test]
fn setup_consumes_catalog_syntax_for_enter() {
    for row in oracle()["editor"].as_array().unwrap() {
        let name = row["name"].as_str().unwrap();
        let at = byte(&row["at"]);
        let state = EditorState::create_from_text(
            Text::new(row["source"].as_str().unwrap()),
            Some(EditorSelection::single(at, at)),
            bundle(name),
        )
        .unwrap();
        let mut editor = basic_setup(state, KeyPlatform::Linux).unwrap();
        let (result, _) = editor
            .key_event(
                KeyEvent::new("Enter", KeyModifiers::default()),
                Duration::ZERO,
                (),
            )
            .unwrap();
        assert!(result.handled, "{name}");
        let state = editor.view().state();
        assert_eq!(json!(state.text().as_str()), row["after"], "{name}");
        assert_eq!(
            json!(state.selection().main().head()),
            row["head"],
            "{name}"
        );
    }
}
