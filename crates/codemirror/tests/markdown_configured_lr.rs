use std::{
    future::{Future, poll_fn, ready},
    ops::Range,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, HistoryIsolation, StateField, Text, Transaction, TransactionExtension,
    commands::CommentTokens,
    language::{
        LanguageData, StreamSettings,
        lezer::ParseError,
        markdown::{
            MarkdownCodeLanguages, MarkdownCodeLoad, MarkdownCodeParser, MarkdownCodeSelection,
            MarkdownConfig, MarkdownParserConfig, configured_markdown_parser, markdown_snapshot,
            markdown_with_parser, plan_markdown_refresh,
        },
    },
    language_data::{
        LANGUAGES, LanguageDescription, NativeLanguage, NativeSyntax, SyntaxUpdateError,
        UnavailableLanguage, json_language, lezer_language, wast_language,
    },
    transaction_extender,
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

static DEFERRED: LanguageDescription = LanguageDescription {
    name: "Deferred",
    aliases: &["lazy"],
    ..LANGUAGES[0]
};

fn deferred_config(
    load: Arc<MarkdownCodeLoad>,
    seen: Arc<Mutex<Vec<String>>>,
    fallback: Option<MarkdownCodeParser>,
) -> MarkdownParserConfig {
    MarkdownParserConfig {
        code_languages: Some(MarkdownCodeLanguages::Deferred {
            resolve: Arc::new(move |name| {
                seen.lock().unwrap().push(name.to_owned());
                LanguageDescription::match_name(
                    std::slice::from_ref(load.description()),
                    name,
                    true,
                )
                .map(|_| MarkdownCodeSelection::Load(Arc::clone(&load)))
            }),
            fallback,
        }),
        ..Default::default()
    }
}

fn with_deferred(source: &str, config: MarkdownParserConfig) -> EditorState {
    EditorState::with_extensions(
        source,
        Extension::group([
            markdown_with_parser(MarkdownConfig::default(), config),
            json_language().data(LanguageData::default().with("probe", "json")),
            wast_language().data(LanguageData::default().with("probe", "fallback")),
        ]),
    )
    .unwrap()
}

fn check_deferred(row: &Value, state: &EditorState, calls: &AtomicUsize) {
    let step = row["step"].as_str().unwrap();
    let source = row["source"].as_str().unwrap();
    assert_eq!(state.text().as_str(), source, "{step}");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        number(&row["attempts"]),
        "{step}"
    );
    let doc = markdown_snapshot(state).unwrap();
    let pairs = |ranges: Vec<Range<usize>>| {
        ranges
            .into_iter()
            .map(|range| [range.start, range.end])
            .collect::<Vec<_>>()
    };
    assert_eq!(
        json!(pairs(json_language().find_regions(state))),
        row["json"],
        "{step}"
    );
    assert_eq!(json!(pairs(doc.wast_regions())), row["fallback"], "{step}");
    if step == "empty" {
        return;
    }
    for [at, before, after] in
        serde_json::from_value::<Vec<[Value; 3]>>(row["probes"].clone()).unwrap()
    {
        let at = number(&at);
        for (side, role) in [(Assoc::Before, before), (Assoc::After, after)] {
            let probe = state.language_data_at::<&str>("probe", at, side);
            let language = match role.as_str().unwrap() {
                "markdown" => {
                    assert!(probe.is_empty(), "{step}: {at} {side:?}");
                    assert_eq!(
                        state
                            .language_data_at::<CommentTokens<'static>>("commentTokens", at, side)
                            .into_iter()
                            .map(|tokens| (tokens.line, tokens.block))
                            .collect::<Vec<_>>(),
                        [(None, Some(("<!--", "-->")))],
                        "{step}: {at} {side:?}"
                    );
                    NativeLanguage::Markdown
                }
                "json" => {
                    assert_eq!(probe, ["json"], "{step}: {at} {side:?}");
                    NativeLanguage::Json
                }
                "fallback" => {
                    assert_eq!(probe, ["fallback"], "{step}: {at} {side:?}");
                    NativeLanguage::Wast
                }
                _ => unreachable!(),
            };
            assert_eq!(
                doc.markdown_language_presence_at(at, side),
                Some(language),
                "{step}: {at} {side:?}"
            );
        }
    }

    let segments = serde_json::from_value::<Vec<[usize; 2]>>(row["segments"].clone()).unwrap();
    let body_painted = row["bodyPainted"].as_bool().unwrap();
    let mut child_paint = Vec::new();
    for [start, end] in &segments {
        let range = *start..*end;
        let spans = doc.highlights(range.clone()).unwrap();
        assert_eq!(!spans.is_empty(), body_painted, "{step}: {range:?}");
        assert_eq!(
            doc.highlight_context(&range).is_some(),
            body_painted,
            "{step}: {range:?}"
        );
        child_paint.extend(spans.into_iter().filter_map(|span| {
            span.tags
                .iter()
                .any(|tag| matches!(tag.name(), "propertyName" | "string"))
                .then_some([span.range.start, span.range.end])
        }));
    }
    assert_eq!(json!(child_paint), row["childPaint"], "{step}");
    for [start, end] in serde_json::from_value::<Vec<[usize; 2]>>(row["gapPaint"].clone()).unwrap()
    {
        assert!(
            !doc.highlights(start..end).unwrap().is_empty(),
            "{step}: quote marker"
        );
    }
    let info = source.find("LaZy").unwrap();
    assert!(
        doc.highlights(info..info + "LaZy".len())
            .unwrap()
            .iter()
            .any(|span| span.tags.iter().any(|tag| tag.name() == "labelName")),
        "{step}: code info"
    );
    let [[start, first_end], _] = segments[..] else {
        unreachable!()
    };
    let emoji = start + source[start..].find('😀').unwrap();
    assert_eq!(doc.language_at(emoji + 1, Assoc::After), None);
    assert_eq!(&source[first_end - 2..first_end], "\r\n");
    assert_eq!(
        doc.markdown_language_presence_at(first_end - 1, Assoc::After),
        None
    );
}

