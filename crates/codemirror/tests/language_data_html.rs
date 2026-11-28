use std::{
    cell::Cell,
    future::Future,
    pin::Pin,
    sync::{Arc, LazyLock, Mutex},
    task::{Context, Poll, Waker},
};

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState,
    Extension, MapMode, Precedence, READ_ONLY, StateEffectType, Text, Transaction,
    autocomplete::session::SourceResult,
    commands::CommentTokens,
    language::{css::CssPropertySnapshot, javascript::javascript_completion_snippet},
    language_data::{
        CloseBracketData, LANGUAGES, LanguageDescription, NativeLanguage, NativeLanguageSupport,
        NativeSyntax, ScriptCompletionSource, attached_language_snapshot,
    },
    transaction_filter,
    view::{EditorView, update_listener},
};
use serde_json::{Value, json};

fn rows() -> impl Iterator<Item = Value> {
    include_str!("fixtures/language_data_html.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn load(name: &str) -> Arc<NativeLanguageSupport> {
    // The native loader can report Pending while another test publishes.
    static LOAD: Mutex<()> = Mutex::new(());
    let _guard = LOAD.lock().unwrap();
    let entry = LanguageDescription::match_name(LANGUAGES, name, false).unwrap();
    let mut future = entry.catalog_support_load().unwrap().load();
    let Poll::Ready(Ok(support)) =
        Pin::new(&mut future).poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("missing catalog support for {name}");
    };
    support
}

fn extension(name: &str) -> Extension {
    if let Some(name) = name.strip_prefix("high ") {
        extension(name).with_precedence(Precedence::High)
    } else {
        load(name).extension().unwrap()
    }
}

