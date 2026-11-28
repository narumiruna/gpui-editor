use std::{
    num::NonZeroUsize,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use gpui_editor_codemirror::{
    Assoc, Compartment, ConfigurationError, EditError, EditorSelection, EditorState, Extension,
    StateField, StateFieldReader, TAB_SIZE, Transaction, TransactionExtension, change_filter,
    collab::{CollabConfig, collab, sendable_updates},
    language::{
        INDENT_UNIT, IndentDecision, IndentOptions, IndentStyle, MarkdownCodeIndent,
        StreamSettings, get_indentation, indent_service,
        lezer::{
            CLOSED_BY, FOLD, GROUP, INDENT, LANGUAGE_DATA, LrIndentContext, LrIndentStrategy,
            LrSyntaxNode,
        },
        markdown::{
            MarkdownCodeLanguages, MarkdownCodeParser, MarkdownConfig, MarkdownParserConfig,
            markdown_snapshot, markdown_with_parser,
        },
        rust::RustDocument,
    },
    language_data::{NativeSyntax, RustLanguage, attached_language_snapshot, rust_language},
    setup::basic_setup,
    transaction_extender,
    view::keymap::KeyPlatform,
};
use serde_json::{Value, json};

type Trace = Arc<Mutex<Vec<Value>>>;

fn records(kind: &str) -> impl Iterator<Item = Value> + '_ {
    include_str!("fixtures/rust_configured_indent.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn byte(v: &Value) -> usize {
    v.as_u64().unwrap().try_into().unwrap()
}

fn style(unit: &str) -> Extension {
    INDENT_UNIT.of(IndentStyle::parse(unit).unwrap())
}

fn state(source: &str, lang: Extension, extra: impl IntoIterator<Item = Extension>) -> EditorState {
    EditorState::with_extensions(
        source,
        Extension::group(
            [
                lang,
                style("\t"),
                TAB_SIZE.of(NonZeroUsize::new(4).unwrap()),
            ]
            .into_iter()
            .chain(extra),
        ),
    )
    .unwrap()
}

fn select(state: &mut EditorState, at: usize) {
    state
        .update(Transaction::default().with_selection(EditorSelection::single(at, at)))
        .unwrap();
}

fn rust_doc(syntax: &NativeSyntax) -> &RustDocument {
    let NativeSyntax::Rust(doc) = syntax else {
        panic!("expected Rust")
    };
    doc
}

fn span(node: LrSyntaxNode<'_>) -> Value {
    json!([
        node.node_type().map(|ty| ty.term()),
        node.name(),
        node.range().start,
        node.range().end
    ])
}

fn log_call(cx: &LrIndentContext<'_>, term: u16, ticks: Option<&StateFieldReader<usize>>) -> Value {
    let state = cx.state().expect("callback must see an editor state");
    let node = cx.node();
    assert_eq!(node.node_type().unwrap().term(), term);
    if let Some(snapshot) = attached_language_snapshot(state) {
        let actual = rust_doc(snapshot.syntax())
            .nodes()
            .find(|n| {
                n.range() == node.range() && n.node_type().is_some_and(|ty| ty.term() == term)
            })
            .unwrap();
        assert!(ptr::eq(
            actual.node_type().unwrap(),
            node.node_type().unwrap()
        ));
    } else {
        assert!(markdown_snapshot(state).is_some());
    }
    let lines = cx.line_context();
    let pos = lines.position();
    let line = |assoc| {
        let at = lines.line_at(pos, assoc).unwrap();
        json!([at.range.start, at.text])
    };
    json!({
        "node": span(node), "parent": node.parent().map(span),
        "children": node.children().map(span).collect::<Vec<_>>(),
        "pos": pos, "head": state.selection().main().head(), "unit": lines.unit(),
        "tab": lines.settings().tab_size, "before": line(Assoc::Before), "after": line(Assoc::After),
        "textAfter": lines.text_after_pos(pos, Assoc::After), "base": cx.base_indent(),
        "simulated": lines.simulated_break(), "column": lines.column(pos, Assoc::After),
        "ticks": ticks.map(|ticks| *state.context().field(ticks).unwrap().unwrap()),
    })
}

fn configure(
    lang: &RustLanguage,
    rows: &Value,
    seen: &Trace,
    ticks: Option<StateFieldReader<usize>>,
) -> RustLanguage {
    let Some(rows) = rows.as_array().filter(|rows| !rows.is_empty()) else {
        return lang.clone();
    };
    let rules: Vec<_> = rows
        .iter()
        .map(|row| {
            let term = byte(&row[0]).try_into().unwrap();
            let rule = row[1].as_str().map(|mode| -> LrIndentStrategy {
                let (mode, seen) = (mode.to_owned(), Arc::clone(seen));
                Arc::new(move |cx| {
                    let call = log_call(cx, term, ticks.as_ref());
                    let head = byte(&call["head"]);
                    seen.lock().unwrap().push(call);
                    match mode.as_str() {
                        "stop" => None,
                        "continue" => cx.continue_indent(),
                        "twice" => {
                            let first = cx.continue_indent();
                            let second = cx.continue_indent();
                            Some(first? + second? + 1)
                        }
                        "context" => Some(cx.base_indent()? + cx.line_context().unit() + head % 2),
                        _ => Some(mode.parse().unwrap()),
                    }
                })
            });
            (term, rule)
        })
        .collect();
    let lang = lang.with_node_props(&[INDENT.add(move |ty| {
        rules
            .iter()
            .find(|(term, _)| *term == ty.term())
            .map(|(_, rule)| rule.clone())
    })]);
    for row in rows {
        let ty = lang.node_types().nth(byte(&row[0])).unwrap();
        assert_eq!(ty.prop(&INDENT).unwrap().is_none(), row[1].is_null());
    }
    lang
}

fn unrelated(lang: &RustLanguage) -> RustLanguage {
    lang.with_node_props(&[
        FOLD.add(|ty| (ty.term() == 43).then_some(None)),
        GROUP.add(|ty| (ty.term() == 226).then(Vec::new)),
    ])
}

fn no_trigger(lang: &RustLanguage) -> RustLanguage {
    lang.with_node_props(&[LANGUAGE_DATA.add(|ty| ty.is_top().then_some(None))])
}

#[test]
fn stock_rules_use_exact_term_rows_and_four_shared_strategies() {
    let expected = records("inventory").next().unwrap();
    let rows: Vec<_> = rust_language().node_types().collect();
    let rule = |term: usize| rows[term].prop(&INDENT).unwrap().as_ref().unwrap();
    let strategies = [rule(64), rule(7), rule(182), rule(45)];
    let inventory: Vec<_> = rows
        .iter()
        .filter_map(|ty| {
            let rule = ty.prop(&INDENT)?.as_ref().unwrap();
            let class = strategies
                .iter()
                .position(|other| Arc::ptr_eq(rule, other))
                .unwrap();
            Some(json!([ty.term(), ty.name(), class]))
        })
        .collect();
    assert_eq!(json!(inventory), expected["rows"]);
}

#[test]
fn attached_rust_uses_typed_indentation_on_matching_pinned_trees() {
    for row in records("query") {
        let id = row["case"].as_str().unwrap();
        let source = row["raw"].as_str().unwrap();
        let at = byte(&row["pos"]);
        let head = byte(&row["head"]);
        assert_eq!(
            source[..at].replace("\r\n", "\n").encode_utf16().count(),
            byte(&row["pos16"]),
            "{id}"
        );
        let opt = &row["opt"];
        let seen = Trace::default();
        let mut lang = rust_language().clone();
        if opt["unrelated"] == true {
            lang = unrelated(&lang);
        }
        if opt["closedByEmpty"] == true {
            lang = lang.with_node_props(&[CLOSED_BY.add(|ty| (ty.term() == 48).then(Vec::new))]);
        }
        lang = configure(&lang, &opt["before"], &seen, None);
        lang = configure(&lang, &row["props"], &seen, None);
        if opt["strict"] == true {
            lang = lang.configure(Some(true), None);
        }
        if opt["quiet"] == true {
            lang = no_trigger(&lang);
        }
        let service = opt["service"].as_str().map(|rule| {
            let decision = if rule == "stop" {
                IndentDecision::Inherit
            } else {
                IndentDecision::Column(rule.parse().unwrap())
            };
            indent_service(move |_, _| decision)
        });
        let mut state = state(source, lang.extension(), service);
        let snapshot = attached_language_snapshot(&state).unwrap();
        if head != 0 {
            select(&mut state, head);
            assert!(Arc::ptr_eq(
                &snapshot,
                &attached_language_snapshot(&state).unwrap()
            ));
        }
        let doc = rust_doc(snapshot.syntax());
        let projected: Vec<_> = doc
            .nodes()
            .map(|n| {
                json!([
                    n.node_type().map(|r| r.term()),
                    n.kind().name(),
                    n.range().start,
                    n.range().end
                ])
            })
            .collect();
        assert_eq!(json!(projected), row["tree"], "{id}");
        let override_at = if opt["overrideAtQuery"] == true {
            at
        } else {
            0
        };
        let get_override = |pos| {
            if pos == override_at {
                opt["override"].as_u64().map(|n| n.try_into().unwrap())
            } else {
                None
            }
        };
        let options = IndentOptions {
            simulate_break: opt["sim"].is_number().then_some(at),
            simulate_double_break: opt["sim"] == 2,
            override_indentation: Some(&get_override),
        };
        let result = get_indentation(&state, at, options, None).unwrap();
        let settings = StreamSettings {
            tab_size: state.tab_size(),
            indent_unit: 4,
        };
        let direct = doc.indentation(at, settings, options);
        let mut expected_calls = row["calls"].clone();
        // Keep the legacy stock line base and delimiter metadata policy. The
        // callback context also keeps the native simulated offset and text cap.
        let expected = match id {
            "lone-cr-stock-scout" | "lone-cr-unrelated" | "lone-cr-unused-indent" => json!(4),
            "closed-by-empty-scout" => json!(8),
            "zero-break-scout" => {
                expected_calls[0]["simulated"] = json!(0);
                row["result"].clone()
            }
            "utf16-100-vs-scalars" => {
                expected_calls[0]["textAfter"] = json!(&source[at..]);
                row["result"].clone()
            }
            _ => row["result"].clone(),
        };
        assert_eq!(json!(result), expected, "{id}");
        assert_eq!(json!(*seen.lock().unwrap()), expected_calls, "{id}");
        let expected_stock = if id.starts_with("lone-cr-") {
            json!(4)
        } else {
            row["stock"].clone()
        };
        assert_eq!(json!(direct), expected_stock, "{id}");
    }
}

#[test]
fn only_current_whole_rust_identity_can_dispatch() {
    let source = "fn f() {\r\n  g();\r\n}";
    let pos = source.find("  g").unwrap();
    let options = IndentOptions::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let rule: LrIndentStrategy = Arc::new({
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Some(17)
        }
    });
    let lang = rust_language().with_node_props(&[
        INDENT.add(move |ty| (ty.term() == 43).then(|| Some(Arc::clone(&rule))))
    ]);
    let slot = Compartment::new();
    let mut current = state(source, slot.of(lang.extension()), []);
    let retained = current.clone();
    let attached = attached_language_snapshot(&current).unwrap();
    let other = attached_language_snapshot(&state(source, lang.extension(), [])).unwrap();
    assert!(!Arc::ptr_eq(&attached, &other));
    assert_eq!(
        get_indentation(&current, pos, options, Some(other.syntax())).unwrap(),
        Some(4)
    );
    assert_eq!(
        get_indentation(&current, pos - 1, options, None).unwrap(),
        Some(4)
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        get_indentation(&current, pos, options, Some(attached.syntax())).unwrap(),
        Some(17)
    );
    current
        .update(
            Transaction::default().with_effects([slot.reconfigure(rust_language().extension())]),
        )
        .unwrap();
    assert_eq!(
        get_indentation(&current, pos, options, Some(attached.syntax())).unwrap(),
        Some(4)
    );
    assert_eq!(
        get_indentation(&current, pos, options, None).unwrap(),
        Some(4)
    );
    current
        .update(Transaction::default().with_effects([slot.reconfigure(Extension::group([]))]))
        .unwrap();
    assert_eq!(get_indentation(&current, pos, options, None).unwrap(), None);
    assert_eq!(
        get_indentation(&retained, pos, options, None).unwrap(),
        Some(17)
    );
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[test]
fn rowless_rust_nodes_keep_native_indentation() {
    let mut source = String::from("fn f() { \r\n  1\r\n}\r\n");
    source.push_str(&"r#\"x\";".repeat(2048));
    let seen = Trace::default();
    let lang = configure(rust_language(), &json!([[43, "17"]]), &seen, None);
    let state = state(&source, lang.extension(), []);
    let attached = attached_language_snapshot(&state).unwrap();
    let doc = rust_doc(attached.syntax());
    assert!(doc.is_recovered());
    assert!(
        doc.nodes()
            .any(|node| node.kind().name() == "Block" && node.node_type().is_none())
    );
    assert_eq!(
        get_indentation(&state, 11, IndentOptions::default(), None).unwrap(),
        Some(4)
    );
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn repeated_continue_shares_the_custom_budget_but_keeps_the_64th_result() {
    let source = "fn f() {\n  if yes {\n    yes();\n  }\n}";
    let calls = Arc::new(AtomicUsize::new(0));
    let outer: LrIndentStrategy = Arc::new({
        let calls = Arc::clone(&calls);
        move |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            Some(17)
        }
    });
    let inner: LrIndentStrategy = Arc::new(|cx| {
        let mut last = None;
        for _ in 0..63 {
            last = cx.continue_indent();
            assert_eq!(last, Some(17));
        }
        assert_eq!(cx.continue_indent(), None);
        last
    });
    let lang = rust_language().with_node_props(&[INDENT.add(move |ty| match ty.term() {
        43 => Some(Some(Arc::clone(&inner))),
        64 => Some(Some(Arc::clone(&outer))),
        _ => None,
    })]);
    let state = state(source, lang.extension(), []);
    for query in 1..=2 {
        assert_eq!(
            get_indentation(&state, 20, IndentOptions::default(), None).unwrap(),
            Some(17)
        );
        assert_eq!(calls.load(Ordering::Relaxed), 63 * query);
    }
}

fn markdown_rust(parser: &MarkdownCodeParser, fallback: bool) -> Extension {
    let named = parser.clone();
    markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Configured {
                resolve: Arc::new(move |name| (name == "rr").then(|| named.clone())),
                fallback: fallback.then(|| parser.clone()),
            }),
            ..Default::default()
        },
    )
}

