use std::sync::{Arc, LazyLock, Mutex};

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState,
    Extension, MapMode, Precedence, READ_ONLY, StateEffectType, Text, Transaction,
    autocomplete::session::SourceResult,
    commands::CommentTokens,
    language::{css::CssPropertySnapshot, javascript::javascript_completion_snippet},
    language_data::{
        CloseBracketData, NativeSyntax, ScriptCompletionSource, attached_language_snapshot,
    },
    transaction_filter,
    view::{EditorView, input::InputStep, update_listener},
};
use serde_json::{Value, json};

use super::byte;

fn rows() -> impl Iterator<Item = Value> {
    include_str!("../fixtures/language_data_vue_angular.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn bundle(key: &str) -> Extension {
    if let Some(key) = key.strip_prefix("high ") {
        return bundle(key).with_precedence(Precedence::High);
    }
    super::bundle(match key {
        "V" => "Vue",
        "A" => "Angular Template",
        "H" => "HTML",
        "J" => "JavaScript",
        "C" => "CSS",
        _ => panic!("unknown stock bundle {key}"),
    })
}

fn sources(state: &EditorState, at: usize, side: Assoc) -> Vec<ScriptCompletionSource> {
    state.language_data_at("autocomplete", at, side)
}

fn source_ids() -> Vec<(String, ScriptCompletionSource)> {
    let only = |state: &EditorState, at| {
        let [source] = sources(state, at, Assoc::Before).try_into().ok().unwrap();
        source
    };
    let js = EditorState::with_extensions("", bundle("J")).unwrap();
    let css = EditorState::with_extensions("", bundle("C")).unwrap();
    let [fixed, local] = sources(&js, 0, Assoc::Before).try_into().ok().unwrap();
    let css = only(&css, 0);
    let mut ids: Vec<(String, _)> = vec![
        ("J-fixed".into(), fixed),
        ("local".into(), local.clone()),
        ("css".into(), css.clone()),
    ];
    for key in ["H", "V", "A"] {
        let source = "<script>let x=0;</script><style>a{}</style>";
        let state = EditorState::with_extensions(source, bundle(key)).unwrap();
        let [fixed, mounted_local] = sources(&state, 9, Assoc::Before).try_into().ok().unwrap();
        assert!(Arc::ptr_eq(&mounted_local, &local));
        assert!(Arc::ptr_eq(&only(&state, 32), &css));
        for (role, source) in [("schema", only(&state, 1)), ("fixed", fixed)] {
            assert!(ids.iter().all(|(_, prior)| !Arc::ptr_eq(prior, &source)));
            ids.push((format!("{key}-{role}"), source));
        }
    }
    ids
}

fn summarize(id: &str, result: SourceResult) -> Value {
    let mut hash = 2166136261u32;
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
    }
    json!([
        result.from,
        result.to,
        result.filter,
        result.options.len(),
        hash
    ])
}

fn replay_queries(
    state: &EditorState,
    row: &Value,
    ids: &[(String, ScriptCompletionSource)],
    triggers: &[Value],
    differences: &mut Vec<String>,
) {
    for query in row["queries"].as_array().unwrap() {
        let at = byte(&query["at"]);
        for (index, side) in [Assoc::Before, Assoc::After].into_iter().enumerate() {
            let pin = &query["sides"][index];
            let files = sources(state, at, side);
            let names: Vec<_> = files
                .iter()
                .map(|file| {
                    ids.iter()
                        .find(|(_, other)| Arc::ptr_eq(file, other))
                        .unwrap()
                        .0
                        .as_str()
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
            let indent: Vec<Vec<_>> = state
                .language_data_at::<fn(&str) -> bool>("indentOnInput", at, side)
                .iter()
                .map(|rule| triggers.iter().map(|t| rule(t.as_str().unwrap())).collect())
                .collect();
            let results = [false, true].map(|explicit| {
                files
                    .iter()
                    .zip(&names)
                    .map(|(file, id)| file(state, at, explicit).map(|r| summarize(id, r)))
                    .collect::<Vec<_>>()
            });
            let label = format!(
                "{} / {} / {} byte {at} {side:?} ({})",
                row["name"],
                row["step"].as_str().unwrap_or("retained"),
                query["label"],
                pin["top"]
            );
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
                assert_eq!(actual, pin[key], "{label}: {key}");
            }
            if json!(results) != pin["results"] {
                differences.push(format!(
                    "{label} / left {}: native {}; pin {}",
                    query["left"],
                    json!(results),
                    pin["results"]
                ));
            }
        }
    }
}

fn changes(edits: &Value) -> ChangeSet {
    ChangeSet::new(
        edits
            .as_array()
            .unwrap()
            .iter()
            .map(|edit| Change::new(byte(&edit[0])..byte(&edit[1]), edit[2].as_str().unwrap()))
            .collect(),
    )
    .unwrap()
}

#[test]
fn stock_templates_replay_configured_data_and_sources() {
    let ids = source_ids();
    let identity = rows().next().unwrap();
    let mut slot = Compartment::new();
    let mut state = EditorState::new("");
    let mut retained = None;
    let mut differences = Vec::new();
    for row in rows().skip(1) {
        match row["kind"].as_str().unwrap() {
            "start" => {
                slot = Compartment::new();
                let keys = identity["keys"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|key| key.as_str().unwrap());
                state = EditorState::with_extensions(
                    row["source"].as_str().unwrap(),
                    Extension::group([
                        slot.of(Extension::default()),
                        CssPropertySnapshot::from_style_keys(keys).extension(),
                    ]),
                )
                .unwrap();
                assert!(retained.is_none());
                continue;
            }
            // Native catalog entries expose the bundle, not a bare template extension.
            "state" if row["step"] == "bare" => continue,
            "state" => {}
            "retained" => {
                let old: EditorState = retained.take().unwrap();
                assert_eq!(row["source"], old.text().as_str());
                replay_queries(
                    &old,
                    &row,
                    &ids,
                    identity["triggers"].as_array().unwrap(),
                    &mut differences,
                );
                continue;
            }
            "input" => continue,
            kind => panic!("unexpected row {kind}"),
        }
        let label = format!("{} / {}", row["name"], row["step"]);
        let before = attached_language_snapshot(&state);
        let head = byte(&row["head"]);
        let mut tr = Transaction::new(changes(&row["edits"]));
        if row["step"] == "selection only" {
            tr = tr.with_selection(EditorSelection::single(head, head));
        }
        if let Some(names) = row["exts"].as_array() {
            tr = tr.with_effects([slot.reconfigure(Extension::group(
                names.iter().map(|name| bundle(name.as_str().unwrap())),
            ))]);
        }
        state.update(tr).unwrap();
        let active = attached_language_snapshot(&state);
        let (key, root) = active.as_ref().map_or((None, ""), |doc| {
            assert_eq!(doc.text(), state.text(), "{label}");
            let (key, root) = match doc.syntax() {
                NativeSyntax::Vue(doc) => ("V", doc.host().root().kind().name()),
                NativeSyntax::Angular(doc) => ("A", doc.html().root().kind().name()),
                NativeSyntax::Html(doc) => ("H", doc.root().kind().name()),
                NativeSyntax::Javascript(doc) => ("J", doc.root().kind().name()),
                _ => panic!("unexpected root: {label}"),
            };
            (Some(key), root)
        });
        assert_eq!(row["active"], json!(key), "{label}");
        assert_eq!(row["root"], root, "{label}");
        assert_eq!(row["source"], state.text().as_str(), "{label}");
        assert_eq!(state.selection().main().head(), head, "{label}");
        // The oracle reuses the bare root when catalog support is added.
        // The native replay starts with the catalog installation instead.
        if row["step"] != "catalog" || row["name"] != "Vue text & directives" {
            assert_eq!(
                row["reused"],
                matches!((&before, &active), (Some(a), Some(b)) if Arc::ptr_eq(a, b)),
                "{label}"
            );
        }
        if row["step"] == "catalog" {
            retained = Some(state.clone());
        }
        replay_queries(
            &state,
            &row,
            &ids,
            identity["triggers"].as_array().unwrap(),
            &mut differences,
        );
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

static MARKER: LazyLock<StateEffectType<usize>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|at, changes| changes.map_pos(*at, Assoc::After, MapMode::Simple))
});

