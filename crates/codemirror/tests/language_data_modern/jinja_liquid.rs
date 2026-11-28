//! Replay the stock catalog installers. The oracle also records upstream-only
//! factory shapes, merged syntax paths, and browser input offers.

use std::{
    ops::Range,
    sync::{Arc, LazyLock, Mutex},
};

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Assoc, Change, ChangePermission, ChangeSet, Compartment,
    EditorSelection, EditorState, Extension, MapMode, READ_ONLY, StateEffectType, Text,
    Transaction, TransactionExtension,
    autocomplete::session::SourceResult,
    change_filter,
    commands::CommentTokens,
    language::{
        css::CssPropertySnapshot,
        html::{HtmlDocument, HtmlEmbeddedLanguage},
    },
    language_data::{
        CloseBracketData, LANGUAGES, LanguageDescription, NativeLanguage, NativeSyntax,
        ScriptCompletionSource, attached_language_snapshot,
    },
    transaction_extender, transaction_filter,
    view::{EditorView, input::InputStep, update_listener},
};
use serde_json::{Value, json};

use super::{bundle, byte, load};

type Sources = Vec<(String, ScriptCompletionSource)>;

fn rows() -> impl Iterator<Item = Value> {
    include_str!("../fixtures/language_data_jinja_liquid.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn array(value: &Value) -> &[Value] {
    value.as_array().unwrap()
}

fn template(key: &str) -> (NativeLanguage, &'static str) {
    match key {
        "jinja" => (NativeLanguage::Jinja, "Jinja"),
        "liquid" => (NativeLanguage::Liquid, "Liquid"),
        _ => unreachable!("unknown template {key}"),
    }
}

fn root(syntax: &NativeSyntax) -> (&'static str, &str) {
    match syntax {
        NativeSyntax::Jinja(doc) => ("jinja", doc.root().name()),
        NativeSyntax::Liquid(doc) => ("liquid", doc.root().kind().name()),
        _ => unreachable!("expected template syntax"),
    }
}

fn sources(state: &EditorState, at: usize, side: Assoc) -> Vec<ScriptCompletionSource> {
    state.language_data_at("autocomplete", at, side)
}

fn source_ids() -> Sources {
    let at = |state: &EditorState, at| sources(state, at, Assoc::Before);
    let js = EditorState::with_extensions("", bundle("JavaScript")).unwrap();
    let css_state = EditorState::with_extensions("", bundle("CSS")).unwrap();
    let [fixed, local] = at(&js, 0).try_into().ok().unwrap();
    let [css] = at(&css_state, 0).try_into().ok().unwrap();
    let mut ids = vec![
        ("standalone:fixed-js".into(), fixed),
        ("local-js".into(), Arc::clone(&local)),
        ("css".into(), Arc::clone(&css)),
    ];
    for (key, name) in [("html", "HTML"), ("jinja", "Jinja"), ("liquid", "Liquid")] {
        let source = "<script>let x=0;</script><style>a{}</style>";
        let state = EditorState::with_extensions(source, bundle(name)).unwrap();
        let [schema] = at(&state, 1).try_into().ok().unwrap();
        let [fixed, mounted_local] = at(&state, 9).try_into().ok().unwrap();
        let [mounted_css] = at(&state, 32).try_into().ok().unwrap();
        assert!(Arc::ptr_eq(&mounted_local, &local), "{key}: local callable");
        assert!(Arc::ptr_eq(&mounted_css, &css), "{key}: CSS callable");
        let mut owned = vec![("schema", schema), ("fixed-js", fixed)];
        if key != "html" {
            let support = bundle(name);
            let state = EditorState::with_extensions(
                "{{ value }}",
                Extension::group([support.clone(), support]),
            )
            .unwrap();
            let [source] = at(&state, 5).try_into().ok().unwrap();
            owned.push(("template", source));
        }
        for (role, source) in owned {
            let id = format!("{key}:{role}");
            assert!(
                ids.iter().all(|(_, prior)| !Arc::ptr_eq(prior, &source)),
                "{id}"
            );
            ids.push((id, source));
        }
    }
    assert_eq!(ids.len(), 11);
    ids
}

fn projected_regions(
    html: &HtmlDocument,
    map: impl Fn(Range<usize>) -> Vec<Range<usize>>,
) -> Value {
    use HtmlEmbeddedLanguage::*;
    let ranges = |range| {
        map(range)
            .into_iter()
            .map(|r| [r.start, r.end])
            .collect::<Vec<_>>()
    };
    let children: Vec<_> = html
        .mounts()
        .iter()
        .map(|mount| {
            let parser = match mount.language() {
                Css => "css",
                CssStyles => "configured:Styles",
                JavaScript => "js",
                TypeScript => "ts",
                Jsx => "jsx",
                Json => "configured:SingleExpression",
                Tsx | Custom(_) => unreachable!("unexpected stock mount"),
            };
            json!({"parser": parser, "ranges": ranges(mount.range())})
        })
        .collect();
    json!({"html": ranges(0..html.text().len()), "children": children})
}

fn regions(syntax: &NativeSyntax) -> Value {
    match syntax {
        NativeSyntax::Jinja(doc) => doc.html().map_or(Value::Null, |html| {
            projected_regions(html.document(), |range| html.source_ranges(range).unwrap())
        }),
        NativeSyntax::Liquid(doc) => doc.html().map_or(Value::Null, |html| {
            projected_regions(html.document(), |range| {
                html.source_ranges(range).unwrap().collect()
            })
        }),
        _ => unreachable!("expected template syntax"),
    }
}

fn check_regions(syntax: &NativeSyntax, row: &Value) {
    let pin = &row["regions"];
    let expected = if pin.is_null() {
        Value::Null
    } else {
        let children: Vec<_> = array(&pin["children"])
            .iter()
            .map(|child| json!({"parser": child["parser"], "ranges": child["ranges"]}))
            .collect();
        json!({"html": pin["html"], "children": children})
    };
    assert_eq!(
        regions(syntax),
        expected,
        "{} / {}",
        row["key"],
        row["name"]
    );
}

fn data<T: Clone + 'static>(
    state: &EditorState,
    at: usize,
    side: Assoc,
    name: &str,
    map: impl Fn(T) -> Value,
) -> Value {
    Value::Array(
        state
            .language_data_at(name, at, side)
            .into_iter()
            .map(map)
            .collect(),
    )
}

