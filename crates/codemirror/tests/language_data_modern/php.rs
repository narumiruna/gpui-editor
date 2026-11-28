use std::{
    cell::Cell,
    sync::{Arc, LazyLock, Mutex},
};

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState,
    Extension, MapMode, Precedence, READ_ONLY, StateEffectType, Text, Transaction,
    autocomplete::session::SourceResult,
    commands::CommentTokens,
    language::{
        css::CssPropertySnapshot,
        html::{HtmlNesting, HtmlOptions},
        javascript::javascript_completion_snippet,
        php::{PhpDocument, PhpOptions},
    },
    language_data::{
        CloseBracketData, LANGUAGES, LanguageDescription, NativeLanguage, NativeSyntax,
        ScriptCompletionSource, attached_language_snapshot,
    },
    transaction_filter,
    view::{EditorView, input::InputStep, update_listener},
};
use serde_json::{Value, json};

use super::{bundle, byte, load};

type Sources = Vec<(&'static str, ScriptCompletionSource)>;

fn rows() -> impl Iterator<Item = Value> {
    include_str!("../fixtures/language_data_php.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn sources(state: &EditorState, at: usize, side: Assoc) -> Vec<ScriptCompletionSource> {
    state.language_data_at("autocomplete", at, side)
}

fn source_ids() -> Sources {
    let js = EditorState::with_extensions("", bundle("JavaScript")).unwrap();
    let css = EditorState::with_extensions("", bundle("CSS")).unwrap();
    let [fixed, local] = sources(&js, 0, Assoc::Before).try_into().ok().unwrap();
    let [css] = sources(&css, 0, Assoc::Before).try_into().ok().unwrap();
    let mut ids = vec![
        ("js-fixed", fixed),
        ("local", Arc::clone(&local)),
        ("css", Arc::clone(&css)),
    ];
    let source = "<script>let x=0;</script><style>a{}</style>";
    for (bundle_name, schema_id, fixed_id) in [
        ("HTML", "html", "html-fixed"),
        ("PHP", "php-html", "php-fixed"),
    ] {
        let state = EditorState::with_extensions(source, bundle(bundle_name)).unwrap();
        let [schema] = sources(&state, 1, Assoc::Before).try_into().ok().unwrap();
        let [fixed, mounted_local] = sources(&state, 9, Assoc::Before).try_into().ok().unwrap();
        let [mounted_css] = sources(&state, 32, Assoc::Before).try_into().ok().unwrap();
        assert!(Arc::ptr_eq(&mounted_local, &local));
        assert!(Arc::ptr_eq(&mounted_css, &css));
        for (id, source) in [(schema_id, schema), (fixed_id, fixed)] {
            assert!(ids.iter().all(|(_, prior)| !Arc::ptr_eq(prior, &source)));
            ids.push((id, source));
        }
    }
    ids
}

fn start(source: &str, extension: Extension, pin: &Value) -> EditorState {
    let keys = pin["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap());
    EditorState::with_extensions(
        source,
        Extension::group([
            extension,
            CssPropertySnapshot::from_style_keys(keys).extension(),
        ]),
    )
    .unwrap()
}

fn summary(id: &str, result: SourceResult) -> Value {
    let mut hash = 2166136261u32;
    let mut split_names = Vec::new();
    for item in result.options.iter() {
        let apply = if id.ends_with("-fixed") && javascript_completion_snippet(item).is_some() {
            "@snippet"
        } else {
            item.insert.as_deref().unwrap_or("")
        };
        for b in format!(
            "{}\0{}\0{}\0{}\0{apply}\n",
            item.label,
            item.kind.as_deref().unwrap_or(""),
            item.detail.as_deref().unwrap_or(""),
            item.boost,
        )
        .bytes()
        {
            hash = (hash ^ u32::from(b)).wrapping_mul(16777619);
        }
        if item.label.contains("<?") || apply.contains("<?") {
            split_names.push(json!([item.label, item.insert]));
        }
    }
    json!({"range": [result.from, result.to], "filter": result.filter,
        "count": result.options.len(), "hash": hash, "splitNames": split_names})
}

fn expected_result(row: &Value, query: &Value, mode: usize, index: usize, pin: &Value) -> Value {
    if row["kind"] != "gap" {
        return pin.clone();
    }
    if row["name"] == "quoted event split at quote"
        && index == 1
        && matches!(
            (query["label"].as_str().unwrap(), byte(&query["at"])),
            ("after gap" | "before gap", 26) | ("gap start", 27)
        )
    {
        // The compact quoted script sees `co` as an identifier. Its empty
        // local result uses that safe range instead of the original caret.
        if mode == 0 {
            assert!(pin.is_null());
        } else {
            assert_eq!(pin["range"], json!([query["at"], null]));
            assert_eq!(pin["count"], 0);
        }
        return json!({"range": [25, null], "filter": true, "count": 0,
            "hash": 2166136261u32, "splitNames": []});
    }
    if pin.is_null() || pin["crossesGap"] == true {
        return Value::Null;
    }
    match (
        row["name"].as_str().unwrap(),
        query["label"].as_str().unwrap(),
    ) {
        ("script hole and cross-gap local", "after gap" | "split JS replacement") if index == 0 => {
            // The fixed source can use the original suffix; native rejects its compact span.
            assert_eq!(pin["range"], json!([49, null]));
            Value::Null
        }
        ("split schema tag and open name", "close suggestion") => {
            // The insertion range maps, but the name to copy does not.
            assert_eq!(
                pin["splitNames"],
                json!([["sec<?= 'é' ?>tion", "sec<?= 'é' ?>tion>"]])
            );
            json!({"range": [47, 47], "filter": true, "count": 0,
                "hash": 2166136261u32, "splitNames": []})
        }
        _ => {
            let mut result = pin.clone();
            result.as_object_mut().unwrap().remove("crossesGap");
            result
        }
    }
}

fn compare_queries(
    state: &EditorState,
    row: &Value,
    ids: &Sources,
    pin: &Value,
    differences: &mut Vec<String>,
) {
    for q in row["queries"].as_array().unwrap() {
        let at = byte(&q["at"]);
        let label = format!(
            "{} / {} / {} byte {at}",
            row["name"], row["step"], q["label"]
        );
        let identify = |f: &ScriptCompletionSource| {
            ids.iter()
                .find(|(_, other)| Arc::ptr_eq(other, f))
                .unwrap()
                .0
        };
        for (i, side) in [Assoc::Before, Assoc::After].into_iter().enumerate() {
            let want = &q["data"][i];
            let names: Vec<_> = sources(state, at, side).iter().map(identify).collect();
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
            let indent: Vec<Vec<_>> = state
                .language_data_at::<fn(&str) -> bool>("indentOnInput", at, side)
                .iter()
                .map(|rule| {
                    pin["triggers"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|t| rule(t.as_str().unwrap()))
                        .collect()
                })
                .collect();
            for (key, actual) in [
                ("sources", json!(names)),
                ("comments", json!(comments)),
                ("brackets", json!(brackets)),
                ("indent", json!(indent)),
                (
                    "words",
                    json!(state.language_data_at::<&str>("wordChars", at, side)),
                ),
            ] {
                if actual != want[key] {
                    differences.push(format!(
                        "{label} {side:?} ({}) {key}: native {actual}; pin {}",
                        want["active"], want[key]
                    ));
                }
            }
        }
        let files = sources(state, at, Assoc::Before);
        for (i, explicit) in [false, true].into_iter().enumerate() {
            let actual: Vec<_> = files
                .iter()
                .map(|f| f(state, at, explicit).map_or(Value::Null, |r| summary(identify(f), r)))
                .collect();
            let expected: Vec<_> = q["results"][i]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(index, r)| expected_result(row, q, i, index, r))
                .collect();
            if actual != expected {
                differences.push(format!(
                    "{label} explicit={explicit}: native {}; expected {}",
                    json!(actual),
                    json!(expected)
                ));
            }
        }
    }
}

fn overlay(doc: &PhpDocument) -> Value {
    let ranges: Vec<_> = doc.html().map_or_else(Vec::new, |html| {
        html.source_ranges(0..html.document().text().len()).unwrap()
    });
    json!(ranges.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>())
}

fn dialect(doc: &PhpDocument) -> Value {
    let names: Vec<_> = doc
        .html()
        .into_iter()
        .flat_map(|html| html.document().nodes())
        .map(|node| node.kind().name())
        .filter(|name| matches!(*name, "SelfClosingTag" | "MismatchedCloseTag"))
        .collect();
    json!(names)
}

#[test]
fn stock_php_replays_attached_data_and_completion() {
    let entry = LanguageDescription::match_name(LANGUAGES, "PHP", false).unwrap();
    let support = load(entry);
    assert_eq!(support.language(), NativeLanguage::Php);
    assert!(Arc::ptr_eq(&support, &load(entry)));
    assert!(NativeLanguage::Php.support().extension().is_none());
    assert!(
        support.extension().is_some(),
        "stock PHP catalog extension is missing"
    );
    let ids = source_ids();
    let pin = rows().next().unwrap();
    let mut slot = Compartment::new();
    let mut state = EditorState::new("");
    let mut retained = None;
    let mut same_compact = None;
    let mut differences = Vec::new();
    for row in rows().skip(1) {
        let source = row["source"].as_str().unwrap();
        match row["kind"].as_str().unwrap() {
            "start" => {
                slot = Compartment::new();
                state = start(source, slot.of(Extension::default()), &pin);
                continue;
            }
            "input" => continue,
            "gap" => {
                let state = start(source, bundle("PHP"), &pin);
                let active = attached_language_snapshot(&state).unwrap();
                let NativeSyntax::Php(doc) = active.syntax() else {
                    unreachable!();
                };
                assert_eq!(overlay(doc), row["segments"], "{}", row["name"]);
                let text = doc.html().unwrap().document().text();
                assert_eq!(text.as_str(), row["rawText"], "{}", row["name"]);
                if row["name"] == "lone CR and LF meet only in the projection" {
                    assert_eq!(row["logicalLines"], json!([3, 2]));
                    assert_eq!(text.line_count(), 2);
                }
                compare_queries(&state, &row, &ids, &pin, &mut differences);
                continue;
            }
            "state" => {}
            kind => panic!("unexpected record {kind}"),
        }
        let step = row["step"].as_str().unwrap();
        let label = format!("{} / {step}", row["name"]);
        // The additional factory switches have no native configured attachment.
        // Compare their trees through PHP's existing parser options.
        let options = PhpOptions::default();
        let parsed = match step {
            "bare PHP" => Some(PhpDocument::parse(Text::new(source))),
            "borrowed" | "borrowed noMatch" => Some(PhpDocument::parse_with_html_nesting(
                Text::new(source),
                options,
                HtmlOptions {
                    match_closing_tags: step == "borrowed",
                    self_closing_tags: step == "borrowed",
                },
                HtmlNesting::default(),
            )),
            "no base" | "plain" => Some(PhpDocument::parse_with_options(
                Text::new(source),
                PhpOptions {
                    plain: step == "plain",
                    html: step != "no base",
                },
            )),
            _ => None,
        };
        if let Some(doc) = parsed {
            assert_eq!(doc.root().name(), row["root"], "{label}");
            assert_eq!(overlay(&doc), row["overlay"], "{label}");
            assert_eq!(dialect(&doc), row["dialect"], "{label}");
            if step == "no base" {
                state
                    .update(
                        Transaction::default()
                            .with_effects([slot.reconfigure(Extension::default())]),
                    )
                    .unwrap();
            }
            continue;
        }
        let before = attached_language_snapshot(&state);
        let mut tr = Transaction::default();
        if let Some(edit) = row["edit"].as_array() {
            tr = Transaction::new(
                ChangeSet::single(Change::new(
                    byte(&edit[0])..byte(&edit[1]),
                    edit[2].as_str().unwrap(),
                ))
                .unwrap(),
            );
        }
        if !row["select"].is_null() {
            let at = byte(&row["select"]);
            tr = tr.with_selection(EditorSelection::single(at, at));
        }
        if let Some(exts) = row["extensions"].as_array() {
            tr = tr.with_effects([slot.reconfigure(Extension::group(exts.iter().map(|ext| {
                let name = ext.as_str().unwrap();
                if name == "high JavaScript" {
                    bundle("JavaScript").with_precedence(Precedence::High)
                } else {
                    bundle(name)
                }
            })))]);
        }
        state.update(tr).unwrap();
        let active = attached_language_snapshot(&state);
        let (language, root) = active.as_ref().map_or((None, ""), |active| {
            assert_eq!(active.text(), state.text(), "{label}");
            match active.syntax() {
                NativeSyntax::Php(doc) => {
                    assert_eq!(overlay(doc), row["overlay"], "{label}");
                    assert_eq!(dialect(doc), row["dialect"], "{label}");
                    if row["name"] == "route switches with the same compact text" {
                        let text = doc.html().unwrap().document().text();
                        if let Some(prior) = &same_compact {
                            assert_eq!(text, prior);
                        } else {
                            same_compact = Some(text.clone());
                        }
                    }
                    (Some("php"), doc.root().name())
                }
                NativeSyntax::Html(doc) => (Some("html"), doc.root().kind().name()),
                NativeSyntax::Javascript(doc) => (Some("javascript"), doc.root().kind().name()),
                _ => panic!("unexpected root for {label}"),
            }
        });
        if row["name"] == "catalog composition" && step == "install" {
            retained = Some(state.clone());
        }
        assert_eq!(json!(language), row["active"], "{label}");
        assert_eq!(root, row["root"], "{label}");
        assert_eq!(source, state.text().as_str(), "{label}");
        assert_eq!(
            state.selection().main().head(),
            byte(&row["head"]),
            "{label}"
        );
        assert_eq!(
            row["reused"],
            matches!((&before, &active), (Some(a), Some(b)) if Arc::ptr_eq(a, b)),
            "{label}"
        );
        compare_queries(&state, &row, &ids, &pin, &mut differences);
    }
    let retained = retained.unwrap();
    let initial = rows()
        .find(|row| row["name"] == "catalog composition" && row["step"] == "install")
        .unwrap();
    assert_eq!(retained.text().as_str(), initial["source"]);
    compare_queries(&retained, &initial, &ids, &pin, &mut differences);
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[test]
fn stock_php_rejects_invalid_original_query_offsets() {
    let source = "<p>é\r\n</p><script>co\r\n</script><style>é\r\n</style><?php /* é\r\n */ ?>";
    let state = EditorState::with_extensions(source, bundle("PHP")).unwrap();
    let ids = source_ids();
    for (start, _) in source.match_indices(['é', '\r']) {
        let at = start + 1;
        for side in [Assoc::Before, Assoc::After] {
            assert!(sources(&state, at, side).is_empty(), "byte {at} {side:?}");
            assert!(
                state
                    .language_data_at::<CommentTokens>("commentTokens", at, side)
                    .is_empty(),
                "byte {at} {side:?}"
            );
        }
        for (id, f) in &ids {
            assert!(f(&state, at, true).is_none(), "{id} at byte {at}");
        }
    }
}

#[test]
fn stock_php_does_not_complete_an_existing_token_instead_of_the_typed_byte() {
    let source = "<div><em";
    let at = source.len();
    let filter = transaction_filter(|_, tr| {
        Ok(if tr.is_user_event("input.type") {
            Transaction::default().with_selection(EditorSelection::single(5, 5))
        } else {
            tr
        })
    });
    let state = EditorState::create_from_text(
        Text::new(source),
        Some(EditorSelection::single(at, at)),
        Extension::group([bundle("PHP"), filter]),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    view.apply_headless_input(
        at..at,
        ">",
        |state| {
            Ok(state
                .replace_selection_parsed(">")?
                .with_user_event("input.type"))
        },
        (),
    )
    .unwrap();
    assert_eq!(view.state().text().as_str(), source);
    assert_eq!(view.state().selection().main().head(), 5);
}

static MARKER: LazyLock<StateEffectType<usize>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|at, changes| changes.map_pos(*at, Assoc::After, MapMode::Simple))
});

fn selection_json(selection: &EditorSelection) -> Value {
    json!(
        selection
            .ranges()
            .iter()
            .map(|r| {
                let assoc = match r.assoc() {
                    Some(Assoc::Before) => -1,
                    Some(Assoc::After) => 1,
                    None => 0,
                };
                json!([r.anchor(), r.head(), assoc])
            })
            .collect::<Vec<_>>()
    )
}

fn input_step(step: &InputStep) -> Value {
    let tr = &step.transaction;
    let changes: Vec<_> = tr
        .changes()
        .changes()
        .map(|(range, text)| json!([range.start, range.end, text]))
        .collect();
    let selection = step.state.selection();
    let markers: Vec<_> = tr
        .effects()
        .iter()
        .filter_map(|fx| fx.get(&MARKER))
        .collect();
    json!({"changes": changes, "after": step.state.text().as_str(),
        "selection": selection_json(selection), "main": selection.main_index(),
        "event": tr.user_event(), "scroll": tr.scroll_into_view(), "markers": markers})
}

#[test]
fn stock_php_replays_accepted_headless_input() {
    let mut differences = Vec::new();
    for row in rows().filter(|row| row["kind"] == "input") {
        let name = row["name"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let mode = row["mode"].as_str().unwrap();
        let typed = row["typed"].as_str().unwrap();
        let selection = EditorSelection::new(
            row["selection"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    let assoc = match r[2].as_i64().unwrap() {
                        -1 => Some(Assoc::Before),
                        1 => Some(Assoc::After),
                        0 => None,
                        n => panic!("unexpected association {n}"),
                    };
                    EditorSelection::range(byte(&r[0]), byte(&r[1]), None, None, assoc)
                })
                .collect(),
            byte(&row["main"]),
        )
        .unwrap();
        assert_eq!(selection_json(&selection), row["selection"], "{name}");
        let range = selection.main().range();
        let at = range.start;
        let to = range.end;
        let rewrite = match mode {
            "rewrite" => Some((at - 3, "Élément>")),
            "rewrite to raw" => Some((at - 3, "script>")),
            "omit typed" => Some((at, "x")),
            _ => None,
        };
        let filter = transaction_filter(move |_, tr| {
            let Some((from, insert)) = rewrite.filter(|_| tr.is_user_event("input.type")) else {
                return Ok(tr);
            };
            let head = from + insert.len();
            Ok(
                Transaction::new(ChangeSet::single(Change::new(from..to, insert))?)
                    .with_selection(EditorSelection::single(head, head))
                    .with_user_event("input.type")
                    .with_scroll_into_view(),
            )
        });
        let observations = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&observations);
        let listener = update_listener(move |update| {
            let transactions: Vec<_> = update.input_steps.iter().map(input_step).collect();
            let markers: Vec<_> = update
                .effects
                .iter()
                .filter_map(|effect| effect.get(&MARKER))
                .collect();
            log.lock()
                .unwrap()
                .push(json!({"transactions": transactions, "markers": markers}));
            Ok(())
        });
        let state = EditorState::create_from_text(
            Text::new(source),
            Some(selection),
            Extension::group([
                bundle("PHP"),
                bundle("PHP"),
                ALLOW_MULTIPLE_SELECTIONS.of(true),
                READ_ONLY.of(mode == "read only"),
                filter,
                listener,
            ]),
        )
        .unwrap();
        let mut view = EditorView::new(state);
        if view.state().read_only() || mode == "composition" {
            // The headless host does not dispatch either category of input.
            assert_eq!(row["transactions"], json!([]), "{name}");
            continue;
        }
        let builds = Cell::new(0);
        view.apply_headless_input(
            range,
            typed,
            |state| {
                builds.set(builds.get() + 1);
                let mut tr = state
                    .replace_selection_parsed(typed)?
                    .with_user_event("input.type")
                    .with_scroll_into_view();
                if mode == "marker" {
                    tr = tr.with_effects([MARKER.of(at + typed.len())]);
                }
                Ok(tr)
            },
            (),
        )
        .unwrap();
        // The pin's handler-only call count excludes a host default on a pass.
        assert_eq!(builds.get(), 1, "{name}");
        let observed = observations.lock().unwrap();
        assert_eq!(observed.len(), 1, "{name}");
        let mut expected = row["transactions"].clone();
        if matches!(name, "split open name" | "split slash name") {
            let rejected = expected.as_array_mut().unwrap().pop().unwrap();
            assert!(rejected["changes"][0][2].as_str().unwrap().contains("<?"));
        }
        assert_eq!(observed[0]["markers"], row["markers"], "{name}");
        let last = expected.as_array().unwrap().last().unwrap();
        if observed[0]["transactions"] != expected || view.state().text().as_str() != last["after"]
        {
            differences.push(format!(
                "{name}: native {}; expected {expected}",
                observed[0]["transactions"]
            ));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
