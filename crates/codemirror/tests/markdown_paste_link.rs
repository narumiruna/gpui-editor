use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, LINE_SEPARATOR, READ_ONLY, SelectionRange,
    Transaction,
    language::markdown::{MarkdownDialect, MarkdownDocument, paste_url_as_link},
};
use serde_json::Value;

const URL: &str = "https://example.test/";

fn selection(ranges: &Value, main: usize) -> EditorSelection {
    let ranges = ranges
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            SelectionRange::new(
                pair[0].as_u64().unwrap().try_into().unwrap(),
                pair[1].as_u64().unwrap().try_into().unwrap(),
            )
        })
        .collect();
    EditorSelection::new(ranges, main).unwrap()
}

fn select(state: &mut EditorState, selection: EditorSelection) {
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
}

#[test]
fn clipboard_event_oracle() {
    // Recorded from dispatched paste events in jsdom 29.1.1 with CodeMirror
    // lang-markdown 6.5.2, language 6.13.1, view 6.44.0, state 6.7.6 and
    // Lezer Markdown 1.8.2. Ranges use native UTF-8 offsets; source CRLF is
    // retained when mapping from the oracle's normalized document.
    let fixture: Value = serde_json::from_str(include_str!("fixtures/markdown_paste_link.json"))
        .expect("valid clipboard fixture");
    for case in fixture.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let source = case["doc"].as_str().unwrap();
        let clipboard = case["clip"].as_str().unwrap_or(URL);
        let main = case["main"].as_u64().unwrap_or(0).try_into().unwrap();
        let mut state = if case["readOnly"] == true {
            EditorState::with_extensions(source, READ_ONLY.of(true)).unwrap()
        } else if let Some(separator) = case["separator"].as_str() {
            EditorState::with_extensions(source, LINE_SEPARATOR.of(separator.to_owned())).unwrap()
        } else {
            EditorState::new(source)
        };
        let initial = selection(&case["ranges"], main);
        let range = initial.main().range();
        select(&mut state, initial.clone());
        let dialect = if case["extended"] == true {
            MarkdownDialect::Extended
        } else {
            MarkdownDialect::CommonMark
        };
        let document = MarkdownDocument::parse_with_dialect(state.text().clone(), dialect);
        let actual = paste_url_as_link(&document, &state, clipboard);
        assert_eq!(state.text().as_str(), source, "{name}");
        assert_eq!(state.selection(), &initial, "{name}");

        let expected = &case["expected"];
        if expected.is_null() {
            assert!(actual.is_none(), "{name}: {actual:?}");
            continue;
        }
        let transaction = actual.unwrap_or_else(|| panic!("{name}: expected a link paste"));
        assert_eq!(transaction.user_event(), Some("input.paste"), "{name}");
        assert!(transaction.scroll_into_view(), "{name}");
        assert!(transaction.selection().is_none(), "{name}");
        assert_eq!(
            transaction.changes().changes().collect::<Vec<_>>(),
            [
                (&(range.start..range.start), "["),
                (
                    &(range.end..range.end),
                    expected["suffix"].as_str().unwrap()
                )
            ],
            "{name}"
        );
        state.update(transaction).unwrap();
        assert_eq!(
            state.text().as_str(),
            expected["doc"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            state.selection(),
            &selection(&expected["ranges"], main),
            "{name}"
        );
        if let Some(serialized) = expected["serialized"].as_str() {
            assert_eq!(state.slice_doc(0..state.text().len()).unwrap(), serialized);
        }
    }
}

#[test]
fn stale_parse_is_rejected_even_when_the_length_is_unchanged() {
    let mut state = EditorState::new("old");
    select(&mut state, EditorSelection::single(0, 3));
    let document = MarkdownDocument::parse(state.text().clone());
    let changes = ChangeSet::single(Change::new(0..3, "new")).unwrap();
    state
        .update(Transaction::new(changes).with_selection(EditorSelection::single(0, 3)))
        .unwrap();
    assert!(paste_url_as_link(&document, &state, URL).is_none());
}

#[test]
fn does_not_split_crlf() {
    let mut state = EditorState::new("one\r\ntwo");
    let document = MarkdownDocument::parse(state.text().clone());
    for (anchor, head) in [(4, 6), (2, 4)] {
        select(&mut state, EditorSelection::single(anchor, head));
        assert!(paste_url_as_link(&document, &state, URL).is_none());
    }
}
