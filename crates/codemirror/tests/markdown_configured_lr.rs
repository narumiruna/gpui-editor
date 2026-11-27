use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorState, Extension,
    Text, Transaction,
    language::{
        LanguageData, StreamSettings,
        lezer::ParseError,
        markdown::{
            MarkdownCodeLanguages, MarkdownCodeParser, MarkdownConfig, MarkdownParserConfig,
            configured_markdown_parser, markdown_snapshot, markdown_with_parser,
        },
    },
    language_data::{SyntaxUpdateError, json_language, lezer_language},
    view::EditorView,
};
use serde_json::{Value, json};

fn rows(kind: &str) -> impl Iterator<Item = Value> {
    include_str!("fixtures/markdown_configured_lr.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn handle(grammar: &str, strict: bool, name: &str) -> MarkdownCodeParser {
    match grammar {
        "json" => MarkdownCodeParser::Json(json_language().configure(Some(strict), Some(name))),
        "lezer" => MarkdownCodeParser::Lezer(lezer_language().configure(Some(strict), Some(name))),
        _ => unreachable!(),
    }
}

fn config(named: MarkdownCodeParser, fallback: MarkdownCodeParser) -> MarkdownParserConfig {
    MarkdownParserConfig {
        code_languages: Some(MarkdownCodeLanguages::Configured {
            resolve: Arc::new(move |info| (info == "key").then(|| named.clone())),
            fallback: Some(fallback),
        }),
        ..Default::default()
    }
}

fn install(named: MarkdownCodeParser, fallback: MarkdownCodeParser) -> Extension {
    markdown_with_parser(MarkdownConfig::default(), config(named, fallback))
}

fn regions(parser: &MarkdownCodeParser, state: &EditorState) -> Vec<[usize; 2]> {
    match parser {
        MarkdownCodeParser::Json(parser) => parser.find_regions(state),
        MarkdownCodeParser::Lezer(parser) => parser.find_regions(state),
        _ => unreachable!(),
    }
    .into_iter()
    .map(|range| [range.start, range.end])
    .collect()
}

fn active(parser: &MarkdownCodeParser, state: &EditorState, at: usize, side: Assoc) -> bool {
    match parser {
        MarkdownCodeParser::Json(parser) => parser.is_active_at(state, at, side),
        MarkdownCodeParser::Lezer(parser) => parser.is_active_at(state, at, side),
        _ => unreachable!(),
    }
}

fn data(parser: &MarkdownCodeParser, label: &'static str) -> Extension {
    let data = LanguageData::default().with("probe", label);
    match parser {
        MarkdownCodeParser::Json(parser) => parser.data(data),
        MarkdownCodeParser::Lezer(parser) => parser.data(data),
        _ => unreachable!(),
    }
}

fn edit_error(error: EditError) -> usize {
    let EditError::Configuration(error) = error else {
        panic!("expected strict mount rejection, got {error:?}");
    };
    config_error(error)
}

fn config_error(error: ConfigurationError) -> usize {
    let ConfigurationError::StrictSyntax(error) = error else {
        panic!("expected strict mount rejection, got {error:?}");
    };
    parse_offset(error)
}

fn parse_offset(error: ParseError) -> usize {
    let ParseError::UnexpectedToken { offset, .. } = error else {
        panic!("expected a stalled parser, got {error}");
    };
    offset
}

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

#[test]
fn configured_lr_resolves_named_and_fallback_children_with_stock_facets() {
    for row in rows("resolve") {
        let grammar = row["name"].as_str().unwrap();
        let (own, other) = if grammar == "json" {
            ("json", "lezer")
        } else {
            ("lezer", "json")
        };
        let named = handle(own, true, "named");
        let fallback = handle(other, true, "fallback");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        let parser = MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Configured {
                resolve: Arc::new({
                    let named = named.clone();
                    move |info| {
                        record.lock().unwrap().push(info.to_owned());
                        (info == "key").then(|| named.clone())
                    }
                }),
                fallback: Some(fallback.clone()),
            }),
            ..Default::default()
        };
        let source = row["source"].as_str().unwrap();
        let state = EditorState::with_extensions(
            source,
            Extension::group([
                markdown_with_parser(MarkdownConfig::default(), parser),
                data(&named, own),
                data(&fallback, other),
            ]),
        )
        .unwrap();
        assert_eq!(json!(*seen.lock().unwrap()), row["seen"], "{grammar}");
        assert_eq!(
            json!([regions(&named, &state), regions(&fallback, &state)]),
            row["regions"],
            "{grammar}"
        );
        for probe in row["probes"].as_array().unwrap() {
            let at = number(&probe[0]);
            for (side, expected) in [(Assoc::Before, &probe[1]), (Assoc::After, &probe[2])] {
                let words = state.language_data_at::<&str>("probe", at, side);
                assert_eq!(
                    &json!([active(&named, &state, at, side), words]),
                    expected,
                    "{grammar}: {at} {side:?}"
                );
            }
        }
    }
}

