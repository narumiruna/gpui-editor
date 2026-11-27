use gpui_editor_codemirror::{
    EditorSelection, EditorState, History, SelectionRange, Text, Transaction,
    language::{
        INDENT_UNIT, IndentStyle,
        markdown::{MarkdownDialect, MarkdownDocument, MarkdownMarkupCommand, markdown_keymap},
    },
};
use serde_json::{Value, json};

#[test]
fn markdown_markup_transactions_match_the_pinned_multi_caret_commands() {
    let mut rows = include_str!("fixtures/markdown_markup.jsonl").lines();
    let keys: Value = serde_json::from_str(rows.next().unwrap()).unwrap();
    let bindings = markdown_keymap();
    assert_eq!(
        keys,
        json!(
            bindings
                .iter()
                .map(|binding| (&binding.key, binding.prevent_default))
                .collect::<Vec<_>>()
        )
    );
    assert_eq!(bindings[0].run, Some(MarkdownMarkupCommand::Continue));
    assert_eq!(bindings[1].run, Some(MarkdownMarkupCommand::DeleteBackward));
    for row in rows {
        let case: Value = serde_json::from_str(row).unwrap();
        let name = case["name"].as_str().unwrap();
        let ranges: Vec<[usize; 2]> = serde_json::from_value(case["selection"].clone()).unwrap();
        let main: usize = serde_json::from_value(case["main"].clone()).unwrap();
        let selection = EditorSelection::new(
            ranges
                .into_iter()
                .map(|[a, b]| SelectionRange::new(a, b))
                .collect(),
            main,
        )
        .unwrap();
        let source = case["source"].as_str().unwrap();
        let mut state = match case["unit"].as_str() {
            Some(unit) => EditorState::with_extensions(
                source,
                INDENT_UNIT.of(IndentStyle::parse(unit).unwrap()),
            )
            .unwrap(),
            None => EditorState::new(source),
        };
        state
            .update(Transaction::default().with_selection(selection.clone()))
            .unwrap();
        let doc =
            MarkdownDocument::parse_with_dialect(state.text().clone(), MarkdownDialect::Extended);
        let command = case["command"].as_str().unwrap();
        let plan = match command {
            "delete" => MarkdownMarkupCommand::DeleteBackward.run(&doc, &state),
            "tight" => doc.continue_markup_transaction(&state, false),
            _ => MarkdownMarkupCommand::Continue.run(&doc, &state),
        }
        .unwrap();
        assert_eq!(plan.is_some(), case["handled"], "{name}");
        if let Some(transaction) = plan {
            assert_eq!(json!(transaction.user_event()), case["event"], "{name}");
            assert_eq!(transaction.scroll_into_view(), case["scroll"], "{name}");
            let mut history = History::new(state);
            history.update(transaction, (), |_| (None, ())).unwrap();
            let updated = history.state();
            let heads: Vec<_> = updated
                .selection()
                .ranges()
                .iter()
                .map(|range| {
                    assert!(
                        range.is_empty() && updated.text().as_str().is_char_boundary(range.head())
                    );
                    range.head()
                })
                .collect();
            assert_eq!(
                updated.text().as_str(),
                case["output"].as_str().unwrap(),
                "{name}"
            );
            assert_eq!(json!(heads), case["selected"], "{name}");
            assert_eq!(
                json!(updated.selection().main_index()),
                case["primary"],
                "{name}"
            );
            assert_eq!(history.undo_depth(), 1);
            assert!(history.undo().is_some());
            assert_eq!(history.state().text().as_str(), source);
            assert_eq!(history.state().selection(), &selection);
        } else {
            assert_eq!(state.text().as_str(), source);
            assert_eq!(state.selection(), &selection);
        }
    }
}

#[test]
fn markup_transactions_do_not_use_a_stale_syntax_snapshot() {
    let state = EditorState::new("- x");
    let doc = MarkdownDocument::parse(Text::new("- y"));
    assert!(
        doc.continue_markup_transaction(&state, true)
            .unwrap()
            .is_none()
    );
    assert!(
        doc.delete_markup_backward_transaction(&state)
            .unwrap()
            .is_none()
    );
}