#[test]
fn markdown_rust_mounts_project_typed_queries_without_granting_edits() {
    for row in records("markdown") {
        let seen = Trace::default();
        let lang = configure(rust_language(), &row["props"], &seen, None);
        let parser = MarkdownCodeParser::Rust(lang.clone());
        let id = row["case"].as_str().unwrap();
        let extension = markdown_rust(&parser, id.ends_with("fallback"));
        let state = state(row["raw"].as_str().unwrap(), extension, []);
        let at = byte(&row["pos"]);
        let regions = lang.find_regions(&state);
        assert!(regions.iter().any(|range| range.contains(&at)), "{id}");
        let result = (!row["result"].is_null()).then(|| byte(&row["result"]));
        let mut calls = row["calls"].clone();
        if id == "continue-through-root" {
            // A native callback sees the mounted tree, not its host parent.
            assert_eq!(calls[2]["parent"][1], "FencedCode");
            calls[2]["parent"] = Value::Null;
        }
        assert_eq!(
            get_indentation(&state, at, IndentOptions::default(), None).unwrap(),
            result,
            "{id}"
        );
        assert_eq!(
            json!(std::mem::take(&mut *seen.lock().unwrap())),
            calls,
            "{id}"
        );
        let snapshot = markdown_snapshot(&state).unwrap();
        let body = regions.first().unwrap().start..regions.last().unwrap().end;
        let scoped = MarkdownCodeIndent::for_parser(&state, &snapshot, body.clone(), &parser);
        if regions.len() == 1 {
            let settings = StreamSettings {
                tab_size: state.tab_size(),
                indent_unit: 4,
            };
            let scoped = scoped.unwrap();
            assert_eq!(
                scoped.at(at - body.start, settings, IndentOptions::default()),
                result
            );
            assert_eq!(json!(std::mem::take(&mut *seen.lock().unwrap())), calls);
            assert_eq!(scoped.at(0, settings, IndentOptions::default()), None);
            assert_eq!(
                scoped.at(body.len(), settings, IndentOptions::default()),
                None
            );
            let other = MarkdownCodeParser::Rust(lang.configure(None, None));
            assert!(MarkdownCodeIndent::for_parser(&state, &snapshot, body, &other).is_none());
        } else {
            assert!(scoped.is_none(), "{id}");
            let gap = regions[1].start - 1;
            assert_eq!(
                get_indentation(&state, gap, IndentOptions::default(), None).unwrap(),
                None
            );
        }
        assert!(seen.lock().unwrap().is_empty(), "{id}");
    }
}

