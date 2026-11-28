use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Precedence,
    Transaction,
    autocomplete::{brackets::close_brackets_enabled, session::SourceResult},
    commands::CommentTokens,
    language::{
        IndentOptions, LanguageData, brackets::bracket_matching_options, css::CssPropertySnapshot,
        fold::foldable_in_state, get_indentation, language_data_provider, sass::SassDialect,
    },
    language_data::{
        CloseBracketData, LANGUAGES, LanguageDescription, NativeLanguage, NativeSyntax,
        StylesheetCompletionSource, attached_language_snapshot, json_language,
    },
};
use serde_json::{Value, json};

const CASES: [(&str, NativeLanguage, &str); 4] = [
    ("CSS", NativeLanguage::Css, "css"),
    ("LESS", NativeLanguage::Less, "less"),
    ("Sass", NativeLanguage::Sass(SassDialect::Indented), "sass"),
    ("SCSS", NativeLanguage::Sass(SassDialect::Scss), "sass"),
];
const SIDE: Assoc = Assoc::Before;
const MARKER: &str = "catalogProbe";
const TRIGGERS: [&str; 4] = ["}", "  }", "x}", "  ]"];

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn bundle(name: &str) -> Extension {
    // Do not race the synchronous stock loader's first poll across these tests.
    static LOAD: Mutex<()> = Mutex::new(());
    let _guard = LOAD.lock().unwrap();
    let entry = LanguageDescription::match_name(LANGUAGES, name, false).unwrap();
    let load = entry.catalog_support_load().unwrap();
    let mut ctx = Context::from_waker(Waker::noop());
    let Poll::Ready(Ok(support)) = Pin::new(&mut load.load()).poll(&mut ctx) else {
        panic!("expected stock support for {name}");
    };
    let Poll::Ready(Ok(cached)) = Pin::new(&mut load.load()).poll(&mut ctx) else {
        panic!("expected cached support for {name}");
    };
    assert!(Arc::ptr_eq(&support, &cached));
    assert_eq!(support.completion_source_capacity(), 1);
    support.extension().unwrap()
}

fn sources(state: &EditorState, at: usize, side: Assoc) -> Vec<StylesheetCompletionSource> {
    state.language_data_at("autocomplete", at, side)
}

fn summarize(result: Option<SourceResult>) -> Value {
    json!(result.map(|result| {
        assert_eq!(result.to, None);
        assert!(result.filter && result.commit_characters.is_empty());
        let mut hash = 2_166_136_261_u32;
        for item in result.options.iter() {
            let entry = format!(
                "{}\0{}\0{}\n",
                item.label,
                item.kind.as_deref().unwrap(),
                item.insert.as_deref().unwrap_or_default()
            );
            for byte in entry.bytes() {
                hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
            }
        }
        (result.from, result.options.len(), hash)
    }))
}