fn check_result(
    result: Option<SourceResult>,
    pin: &Value,
    row: &Value,
    query: &Value,
    id: &str,
    label: &str,
) {
    let at = byte(&query["at"]);
    if pin["crossesHtmlGap"] == true {
        let from = byte(&pin["from"]);
        let to = pin["to"].as_u64().map_or(at, |_| byte(&pin["to"]));
        assert!(array(&row["regions"]["html"]).windows(2).any(|pair| {
            from < byte(&pair[0][1])
                && byte(&pair[0][1]) < byte(&pair[1][0])
                && byte(&pair[1][0]) < to
        }));
        assert!(
            result.is_none(),
            "{label}: replacement crosses a template gap"
        );
        return;
    }
    let compact_word = match (
        id.ends_with(":fixed-js"),
        text(&row["name"]),
        text(&query["name"]),
    ) {
        (true, "catalog mixed", "JS after gap") => Some((78..80, 89..92, "local")),
        (true, "replacement spans a template gap", "site 2") => Some((48..51, 58..61, "prefix")),
        _ => None,
    };
    if let Some((left, right, word)) = compact_word {
        // Upstream scans the original suffix. The native source scans the
        // compact identifier and cannot return that range safely.
        let source = text(&row["source"]);
        assert_eq!(pin["from"], right.start, "{label}");
        assert_eq!(at, right.end, "{label}");
        assert!(
            array(&row["regions"]["html"]).windows(2).any(|pair| {
                byte(&pair[0][0]) <= left.start
                    && left.end == byte(&pair[0][1])
                    && right.start == byte(&pair[1][0])
                    && right.end <= byte(&pair[1][1])
            }),
            "{label}"
        );
        assert_eq!(
            format!("{}{}", &source[left], &source[right]),
            word,
            "{label}"
        );
        assert!(
            result.is_none(),
            "{label}: compact identifier crosses a template gap"
        );
        return;
    }
    let Some(result) = result else {
        assert!(pin.is_null(), "{label}: missing result; expected {pin}");
        return;
    };
    assert!(
        !pin.is_null(),
        "{label}: unexpected result at {}",
        result.from
    );
    let to = result.to.unwrap_or(at);
    assert!(
        row["source"]
            .as_str()
            .unwrap()
            .get(result.from..to)
            .is_some(),
        "{label}: invalid byte range"
    );
    assert_eq!(
        json!([result.from, result.to, result.filter]),
        json!([pin["from"], pin["to"], pin["filter"]]),
        "{label}"
    );
    if id.ends_with(":template") {
        let options: Vec<_> = result
            .options
            .iter()
            .map(|item| (&item.label, &item.kind))
            .collect();
        assert_eq!(json!(options), pin["templateOptions"], "{label}");
    }
    let options: Vec<_> = result
        .options
        .iter()
        .map(|item| item.label.as_str())
        .collect();
    let head = &options[..options.len().min(3)];
    let tail = &options[options.len().saturating_sub(2)..];
    if row["name"] == "split closing name, incomplete HTML" && id.ends_with(":schema") {
        // Keep the intact ancestor and the other schema options, not a copy
        // of template markup in the name of the inner element.
        assert!(result.options.iter().all(|item| {
            !item.label.contains("{{") && !item.insert.as_deref().is_some_and(|s| s.contains("{{"))
        }));
        if query["name"] == "site 2" {
            assert_eq!(pin["tail"], json!(["</d{{ gap }}iv>", "</div>"]));
            assert_eq!(options.len() + 1, byte(&pin["count"]), "{label}");
            assert_eq!(json!(head), pin["head"], "{label}");
            assert_eq!(tail, ["<wbr", "</div>"], "{label}");
        } else {
            assert_eq!(pin["head"], json!(["d{{ gap }}iv", "div"]));
            assert_eq!(options, ["div"], "{label}");
        }
        return;
    }
    // The fixture hash includes upstream JS functions and object fields. This
    // replay compares the native range, filtering, count, and ordered edge labels.
    assert_eq!(
        json!([options.len(), head, tail]),
        json!([pin["count"], pin["head"], pin["tail"]]),
        "{label}"
    );
}