#[test]
fn contiguous_markdown_rust_input_uses_the_provisional_host() {
    for row in records("markdown-input") {
        let source = row["raw"].as_str().unwrap();
        let caret = byte(&row["pos"]);
        let seen = Trace::default();
        let lang = configure(rust_language(), &row["props"], &seen, None);
        let parser = MarkdownCodeParser::Rust(lang.clone());
        let mut state = state(source, markdown_rust(&parser, false), []);
        select(&mut state, caret);
        let regions = lang.find_regions(&state);
        let [body] = regions.as_slice() else {
            panic!("expected one Rust segment: {regions:?}");
        };
        let snap = markdown_snapshot(&state).unwrap();
        let input = state
            .replace_selection("}")
            .unwrap()
            .with_user_event("input.type");
        let planned = snap
            .reindent_mounted_input(&state, body.clone(), &parser, input)
            .unwrap()
            .unwrap();
        assert_eq!(json!(*seen.lock().unwrap()), row["calls"]);
        assert_eq!(state.text().as_str(), source);
        assert_eq!(state.selection().main().head(), caret);
        assert!(
            planned
                .changes()
                .changes()
                .all(|(span, _)| { span.start >= body.start && span.end <= body.end })
        );
        state.update(planned).unwrap();
        assert_eq!(state.text().as_str(), row["result"]);
        assert_eq!(state.selection().main().head(), byte(&row["head"]));
        assert!(MarkdownCodeIndent::for_parser(&state, &snap, body.clone(), &parser).is_none());
    }
}

