use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Text,
    Transaction,
    autocomplete::{brackets::close_brackets_enabled, session::SourceResult},
    commands::CommentTokens,
    language::{
        IndentOptions,
        brackets::bracket_matching_options,
        fold::foldable_in_state,
        get_indentation,
        javascript::{JavascriptOptions, auto_close_tags},
    },
    language_data::{
        CloseBracketData, NativeLanguage, NativeSyntax, ScriptCompletionSource, SupportedDocument,
        attached_language_snapshot,
    },
    lint::LintConfiguration,
    view::{
        EditorView,
        input::{InputChange, InputHandling, run_input_handlers},
        update_listener,
    },
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const TRIGGERS: &[&str] = &[
    "  }",
    "  ]",
    "case ",
    "case x:",
    "else:",
    "elif ",
    "  default:",
    "  </",
];

fn rows() -> impl Iterator<Item = Value> {
    include_str!("../fixtures/language_data_script.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn summarize(result: SourceResult, document: &SupportedDocument, at: usize) -> Value {
    let mut hash = 0xcbf29ce484222325u64;
    for item in result.options.iter() {
        assert!(item.display_label.is_none());
        assert!(item.sort_text.is_none());
        assert!(item.info.is_none());
        assert!(item.insert.is_none());
        assert!(item.section.is_none());
        assert!(item.commit_characters.is_none());
        let apply = u8::from(document.completion_snippet(at, item).is_some());
        for byte in format!(
            "{}\0{}\0{}\0{}\0{apply}\n",
            item.label,
            item.kind.as_deref().unwrap_or_default(),
            item.detail.as_deref().unwrap_or_default(),
            item.boost,
        )
        .bytes()
        {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
    assert!(result.commit_characters.is_empty());
    json!({"from": result.from, "to": result.to, "filter": result.filter,
        "count": result.options.len(), "hash": format!("{hash:016x}")})
}

#[test]
fn script_factories_replay_source_order_and_reconfiguration() {
    let mut scenario = String::new();
    let mut slot = Compartment::new();
    let mut state = EditorState::new("");
    let mut retained = None;
    let mut original = String::new();
    for row in rows().filter(|row| row["kind"] == "state") {
        let name = row["name"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let step = row["step"].as_str().unwrap();
        if scenario != name {
            assert_retained(retained.take(), &original);
            scenario = name.to_owned();
            original = source.to_owned();
            slot = Compartment::new();
            state = EditorState::with_extensions(source, slot.of(Extension::default())).unwrap();
        }
        let before = attached_language_snapshot(&state);
        let mut tr = Transaction::default();
        if let Some(edit) = row["edit"].as_array() {
            let range = super::byte(&edit[0])..super::byte(&edit[1]);
            tr = Transaction::new(
                ChangeSet::single(Change::new(range, edit[2].as_str().unwrap())).unwrap(),
            );
        }
        if !row["select"].is_null() {
            let at = super::byte(&row["select"]);
            tr = tr.with_selection(EditorSelection::single(at, at));
        }
        if let Some(names) = row["extensions"].as_array() {
            tr = tr.with_effects([slot.reconfigure(Extension::group(names.iter().map(|name| {
                let name = name.as_str().unwrap();
                if let Some(name) = name.strip_prefix("high ") {
                    return super::bundle(name)
                        .with_precedence(gpui_editor_codemirror::Precedence::High);
                }
                super::extension(name)
            })))]);
        }
        state.update(tr).unwrap();
        let at = super::byte(&row["at"]);
        let side = Assoc::Before;
        let snapshot = attached_language_snapshot(&state);
        let reused = matches!((&before, &snapshot), (Some(a), Some(b)) if Arc::ptr_eq(a, b));
        let active = snapshot
            .as_ref()
            .and_then(|doc| doc.language_at(at, side))
            .map(|language| match language {
                NativeLanguage::Python => "Python",
                NativeLanguage::Javascript(JavascriptOptions {
                    typescript: true,
                    jsx: false,
                }) => "TypeScript",
                NativeLanguage::Javascript(JavascriptOptions {
                    typescript: false,
                    jsx: false,
                }) => "JavaScript",
                NativeLanguage::Javascript(JavascriptOptions {
                    typescript: false,
                    jsx: true,
                }) => "JSX",
                NativeLanguage::Javascript(JavascriptOptions {
                    typescript: true,
                    jsx: true,
                }) => "TSX",
                _ => unreachable!("unexpected {language:?}"),
            });
        let root = match snapshot.as_ref().map(|doc| doc.syntax()) {
            Some(NativeSyntax::Python(doc)) => doc.root().name(),
            Some(NativeSyntax::Javascript(doc)) => doc.root().kind().name(),
            None => "",
            _ => unreachable!("unexpected active syntax"),
        };
        let queries: Vec<_> = row["queries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|query| {
                let position = super::byte(&query["at"]);
                let explicit = query["explicit"].as_bool().unwrap();
                let results: Vec<_> = state
                    .language_data_at::<ScriptCompletionSource>("autocomplete", position, side)
                    .iter()
                    .map(|source| {
                        source(&state, position, explicit)
                            .map(|result| summarize(result, snapshot.as_deref().unwrap(), position))
                    })
                    .collect();
                json!({"at": position, "explicit": explicit, "results": results})
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
        let triggers: Vec<Vec<_>> = state
            .language_data_at::<fn(&str) -> bool>("indentOnInput", at, side)
            .iter()
            .map(|rule| TRIGGERS.iter().map(|text| rule(text)).collect())
            .collect();
        let indent = get_indentation(
            &state,
            super::byte(&row["indentAt"]),
            IndentOptions::default(),
            None,
        )
        .unwrap();
        let fold = foldable_in_state(
            &state,
            super::byte(&row["lineRange"][0]),
            super::byte(&row["lineRange"][1]),
        )
        .map(|range| [range.start, range.end]);
        let actual = json!({"source": state.text().as_str(), "head": state.selection().main().head(),
            "root": root, "active": active, "reused": reused, "queries": queries, "comments": comments,
            "brackets": brackets, "triggers": triggers, "indent": indent, "fold": fold,
            "wordChars": state.language_data_at::<&str>("wordChars", at, side)});
        for (key, actual) in actual.as_object().unwrap() {
            assert_eq!(actual, &row[key], "{name}, {step}: {key}");
        }
        assert!(bracket_matching_options(&state).unwrap().is_none());
        assert!(!close_brackets_enabled(&state).unwrap());
        assert!(!LintConfiguration::get(&state).has_sources());
        let input = InputChange::new(&state, at..at, ">").unwrap();
        assert!(matches!(
            run_input_handlers(input, || panic!("no default requested")).unwrap(),
            InputHandling::Pass
        ));
        if retained.is_none() {
            retained = snapshot;
        }
    }
    assert_retained(retained, &original);
}

#[test]
fn javascript_catalog_dialects_share_only_the_local_source() {
    let names = ["JavaScript", "TypeScript", "JSX", "TSX"];
    let sources = names.map(|name| {
        let state =
            EditorState::with_extensions("const identifier = 1", super::bundle(name)).unwrap();
        let sources =
            state.language_data_at::<ScriptCompletionSource>("autocomplete", 6, Assoc::Before);
        assert_eq!(sources.len(), 2);
        sources
    });
    for (index, sources_at) in sources.iter().enumerate() {
        for earlier in &sources[..index] {
            assert!(!Arc::ptr_eq(&earlier[0], &sources_at[0]));
            assert!(Arc::ptr_eq(&earlier[1], &sources_at[1]));
        }
    }
}

fn assert_retained(document: Option<Arc<SupportedDocument>>, original: &str) {
    if let Some(document) = document {
        assert_eq!(document.text().as_str(), original);
    }
}

#[test]
fn jsx_catalog_keeps_the_direct_scoped_helper_separate() {
    for row in rows().filter(|row| row["kind"] == "input" && row["mode"] == "plain") {
        let name = row["name"].as_str().unwrap();
        let entry = gpui_editor_codemirror::language_data::LanguageDescription::match_name(
            gpui_editor_codemirror::language_data::LANGUAGES,
            name,
            false,
        )
        .unwrap();
        let support = super::load(entry);
        let NativeLanguage::Javascript(options) = support.language() else {
            unreachable!("unexpected {name}");
        };
        let source = row["source"].as_str().unwrap();
        let at = super::byte(&row["at"]);
        let mut state =
            EditorState::with_extensions(source, support.extension().unwrap_or_default()).unwrap();
        state
            .update(Transaction::default().with_selection(EditorSelection::single(at, at)))
            .unwrap();
        let input = InputChange::new(&state, at..at, ">").unwrap();
        assert!(matches!(
            run_input_handlers(input, || panic!("no default requested")).unwrap(),
            InputHandling::Pass
        ));
        let tr = auto_close_tags(&state, ">", options).unwrap();
        if row["hasHandler"] == true {
            assert!(support.extension().is_some());
            assert_eq!(
                row["events"],
                json!([["input.type", true], ["input.complete", true]])
            );
            let tr = tr.unwrap();
            assert_eq!(tr.user_event(), Some("input.complete"));
            assert!(tr.scroll_into_view());
            state.update(tr).unwrap();
        } else {
            assert!(tr.is_none());
            state.update(state.replace_selection(">").unwrap()).unwrap();
        }
        assert_eq!(state.text().as_str(), row["after"]);
        assert_eq!(state.selection().main().head(), super::byte(&row["head"]));
    }
}

#[test]
fn headless_jsx_catalog_input_uses_the_actual_default_then_completes() {
    for row in rows().filter(|row| row["kind"] == "input") {
        let name = row["name"].as_str().unwrap();
        let mode = row["mode"].as_str().unwrap();
        if matches!(mode, "composition" | "read only") {
            // Those paths are gated by the host before headless dispatch.
            continue;
        }
        let at = super::byte(&row["at"]);
        let typed = row["typed"].as_str().unwrap_or(">");
        let source = row["source"].as_str().unwrap();
        let updates = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&updates);
        let state = EditorState::create_from_text(
            Text::new(source),
            Some(EditorSelection::single(at, at)),
            Extension::group([
                super::bundle(name),
                update_listener(move |update| {
                    observed.lock().unwrap().push(
                        update
                            .input_steps
                            .iter()
                            .map(|step| {
                                json!([
                                    step.transaction.user_event(),
                                    step.transaction.scroll_into_view()
                                ])
                            })
                            .collect::<Vec<_>>(),
                    );
                    Ok(())
                }),
            ]),
        )
        .unwrap();
        let mut view = EditorView::new(state);
        let (from, insert, cursor) = if let Some(default) = row["default"].as_array() {
            (
                super::byte(&default[0]),
                default[1].as_str().unwrap(),
                super::byte(&default[2]),
            )
        } else {
            (
                at,
                if mode == "existing close" {
                    "></X>"
                } else {
                    typed
                },
                at + typed.len(),
            )
        };
        let mut builds = 0;
        view.apply_headless_input(
            at..at,
            typed,
            |_| {
                builds += 1;
                Ok(
                    Transaction::new(ChangeSet::single(Change::new(from..at, insert))?)
                        .with_selection(EditorSelection::single(cursor, cursor))
                        .with_user_event("input.type")
                        .with_scroll_into_view(),
                )
            },
            (),
        )
        .unwrap_or_else(|error| panic!("{name}: {mode}: {error:?}"));
        assert_eq!(builds, 1, "{name}: {mode}");
        let updates = updates.lock().unwrap();
        assert_eq!(updates.len(), 1, "{name}: {mode}");
        let handled = row["handled"] == true;
        let events = if handled {
            row["events"].clone()
        } else {
            json!([["input.type", true]])
        };
        assert_eq!(json!(updates[0]), events, "{name}: {mode}");
        assert_eq!(view.state().text().as_str(), row["after"], "{name}: {mode}");
        assert_eq!(
            view.state().selection().main().head(),
            super::byte(&row["head"]),
            "{name}: {mode}"
        );
        assert_eq!(
            view.undo_depth(),
            if handled { 2 } else { 1 },
            "{name}: {mode}"
        );
        if row["assoc"] == -1 {
            assert_eq!(view.state().selection().main().assoc(), Some(Assoc::Before));
        }
    }
}