fn check_queries(
    state: &EditorState,
    row: &Value,
    ids: &Sources,
    triggers: &[Value],
    providers: usize,
) {
    for query in array(&row["queries"]) {
        let at = byte(&query["at"]);
        for (i, side) in [Assoc::Before, Assoc::After].into_iter().enumerate() {
            let pin = &query["sides"][i];
            let label = format!(
                "{} / {} / {} / {side:?} byte {at}",
                row["key"], row["name"], query["name"]
            );
            let files = sources(state, at, side);
            let names: Vec<_> = files
                .iter()
                .map(|source| {
                    ids.iter()
                        .find(|(_, known)| Arc::ptr_eq(source, known))
                        .unwrap()
                        .0
                        .as_str()
                })
                .collect();
            let want = &pin["data"];
            let brackets = data(state, at, side, "closeBrackets", |b: CloseBracketData| {
                assert!(b.before.is_none() && b.string_prefixes.is_empty());
                json!({"brackets": b.brackets})
            });
            let comments = data(state, at, side, "commentTokens", |c: CommentTokens| {
                let mut value = json!({});
                if let Some(line) = c.line {
                    value["line"] = json!(line);
                }
                if let Some((open, close)) = c.block {
                    value["block"] = json!({"open": open, "close": close});
                }
                value
            });
            let indent = data(
                state,
                at,
                side,
                "indentOnInput",
                |rule: fn(&str) -> bool| {
                    json!(triggers.iter().map(|t| rule(text(t))).collect::<Vec<_>>())
                },
            );
            let pin_indent: Vec<_> = array(&want["indentOnInput"])
                .iter()
                .map(|v| v["matches"].clone())
                .collect();
            let mut pin_brackets = want["closeBrackets"].clone();
            if row["name"] == "reattached with bracket precedence" && pin["facet"] == "html" {
                // The stock integration API does not expose the HTML facet
                // for adding the oracle's two extra inputs.
                assert_eq!(
                    pin_brackets,
                    json!([{"brackets": ["["]}, {"brackets": ["{"]}, {"brackets": ["("]}])
                );
                pin_brackets = json!([pin_brackets[1]]);
            }
            for (key, actual, expected) in [
                ("autocomplete", json!(names), want["autocomplete"].clone()),
                ("closeBrackets", brackets, pin_brackets),
                ("commentTokens", comments, want["commentTokens"].clone()),
                (
                    "wordChars",
                    json!(state.language_data_at::<&str>("wordChars", at, side)),
                    want["wordChars"].clone(),
                ),
                ("indentOnInput", indent, json!(pin_indent)),
            ] {
                let repeated: Vec<_> = (0..providers)
                    .flat_map(|_| array(&expected).iter())
                    .collect();
                assert_eq!(actual, json!(repeated), "{label}: {key}");
            }
            for (explicit, expected) in [false, true].into_iter().zip(array(&pin["results"])) {
                let expected = array(expected);
                assert_eq!(files.len(), expected.len() * providers, "{label}");
                for ((file, id), result) in files.iter().zip(&names).zip(expected.iter().cycle()) {
                    check_result(
                        file(state, at, explicit),
                        result,
                        row,
                        query,
                        id,
                        &format!("{label}: {id}, explicit={explicit}"),
                    );
                }
            }
        }
    }
}

