//! Oracles from the SRI-verified `@codemirror/lang-go` 6.0.1 tarball in external scratch.
//! The oracle used `@codemirror/autocomplete` 6.20.4, `@codemirror/state` 6.7.6,
//! `@codemirror/language` 6.13.1, and `@lezer/go` 1.0.1. Rust tests use UTF-8 bytes.

use std::time::Duration;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Text, Transaction,
    autocomplete::session::{CompletionConfig, CompletionSession, SourceId, SourceResult},
    language::go::{
        GoDocument, SNIPPETS, go_completion_snippet, go_local_completion, go_snippet_completion,
    },
};
use serde_json::{Value, json};

#[test]
fn default_sources_match_the_pinned_contexts() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/go_completion_contexts.json")).unwrap();
    let docs: Vec<_> = fixture["docs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|doc| GoDocument::parse(Text::new(doc.as_str().unwrap())))
        .collect();
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let doc = &docs[case["doc"].as_u64().unwrap() as usize];
        let position = case["position"].as_u64().unwrap() as usize;
        let explicit = case["explicit"].as_bool().unwrap();
        let fixed = go_snippet_completion(doc, position, explicit);
        let local = go_local_completion(doc, position, explicit);
        assert_eq!(
            json!(fixed.as_ref().map(|result| result.from)),
            case["fixed"],
            "fixed: {id} ({})",
            case["inner"]
        );
        assert_eq!(
            json!(local.as_ref().map(|result| (result.from, labels(result)))),
            case["local"],
            "local: {id} ({})",
            case["inner"]
        );
        for result in fixed.iter().chain(local.iter()) {
            assert!(result.filter && result.to.is_none() && result.commit_characters.is_empty());
        }
    }
}

fn labels(result: &SourceResult) -> Vec<String> {
    result
        .options
        .iter()
        .map(|item| format!("{}:{}", item.kind.as_deref().unwrap(), item.label))
        .collect()
}

#[test]
fn fixed_catalog_matches_upstream_order_metadata_and_actions() {
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/go_completion_catalog.json")).unwrap();
    let doc = GoDocument::parse(Text::new(""));
    let fixed = go_snippet_completion(&doc, 0, true).unwrap();
    let options: Vec<_> = fixed
        .options
        .iter()
        .map(|item| {
            assert!(item.insert.is_none());
            json!([
                item.label,
                item.kind,
                item.detail,
                go_completion_snippet(item).is_some()
            ])
        })
        .collect();
    assert_eq!(json!(options), expected);
    for (item, snippet) in fixed.options.iter().zip(SNIPPETS) {
        assert_eq!(go_completion_snippet(item), Some(snippet));
    }
}

#[test]
fn every_snippet_matches_upstream_text_and_field_stops() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/go_completion_snippets.json")).unwrap();
    let doc = GoDocument::parse(Text::new(""));
    let fixed = go_snippet_completion(&doc, 0, true).unwrap();
    let snippets = fixed.options.iter().filter_map(go_completion_snippet);
    assert_eq!(snippets.clone().count(), cases.len());
    assert_eq!(SNIPPETS.len(), cases.len());
    for (snippet, case) in snippets.zip(cases) {
        assert_eq!(snippet.label, case["label"]);
        assert_eq!(json!(snippet.detail), case["detail"]);
        let source = "// π界\n  xx!";
        let from = source.find("xx").unwrap();
        let to = from + 2;
        let mut state = EditorState::new(source);
        state
            .update(Transaction::default().with_selection(EditorSelection::single(to, to + 1)))
            .unwrap();
        let (transaction, session) = snippet.snippet().insert(&state, from..to, "  ").unwrap();
        state.update(transaction).unwrap();
        assert_eq!(state.text().as_str(), case["text"]);
        let mut stops = vec![ranges(state.selection())];
        if let Some(mut session) = session {
            while let Some(selection) = session.next_field() {
                stops.push(ranges(&selection));
            }
        }
        assert_eq!(json!(stops), case["stops"], "{case}");
    }
}

fn ranges(selection: &EditorSelection) -> Vec<[usize; 2]> {
    selection
        .ranges()
        .iter()
        .map(|site| [site.range().start, site.range().end])
        .collect()
}

#[test]
fn source_validity_controls_when_the_session_requests_a_new_result() {
    for (replacement, valid_fixed, valid_local) in [
        ("f0_o", true, true),
        ("", true, false),
        ("$f$o", false, true),
        ("π😀", false, true),
        ("¡", false, true),
        ("\u{a0}", false, false),
        ("f o", false, false),
        ("fo\n", false, false),
    ] {
        type Source = fn(&GoDocument, usize, bool) -> Option<SourceResult>;
        let sources: [(Source, bool); 2] = [
            (go_snippet_completion, valid_fixed),
            (go_local_completion, valid_local),
        ];
        for (source, valid) in sources {
            let text = "func f() { println(fo) }\nvar foo int";
            let from = text.find("fo").unwrap();
            let to = from + 2;
            let doc = GoDocument::parse(Text::new(text));
            let mut state = EditorState::new(text);
            state
                .update(Transaction::default().with_selection(EditorSelection::single(to, to)))
                .unwrap();
            let now = Duration::ZERO;
            let mut session =
                CompletionSession::new(&state, CompletionConfig::default(), [SourceId(1)]);
            session.start(true, now);
            let at = session.deadline().unwrap();
            let query = session.take_queries(at).pop().unwrap();
            assert!(
                session
                    .finish_query(&query, source(&doc, to, true), at)
                    .unwrap()
            );
            assert_eq!(session.deadline(), None);

            let changes = ChangeSet::single(Change::new(from..to, replacement)).unwrap();
            let desc = changes.desc(text.len()).unwrap();
            let head = from + replacement.len();
            state
                .update(
                    Transaction::new(changes).with_selection(EditorSelection::single(head, head)),
                )
                .unwrap();
            session
                .update(&state, &desc, true, Some("input.type"), at)
                .unwrap();
            assert_eq!(session.deadline().is_none(), valid, "{replacement:?}");
        }
    }
}

#[test]
fn sources_reject_positions_outside_document_and_mid_character() {
    let doc = GoDocument::parse(Text::new("package π"));
    for position in [9, 11, usize::MAX] {
        assert!(go_snippet_completion(&doc, position, true).is_none());
        assert!(go_local_completion(&doc, position, true).is_none());
    }
}