fn autocomplete(state: &EditorState, at: usize, side: Assoc) -> Vec<ScriptCompletionSource> {
    state.language_data_at("autocomplete", at, side)
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn source_ids() -> [(&'static str, ScriptCompletionSource); 5] {
    let support = load("HTML");
    assert!(Arc::ptr_eq(&support, &load("HTML")));
    assert!(NativeLanguage::Html.support().extension().is_none());
    let html = EditorState::with_extensions(
        "<script>let x=0;</script><style>a{}</style>",
        support.extension().unwrap(),
    )
    .unwrap();
    let js = EditorState::with_extensions("", extension("JavaScript")).unwrap();
    let css = EditorState::with_extensions("", extension("CSS")).unwrap();
    let [fixed, local] = autocomplete(&html, 9, Assoc::Before)
        .try_into()
        .ok()
        .unwrap();
    let [js_fixed, js_local] = autocomplete(&js, 0, Assoc::Before).try_into().ok().unwrap();
    let [style] = autocomplete(&html, 32, Assoc::Before)
        .try_into()
        .ok()
        .unwrap();
    let [css_source] = autocomplete(&css, 0, Assoc::Before)
        .try_into()
        .ok()
        .unwrap();
    let [schema] = autocomplete(&html, 1, Assoc::Before)
        .try_into()
        .ok()
        .unwrap();
    assert!(!Arc::ptr_eq(&fixed, &js_fixed));
    assert!(Arc::ptr_eq(&local, &js_local));
    assert!(Arc::ptr_eq(&style, &css_source));
    [
        ("html", schema),
        ("fixed", fixed),
        ("local", local),
        ("css", style),
        ("js-fixed", js_fixed),
    ]
}

fn id(
    ids: &[(&'static str, ScriptCompletionSource)],
    source: &ScriptCompletionSource,
) -> &'static str {
    ids.iter()
        .find(|(_, other)| Arc::ptr_eq(source, other))
        .unwrap()
        .0
}

fn summarize(source: &str, result: SourceResult) -> Value {
    let mut hash = 2166136261u32;
    for item in result.options.iter() {
        let apply = if matches!(source, "fixed" | "js-fixed")
            && javascript_completion_snippet(item).is_some()
        {
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
    }
    json!([
        result.from,
        result.to,
        result.filter,
        result.options.len(),
        hash
    ])
}

fn side_data(
    state: &EditorState,
    at: usize,
    side: Assoc,
    ids: &[(&'static str, ScriptCompletionSource)],
) -> Value {
    let sources: Vec<_> = autocomplete(state, at, side)
        .iter()
        .map(|src| id(ids, src))
        .collect();
    let comments: Vec<_> = state
        .language_data_at::<CommentTokens>("commentTokens", at, side)
        .iter()
        .map(|tokens| json!([tokens.line, tokens.block]))
        .collect();
    let brackets: Vec<_> = state
        .language_data_at::<CloseBracketData>("closeBrackets", at, side)
        .iter()
        .map(|d| json!([d.brackets, d.before, d.string_prefixes]))
        .collect();
    json!({"sources": sources, "comments": comments, "brackets": brackets,
        "words": state.language_data_at::<&str>("wordChars", at, side)})
}

fn change(from: usize, to: usize, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(from..to, insert)).unwrap()
}

#[test]
fn stock_html_state_and_callable_sources_replay() {
    let keys = rows().next().unwrap();
    let keys = || {
        keys["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key.as_str().unwrap())
    };
    let ids = source_ids();
    let mut slot = Compartment::new();
    let mut state = EditorState::new("");
    let mut retained = None;
    let mut differences = Vec::new();
    for row in rows().skip(1).filter(|row| row["kind"] != "input") {
        if row["kind"] == "start" {
            slot = Compartment::new();
            state = EditorState::with_extensions(
                row["source"].as_str().unwrap(),
                Extension::group([
                    slot.of(Extension::default()),
                    CssPropertySnapshot::from_style_keys(keys()).extension(),
                ]),
            )
            .unwrap();
            continue;
        }
        let label = format!("{} / {}", row["name"], row["step"]);
        let before = attached_language_snapshot(&state);
        if row["step"] == "edit host prefix" {
            retained = before.clone();
        }
        let mut tr = Transaction::default();
        if let Some(edit) = row["edit"].as_array() {
            tr = Transaction::new(change(
                byte(&edit[0]),
                byte(&edit[1]),
                edit[2].as_str().unwrap(),
            ));
        }
        if !row["select"].is_null() {
            let at = byte(&row["select"]);
            tr = tr.with_selection(EditorSelection::single(at, at));
        }
        if let Some(names) = row["extensions"].as_array() {
            tr = tr.with_effects([slot.reconfigure(Extension::group(
                names.iter().map(|name| extension(name.as_str().unwrap())),
            ))]);
        }
        state.update(tr).unwrap();
        let active = attached_language_snapshot(&state);
        let (root, language) = active.as_ref().map_or(("", None), |doc| {
            assert_eq!(doc.text(), state.text(), "{label}");
            match doc.syntax() {
                NativeSyntax::Html(doc) => (doc.root().kind().name(), Some("HTML")),
                NativeSyntax::Javascript(doc) => (doc.root().kind().name(), Some("JavaScript")),
                _ => panic!("{label}: unexpected active root"),
            }
        });
        assert_eq!(row["root"], root, "{label}");
        assert_eq!(row["active"], json!(language), "{label}");
        assert_eq!(row["source"], state.text().as_str(), "{label}");
        assert_eq!(row["head"], state.selection().main().head(), "{label}");
        assert_eq!(
            row["reused"],
            matches!((&before, &active), (Some(a), Some(b)) if Arc::ptr_eq(a,b)),
            "{label}"
        );
        for q in row["queries"].as_array().unwrap() {
            let at = byte(&q["at"]);
            let data = [Assoc::Before, Assoc::After].map(|side| side_data(&state, at, side, &ids));
            let results = [false, true].map(|explicit| {
                autocomplete(&state, at, Assoc::Before)
                    .iter()
                    .map(|f| f(&state, at, explicit).map(|result| summarize(id(&ids, f), result)))
                    .collect::<Vec<_>>()
            });
            for (key, actual) in [("data", json!(data)), ("results", json!(results))] {
                if q[key] != actual {
                    differences.push(format!(
                        "{label} {} byte {at}, {key}: native {actual}; pin {}",
                        q["label"], q[key]
                    ));
                }
            }
        }
    }
    assert!(
        retained
            .unwrap()
            .text()
            .as_str()
            .starts_with("<!-- 字😀 -->\r\n")
    );
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

static MARKER: LazyLock<StateEffectType<usize>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|at, changes| changes.map_pos(*at, Assoc::After, MapMode::Simple))
});

#[test]
fn stock_html_headless_transactions_replay() {
    for row in rows().filter(|row| row["kind"] == "input") {
        let name = row["name"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let mode = row["mode"].as_str().unwrap();
        let typed = row["typed"].as_str().unwrap();
        let selection = EditorSelection::new(
            row["carets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|at| EditorSelection::cursor(byte(at), None, None, None))
                .collect(),
            0,
        )
        .unwrap();
        let at = selection.main().head();
        let observations = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&observations);
        let listener = update_listener(move |update| {
            let steps = update.input_steps.iter().map(|step| {
                let tr = &step.transaction;
                let markers: Vec<_> = tr.effects().iter().filter_map(|fx| fx.get(&MARKER)).collect();
                let changes: Vec<_> = tr.changes().changes()
                    .map(|(range, text)| json!([range.start, range.end, text])).collect();
                let ranges: Vec<_> = step.state.selection().ranges().iter().map(|r| {
                    json!([r.anchor(), r.head(), match r.assoc() { Some(Assoc::Before) => -1,
                        Some(Assoc::After) => 1, None => 0 }])
                }).collect();
                json!({"changes": changes, "after": step.state.text().as_str(), "ranges": ranges,
                    "main": step.state.selection().main_index(), "event": tr.user_event(),
                    "scroll": tr.scroll_into_view(), "markers": markers})
            }).collect::<Vec<_>>();
            let markers: Vec<_> = update
                .effects
                .iter()
                .filter_map(|effect| effect.get(&MARKER))
                .collect();
            log.lock()
                .unwrap()
                .push(json!({"transactions": steps, "markers": markers}));
            Ok(())
        });
        let replacement = match mode {
            "rewrite" => Some((1, "rewritten>", 11)),
            "omit typed" => Some((at, "x", at + 1)),
            _ => None,
        };
        let filter = transaction_filter(move |_, tr| {
            let Some((from, insert, head)) = replacement.filter(|_| tr.is_user_event("input.type"))
            else {
                return Ok(tr);
            };
            Ok(Transaction::new(change(from, at, insert))
                .with_selection(EditorSelection::single(head, head))
                .with_user_event("input.type")
                .with_scroll_into_view())
        });
        let state = EditorState::create_from_text(
            Text::new(source),
            Some(selection),
            Extension::group([
                extension("HTML"),
                extension("HTML"),
                ALLOW_MULTIPLE_SELECTIONS.of(true),
                READ_ONLY.of(mode == "read only"),
                filter,
                listener,
            ]),
        )
        .unwrap();
        let mut view = EditorView::new(state);
        if mode == "composition" || view.state().read_only() {
            assert_eq!(row["transactions"], json!([]), "{name}");
            continue;
        }
        let builds = Cell::new(0);
        view.apply_headless_input(
            at..at,
            typed,
            |state| {
                builds.set(builds.get() + 1);
                let mut tr = state
                    .replace_selection_parsed(typed)?
                    .with_user_event("input.type")
                    .with_scroll_into_view();
                if mode == "marker" {
                    tr = tr.with_effects([MARKER.of(at + 1)]);
                }
                Ok(tr)
            },
            (),
        )
        .unwrap();
        assert_eq!(builds.get(), 1, "{name}");
        let actual = observations.lock().unwrap();
        assert_eq!(actual.len(), 1, "{name}");
        assert_eq!(actual[0]["transactions"], row["transactions"], "{name}");
        assert_eq!(actual[0]["markers"], row["markers"], "{name}");
    }
}
