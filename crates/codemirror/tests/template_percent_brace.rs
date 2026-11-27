use gpui_editor_codemirror::{
    EditError, EditorSelection, EditorState, History, READ_ONLY, SelectionRange, Transaction,
    language::{jinja, liquid},
};
use serde_json::Value;

fn state(doc: &str, selection: EditorSelection, read_only: bool) -> EditorState {
    let mut state = EditorState::with_extensions(doc, READ_ONLY.of(read_only)).unwrap();
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    state
}

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn selection(value: &Value) -> EditorSelection {
    let ranges = value["ranges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|range| SelectionRange::new(byte(&range[0]), byte(&range[1])))
        .collect();
    EditorSelection::new(ranges, byte(&value["main"])).unwrap()
}

#[test]
fn matches_jinja_and_liquid_dom_typing() {
    // Input.insertText in headless Chromium against both verified npm tarballs.
    // The fixture records the handled transaction, not the browser fallback.
    // Offsets were converted from CodeMirror positions to native UTF-8 bytes.
    for line in include_str!("fixtures/template_percent_brace.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let name = case["name"].as_str().unwrap();
        let state = state(
            case["doc"].as_str().unwrap(),
            selection(&case),
            case["read_only"].as_bool().unwrap(),
        );
        let typed = case["typed"].as_str().unwrap();
        let expected = &case["transaction"];
        for result in [
            jinja::close_percent_brace(&state, typed),
            liquid::close_percent_brace(&state, typed),
        ] {
            let result = result.unwrap();
            if expected.is_null() {
                assert!(result.is_none(), "{name}");
                continue;
            }
            let transaction = result.unwrap_or_else(|| panic!("unhandled: {name}"));
            assert_eq!(
                transaction.user_event(),
                expected["user_event"].as_str(),
                "{name}"
            );
            assert_eq!(
                transaction.scroll_into_view(),
                expected["scroll_into_view"].as_bool().unwrap(),
                "{name}",
            );
            let changes: Vec<_> = expected["changes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|edit| (byte(&edit[0])..byte(&edit[1]), edit[2].as_str().unwrap()))
                .collect();
            assert_eq!(
                transaction
                    .changes()
                    .changes()
                    .map(|(range, insert)| (range.clone(), insert))
                    .collect::<Vec<_>>(),
                changes,
                "{name}",
            );
            let expected_selection = selection(expected);
            assert_eq!(transaction.selection(), Some(&expected_selection), "{name}");
            let mut after = state.clone();
            after.update(transaction).unwrap();
            assert_eq!(
                after.text().as_str(),
                expected["doc"].as_str().unwrap(),
                "{name}"
            );
            assert_eq!(after.selection(), &expected_selection, "{name}");
        }
    }
}

#[test]
fn scoped_typing_uses_full_document_positions_and_one_undo() {
    let prefix = "😀\r\n";
    let start = prefix.len();
    let document = format!("{prefix}{{}}é!");
    let original = EditorSelection::new(
        vec![
            SelectionRange::new(start + 1, start + 1),
            SelectionRange::new(start + 4, start + 2),
        ],
        0,
    )
    .unwrap();
    let mut history = History::new(state(&document, original.clone(), false));
    let transaction = liquid::close_percent_brace_in(history.state(), start..start + 4, "%")
        .unwrap()
        .unwrap();
    history.update(transaction, (), |_| (None, ())).unwrap();
    assert_eq!(
        history.state().text().as_str(),
        format!("{prefix}{{%%}}%%!")
    );
    assert_eq!(
        history.state().selection().ranges(),
        [
            SelectionRange::new(start + 2, start + 2),
            SelectionRange::new(start + 5, start + 5),
        ],
    );
    assert_eq!(history.undo(), Some(()));
    assert_eq!(history.state().text().as_str(), document);
    assert_eq!(history.state().selection(), &original);
    assert_eq!(history.undo(), None);
}

#[test]
fn scoped_typing_requires_both_braces_and_all_selections() {
    let document = "😀{}é!";
    let primary = state(document, EditorSelection::single(5, 5), false);
    for scope in [0..4, 4..5, 5..6, 5..5, 6..document.len()] {
        assert!(
            jinja::close_percent_brace_in(&primary, scope, "%")
                .unwrap()
                .is_none()
        );
    }
    assert!(
        jinja::close_percent_brace_in(&primary, 4..6, "%")
            .unwrap()
            .is_some()
    );

    for secondary in [SelectionRange::new(0, 0), SelectionRange::new(9, 6)] {
        let selection =
            EditorSelection::new(vec![primary.selection().main(), secondary], 0).unwrap();
        let multiple = state(document, selection, false);
        assert!(
            jinja::close_percent_brace_in(&multiple, 4..8, "%")
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn invalid_scope_is_an_error_only_for_eligible_input() {
    let editor = state("😀{}", EditorSelection::single(5, 5), false);
    let reversed = std::ops::Range { start: 5, end: 4 };
    for scope in [1..6, 0..1, 0..7, reversed] {
        assert!(matches!(
            jinja::close_percent_brace_in(&editor, scope.clone(), "%"),
            Err(EditError::InvalidRange),
        ));
        assert!(
            jinja::close_percent_brace_in(&editor, scope, "%%")
                .unwrap()
                .is_none()
        );
    }
}