#[test]
fn deferred_markdown_joins_without_polling_and_keeps_old_snapshots() {
    let rows: Vec<Value> = include_str!("fixtures/markdown_pending.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let row = |step: &str| rows.iter().find(|row| row["step"] == step).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(AtomicUsize::new(0));
    let load = Arc::new(MarkdownCodeLoad::new(&DEFERRED, {
        let (calls, polls) = (calls.clone(), polls.clone());
        move |description| {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            let polls = polls.clone();
            poll_fn(move |_| {
                polls.fetch_add(1, Ordering::SeqCst);
                Poll::Ready(if attempt == 0 {
                    Err(UnavailableLanguage {
                        name: description.name,
                        source: description.source,
                    })
                } else {
                    Ok(NativeLanguage::Json.support())
                })
            })
        }
    }));
    let source = row("pending")["source"].as_str().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let fallback = Some(MarkdownCodeParser::Wast(wast_language().clone()));
    let config = deferred_config(load.clone(), seen.clone(), fallback);
    assert!(config.code_parser("LaZy\t meta").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    seen.lock().unwrap().clear();
    let pending = with_deferred(source, config);
    assert_eq!(json!(*seen.lock().unwrap()), row("pending")["seen"]);
    check_deferred(row("pending"), &pending, &calls);
    assert!(plan_markdown_refresh(&pending).unwrap().is_none());
    assert_eq!(json!(*seen.lock().unwrap()), row("pending")["seen"]);
    let mut first = load.load();
    let without_fallback =
        with_deferred(source, deferred_config(load.clone(), Arc::default(), None));
    check_deferred(row("without-fallback"), &without_fallback, &calls);
    let fallback = MarkdownCodeParser::Wast(wast_language().clone());
    for (name, owns_body) in [("lazy", false), ("unknown", true)] {
        let source = format!("```{name}\r\n(module)\r\n```");
        let start = source.find("(module)").unwrap();
        let state = with_deferred(
            &source,
            deferred_config(load.clone(), Arc::default(), Some(fallback.clone())),
        );
        let doc = markdown_snapshot(&state).unwrap();
        assert_eq!(
            doc.reindent_mounted_input(
                &state,
                start..start + "(module)".len(),
                &fallback,
                Transaction::default()
            )
            .unwrap()
            .is_some(),
            owns_body,
            "{name}"
        );
    }
    assert!(first.same_attempt(&load.load()));
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    let snapshot = markdown_snapshot(&pending).unwrap();
    assert!(matches!(
        Pin::new(&mut first).poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Err(_))
    ));
    assert!(load.support().is_none());
    assert!(plan_markdown_refresh(&pending).unwrap().is_none());
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    let mut unchanged = pending.clone();
    unchanged.update(Transaction::default()).unwrap();
    assert!(Arc::ptr_eq(
        &snapshot,
        &markdown_snapshot(&unchanged).unwrap()
    ));
    check_deferred(row("failed"), &unchanged, &calls);

    let edit = |state: &EditorState, step| {
        let row = row(step);
        let mut next = state.clone();
        let at = number(&row["edit"][0]);
        let change = Change::new(at..at, row["edit"][1].as_str().unwrap());
        next.update(Transaction::new(ChangeSet::single(change).unwrap()))
            .unwrap();
        check_deferred(row, &next, &calls);
        next
    };
    let retrying = edit(&pending, "retry");
    let mut retry = load.load();
    assert!(!retry.same_attempt(&first));
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        Pin::new(&mut retry).poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Ok(_))
    ));
    assert!(load.support().is_some());
    unchanged
        .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
        .unwrap();
    assert!(Arc::ptr_eq(
        &snapshot,
        &markdown_snapshot(&unchanged).unwrap()
    ));
    check_deferred(row("retained"), &unchanged, &calls);
    let seen_before = seen.lock().unwrap().clone();
    let refresh = plan_markdown_refresh(&unchanged).unwrap().unwrap();
    assert!(refresh.changes().is_empty());
    assert!(refresh.selection().is_none());
    assert_eq!(refresh.effects().len(), 1);
    assert!(!refresh.add_to_history());
    check_deferred(row("retained"), &unchanged, &calls);
    let mut refreshed = unchanged.clone();
    refreshed.update(refresh.clone()).unwrap();
    check_deferred(row("refresh"), &refreshed, &calls);
    assert_eq!(unchanged.selection(), refreshed.selection());
    assert_eq!(*seen.lock().unwrap(), seen_before);
    assert!(plan_markdown_refresh(&refreshed).unwrap().is_none());
    assert_eq!(
        refreshed.update(refresh),
        Err(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        ))
    );
    check_deferred(row("retained"), &unchanged, &calls);
    let ready = edit(&retrying, "ready");
    assert!(json_language().find_regions(&retrying).is_empty());
    assert!(!Arc::ptr_eq(&snapshot, &markdown_snapshot(&ready).unwrap()));
    assert_eq!(polls.load(Ordering::SeqCst), 2);
}

