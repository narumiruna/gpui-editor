use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Precedence,
    Transaction,
    autocomplete::{brackets::close_brackets_enabled, session::SourceResult},
    commands::CommentTokens,
    language::{
        IndentOptions, LanguageData,
        brackets::bracket_matching_options,
        fold::foldable_in_state,
        get_indentation, language_data_provider,
        sql::{self, NativeSqlSchemaExtension, SqlDialect, SqlNamespace, SqlSchemaConfig},
    },
    language_data::{
        CloseBracketData, LANGUAGES, LanguageDescription, LoadInvocation, NativeLanguage,
        NativeLanguageSupport, NativeSyntax, SqlCompletionSource, attached_language_snapshot,
        json_language,
    },
};
use serde_json::{Value, json};

const SIDE: Assoc = Assoc::Before;
const MARKER: &str = "catalogProbe";

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn load(entry: &LanguageDescription) -> Arc<NativeLanguageSupport> {
    let mut future = entry.catalog_support_load().unwrap().load();
    let Poll::Ready(Ok(support)) =
        Pin::new(&mut future).poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("expected stock support for {}", entry.name);
    };
    support
}

fn sources(state: &EditorState, at: usize) -> Vec<SqlCompletionSource> {
    state.language_data_at("autocomplete", at, SIDE)
}