#[test]
fn strict_lr_mount_errors_use_host_bytes_and_do_not_retry_the_fallback() {
    for row in rows("reject").chain(rows("projection")) {
        let name = row["name"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let strict = handle(name, true, "strict");
        let loose = handle(name, false, "recovery");
        let use_fallback = matches!(
            row["caseName"].as_str(),
            Some("unknown fallback" | "bare fallback")
        ) || row["kind"] == "projection";
        let extension = if use_fallback {
            install(loose, strict)
        } else {
            install(strict, loose)
        };
        let error = EditorState::with_extensions(source, extension).unwrap_err();
        assert_eq!(
            json!(config_error(error)),
            row["offset"],
            "{name}: {}",
            row["caseName"]
        );
    }
    let row = rows("empty").next().unwrap();
    let strict = handle("json", true, "strict");
    let state = EditorState::with_extensions(
        row["source"].as_str().unwrap(),
        install(strict.clone(), strict.clone()),
    )
    .unwrap();
    assert!(regions(&strict, &state).is_empty());
    assert!(
        strict
            .parse(Text::new(""), StreamSettings::default())
            .is_err()
    );
}

#[test]
fn rejected_strict_lr_reconfiguration_preserves_snapshot_and_repair_can_commit() {
    for row in rows("swap") {
        let grammar = row["name"].as_str().unwrap();
        let loose = handle(grammar, false, "recovery");
        let strict = handle(grammar, true, "strict");
        let source = row["broken"].as_str().unwrap();
        let repaired = row["repaired"].as_str().unwrap();
        let from = number(&row["edit"][0]);
        let to = number(&row["edit"][1]);
        let insert = row["edit"][2].as_str().unwrap();
        let step = |start, end, text| {
            Transaction::new(ChangeSet::single(Change::new(start..end, text)).unwrap())
        };
        let repair = || step(from, to, insert);
        let undo_text = || step(from, from + insert.len(), &source[from..to]);
        let slot = Compartment::new();
        let retained =
            EditorState::with_extensions(source, slot.of(install(loose.clone(), loose.clone())))
                .unwrap();
        let original = markdown_snapshot(&retained).unwrap();
        let original_config = configured_markdown_parser(&retained).unwrap();
        let strict_effect = || slot.reconfigure(install(strict.clone(), loose.clone()));
        let mut view = EditorView::new(retained.clone());
        let error = view
            .update(
                Transaction::default().with_effects([strict_effect()]),
                (),
                |_| (None, ()),
            )
            .unwrap_err();
        assert_eq!(json!(edit_error(error)), row["rejected"]);
        assert!(Arc::ptr_eq(
            &original,
            &markdown_snapshot(view.state()).unwrap()
        ));
        assert!(Arc::ptr_eq(
            &original_config,
            &configured_markdown_parser(view.state()).unwrap()
        ));
        assert_eq!(view.state().text(), retained.text());
        assert_eq!(view.state().selection(), retained.selection());
        assert_eq!((view.revision(), view.undo_depth()), (0, 0));

        let mut cold = retained.clone();
        cold.update(repair()).unwrap();
        let pre_swap = markdown_snapshot(&cold).unwrap();
        cold.update(Transaction::default().with_effects([strict_effect()]))
            .unwrap();
        assert_eq!(cold.text().as_str(), repaired);
        assert!(!Arc::ptr_eq(&pre_swap, &markdown_snapshot(&cold).unwrap()));
        view.update(repair().with_effects([strict_effect()]), (), |_| (None, ()))
            .unwrap();
        assert_eq!(view.state().text().as_str(), repaired);
        let repaired_snapshot = markdown_snapshot(view.state()).unwrap();
        let error = view.update(undo_text(), (), |_| (None, ())).unwrap_err();
        assert_eq!(json!(edit_error(error)), row["rejected"]);
        assert_eq!(view.undo(), None);
        assert_eq!(
            (view.revision(), view.undo_depth(), view.redo_depth()),
            (1, 1, 0)
        );
        assert!(Arc::ptr_eq(
            &repaired_snapshot,
            &markdown_snapshot(view.state()).unwrap()
        ));
        assert!(Arc::ptr_eq(
            &original,
            &markdown_snapshot(&retained).unwrap()
        ));
        view.update(
            Transaction::default().with_effects([slot.reconfigure(install(loose.clone(), loose))]),
            (),
            |_| (None, ()),
        )
        .unwrap();
        assert_eq!(view.undo(), Some(()));
        assert_eq!(view.state().text().as_str(), source);
        assert_eq!(view.redo(), Some(()));
        assert_eq!(view.state().text().as_str(), repaired);
    }
}

#[test]
fn configured_lezer_projects_comments_across_quote_gaps_and_reuses_the_tree_for_data() {
    let row = rows("highlight").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let parser = handle("lezer", true, "selected");
    let slot = Compartment::new();
    let original = EditorState::with_extensions(
        source,
        Extension::group([
            install(parser.clone(), parser.clone()),
            slot.of(data(&parser, "before")),
        ]),
    )
    .unwrap();
    let snapshot = markdown_snapshot(&original).unwrap();
    let comments: Vec<_> = snapshot
        .highlights(0..source.len())
        .unwrap()
        .into_iter()
        .filter(|span| span.tags.iter().any(|tag| tag.name() == "blockComment"))
        .map(|span| [span.range.start, span.range.end])
        .collect();
    assert_eq!(json!(comments), row["comments"]);
    assert_eq!(&source[comments[0][1]..comments[1][0]], "> ");

    let mut next = original.clone();
    next.update(Transaction::default().with_effects([slot.reconfigure(data(&parser, "after"))]))
        .unwrap();
    assert!(Arc::ptr_eq(&snapshot, &markdown_snapshot(&next).unwrap()));
    let at = number(&row["at"]);
    for (state, key) in [(&original, "before"), (&next, "after")] {
        assert_eq!(
            json!(state.language_data_at::<&str>("probe", at, Assoc::After)),
            row[key]
        );
    }
}

#[test]
fn detached_configured_lr_documents_keep_strict_error_policy() {
    for (grammar, source, closing) in [
        ("json", "{\r\n  \"😀\":0\r}", "}"),
        ("lezer", "// 😀\r@top Main { \"x\" }", "}"),
    ] {
        let strict = handle(grammar, true, "chosen");
        assert!(strict == strict.clone());
        assert!(strict != handle(grammar, true, "chosen"));
        let mut doc = strict
            .parse(Text::new(source), StreamSettings::default())
            .unwrap();
        let at = source.rfind(closing).unwrap();
        let remove = ChangeSet::single(Change::new(at..source.len(), "")).unwrap();
        let Err(SyntaxUpdateError::StrictSyntax(error)) = doc.apply_changes(&remove) else {
            panic!("strict {grammar} body should reject an unclosed rule");
        };
        assert_eq!(parse_offset(error), at);
        assert_eq!(doc.text().as_str(), source);
        let mut loose = handle(grammar, false, "chosen")
            .parse(Text::new(source), StreamSettings::default())
            .unwrap();
        loose.apply_changes(&remove).unwrap();
        assert_eq!(loose.text().as_str(), &source[..at]);
    }
}
