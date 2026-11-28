use std::sync::{Arc, LazyLock, Mutex};

use gpui_editor_codemirror::{
    Assoc, Change, ChangePermission, ChangeSet, Compartment, EditorSelection, EditorState,
    Extension, Text, Transaction,
    autocomplete::{brackets::close_brackets_enabled, session::SourceResult},
    change_filter,
    commands::CommentTokens,
    language::{
        IndentOptions, LanguageData,
        brackets::bracket_matching_options,
        fold::foldable_in_state,
        get_indentation,
        xml::{XmlElementSpec, XmlSchema, xml_completion_source},
    },
    language_data::{NativeSyntax, XmlCompletionSource, attached_language_snapshot, xml_extension},
    lint::LintConfiguration,
    transaction_filter,
    view::{
        EditorView,
        input::{InputChange, InputHandling, run_input_handlers},
        update_listener,
    },
};
use serde_json::{Value, json};

use super::{bundle, byte};

const TRIGGERS: &[&str] = &["  </", "\t</", "</ ", "<", "  >"];

fn rows(kind: &str) -> impl Iterator<Item = Value> {
    include_str!("../fixtures/language_data_xml.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn configured_xml(name: &str, close_tags: bool) -> Extension {
    xml_extension(
        XmlSchema {
            elements: &[XmlElementSpec {
                name,
                top: true,
                ..XmlElementSpec::default()
            }],
            ..XmlSchema::default()
        },
        close_tags,
    )
}

fn names(list: &Value, outside: &Extension) -> Extension {
    static CONFIGURED: LazyLock<Extension> = LazyLock::new(|| configured_xml("custom", true));
    static DISABLED: LazyLock<Extension> = LazyLock::new(|| configured_xml("other", false));
    Extension::group(
        list.as_array()
            .unwrap()
            .iter()
            .map(|name| match name.as_str().unwrap() {
                "outside" => outside.clone(),
                "configured" => CONFIGURED.clone(),
                "disabled" => DISABLED.clone(),
                name => bundle(name),
            }),
    )
}

fn sources(state: &EditorState, at: usize) -> Vec<XmlCompletionSource> {
    state.language_data_at("autocomplete", at, Assoc::Before)
}

fn results(
    sources: &[XmlCompletionSource],
    state: &EditorState,
    at: usize,
    explicit: bool,
) -> Value {
    fn summarize(result: SourceResult) -> Value {
        let options: Vec<_> = result
            .options
            .iter()
            .map(|item| json!({"label": item.label, "type": item.kind, "boost": item.boost}))
            .collect();
        json!({"from": result.from, "to": result.to, "filter": result.filter, "options": options})
    }
    json!(
        sources
            .iter()
            .map(|source| source(state, at, explicit).map(summarize))
            .collect::<Vec<_>>()
    )
}

#[test]
fn catalog_xml_completion_uses_the_stock_empty_schema_without_entities() {
    for row in rows("query") {
        let state =
            EditorState::with_extensions(row["source"].as_str().unwrap(), bundle("XML")).unwrap();
        let at = byte(&row["at"]);
        let explicit = row["explicit"].as_bool().unwrap();
        let sources = sources(&state, at);
        assert_eq!(sources.len(), 1);
        assert_eq!(
            results(&sources, &state, at, explicit),
            row["results"],
            "{row}"
        );
        if row["marked"] == "<a>α &am|" {
            let snapshot = attached_language_snapshot(&state).unwrap();
            let NativeSyntax::Xml(doc) = snapshot.syntax() else {
                panic!("expected XML syntax");
            };
            let direct = xml_completion_source(doc, at, explicit, XmlSchema::default()).unwrap();
            assert_eq!(direct.options.len(), 5);
            assert_eq!(direct.options[0].label, "&amp;");
            assert!(
                (sources[0])(&state, 4, explicit).is_none(),
                "interior UTF-8 byte"
            );
        }
    }
}

#[test]
fn configured_xml_composes_sources_and_preserves_the_stock_parser() {
    let compartment = Compartment::new();
    let one = configured_xml("custom", true);
    let two = configured_xml("other", false);
    let mut state = EditorState::with_extensions("<", compartment.of(one.clone())).unwrap();
    let stock_tree = attached_language_snapshot(&state).unwrap();
    let one_source = sources(&state, 1).remove(0);
    let order = |state: &EditorState| {
        sources(state, 1)
            .iter()
            .map(|source| source(state, 1, false).unwrap().options[0].label.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(&state), ["custom"]);
    let retained = state.clone();
    state
        .update(
            Transaction::default().with_effects([compartment.reconfigure(Extension::group([
                two,
                one.clone(),
                one,
            ]))]),
        )
        .unwrap();
    assert_eq!(order(&state), ["other", "custom"]);
    assert_eq!(order(&retained), ["custom"]);
    assert!(Arc::ptr_eq(&sources(&state, 1)[1], &one_source));
    assert!(Arc::ptr_eq(
        &attached_language_snapshot(&state).unwrap(),
        &stock_tree
    ));
    state
        .update(Transaction::default().with_effects([compartment.reconfigure(bundle("JSON"))]))
        .unwrap();
    assert!(one_source(&state, 1, true).is_none());
}

#[test]
fn catalog_xml_tracks_syntax_and_preserves_provider_order() {
    let slot = Compartment::new();
    let outside_source: XmlCompletionSource = Arc::new(|_, _, _| None);
    let outside = gpui_editor_codemirror::language::language_data_provider({
        let source = outside_source.clone();
        move |_, _, _| {
            vec![Arc::new(
                LanguageData::default().with("autocomplete", source.clone()),
            )]
        }
    });
    let mut rows = rows("state").peekable();
    let original = rows.peek().unwrap()["source"].as_str().unwrap().to_owned();
    let mut state = EditorState::with_extensions(&original, slot.of(Extension::default())).unwrap();
    let mut retained = None;
    let mut catalog_source = None;

    for row in rows {
        let step = &row["step"];
        let before = attached_language_snapshot(&state);
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
        if !row["names"].is_null() {
            tr = tr.with_effects([slot.reconfigure(names(&row["names"], &outside))]);
        }
        state.update(tr).unwrap();
        let snapshot = attached_language_snapshot(&state);
        let xml = snapshot
            .as_ref()
            .and_then(|snapshot| match snapshot.syntax() {
                NativeSyntax::Xml(doc) => Some(doc),
                _ => None,
            });
        let active = xml.is_some();
        let at = byte(&row["at"]);
        let sources = sources(&state, at);
        if active {
            let installed = sources
                .iter()
                .find(|source| !Arc::ptr_eq(source, &outside_source))
                .unwrap();
            assert!(Arc::ptr_eq(
                catalog_source.get_or_insert_with(|| installed.clone()),
                installed
            ));
        } else {
            assert!(
                (catalog_source.as_ref().unwrap())(&state, at, true).is_none(),
                "{step}"
            );
        }
        let nodes = xml.map_or_else(Vec::new, |doc| {
            doc.nodes()
                .map(|node| {
                    let range = node.range();
                    json!([node.kind().name(), range.start, range.end])
                })
                .collect()
        });
        let indent = active
            .then(|| {
                get_indentation(
                    &state,
                    byte(&row["indentAt"]),
                    IndentOptions::default(),
                    None,
                )
                .unwrap()
            })
            .flatten();
        let fold = active
            .then(|| {
                foldable_in_state(
                    &state,
                    byte(&row["lineRange"][0]),
                    byte(&row["lineRange"][1]),
                )
            })
            .flatten();
        let comments: Vec<_> = state
            .language_data_at::<CommentTokens>("commentTokens", at, Assoc::Before)
            .iter()
            .map(|tokens| json!([tokens.line, tokens.block]))
            .collect();
        let triggers: Vec<Vec<_>> = state
            .language_data_at::<fn(&str) -> bool>("indentOnInput", at, Assoc::Before)
            .iter()
            .map(|rule| TRIGGERS.iter().map(|text| rule(text)).collect())
            .collect();
        let actual = json!({"source": state.text().as_str(), "head": state.selection().main().head(),
            "active": active, "nodes": nodes,
            "reused": active && matches!((&before, &snapshot), (Some(a), Some(b)) if Arc::ptr_eq(a, b)),
            "results": results(&sources, &state, at, true), "comments": comments, "triggers": triggers,
            "indent": indent, "fold": fold.map(|range| [range.start, range.end])});
        for (key, value) in actual.as_object().unwrap() {
            assert_eq!(value, &row[key], "{step}: {key}");
        }
        if retained.is_none() {
            retained = snapshot;
        }
        if step == "detach" {
            assert!(attached_language_snapshot(&state).is_none());
        }
        assert!(bracket_matching_options(&state).unwrap().is_none());
        assert!(!close_brackets_enabled(&state).unwrap());
        assert!(!LintConfiguration::get(&state).has_sources());
    }
    assert_eq!(retained.unwrap().text().as_str(), original);
}

fn change(from: usize, to: usize, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(from..to, insert)).unwrap()
}

fn typed(changes: ChangeSet, cursor: usize) -> Transaction {
    Transaction::new(changes)
        .with_selection(EditorSelection::single(cursor, cursor))
        .with_user_event("input.type")
        .with_scroll_into_view()
}

#[test]
fn catalog_xml_closes_tags_after_the_accepted_default() {
    for row in rows("input") {
        let mode = row["mode"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let at = byte(&row["at"]);
        let from = byte(&row["from"]);
        let input = row["typed"].as_str().unwrap();
        let slot = Compartment::new();
        let extensions = if mode == "detached" {
            bundle("XML")
        } else {
            names(&row["names"], &Extension::default())
        };
        let mut extensions = vec![slot.of(extensions)];
        if let Some(rewrite) = row["rewrite"].as_array() {
            let start = byte(&rewrite[0]);
            let end = byte(&rewrite[1]);
            let replacement = rewrite[2].as_str().unwrap().to_owned();
            extensions.push(transaction_filter(move |_, tr| {
                Ok(if tr.is_user_event("input.type") {
                    typed(change(start, end, &replacement), start + replacement.len())
                } else {
                    tr
                })
            }));
        }
        if mode == "filtered completion" {
            extensions.push(change_filter(|_, tr| {
                if tr.is_user_event("input.complete") {
                    ChangePermission::Reject
                } else {
                    ChangePermission::Allow
                }
            }));
        }
        let updates = Arc::new(Mutex::new(Vec::new()));
        let observed = updates.clone();
        extensions.push(update_listener(move |update| {
            let steps: Vec<_> = update.input_steps.iter().map(|step| {
                let tr = &step.transaction;
                let caret = step.state.selection().main();
                let assoc = match caret.assoc() {
                    Some(Assoc::Before) => -1,
                    Some(Assoc::After) => 1,
                    None => 0,
                };
                let changes: Vec<_> = tr.changes().changes()
                    .map(|(range, text)| json!([range.start, range.end, text])).collect();
                json!({"before": step.start_state.text().as_str(), "after": step.state.text().as_str(),
                    "head": caret.head(), "assoc": assoc, "event": tr.user_event(),
                    "scroll": tr.scroll_into_view(), "changes": changes})
            }).collect();
            observed.lock().unwrap().push(json!(steps));
            Ok(())
        }));
        let mut state = EditorState::create_from_text(
            Text::new(source),
            Some(EditorSelection::single(at, at)),
            Extension::group(extensions),
        )
        .unwrap();
        if mode == "detached" {
            state
                .update(
                    Transaction::default().with_effects([slot.reconfigure(Extension::default())]),
                )
                .unwrap();
        }
        let change_input = InputChange::new(&state, from..at, input).unwrap();
        assert!(matches!(
            run_input_handlers(change_input, || panic!("no atomic input handler")).unwrap(),
            InputHandling::Pass
        ));
        let mut view = EditorView::new(state);
        let mut builds = 0;
        view.apply_headless_input(
            from..at,
            input,
            |_| {
                builds += 1;
                Ok(typed(change(from, at, input), byte(&row["cursor"])))
            },
            (),
        )
        .unwrap_or_else(|error| panic!("{mode}: {error:?}"));
        assert_eq!(builds, 1, "{mode}");
        assert_eq!(*updates.lock().unwrap(), [row["steps"].clone()], "{mode}");
        let steps = row["steps"].as_array().unwrap();
        assert_eq!(
            view.state().text().as_str(),
            steps.last().unwrap()["after"],
            "{mode}"
        );
        assert_eq!(
            view.undo_depth(),
            if steps.len() == 2 && mode != "filtered completion" {
                2
            } else {
                1
            },
            "{mode}"
        );
    }
}