fn native_unfinished_interpolation(row: &Value) -> Value {
    // At this exact malformed EOF, upstream exposes the unterminated string
    // tail as HTML on the left, and offers tag completions on the right.
    // Native recovery has no HTML overlay and offers expression completions
    // from the same word on both sides. Use the independently pinned stock
    // template expression result to check the complete native option list.
    assert_eq!(row["source"], "{{ \"<em");
    assert_eq!(row["regions"], json!({"html": [[3, 7]], "children": []}));
    assert_eq!(array(&row["queries"]).len(), 1);
    let query = &row["queries"][0];
    assert_eq!(query["at"], 7);
    assert_eq!(
        query["paths"],
        json!(["Template/Document/Element/OpenTag/TagName", "Template"])
    );
    let left = &query["sides"][0];
    let right = &query["sides"][1];
    assert_eq!(left["facet"], "html");
    assert_eq!(right["facet"], row["key"]);
    for explicit in 0..2 {
        let html = &left["results"][explicit][0];
        let tags = &right["results"][explicit][0];
        assert_eq!(html["from"], 5);
        assert_eq!(html["count"], 112);
        assert_eq!(tags["from"], 5);
        assert_eq!(tags["count"], if row["key"] == "jinja" { 32 } else { 28 });
    }

    let stock = rows()
        .find(|other| other["key"] == row["key"] && other["name"] == "stock template sources")
        .unwrap();
    let variable = &stock["queries"][1];
    assert_eq!(variable["at"], 16);
    let mut template = right.clone();
    template["results"] = variable["sides"][0]["results"].clone();
    for explicit in 0..2 {
        let expression = &mut template["results"][explicit][0];
        assert_eq!(expression["from"], 14);
        expression["from"] = json!(5);
    }
    let mut native = row.clone();
    native["queries"][0]["sides"] = json!([template, template]);
    native
}

fn start(source: &str, extension: Extension, capture: &Value) -> EditorState {
    let keys = array(&capture["cssKeys"]).iter().map(text);
    EditorState::with_extensions(
        source,
        Extension::group([
            extension,
            CssPropertySnapshot::from_style_keys(keys).extension(),
        ]),
    )
    .unwrap()
}