#[test]
fn deferred_refresh_publishes_ready_handles_without_remounting_other_fences() {
    let new_load = |available| {
        let calls = Arc::new(AtomicUsize::new(0));
        let polls = Arc::new(AtomicUsize::new(0));
        let load = Arc::new(MarkdownCodeLoad::new(&DEFERRED, {
            let (calls, polls) = (calls.clone(), polls.clone());
            move |description| {
                calls.fetch_add(1, Ordering::SeqCst);
                let polls = polls.clone();
                poll_fn(move |_| {
                    polls.fetch_add(1, Ordering::SeqCst);
                    Poll::Ready(if available {
                        Ok(NativeLanguage::Json.support())
                    } else {
                        Err(UnavailableLanguage {
                            name: description.name,
                            source: description.source,
                        })
                    })
                })
            }
        }));
        (load, calls, polls)
    };
    let (alpha, alpha_calls, alpha_polls) = new_load(true);
    let (beta, beta_calls, beta_polls) = new_load(true);
    let (failed, failed_calls, failed_polls) = new_load(false);
    let strict = handle("lezer", true, "existing");
    let seen = Arc::new(AtomicUsize::new(0));
    let config = MarkdownParserConfig {
        code_languages: Some(MarkdownCodeLanguages::Deferred {
            resolve: Arc::new({
                let (alpha, beta, failed) = (alpha.clone(), beta.clone(), failed.clone());
                let (seen, strict) = (seen.clone(), strict.clone());
                move |name| {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Some(match name {
                        "one" => MarkdownCodeSelection::Load(alpha.clone()),
                        "two" => MarkdownCodeSelection::Load(beta.clone()),
                        "failed" => MarkdownCodeSelection::Load(failed.clone()),
                        "existing" => MarkdownCodeSelection::Ready(strict.clone()),
                        _ => unreachable!(),
                    })
                }
            }),
            fallback: None,
        }),
        ..Default::default()
    };
    let source = concat!(
        "# 😀\n\n```two\n[2]\n```\n\n```one\n[1]\n```\n\n",
        "```existing\n@top Main { \"x\" }\n```\n\n```failed\n[0]\n```\n\n```one\n[3]\n```"
    );
    let mut state = with_deferred(source, config);
    let previous = markdown_snapshot(&state).unwrap();
    let root = |document: &gpui_editor_codemirror::language_data::SupportedDocument| {
        let NativeSyntax::Markdown(doc) = document.syntax() else {
            unreachable!()
        };
        assert_eq!(doc.text().as_str(), source);
        doc.root() as *const _
    };
    let poll = |load: &MarkdownCodeLoad| {
        Pin::new(&mut load.load()).poll(&mut Context::from_waker(Waker::noop()))
    };
    assert!(matches!(poll(&failed), Poll::Ready(Err(_))));
    assert!(plan_markdown_refresh(&state).unwrap().is_none());
    assert_eq!(beta_polls.load(Ordering::SeqCst), 0);
    assert!(matches!(poll(&alpha), Poll::Ready(Ok(_))));
    let first = plan_markdown_refresh(&state).unwrap().unwrap();
    assert!(matches!(poll(&beta), Poll::Ready(Ok(_))));
    // The plan does not adopt results that become ready before dispatch.
    state.update(first).unwrap();
    let partial = markdown_snapshot(&state).unwrap();
    assert_eq!(root(&previous), root(&partial));
    assert_eq!(regions(&strict, &state).len(), 1);
    let expected = |token: &str| {
        let start = source.find(token).unwrap();
        [start, start + token.len()]
    };
    assert_eq!(
        regions(&MarkdownCodeParser::Json(json_language().clone()), &state),
        [expected("[1]"), expected("[3]")]
    );
    for token in ["[0]", "[2]"] {
        let [start, end] = expected(token);
        assert!(partial.highlights(start..end).unwrap().is_empty());
        assert_eq!(
            partial.language_at(start + 1, Assoc::After),
            Some(NativeLanguage::Markdown)
        );
    }
    state
        .update(plan_markdown_refresh(&state).unwrap().unwrap())
        .unwrap();
    assert_eq!(
        regions(&MarkdownCodeParser::Json(json_language().clone()), &state),
        [expected("[2]"), expected("[1]"), expected("[3]")]
    );
    assert_eq!(root(&previous), root(&markdown_snapshot(&state).unwrap()));
    assert_eq!(
        partial.language_at(expected("[2]")[0] + 1, Assoc::After),
        Some(NativeLanguage::Markdown)
    );
    assert!(plan_markdown_refresh(&state).unwrap().is_none());
    assert_eq!(seen.load(Ordering::SeqCst), 5);
    for count in [
        alpha_calls,
        beta_calls,
        failed_calls,
        alpha_polls,
        beta_polls,
        failed_polls,
    ] {
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn checked_markdown_refresh_rejects_stale_and_combined_updates_without_losing_history() {
    let load = Arc::new(MarkdownCodeLoad::new(&DEFERRED, |_| {
        ready(Ok(NativeLanguage::Json.support()))
    }));
    let config = deferred_config(load.clone(), Arc::default(), None);
    let markdown = markdown_with_parser(MarkdownConfig::default(), config);
    let slot = Compartment::new();
    let detach = Arc::new(AtomicUsize::new(0));
    let strict = handle("json", true, "guard");
    let reject = Arc::new(AtomicUsize::new(0));
    let strict_field = StateField::new(|_| Ok(()), {
        let reject = reject.clone();
        move |_, update| {
            if reject.load(Ordering::SeqCst) != 0 && !update.transaction().effects().is_empty() {
                let Err(SyntaxUpdateError::StrictSyntax(error)) =
                    strict.parse(Text::new("{"), StreamSettings::default())
                else {
                    panic!("strict guard must reject unclosed input");
                };
                return Err(ConfigurationError::StrictSyntax(error));
            }
            Ok(())
        }
    });
    let state = EditorState::with_extensions(
        "# 😀\r\n\r\n```lazy\r\n[0]\r\n```\r\n\r\ntail",
        Extension::group([
            slot.of(markdown.clone()),
            strict_field.extension(),
            transaction_extender({
                let (slot, detach) = (slot.clone(), detach.clone());
                move |_, transaction| TransactionExtension {
                    effects: if detach.load(Ordering::SeqCst) != 0
                        && !transaction.effects().is_empty()
                    {
                        vec![slot.reconfigure(Extension::default())]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                }
            }),
        ]),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    let insert = |at, text| Transaction::new(ChangeSet::single(Change::new(at..at, text)).unwrap());
    for text in ["a", "b"] {
        view.update(
            insert(view.state().text().len(), text)
                .isolate_history(HistoryIsolation::Full)
                .with_selection(EditorSelection::single(2, 6)),
            (),
            |_| (None, ()),
        )
        .unwrap();
    }
    assert_eq!(view.undo(), Some(()));
    let retained = view.state().clone();
    let original = markdown_snapshot(&retained).unwrap();
    let parser = configured_markdown_parser(&retained).unwrap();
    let checkpoint = (view.revision(), view.undo_depth(), view.redo_depth());
    assert_eq!((checkpoint.1, checkpoint.2), (1, 1));
    assert!(matches!(
        Pin::new(&mut load.load()).poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Ok(_))
    ));
    let refresh = plan_markdown_refresh(&retained).unwrap().unwrap();
    let before = |view: &EditorView<()>| {
        assert_eq!(view.state().text(), retained.text());
        assert_eq!(view.state().selection(), retained.selection());
        assert_eq!(
            (view.revision(), view.undo_depth(), view.redo_depth()),
            checkpoint
        );
    };
    let invalid = Err(EditError::Configuration(
        ConfigurationError::InvalidFieldData,
    ));
    let alternative =
        markdown_with_parser(MarkdownConfig::default(), MarkdownParserConfig::default());
    let change_parser = || slot.reconfigure(alternative.clone());
    for combined in [
        insert(0, "x")
            .with_effects(refresh.effects().iter().cloned())
            .with_add_to_history(false),
        refresh.clone().with_effects([change_parser()]),
        refresh
            .clone()
            .with_selection(EditorSelection::single(0, 0)),
    ] {
        assert_eq!(view.update(combined, (), |_| (None, ())), invalid);
        before(&view);
        assert!(Arc::ptr_eq(
            &original,
            &markdown_snapshot(view.state()).unwrap()
        ));
    }
    detach.store(1, Ordering::SeqCst);
    assert_eq!(view.update(refresh.clone(), (), |_| (None, ())), invalid);
    detach.store(0, Ordering::SeqCst);
    before(&view);
    assert!(Arc::ptr_eq(
        &original,
        &markdown_snapshot(view.state()).unwrap()
    ));
    reject.store(1, Ordering::SeqCst);
    let error = view
        .update(refresh.clone(), (), |_| (None, ()))
        .unwrap_err();
    assert_eq!(edit_error(error), 1);
    reject.store(0, Ordering::SeqCst);
    before(&view);
    assert!(Arc::ptr_eq(
        &original,
        &markdown_snapshot(view.state()).unwrap()
    ));

    let mut detached = retained.clone();
    detached
        .update(Transaction::default().with_effects([slot.reconfigure(Extension::default())]))
        .unwrap();
    assert!(plan_markdown_refresh(&detached).unwrap().is_none());
    assert_eq!(detached.update(refresh.clone()), invalid);

    let mut aba = retained.clone();
    aba.update(Transaction::default().with_effects([change_parser()]))
        .unwrap();
    let swapped = markdown_snapshot(&aba).unwrap();
    assert_eq!(aba.update(refresh.clone()), invalid);
    assert!(Arc::ptr_eq(&swapped, &markdown_snapshot(&aba).unwrap()));
    aba.update(Transaction::default().with_effects([slot.reconfigure(markdown.clone())]))
        .unwrap();
    let restored = markdown_snapshot(&aba).unwrap();
    assert!(Arc::ptr_eq(
        &parser,
        &configured_markdown_parser(&aba).unwrap()
    ));
    assert_ne!(Arc::as_ptr(&original), Arc::as_ptr(&restored));
    assert_eq!(aba.text(), retained.text());
    assert_eq!(aba.update(refresh.clone()), invalid);
    assert!(Arc::ptr_eq(&restored, &markdown_snapshot(&aba).unwrap()));

    let mut edited = retained.clone();
    edited.update(insert(0, "!")).unwrap();
    assert_eq!(edited.update(refresh.clone()), invalid);
    let remove = ChangeSet::single(Change::new(0..1, "")).unwrap();
    edited.update(Transaction::new(remove)).unwrap();
    assert_eq!(edited.text(), retained.text());
    assert_eq!(edited.update(refresh.clone()), invalid);

    // Only the final parser configuration is effective. The snapshot is still current.
    let refresh = refresh.with_effects([change_parser(), slot.reconfigure(markdown)]);
    view.update(refresh, (), |_| (None, ())).unwrap();
    before(&view);
    assert!(!json_language().find_regions(view.state()).is_empty());
    assert!(json_language().find_regions(&retained).is_empty());
    assert_eq!(view.redo(), Some(()));
    assert_eq!(
        view.state().text().as_str(),
        format!("{}b", retained.text().as_str())
    );
    assert_eq!(view.undo(), Some(()));
    assert_eq!(view.state().text(), retained.text());
}

#[test]
fn empty_named_fence_starts_attempt_without_claiming_child_syntax() {
    let row: Value = serde_json::from_str(
        include_str!("fixtures/markdown_pending.jsonl")
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(AtomicUsize::new(0));
    let load = Arc::new(MarkdownCodeLoad::new(&DEFERRED, {
        let (calls, polls) = (calls.clone(), polls.clone());
        move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            let polls = polls.clone();
            poll_fn(move |_| {
                polls.fetch_add(1, Ordering::SeqCst);
                Poll::Ready(Ok(NativeLanguage::Json.support()))
            })
        }
    }));
    let config = deferred_config(
        load.clone(),
        Arc::default(),
        Some(MarkdownCodeParser::Wast(wast_language().clone())),
    );
    let state = with_deferred(row["source"].as_str().unwrap(), config);
    check_deferred(&row, &state, &calls);
    assert!(plan_markdown_refresh(&state).unwrap().is_none());
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        Pin::new(&mut load.load()).poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Ok(_))
    ));
    assert!(plan_markdown_refresh(&state).unwrap().is_none());
    check_deferred(&row, &state, &calls);
    assert_eq!(polls.load(Ordering::SeqCst), 1);
}