#[test]
fn stock_stylesheet_catalog_bundles_match_the_pinned_editor() {
    let mut rows = include_str!("fixtures/language_data_stylesheet.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .peekable();
    let header = rows.next().unwrap();
    let keys = header["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|key| key.as_str().unwrap());
    let properties = CssPropertySnapshot::from_style_keys(keys).extension();
    let mut identities = Vec::new();
    let mut markers = Vec::new();
    for (name, language, id) in CASES {
        let state = EditorState::with_extensions("", bundle(name)).unwrap();
        assert_eq!(
            attached_language_snapshot(&state)
                .unwrap()
                .language_at(0, SIDE),
            Some(language)
        );
        let found = sources(&state, 0, SIDE);
        let [source] = found.as_slice() else {
            panic!("expected one source for {name}");
        };
        for (previous_id, previous_source) in &identities {
            assert_eq!(*previous_id == id, Arc::ptr_eq(previous_source, source));
        }
        identities.push((id, source.clone()));
        let facet = language.stylesheet_data_facet().unwrap();
        let marker = if id == "sass" { "Sass" } else { name };
        if name != "SCSS" {
            markers.push(facet.of(LanguageData::default().with(MARKER, marker)));
        }
    }
    let outside: StylesheetCompletionSource = Arc::new(|_, _, _| None);
    identities.push(("outside", outside.clone()));
    let outside_provider = language_data_provider(move |_, _, _| {
        vec![Arc::new(
            LanguageData::default()
                .with(MARKER, "outside")
                .with("wordChars", "!")
                .with("autocomplete", outside.clone()),
        )]
    });
    let extension = |name: &str| {
        let (base, precedence) = name
            .strip_prefix("high ")
            .map_or((name, Precedence::Default), |base| (base, Precedence::High));
        let extension = match base {
            "outside" => outside_provider.clone(),
            "JSON" => json_language().extension(),
            _ => bundle(base),
        };
        if precedence == Precedence::Default {
            extension
        } else {
            extension.with_precedence(precedence)
        }
    };
    let markers = Extension::group(markers);
    for (name, _, id) in CASES {
        let header = rows.next().unwrap();
        assert_eq!(header["name"], name);
        let initial = header["source"].as_str().unwrap();
        let slot = Compartment::new();
        let mut state = EditorState::with_extensions(
            initial,
            Extension::group([
                markers.clone(),
                properties.clone(),
                slot.of(Extension::default()),
            ]),
        )
        .unwrap();
        let mut retained = None;
        while let Some(row) = rows.next_if(|row| row["step"].is_string()) {
            let step = row["step"].as_str().unwrap();
            let before = attached_language_snapshot(&state);
            let mut tr = if let Some(edit) = row["edit"].as_array() {
                Transaction::new(
                    ChangeSet::single(Change::new(
                        byte(&edit[0])..byte(&edit[1]),
                        edit[2].as_str().unwrap(),
                    ))
                    .unwrap(),
                )
            } else {
                Transaction::default()
            };
            if !row["select"].is_null() {
                let at = byte(&row["select"]);
                tr = tr.with_selection(EditorSelection::single(at, at));
            }
            if let Some(installed) = row["extensions"].as_array() {
                tr = tr.with_effects([slot.reconfigure(Extension::group(
                    installed
                        .iter()
                        .map(|name| extension(name.as_str().unwrap())),
                ))]);
            }
            state.update(tr).unwrap();
            let snapshot = attached_language_snapshot(&state);
            let reused = match (&before, &snapshot) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            let at = byte(&row["completions"][0]["at"]);
            let active = snapshot.as_ref().and_then(|doc| doc.language_at(at, SIDE));
            let active_facet = active.and_then(NativeLanguage::stylesheet_data_facet);
            let active_facets: Vec<_> = CASES
                .iter()
                .filter_map(|(name, language, _)| {
                    (language.stylesheet_data_facet() == active_facet).then_some(*name)
                })
                .collect();
            let (parser, root) = match snapshot.as_ref().map(|doc| doc.syntax()) {
                Some(NativeSyntax::Css(doc)) => (Some("CSS"), doc.root().kind().name()),
                Some(NativeSyntax::Less(doc)) => (Some("LESS"), doc.root().kind().name()),
                Some(NativeSyntax::Sass(doc)) => {
                    let name = match doc.dialect() {
                        SassDialect::Indented => "Sass",
                        SassDialect::Scss => "SCSS",
                    };
                    assert_eq!(active, Some(NativeLanguage::Sass(doc.dialect())));
                    (Some(name), doc.root().name())
                }
                Some(NativeSyntax::Json(doc)) => (Some("JSON"), doc.root().kind().name()),
                None => (None, ""),
                _ => panic!("unexpected parser: {name}, {step}"),
            };
            let comments: Vec<_> = state
                .language_data_at::<CommentTokens>("commentTokens", at, SIDE)
                .iter()
                .map(|tokens| json!([tokens.line, tokens.block]))
                .collect();
            let brackets: Vec<_> = state
                .language_data_at::<CloseBracketData>("closeBrackets", at, SIDE)
                .iter()
                .map(|data| json!([data.brackets, data.before, data.string_prefixes]))
                .collect();
            let indent_on_input: Vec<_> = state
                .language_data_at::<fn(&str) -> bool>("indentOnInput", at, SIDE)
                .iter()
                .map(|trigger| TRIGGERS.map(trigger))
                .collect();
            let completions: Vec<_> = row["completions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|query| {
                    let at = byte(&query["at"]);
                    let found = sources(&state, at, SIDE);
                    let after = sources(&state, at, Assoc::After);
                    assert_eq!(found.len(), after.len());
                    assert!(
                        found
                            .iter()
                            .zip(after.iter())
                            .all(|(a, b)| Arc::ptr_eq(a, b))
                    );
                    let results: Vec<_> = found
                        .iter()
                        .map(|source| {
                            let id = identities
                                .iter()
                                .find(|(_, known)| Arc::ptr_eq(known, source))
                                .unwrap()
                                .0;
                            json!({"id": id, "implicit": summarize(source(&state, at, false)),
                        "explicit": summarize(source(&state, at, true))})
                        })
                        .collect();
                    json!({"at": at, "sources": results})
                })
                .collect();
            let services = if row["services"].is_null() {
                Value::Null
            } else {
                let spec = &row["services"];
                let indent_at = byte(&spec["indentAt"]);
                let start = byte(&spec["line"][0]);
                let end = byte(&spec["line"][1]);
                let indent =
                    get_indentation(&state, indent_at, IndentOptions::default(), None).unwrap();
                let fold =
                    foldable_in_state(&state, start, end).map(|range| [range.start, range.end]);
                json!({"indentAt": indent_at, "indent": indent, "line": [start, end], "fold": fold})
            };
            let actual = json!({"head": state.selection().main().head(), "parser": parser, "root": root,
                "activeFacets": active_facets, "reused": reused, "comments": comments, "brackets": brackets,
                "wordChars": state.language_data_at::<&str>("wordChars", at, SIDE), "indentOnInput": indent_on_input,
                "markers": state.language_data_at::<&str>(MARKER, at, SIDE), "completions": completions, "services": services});
            for (key, actual) in actual.as_object().unwrap() {
                assert_eq!(actual, &row[key], "{name}, {step}: {key}");
            }
            assert!(
                sources(&state, 4, SIDE).is_empty(),
                "UTF-8 interior: {name}, {step}"
            );
            assert!(bracket_matching_options(&state).unwrap().is_none());
            assert!(!close_brackets_enabled(&state).unwrap());
            if step == "install" {
                retained = Some((state.clone(), at));
            }
        }
        let (original, at) = retained.unwrap();
        assert_eq!(original.text().as_str(), initial);
        assert_eq!(
            attached_language_snapshot(&original).unwrap().text(),
            original.text()
        );
        let found = sources(&original, at, SIDE);
        assert_eq!(found.len(), 1);
        let known = identities.iter().find(|(key, _)| *key == id).unwrap();
        assert!(Arc::ptr_eq(&found[0], &known.1));
        assert_eq!(found[0](&original, at, false).unwrap().options.len(), 3);
        assert!(found[0](&state, at, false).is_none());
    }
    assert!(rows.next().is_none());
}

#[test]
fn sass_dialect_switch_reparses_even_without_text_changes() {
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions("", slot.of(bundle("Sass"))).unwrap();
    let indented = attached_language_snapshot(&state).unwrap();
    let source = sources(&state, 0, SIDE).pop().unwrap();
    state
        .update(Transaction::default().with_effects([slot.reconfigure(bundle("SCSS"))]))
        .unwrap();
    let braced = attached_language_snapshot(&state).unwrap();
    assert_eq!(indented.text(), braced.text());
    assert!(!Arc::ptr_eq(&indented, &braced));
    for (snapshot, dialect) in [
        (indented, SassDialect::Indented),
        (braced, SassDialect::Scss),
    ] {
        let NativeSyntax::Sass(doc) = snapshot.syntax() else {
            panic!("expected Sass syntax");
        };
        assert_eq!(doc.dialect(), dialect);
    }
    assert!(Arc::ptr_eq(&source, &sources(&state, 0, SIDE)[0]));
}

#[test]
fn property_reconfiguration_keeps_the_parser_and_callable() {
    for (name, _, _) in CASES {
        let source = if name == "Sass" {
            "a\n  color: red\n"
        } else {
            "a { color: red }"
        };
        let at = source.find("color").unwrap() + 3;
        let slot = Compartment::new();
        let host = CssPropertySnapshot::from_style_keys(["hostOnly"]).extension();
        let mut state = EditorState::with_extensions(
            source,
            Extension::group([bundle(name), host, slot.of(Extension::default())]),
        )
        .unwrap();
        let retained = state.clone();
        let parser = attached_language_snapshot(&state).unwrap();
        let callback = sources(&state, at, SIDE).pop().unwrap();
        let complete = |state: &EditorState| callback(state, at, false).unwrap();
        assert_eq!(complete(&state).options[0].label, "host-only");

        let empty = CssPropertySnapshot::from_style_keys([] as [&str; 0]).extension();
        state
            .update(
                Transaction::default()
                    .with_effects([slot.reconfigure(empty.with_precedence(Precedence::High))]),
            )
            .unwrap();
        assert!(complete(&state).options.is_empty());
        assert_eq!(complete(&retained).options[0].label, "host-only");
        assert!(Arc::ptr_eq(
            &parser,
            &attached_language_snapshot(&state).unwrap()
        ));
        assert!(Arc::ptr_eq(&callback, &sources(&state, at, SIDE)[0]));

        state
            .update(Transaction::default().with_effects([bundle(name).reconfigure()]))
            .unwrap();
        assert_eq!(complete(&state).options.len(), 741);
        assert!(Arc::ptr_eq(
            &parser,
            &attached_language_snapshot(&state).unwrap()
        ));
        assert!(Arc::ptr_eq(&callback, &sources(&state, at, SIDE)[0]));
    }
}
