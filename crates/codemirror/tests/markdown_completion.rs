use std::time::Duration;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Text, Transaction,
    autocomplete::session::{
        CompletionConfig, CompletionItem, CompletionSession, CompletionStatus, SourceId,
    },
    language::markdown::{MarkdownDialect, MarkdownDocument, markdown_completion_source},
};
use serde_json::Value;

// Captured in scratch Node from the SHA-512-verified lang-markdown 6.5.2
// and lang-html 6.4.12 tarballs and the pinned parser dependencies. Call
// htmlTagCompletion directly, without language-data routing. Convert UTF-16
// ranges to native byte offsets (including CRLF) when capturing the results.
fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/markdown_completion.json")).unwrap()
}

#[test]
fn matches_the_pinned_source_contexts_ranges_and_options() {
    let mut oracle = oracle();
    let labels: Vec<String> = serde_json::from_value(oracle["options"].take()).unwrap();
    let expected_options: Vec<_> = labels
        .into_iter()
        .map(|label| {
            let mut item = CompletionItem::new(label);
            item.kind = Some("type".into());
            item
        })
        .collect();
    let cases: Vec<(String, String, Option<[usize; 2]>)> =
        serde_json::from_value(oracle["contexts"].take()).unwrap();
    for (dialect, marked, expected) in cases {
        let dialect = match dialect.as_str() {
            "commonmark" => MarkdownDialect::CommonMark,
            "extended" => MarkdownDialect::Extended,
            _ => panic!("unexpected dialect {dialect}"),
        };
        let (before, after) = marked.split_once('¦').unwrap();
        let doc =
            MarkdownDocument::parse_with_dialect(Text::new(format!("{before}{after}")), dialect);
        for explicit in [false, true] {
            let result = markdown_completion_source(&doc, before.len(), explicit);
            assert_eq!(
                result.as_ref().map(|r| [r.from, r.to.unwrap()]),
                expected,
                "{marked:?} (explicit: {explicit})",
            );
            if let Some(result) = result {
                assert_eq!(&*result.options, expected_options);
                assert!(result.filter);
                assert!(result.commit_characters.is_empty());
            }
        }
    }
}

#[test]
fn rejects_invalid_byte_positions() {
    let doc = MarkdownDocument::parse(Text::new("<é"));
    for position in [2, 4, usize::MAX] {
        assert!(markdown_completion_source(&doc, position, true).is_none());
    }
}

#[test]
fn reuses_results_only_while_the_whole_replacement_matches() {
    let cases: Vec<(String, bool)> = serde_json::from_value(oracle()["validity"].take()).unwrap();
    for (replacement, valid) in cases {
        let mut state = EditorState::new("<s");
        state
            .update(Transaction::default().with_selection(EditorSelection::single(2, 2)))
            .unwrap();
        let mut session =
            CompletionSession::new(&state, CompletionConfig::default(), [SourceId(0)]);
        session.start(true, Duration::ZERO);
        let now = session.deadline().unwrap();
        let query = session.take_queries(now).pop().unwrap();
        let doc = MarkdownDocument::parse(query.state().text().clone());
        let result = markdown_completion_source(&doc, query.position, query.explicit).unwrap();
        assert!(session.finish_query(&query, Some(result), now).unwrap());
        assert_eq!(session.status(), CompletionStatus::Active);

        let changes = ChangeSet::single(Change::new(0..2, replacement.as_str())).unwrap();
        let desc = changes.desc(state.text().len()).unwrap();
        let end = replacement.len();
        state
            .update(Transaction::new(changes).with_selection(EditorSelection::single(end, end)))
            .unwrap();
        session
            .update(&state, &desc, true, Some("input.type"), now)
            .unwrap();
        // Results can stay valid even when filtering finds no matching options.
        assert_eq!(
            session.status() != CompletionStatus::Pending,
            valid,
            "{replacement:?}"
        );
    }
}
