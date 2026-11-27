use gpui_editor_codemirror::{
    EditError, EditorSelection, EditorState, History, READ_ONLY, SelectionRange, Transaction,
    language_data::NativeLanguage,
};
use serde_json::Value;

const LANGUAGES: [NativeLanguage; 3] = [
    NativeLanguage::Jinja,
    NativeLanguage::Liquid,
    NativeLanguage::Php,
];

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
fn matches_default_mounted_html_typing_in_chromium() {
    for line in include_str!("fixtures/native_mounted_input.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        let language = match row["language"].as_str().unwrap() {
            "jinja" => NativeLanguage::Jinja,
            "liquid" => NativeLanguage::Liquid,
            "php" => NativeLanguage::Php,
            _ => unreachable!(),
        };
        let label = format!("{language:?}/{name}");
        let outcome = &row["outcome"];
        let mut state = state(
            row["doc"].as_str().unwrap(),
            selection(&row),
            row["read_only"].as_bool().unwrap_or(false),
        );
        // The native API is for default direct input. The host does not call it
        // during composition or when the input handler is disabled.
        if matches!(name, "ime" | "handler-disabled") {
            assert_eq!(outcome["batches"].as_array().unwrap().len(), 1, "{label}");
            assert_eq!(
                outcome["batches"][0].as_array().unwrap().len(),
                1,
                "{label}"
            );
            continue;
        }
        let result = language
            .support()
            .auto_close_tags(&state, row["typed"].as_str().unwrap())
            .unwrap();
        if row["native_skip"].is_string() || outcome["batches"][0][1] != "input.complete" {
            assert!(result.is_none(), "unexpected close: {label}");
            continue;
        }
        assert_eq!(
            outcome["batches"],
            serde_json::json!([["input.type", "input.complete"]])
        );
        let transaction = result.unwrap_or_else(|| panic!("not handled: {label}"));
        assert_eq!(transaction.user_event(), Some("input.complete"), "{label}");
        assert!(transaction.scroll_into_view(), "{label}");
        let expected = selection(outcome);
        assert_eq!(transaction.selection(), Some(&expected), "{label}");
        state.update(transaction).unwrap();
        assert_eq!(state.text().as_str(), outcome["doc"], "{label}");
        assert_eq!(state.selection(), &expected, "{label}");
    }
}

#[test]
fn scoped_input_keeps_the_suffix_and_uses_full_document_coordinates() {
    let prefix = "😀\r\n```\r\n";
    let suffix = ">α\r\n```";
    for (language, body) in LANGUAGES
        .into_iter()
        .zip(["{{ é }}", "{{ é }}", "<?= 'é' ?>"])
    {
        let doc = format!("{prefix}{body}<Élément{suffix}");
        let start = prefix.len();
        let end = start + body.len() + "<Élément".len();
        let caret = EditorSelection::single(end, end);
        let mut history = History::new(state(&doc, caret.clone(), false));
        let transaction = language
            .support()
            .auto_close_tags_in(history.state(), start..end, ">")
            .unwrap()
            .unwrap();
        assert_eq!(
            transaction.changes().changes().collect::<Vec<_>>(),
            [(&(end..end), "></Élément>")],
        );
        history.update(transaction, (), |_| (None, ())).unwrap();
        assert_eq!(
            history.state().text().as_str(),
            format!("{prefix}{body}<Élément></Élément>{suffix}")
        );
        assert_eq!(
            history.state().selection(),
            &EditorSelection::single(end + 1, end + 1)
        );
        assert_eq!(history.undo(), Some(()));
        assert_eq!(history.state().text().as_str(), doc);
        assert_eq!(history.state().selection(), &caret);
        assert_eq!(history.undo(), None);
    }
}

#[test]
fn scoped_input_rejects_invalid_ranges_and_out_of_scope_selections() {
    let doc = "😀<é>😀";
    let main = SelectionRange::new(7, 7);
    let editor = state(doc, EditorSelection::single(7, 7), false);
    let support = NativeLanguage::Jinja.support();
    let reversed = std::ops::Range { start: 7, end: 4 };
    for scope in [1..7, 4..6, 4..9, 4..13, reversed] {
        assert!(matches!(
            support.auto_close_tags_in(&editor, scope.clone(), ">"),
            Err(EditError::InvalidRange)
        ));
        assert!(
            support
                .auto_close_tags_in(&editor, scope, ">>")
                .unwrap()
                .is_none()
        );
    }
    for outside in [SelectionRange::new(0, 0), SelectionRange::new(12, 8)] {
        let (ranges, main_index) = if outside.head() < main.head() {
            (vec![outside, main], 1)
        } else {
            (vec![main, outside], 0)
        };
        let editor = state(
            doc,
            EditorSelection::new(ranges, main_index).unwrap(),
            false,
        );
        assert!(
            support
                .auto_close_tags_in(&editor, 4..7, ">")
                .unwrap()
                .is_none()
        );
    }
    assert!(
        support
            .auto_close_tags_in(&editor, 7..7, ">")
            .unwrap()
            .is_none()
    );
    assert!(
        NativeLanguage::Html
            .support()
            .auto_close_tags(&editor, ">")
            .unwrap()
            .is_none()
    );
}
