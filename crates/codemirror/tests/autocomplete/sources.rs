use std::{cell::Cell, sync::Arc, time::Duration};

use gpui_editor_codemirror::{
    EditorSelection, EditorState, Transaction,
    autocomplete::session::{
        CompletionConfig, CompletionItem, CompletionNode, CompletionQuery, CompletionSession,
        QueryMatch, QueryToken, SourceId, SourceResult, complete_any_word, if_in, if_not_in,
    },
    word_characters,
};

fn query_at(
    state: &mut EditorState,
    position: usize,
    explicit: bool,
) -> (CompletionSession, CompletionQuery) {
    state
        .update(Transaction::default().with_selection(EditorSelection::single(position, position)))
        .unwrap();
    let mut session = CompletionSession::new(state, CompletionConfig::default(), [SourceId(1)]);
    session.start(explicit, Duration::ZERO);
    let query = session
        .take_queries(Duration::from_millis(100))
        .pop()
        .unwrap();
    (session, query)
}

// The UTF-8 ranges correspond to the pinned HTML/JavaScript oracle around `caf`.
#[test]
fn syntax_combinators_stop_at_the_first_language_root_but_token_before_does_not() {
    let doc = "<script>let café = 1</script>";
    let pos = doc.find("café").unwrap() + "caf".len();
    let (_session, query) = query_at(&mut EditorState::new(doc), pos, false);
    let node = |name, from, is_top| CompletionNode { name, from, is_top };
    let leaf = node("VariableDefinition", 12, false);
    let element = node("Element", 0, false);
    let path = [
        leaf,
        node("VariableDeclaration", 8, false),
        node("Script", 8, true),
        element,
        node("Document", 0, true),
    ];
    let calls = Cell::new(0);
    let source = |context: &CompletionQuery| {
        calls.set(calls.get() + 1);
        Some(SourceResult::new(
            context.position,
            [CompletionItem::new("ran")],
        ))
    };
    assert_eq!(if_in(&["Script"], source)(&query, &path).unwrap().from, pos);
    assert!(if_in(&["Element"], source)(&query, &path).is_none());
    assert!(if_not_in(&["VariableDeclaration"], source)(&query, &path).is_none());
    assert!(if_not_in(&["Element"], source)(&query, &path).is_some());
    assert!(if_in(&["Script"], source)(&query, &[]).is_none());
    assert!(if_not_in(&["Script"], source)(&query, &[]).is_some());
    assert_eq!(calls.get(), 3);
    assert_eq!(
        query.token_before(&["Element", "VariableDefinition"], &path),
        Some(QueryToken {
            node: leaf,
            matched: QueryMatch {
                range: 12..pos,
                text: "caf"
            }
        })
    );
    assert_eq!(
        query.token_before(&["Element"], &path),
        Some(QueryToken {
            node: element,
            matched: QueryMatch {
                range: 0..pos,
                text: "<script>let caf"
            }
        })
    );
    assert!(query.token_before(&["Missing"], &path).is_none());
    assert!(
        query
            .token_before(&["Invalid"], &[node("Invalid", pos + 1, false)])
            .is_none()
    );
}

fn labels(result: &SourceResult) -> Vec<&str> {
    result
        .options
        .iter()
        .map(|option| {
            assert_eq!(option.kind.as_deref(), Some("text"));
            option.label.as_str()
        })
        .collect()
}