#[test]
fn deferred_resolver_uses_ready_parsers_and_cached_support_without_loading() {
    let calls = Arc::new(AtomicUsize::new(0));
    let load = Arc::new(MarkdownCodeLoad::with_support(
        &DEFERRED,
        NativeLanguage::Json.support(),
        {
            let calls = calls.clone();
            move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                ready(Ok(NativeLanguage::Json.support()))
            }
        },
    ));
    let named = handle("lezer", true, "immediate");
    let config = MarkdownParserConfig {
        code_languages: Some(MarkdownCodeLanguages::Deferred {
            resolve: Arc::new({
                let named = named.clone();
                move |name| match name {
                    "lazy" => Some(MarkdownCodeSelection::Load(load.clone())),
                    "immediate" => Some(MarkdownCodeSelection::Ready(named.clone())),
                    _ => None,
                }
            }),
            fallback: None,
        }),
        default_code_language: Some(NativeLanguage::Wast),
        ..Default::default()
    };
    let source =
        "```lazy\n[0]\n```\n\n```immediate\n@top Main { \"x\" }\n```\n\n```other\n(module)\n```";
    let state = with_deferred(source, config);
    assert_eq!(json_language().find_regions(&state).len(), 1);
    assert_eq!(regions(&named, &state).len(), 1);
    assert_eq!(markdown_snapshot(&state).unwrap().wast_regions().len(), 1);
    assert!(plan_markdown_refresh(&state).unwrap().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