#[test]
fn stock_jinja_liquid_replay_catalog_data() {
    let ids = source_ids();
    let capture = rows().next().unwrap();
    let triggers = array(&capture["triggers"]);
    for key in ["jinja", "liquid"] {
        let (language, name) = template(key);
        let entry = LanguageDescription::match_name(LANGUAGES, name, false).unwrap();
        let identity = rows()
            .find(|r| r["kind"] == "identity" && r["key"] == key)
            .unwrap();
        assert_eq!(
            json!({"name": entry.name, "aliases": entry.aliases, "extensions": entry.extensions}),
            identity["catalog"]
        );
        let support = load(entry);
        assert_eq!(support.language(), language);
        assert!(Arc::ptr_eq(&support, &load(entry)));
        assert!(language.support().extension().is_none());
        let stock = support.extension().unwrap();
        let slot = Compartment::new();
        let mut current: Option<EditorState> = None;
        let mut retained = None;
        for row in rows().filter(|row| row["key"] == key && row["kind"] == "state") {
            let source = text(&row["source"]);
            let name = text(&row["name"]);
            let (state, providers) = match name {
                "catalog mixed" => {
                    let mut state = start(source, slot.of(stock.clone()), &capture);
                    let old = attached_language_snapshot(&state).unwrap();
                    let caret = array(&row["queries"])
                        .iter()
                        .find(|query| query["name"] == "inside CSS gap")
                        .map(|query| byte(&query["at"]))
                        .unwrap();
                    state
                        .update(
                            Transaction::default()
                                .with_selection(EditorSelection::single(caret, caret))
                                .with_effects([slot.reconfigure(Extension::group([
                                    stock.clone(),
                                    stock.clone(),
                                ]))]),
                        )
                        .unwrap();
                    assert!(Arc::ptr_eq(
                        &old,
                        &attached_language_snapshot(&state).unwrap()
                    ));
                    // Reject a byte inside the leading scalar and a byte
                    // between CR and LF, including direct callable probes.
                    assert!(source.starts_with("😀\r\n"));
                    for at in [1, 5] {
                        for side in [Assoc::Before, Assoc::After] {
                            assert!(
                                sources(&state, at, side).is_empty(),
                                "{key}: byte {at} {side:?}"
                            );
                        }
                        for (id, source) in &ids {
                            assert!(
                                source(&state, at, true).is_none(),
                                "{key}: {id} at byte {at}"
                            );
                        }
                    }
                    retained = Some((state.clone(), old, row.clone()));
                    (state, 1)
                }
                "combined support only" => {
                    // Full bundles add separate active-data providers. Each
                    // provider reads the same ordered facet inputs from the
                    // selected parser, which stays the first template.
                    let mut state = current.take().unwrap();
                    let before = attached_language_snapshot(&state).unwrap();
                    let (_, other) = template(if key == "jinja" { "liquid" } else { "jinja" });
                    state
                        .update(Transaction::default().with_effects([slot.reconfigure(
                            Extension::group([
                                stock.clone(),
                                bundle(other),
                                bundle("HTML"),
                                bundle("JavaScript"),
                                bundle("CSS"),
                            ]),
                        )]))
                        .unwrap();
                    assert!(Arc::ptr_eq(
                        &before,
                        &attached_language_snapshot(&state).unwrap()
                    ));
                    (state, 5)
                }
                "MIME edit and reconfigure" => {
                    let mut state = current.take().unwrap();
                    let changes = ChangeSet::new(
                        array(&row["edits"])
                            .iter()
                            .map(|edit| Change::new(byte(&edit[0])..byte(&edit[1]), text(&edit[2])))
                            .collect(),
                    )
                    .unwrap();
                    state
                        .update(
                            Transaction::new(changes)
                                .with_effects([slot.reconfigure(stock.clone())]),
                        )
                        .unwrap();
                    let (old, snapshot, initial) = retained.take().unwrap();
                    assert_eq!(row["retainedTree"], true);
                    assert!(Arc::ptr_eq(
                        &snapshot,
                        &attached_language_snapshot(&old).unwrap()
                    ));
                    assert_eq!(old.text().as_str(), initial["source"]);
                    assert!(!Arc::ptr_eq(
                        &snapshot,
                        &attached_language_snapshot(&state).unwrap()
                    ));
                    check_regions(snapshot.syntax(), &initial);
                    check_queries(&old, &initial, &ids, triggers, 1);
                    (state, 1)
                }
                "bare root" => {
                    // No public bare tracked installer for these templates.
                    // Check direct parser output, not the upstream bare facets.
                    let syntax = language.parse(Text::new(source)).unwrap();
                    assert_eq!(json!(root(&syntax)), json!([row["active"], row["root"]]));
                    check_regions(&syntax, &row);
                    continue;
                }
                "detached" | "reattached with bracket precedence" => {
                    let mut state = current.take().unwrap();
                    let extension = if name == "detached" {
                        Extension::default()
                    } else {
                        stock.clone()
                    };
                    state
                        .update(Transaction::default().with_effects([slot.reconfigure(extension)]))
                        .unwrap();
                    (state, 1)
                }
                _ => (start(source, stock.clone(), &capture), 1),
            };
            assert_eq!(state.text().as_str(), source, "{key} / {name}");
            if let Some(active) = attached_language_snapshot(&state) {
                assert_eq!(
                    json!(root(active.syntax())),
                    json!([row["active"], row["root"]])
                );
                if name == "unfinished interpolation versus closed" {
                    // Do not infer an editable HTML route from text which the
                    // native recovery keeps inside the unfinished template.
                    assert!(regions(active.syntax()).is_null());
                    check_queries(
                        &state,
                        &native_unfinished_interpolation(&row),
                        &ids,
                        triggers,
                        providers,
                    );
                } else {
                    check_regions(active.syntax(), &row);
                    check_queries(&state, &row, &ids, triggers, providers);
                }
            } else {
                assert_eq!(name, "detached");
                assert!(row["active"].is_null());
                check_queries(&state, &row, &ids, triggers, providers);
                for (id, source) in &ids {
                    assert!(source(&state, 25, true).is_none(), "detached: {id}");
                }
            }
            current = Some(state);
        }
    }
}

