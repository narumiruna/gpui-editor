//! Fixtures from the SHA-512-verified `@codemirror/lang-python` 6.2.1 tarball.
//! The scratch oracle used the inventory pins for `@lezer/python`,
//! `@codemirror/autocomplete`, `@codemirror/state`, and `@codemirror/language`.
//! The oracle converted upstream UTF-16 offsets to UTF-8 byte offsets.

use std::time::Duration;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Text, Transaction,
    autocomplete::{
        active_snippet, next_snippet_field,
        session::{CompletionConfig, CompletionSession, SourceId},
        snippet_state_extension, start_snippet,
    },
    language::python::{
        PythonDocument, SNIPPETS, global_completion_source, local_completion_source,
        python_completion_snippet,
    },
};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/python_completion.json")).unwrap()
}

#[test]
fn local_and_global_queries_match_pinned_contexts() {
    for case in fixture()["contexts"].as_array().unwrap() {
        let markup = case["markup"].as_str().unwrap();
        let position = markup.find('|').unwrap();
        let doc = PythonDocument::parse(Text::new(markup.replacen('|', "", 1)));
        let explicit = case["explicit"].as_bool().unwrap();
        let local = local_completion_source(&doc, position, explicit);
        let global = global_completion_source(&doc, position, explicit);
        let snapshot = local.as_ref().map(|result| {
            let names: Vec<_> = result
                .options
                .iter()
                .map(|item| format!("{}:{}", item.kind.as_deref().unwrap(), item.label))
                .collect();
            (result.from, names)
        });
        assert_eq!(json!(snapshot), case["local"], "local: {}", case["id"]);
        assert_eq!(
            json!(global.as_ref().map(|result| result.from)),
            case["global"],
            "global: {}",
            case["id"],
        );
        for result in local.iter().chain(global.iter()) {
            assert!(result.filter && result.to.is_none() && result.commit_characters.is_empty());
        }
    }
}

#[test]
fn globals_match_the_pinned_catalog_in_order() {
    let doc = PythonDocument::parse(Text::new(""));
    let result = global_completion_source(&doc, 0, true).unwrap();
    let mut snippets = Vec::new();
    let catalog: Vec<_> = result
        .options
        .iter()
        .map(|item| {
            let mut entry = vec![item.label.as_str(), item.kind.as_deref().unwrap()];
            assert!(item.insert.is_none());
            if let Some(detail) = &item.detail {
                entry.push(detail);
            }
            snippets.extend(python_completion_snippet(item).copied());
            entry
        })
        .collect();
    assert_eq!(json!(catalog), fixture()["catalog"]);
    assert_eq!(snippets, SNIPPETS);
}

#[test]
fn native_snippet_actions_match_the_pinned_text_and_field_stops() {
    let doc = PythonDocument::parse(Text::new("  xx"));
    let result = global_completion_source(&doc, 4, true).unwrap();
    let cases = fixture();
    let cases = cases["snippets"].as_array().unwrap();
    assert_eq!(cases.len(), SNIPPETS.len());
    for case in cases {
        let item = result
            .options
            .iter()
            .find(|item| item.label == case["label"] && json!(item.detail) == case["detail"])
            .unwrap();
        let snippet = python_completion_snippet(item).unwrap().snippet();
        let mut state =
            EditorState::with_extensions(doc.text().as_str(), snippet_state_extension()).unwrap();
        state
            .update(Transaction::default().with_selection(EditorSelection::single(4, 4)))
            .unwrap();
        let (mut insert, session) = snippet.insert(&state, result.from..4, "  ").unwrap();
        if let Some(session) = session {
            insert = insert.with_effects([start_snippet(session)]);
        }
        state.update(insert).unwrap();
        assert_eq!(state.text().as_str(), case["text"]);
        let selection = |state: &EditorState| {
            state
                .selection()
                .ranges()
                .iter()
                .map(|site| [site.range().start, site.range().end])
                .collect::<Vec<_>>()
        };
        let mut stops = vec![selection(&state)];
        while let Some(next) = next_snippet_field(&state) {
            state.update(next).unwrap();
            stops.push(selection(&state));
        }
        assert!(active_snippet(&state).is_none());
        assert_eq!(json!(stops), case["stops"], "{}", case["detail"]);
    }
}

#[test]
fn cached_source_ranges_follow_the_pinned_valid_for_patterns() {
    for case in fixture()["validity"].as_array().unwrap() {
        let value = case["value"].as_str().unwrap();
        for source in ["local", "global"] {
            let mut state = EditorState::new("p\nprefix = 1");
            state
                .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
                .unwrap();
            let doc = PythonDocument::parse(state.text().clone());
            let mut session =
                CompletionSession::new(&state, CompletionConfig::default(), [SourceId(1)]);
            session.start(true, Duration::ZERO);
            let at = session.deadline().unwrap();
            let query = session.take_queries(at).pop().unwrap();
            let result = if source == "local" {
                local_completion_source(&doc, query.position, query.explicit)
            } else {
                global_completion_source(&doc, query.position, query.explicit)
            };
            assert!(result.is_some());
            session.finish_query(&query, result, at).unwrap();

            let changes = ChangeSet::single(Change::new(0..1, value)).unwrap();
            let desc = changes.desc(state.text().len()).unwrap();
            let head = value.len();
            let transaction = Transaction::new(changes)
                .with_selection(EditorSelection::single(head, head))
                .with_user_event("input.type");
            state.update(transaction).unwrap();
            session
                .update(&state, &desc, true, Some("input.type"), at)
                .unwrap();
            assert_eq!(
                session.deadline().is_none(),
                case[source],
                "{source}: {value:?}"
            );
        }
    }
}

#[test]
fn sources_reject_offsets_outside_the_document_or_within_a_code_point() {
    let doc = PythonDocument::parse(Text::new("café"));
    for position in [4, 6, usize::MAX] {
        assert!(local_completion_source(&doc, position, true).is_none());
        assert!(global_completion_source(&doc, position, true).is_none());
    }
}