fn summarize(result: Option<SourceResult>) -> Value {
    let Some(result) = result else {
        return Value::Null;
    };
    let mut hash = 0xcbf29ce484222325_u64;
    for option in result.options.iter() {
        let kind = option.kind.as_deref().unwrap();
        let entry = format!("{}\0{kind}\0{}\n", option.label, option.boost);
        for byte in entry.bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
    json!({"from": result.from, "to": result.to, "filter": result.filter,
        "count": result.options.len(), "hash": format!("{hash:016x}")})
}

#[test]
fn stock_sql_catalog_bundles_match_the_pinned_editor() {
    let records: Vec<Value> = include_str!("fixtures/language_data_sql.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let mut extensions = Vec::new();
    let mut identities = Vec::new();
    let mut markers = Vec::new();
    let mut dialects = Vec::new();
    for entry in LANGUAGES {
        let LoadInvocation::SqlDialect(export) = entry.source.invocation else {
            continue;
        };
        let dialect = SqlDialect::from_export(export).unwrap();
        let support = load(entry);
        assert_eq!(support.language(), NativeLanguage::Sql(dialect));
        assert!(Arc::ptr_eq(&support, &load(entry)));
        assert_eq!(support.completion_source_capacity(), 1);
        let bundle = support.extension().unwrap();
        let state = EditorState::with_extensions("", bundle.clone()).unwrap();
        let found = sources(&state, 0);
        let [source] = found.as_slice() else {
            panic!("one source for {}", entry.name)
        };
        assert!(
            identities
                .iter()
                .all(|(_, previous)| !Arc::ptr_eq(previous, source))
        );
        assert!(
            dialects
                .iter()
                .all(|(_, _, previous): &(_, _, SqlDialect)| {
                    previous.data_facet() != dialect.data_facet()
                })
        );
        identities.push((entry.name, source.clone()));
        extensions.push((entry.name, bundle));
        markers.push(
            dialect
                .data_facet()
                .of(LanguageData::default().with(MARKER, entry.name)),
        );
        dialects.push((entry.name, export, dialect));
    }
    assert_eq!(dialects.len(), 8);
    let outside: SqlCompletionSource = Arc::new(|_, _, _| None);
    identities.push(("outside", outside.clone()));
    extensions.push((
        "outside",
        language_data_provider(move |_, _, _| {
            vec![Arc::new(
                LanguageData::default()
                    .with(MARKER, "outside")
                    .with("autocomplete", outside.clone()),
            )]
        }),
    ));
    extensions.push(("JSON", json_language().extension()));
    let extension = |name: &str| {
        let (base, precedence) = name
            .strip_prefix("high ")
            .map_or((name, Precedence::Default), |base| (base, Precedence::High));
        let value = extensions
            .iter()
            .find(|(key, _)| *key == base)
            .unwrap()
            .1
            .clone();
        if precedence == Precedence::Default {
            value
        } else {
            value.with_precedence(precedence)
        }
    };
    let markers = Extension::group(markers);
    let mut scenarios = records.split(|row| row["name"].is_string()).skip(1);
    for (name, export, _) in dialects.iter().copied() {
        let header = records.iter().find(|row| row["name"] == name).unwrap();
        assert_eq!(header["exportName"], export);
        let initial = header["source"].as_str().unwrap();
        let rows = scenarios.next().unwrap();
        let slot = Compartment::new();
        let mut state = EditorState::with_extensions(
            initial,
            Extension::group([markers.clone(), slot.of(Extension::default())]),
        )
        .unwrap();
        let mut retained = None;
        for row in rows {
            let step = row["step"].as_str().unwrap();
            let before = attached_language_snapshot(&state);
            let mut tr = if let Some(change) = row["edit"].as_array() {
                let range = byte(&change[0])..byte(&change[1]);
                Transaction::new(
                    ChangeSet::single(Change::new(range, change[2].as_str().unwrap())).unwrap(),
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
                        .map(|item| extension(item.as_str().unwrap())),
                ))]);
            }
            state.update(tr).unwrap();
            let at = state.text().len();
            let snapshot = attached_language_snapshot(&state);
            let reused = matches!((&before, &snapshot), (Some(a), Some(b)) if Arc::ptr_eq(a, b));
            let active: Vec<_> = dialects
                .iter()
                .filter(|(_, _, dialect)| {
                    snapshot.as_ref().and_then(|doc| doc.language_at(at, SIDE))
                        == Some(NativeLanguage::Sql(*dialect))
                })
                .map(|(name, _, _)| *name)
                .collect();
            let (parser, root, first_statement) = match snapshot.as_ref().map(|doc| doc.syntax()) {
                Some(NativeSyntax::Sql(doc)) => {
                    assert_eq!(doc.text(), state.text());
                    let first = doc
                        .root()
                        .children()
                        .find(|node| node.kind() == sql::SqlNodeKind::Statement)
                        .unwrap();
                    let tokens: Vec<_> = first
                        .children()
                        .map(|node| {
                            let range = node.range();
                            json!([node.kind().name(), range.start, range.end])
                        })
                        .collect();
                    (
                        Some(Arc::from(sql::NAME)),
                        doc.root().kind().name(),
                        json!(tokens),
                    )
                }
                Some(NativeSyntax::Json(doc)) => (
                    gpui_editor_codemirror::language_data::JsonLanguage::active_name(&state),
                    doc.root().kind().name(),
                    Value::Null,
                ),
                None => (None, "", Value::Null),
                _ => panic!("unexpected syntax for {name}: {step}"),
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
            let completions: Vec<_> = row["completions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|query| {
                    let at = byte(&query["at"]);
                    let results: Vec<_> = sources(&state, at)
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
            let actual = json!({"head": state.selection().main().head(), "parser": parser.as_deref(),
                "root": root, "active": active, "reused": reused, "comments": comments, "brackets": brackets,
                "wordChars": state.language_data_at::<&str>("wordChars", at, SIDE),
                "indentOnInput": state.language_data_at::<fn(&str) -> bool>("indentOnInput", at, SIDE).len(),
                "markers": state.language_data_at::<&str>(MARKER, at, SIDE), "firstStatement": first_statement,
                "completions": completions, "services": services});
            for (key, actual) in actual.as_object().unwrap() {
                assert_eq!(actual, &row[key], "{name}, {step}: {key}");
            }
            assert!(
                sources(&state, 4).is_empty(),
                "UTF-8 interior: {name}, {step}"
            );
            assert!(bracket_matching_options(&state).unwrap().is_none());
            assert!(!close_brackets_enabled(&state).unwrap());
            if step == "install" {
                retained = Some(state.clone());
            }
        }
        let original = retained.unwrap();
        assert_eq!(original.text().as_str(), initial);
        assert_eq!(
            attached_language_snapshot(&original).unwrap().text(),
            original.text()
        );
        let [source] = sources(&original, initial.len())
            .try_into()
            .unwrap_or_else(|_| panic!("retained source for {name}"));
        assert!(source(&original, initial.len(), false).is_some());
    }
    assert!(scenarios.next().is_none());
}

#[test]
fn stock_catalog_source_leaves_opt_in_schema_and_direct_queries_separate() {
    let dialect = SqlDialect::StandardSql;
    let language = NativeLanguage::Sql(dialect);
    let stock = load(LanguageDescription::match_name(LANGUAGES, "SQL", false).unwrap());
    assert!(language.support().extension().is_none());
    let text = "se";
    let schema = SqlNamespace::Completions(vec!["probe_table".into()]);
    let schema = NativeSqlSchemaExtension::new(
        dialect,
        SqlSchemaConfig {
            schema: Some(&schema),
            ..Default::default()
        },
    )
    .extension();
    let data_only =
        EditorState::with_extensions(text, language.support().tracked_language_data_extension())
            .unwrap();
    assert!(sources(&data_only, text.len()).is_empty());
    assert!(attached_language_snapshot(&data_only).is_none());
    let state =
        EditorState::with_extensions(text, Extension::group([stock.extension().unwrap(), schema]))
            .unwrap();
    let registered = sources(&state, text.len());
    let [source] = registered.as_slice() else {
        panic!("expected only stock keywords")
    };
    let installed = source(&state, text.len(), false).unwrap();
    assert!(
        !installed
            .options
            .iter()
            .any(|option| option.label == "probe_table")
    );
    assert!(
        installed
            .options
            .iter()
            .any(|option| option.label == "select" && option.boost == -1)
    );
    let direct = language.support().parse(state.text().clone()).unwrap();
    let mut direct_sources = direct.indexed_completions(text.len(), false);
    let (slot, keywords) = direct_sources.next().unwrap();
    assert_eq!(slot, 0);
    assert!(direct_sources.next().is_none());
    assert!(Arc::ptr_eq(&installed.options, &keywords.options));
    let NativeSyntax::Sql(doc) = direct.syntax() else {
        unreachable!()
    };
    let schema = NativeSqlSchemaExtension::source_for(&state, dialect).unwrap();
    assert_eq!(
        schema.complete(doc, text.len(), false).unwrap().options[0].label,
        "probe_table"
    );
}