#[test]
fn document_words_use_unicode_scope_order_and_deduplicate_after_skipping_the_active_token() {
    // CRLF also exercises conversion between line-relative matches and byte offsets.
    let doc = "🦀 café 𐐀elta\r\ncafé Ⅻ_β\r\nfoo-bar foo-baz foo-bar\r\n!";
    let mut state = EditorState::new(doc);
    let (_session, query) = query_at(&mut state, "🦀 ca".len(), false);
    let result = complete_any_word(&query).unwrap();
    assert_eq!(result.from, "🦀 ".len());
    assert_eq!(result.to, None);
    assert_eq!(
        labels(&result),
        ["𐐀elta", "café", "Ⅻ_β", "foo", "bar", "baz"]
    );

    let from = doc.find("foo-").unwrap();
    let scope = word_characters(move |_, pos| (pos >= from).then(|| Arc::from("-")));
    let mut custom = EditorState::with_extensions(doc, scope).unwrap();
    let (_session, query) = query_at(&mut custom, from + "foo-b".len(), false);
    let result = complete_any_word(&query).unwrap();
    assert_eq!(result.from, from);
    assert_eq!(
        labels(&result),
        ["café", "𐐀elta", "Ⅻ_β", "foo-baz", "foo-bar"]
    );

    let (_session, query) = query_at(&mut custom, doc.len(), false);
    assert!(complete_any_word(&query).is_none());
    let (_session, query) = query_at(&mut custom, doc.len(), true);
    let result = complete_any_word(&query).unwrap();
    assert_eq!(result.from, doc.len());
    assert_eq!(
        labels(&result),
        ["café", "𐐀elta", "Ⅻ_β", "foo-bar", "foo-baz"]
    );

    let (_session, query) = query_at(&mut EditorState::new("last"), 2, false);
    let result = complete_any_word(&query).unwrap();
    assert_eq!(result.from, 0);
    assert!(result.options.is_empty());
}

#[test]
fn document_words_escape_language_chars_and_bound_the_prefix_and_result_size() {
    let doc = "x]子-\\ζ x]子-a";
    let mut state =
        EditorState::with_extensions(doc, word_characters(|_, _| Some(Arc::from("]-\\")))).unwrap();
    let (_session, query) = query_at(&mut state, doc.len(), false);
    let result = complete_any_word(&query).unwrap();
    assert_eq!(result.from, doc.find(' ').unwrap() + 1);
    assert_eq!(labels(&result), ["x]子-\\ζ"]);

    // Each Deseret letter takes two UTF-16 units and four UTF-8 bytes.
    let doc = "𐐀".repeat(150);
    let (_session, query) = query_at(&mut EditorState::new(doc.as_str()), doc.len(), false);
    assert_eq!(complete_any_word(&query).unwrap().from, 25 * "𐐀".len());

    let doc = (0..2100)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let (_session, query) = query_at(&mut EditorState::new(doc), 0, true);
    let result = complete_any_word(&query).unwrap();
    assert_eq!(result.options.len(), 2000);
    assert_eq!(result.options.first().unwrap().label, "w1");
    assert_eq!(result.options.last().unwrap().label, "w2000");
}

#[test]
fn document_word_result_reuses_its_candidates_while_text_starts_with_a_word() {
    let mut state = EditorState::new("f\nfar foo");
    let (mut session, query) = query_at(&mut state, 1, false);
    let now = Duration::from_millis(100);
    session
        .finish_query(&query, complete_any_word(&query), now)
        .unwrap();
    let type_text = |state: &mut EditorState, session: &mut CompletionSession, text| {
        let transaction = state
            .replace_selection(text)
            .unwrap()
            .with_user_event("input.type");
        let changes = transaction.changes().desc(state.text().len()).unwrap();
        state.update(transaction).unwrap();
        session
            .update(state, &changes, true, Some("input.type"), now)
            .unwrap();
    };
    type_text(&mut state, &mut session, "o");
    assert_eq!(session.deadline(), None);
    let options = session.options().unwrap();
    let [option] = options.as_slice() else {
        panic!("expected one completion")
    };
    assert_eq!(option.item.label, "foo");
    assert_eq!(option.range, 0..2);

    // The upstream `validFor` accepts `fo.` even though filtering finds no option.
    type_text(&mut state, &mut session, ".");
    assert!(session.options().unwrap().is_empty());
    assert_eq!(session.deadline(), None);
}