fn input_step(step: &InputStep) -> Value {
    let tr = &step.transaction;
    let edits: Vec<_> = tr
        .changes()
        .changes()
        .map(|(range, text)| json!([range.start, range.end, text]))
        .collect();
    let ranges: Vec<_> = step
        .state
        .selection()
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
        .collect();
    let markers: Vec<_> = tr
        .effects()
        .iter()
        .filter_map(|fx| fx.get(&MARKER))
        .collect();
    json!({"edits": edits, "after": step.state.text().as_str(), "ranges": ranges,
        "event": tr.user_event(), "scroll": tr.scroll_into_view(), "markers": markers})
}

#[test]
fn stock_templates_replay_accepted_headless_input() {
    for row in rows().filter(|row| row["kind"] == "input") {
        let key = row["key"].as_str().unwrap();
        let name = row["name"].as_str().unwrap();
        let mode = row["mode"].as_str().unwrap();
        let typed = row["typed"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let selection = EditorSelection::new(
            row["carets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| EditorSelection::cursor(byte(p), None, None, None))
                .collect(),
            0,
        )
        .unwrap();
        let at = selection.main().head();
        let replacement = match mode {
            "rewrite" => Some((0, "<rewritten>", 11)),
            "omit" => Some((at, "x", at + 1)),
            _ => None,
        };
        let filter = transaction_filter(move |_, tr| {
            let Some((from, insert, head)) = replacement.filter(|_| tr.is_user_event("input.type"))
            else {
                return Ok(tr);
            };
            Ok(
                Transaction::new(ChangeSet::single(Change::new(from..at, insert))?)
                    .with_selection(EditorSelection::single(head, head))
                    .with_user_event("input.type")
                    .with_scroll_into_view(),
            )
        });
        let observations = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&observations);
        let listener = update_listener(move |update| {
            observed.lock().unwrap().push(
                update
                    .input_steps
                    .iter()
                    .map(input_step)
                    .collect::<Vec<_>>(),
            );
            Ok(())
        });
        let state = EditorState::create_from_text(
            Text::new(source),
            Some(selection),
            Extension::group([
                bundle(key),
                bundle(key),
                bundle(if key == "V" { "A" } else { "V" }),
                bundle("H"),
                ALLOW_MULTIPLE_SELECTIONS.of(true),
                READ_ONLY.of(mode == "readonly"),
                filter,
                listener,
            ]),
        )
        .unwrap();
        let mut view = EditorView::new(state);
        // The host gates read-only and composing input before headless dispatch.
        if view.state().read_only() || mode == "composition" {
            assert_eq!(row["transactions"], json!([]), "{key} {name}");
            continue;
        }
        let mut builds = 0;
        view.apply_headless_input(
            at..at,
            typed,
            |state| {
                builds += 1;
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
        assert_eq!(builds, 1, "{key} {name}");
        assert_eq!(
            json!(*observations.lock().unwrap()),
            json!([row["transactions"]]),
            "{key} {name}"
        );
    }
}