static MARKER: LazyLock<StateEffectType<usize>> = LazyLock::new(|| {
    StateEffectType::with_mapping(|at, changes| changes.map_pos(*at, Assoc::After, MapMode::Simple))
});

fn assoc(value: &Value) -> Option<Assoc> {
    match value.as_i64().unwrap() {
        -1 => Some(Assoc::Before),
        0 => None,
        1 => Some(Assoc::After),
        n => unreachable!("association {n}"),
    }
}

fn selection_json(selection: &EditorSelection) -> Value {
    let ranges: Vec<_> = selection
        .ranges()
        .iter()
        .map(|r| {
            let side = match r.assoc() {
                Some(Assoc::Before) => -1,
                None => 0,
                Some(Assoc::After) => 1,
            };
            json!([r.anchor(), r.head(), side])
        })
        .collect();
    json!({"main": selection.main_index(), "ranges": ranges})
}

fn input_step(step: &InputStep) -> Value {
    let tr = &step.transaction;
    let active = attached_language_snapshot(&step.state);
    let edits: Vec<_> = tr
        .changes()
        .changes()
        .map(|(range, insert)| json!([range.start, range.end, insert]))
        .collect();
    let effects: Vec<_> = tr
        .effects()
        .iter()
        .map(|effect| {
            if let Some(at) = effect.get(&MARKER) {
                json!(["marker", at])
            } else {
                assert!(active.is_none(), "expected the compartment to detach");
                json!(["reconfigure", []])
            }
        })
        .collect();
    json!({"edits": edits, "source": step.state.text().as_str(), "selection": selection_json(step.state.selection()),
        "scroll": tr.scroll_into_view(), "event": tr.user_event(), "root": active.as_ref().map(|a| root(a.syntax()).0), "effects": effects})
}