fn edit_counter(calls: &Arc<AtomicUsize>) -> StateField<usize> {
    let calls = Arc::clone(calls);
    StateField::new(
        |_| Ok(0),
        move |count, update| {
            let changed = usize::from(!update.transaction().changes().is_empty());
            calls.fetch_add(changed, Ordering::Relaxed);
            Ok(count + changed)
        },
    )
}

#[test]
fn attached_rust_input_uses_one_resolved_candidate_and_commits_atomically() {
    for row in records("input") {
        let id = row["case"].as_str().unwrap();
        let source = row["raw"].as_str().unwrap();
        let caret = byte(&row["pos"]);
        let seen = Trace::default();
        let applications = Arc::new(AtomicUsize::new(0));
        let field = edit_counter(&applications);
        let props = match id {
            "stop" => json!([[43, "stop"]]),
            "disabled-if" => json!([[64, null]]),
            _ => json!([[43, "17"]]),
        };
        let stock = rust_language();
        let watch = configure(stock, &props, &seen, Some(field.reader()));
        let strict = watch.configure(Some(true), None);
        let slot = Compartment::new();
        let initial = match id {
            "reconfigure" | "reconfigure-service" => stock.extension(),
            "starting-data-disabled" => no_trigger(stock).extension(),
            _ => watch.extension(),
        };
        let filters = Arc::new(AtomicUsize::new(0));
        let extenders = Arc::new(AtomicUsize::new(0));
        let mut state = state(
            source,
            slot.of(initial),
            [
                field.extension(),
                collab(CollabConfig::default()),
                change_filter({
                    let filters = Arc::clone(&filters);
                    move |_, _| {
                        filters.fetch_add(1, Ordering::Relaxed);
                        Default::default()
                    }
                }),
                transaction_extender({
                    let extenders = Arc::clone(&extenders);
                    move |_, _| {
                        extenders.fetch_add(1, Ordering::Relaxed);
                        TransactionExtension::default()
                    }
                }),
            ],
        );
        select(&mut state, caret);
        let original = attached_language_snapshot(&state).unwrap();
        let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
        let view = editor.view_mut();
        let input = view
            .state()
            .replace_selection(row["insert"].as_str().unwrap())
            .unwrap()
            .with_user_event(row["event"].as_str().unwrap_or("input.type"));
        let input = match id {
            "starting-data-disabled" => input.with_effects([slot.reconfigure(watch.extension())]),
            "reconfigure-service" => input.with_effects([slot.reconfigure(Extension::group([
                watch.extension(),
                indent_service(|_, _| IndentDecision::Column(5)),
            ]))]),
            "reconfigure" => input.with_effects([
                slot.reconfigure(Extension::group([watch.extension(), style("   ")]))
            ]),
            "strict-repair" | "strict-reject" => {
                input.with_effects([slot.reconfigure(strict.extension())])
            }
            "starting-data-only" => input.with_effects([slot.reconfigure(Extension::group([]))]),
            _ => input,
        };
        filters.store(0, Ordering::Relaxed);
        extenders.store(0, Ordering::Relaxed);
        let result = view.update(input, (), |_| (None, ()));
        assert_eq!(filters.load(Ordering::Relaxed), 1, "{id}");
        assert_eq!(extenders.load(Ordering::Relaxed), 1, "{id}");
        assert_eq!(json!(*seen.lock().unwrap()), row["calls"], "{id}");
        if id == "strict-reject" {
            assert!(matches!(
                result,
                Err(EditError::Configuration(ConfigurationError::StrictSyntax(
                    _
                )))
            ));
            assert_eq!(view.state().text().as_str(), source);
            assert_eq!(view.state().selection().main().head(), caret);
            assert_eq!(view.undo_depth(), 0);
            assert!(sendable_updates(view.state()).unwrap().is_empty());
            assert_eq!(applications.load(Ordering::Relaxed), 0);
            assert!(Arc::ptr_eq(
                &original,
                &attached_language_snapshot(view.state()).unwrap()
            ));
            continue;
        }
        result.unwrap();
        assert_eq!(view.state().text().as_str(), row["result"], "{id}");
        assert_eq!(
            view.state().selection().main().head(),
            byte(&row["head"]),
            "{id}"
        );
        assert_eq!(view.undo_depth(), 1, "{id}");
        let updates = sendable_updates(view.state()).unwrap();
        assert_eq!(updates.len(), 1, "{id}");
        let mut replay = original.text().clone();
        updates[0].changes().apply_to(&mut replay).unwrap();
        assert_eq!(&replay, view.state().text(), "{id}");
        assert_eq!(
            applications.load(Ordering::Relaxed),
            if id == "starting-data-disabled" { 1 } else { 2 },
            "{id}"
        );
        if id == "starting-data-only" {
            assert!(attached_language_snapshot(view.state()).is_none());
        }
        if id == "17" {
            assert_eq!(view.undo(), Some(()));
            assert_eq!(view.state().text().as_str(), source);
            assert_eq!(view.state().selection().main().head(), caret);
        }
    }
}

#[test]
fn stock_and_unrelated_props_do_not_need_provisional_rust_state() {
    let stock = rust_language();
    let inactive = stock.with_node_props(&[INDENT.add(|ty| (ty.term() == 43).then_some(None))]);
    for lang in [stock.clone(), unrelated(stock), inactive] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut state = state(
            "fn f() {\r    \r\n",
            lang.extension(),
            [edit_counter(&calls).extension()],
        );
        select(&mut state, "fn f() {\r    ".len());
        let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
        editor
            .insert_text("}", std::time::Duration::ZERO, ())
            .unwrap();
        assert_eq!(editor.view().state().text().as_str(), "fn f() {\r}\r\n");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