#[test]
fn stock_jinja_liquid_replay_headless_input() {
    for row in rows().filter(|row| row["kind"] == "input") {
        let key = text(&row["key"]);
        let (_, catalog_name) = template(key);
        let name = text(&row["name"]);
        let mode = text(&row["mode"]);
        let typed = text(&row["typed"]);
        let label = format!("{key} / {name}");
        if mode == "compose" {
            // The headless API does not track browser composition. The GPUI
            // host does not offer composing input to the direct typing path.
            assert_eq!(array(&row["batches"]).len(), 1, "{label}");
            continue;
        }
        let selection = EditorSelection::new(
            array(&row["selection"]["ranges"])
                .iter()
                .map(|r| EditorSelection::range(byte(&r[0]), byte(&r[1]), None, None, assoc(&r[2])))
                .collect(),
            byte(&row["selection"]["main"]),
        )
        .unwrap();
        assert_eq!(selection_json(&selection), row["selection"], "{label}");
        let range = byte(&row["reported"][0])..byte(&row["reported"][1]);
        let slot = Compartment::new();
        let installed = if row["bare"] == true {
            Extension::default()
        } else {
            bundle(catalog_name)
        };
        let mut extensions = vec![
            slot.of(installed),
            ALLOW_MULTIPLE_SELECTIONS.of(true),
            READ_ONLY.of(mode == "readonly"),
        ];
        match mode {
            "protect" => extensions.push(change_filter(|_, tr| {
                if tr.is_user_event("input.type")
                    && tr
                        .changes()
                        .changes()
                        .any(|(_, insert)| insert.contains("%%"))
                {
                    ChangePermission::Reject
                } else {
                    ChangePermission::Allow
                }
            })),
            "protect-close" => extensions.push(change_filter(|_, tr| {
                if tr.is_user_event("input.complete") {
                    ChangePermission::Reject
                } else {
                    ChangePermission::Allow
                }
            })),
            "rewrite" | "omit" => {
                let main = selection.main().range();
                let (from, insert) = if mode == "rewrite" {
                    (0, "<rewritten>")
                } else {
                    (main.start, "x")
                };
                let to = main.end;
                let head = from + insert.len();
                extensions.push(transaction_filter(move |_, tr| {
                    if !tr.is_user_event("input.type") {
                        return Ok(tr);
                    }
                    Ok(
                        Transaction::new(ChangeSet::single(Change::new(from..to, insert))?)
                            .with_selection(EditorSelection::single(head, head))
                            .with_user_event("input.type")
                            .with_scroll_into_view(),
                    )
                }));
            }
            "effect" | "detach" => {
                let detach = mode == "detach";
                extensions.push(transaction_extender(move |start, tr| {
                    let mut extension = TransactionExtension::default();
                    if tr.is_user_event("input.type") {
                        extension.effects.push(if detach {
                            slot.reconfigure(Extension::default())
                        } else {
                            MARKER.of(tr.changes().desc(start.text().len()).unwrap().new_length())
                        });
                    }
                    extension
                }));
            }
            "normal" | "reported" | "readonly" => {}
            _ => unreachable!("unknown input mode {mode}"),
        }
        let observed = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&observed);
        extensions.push(update_listener(move |update| {
            log.lock().unwrap().push(
                update
                    .input_steps
                    .iter()
                    .map(input_step)
                    .collect::<Vec<_>>(),
            );
            Ok(())
        }));
        let state = EditorState::create_from_text(
            Text::new(text(&row["source"])),
            Some(selection),
            Extension::group(extensions),
        )
        .unwrap();
        let mut view = EditorView::new(state);
        if view.state().read_only() {
            assert_eq!(row["batches"], json!([]), "{label}");
            assert_eq!(row["built"], 0, "{label}");
            continue;
        }
        let mut builds = 0;
        view.apply_headless_input(
            range,
            typed,
            |state| {
                builds += 1;
                Ok(state
                    .replace_selection_parsed(typed)?
                    .with_user_event("input.type")
                    .with_scroll_into_view())
            },
            (),
        )
        .unwrap();
        assert_eq!(builds, byte(&row["built"]), "{label}: default builds");
        let mut expected = row["batches"].clone();
        if row["bare"] == true {
            // Native fallback has no installed root, not an upstream bare root.
            assert_eq!(expected[0][0]["root"], key);
            expected[0][0]["root"] = Value::Null;
        }
        match name {
            "greater-than: source name may span protected gap"
            | "greater-than: upstream recovery in missing interpolation" => {
                let omitted = expected[0].as_array_mut().unwrap().pop().unwrap();
                assert_eq!(omitted["event"], "input.complete", "{label}");
                let insert = if name.contains("protected gap") {
                    "</d{{ a }}iv>"
                } else {
                    "</em>"
                };
                assert_eq!(omitted["edits"][0][2], insert, "{label}");
            }
            _ => {}
        }
        assert_eq!(
            json!(*observed.lock().unwrap()),
            expected,
            "{label}: input steps"
        );
        assert_eq!(
            view.state().text().as_str(),
            expected[0].as_array().unwrap().last().unwrap()["source"],
            "{label}: final source"
        );
    }
}
