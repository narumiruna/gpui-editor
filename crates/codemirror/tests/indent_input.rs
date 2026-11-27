use std::num::NonZeroUsize;

use gpui_editor_codemirror::{
    EditorSelection, EditorState, History, SelectionRange, Transaction,
    commands::{IndentUnit, Indentation},
    language_data::{LANGUAGES, LanguageDescription, NativeLanguage},
};
use serde_json::{Value, json};

#[test]
fn reindents_input_on_a_lone_cr_line_in_one_undo_step() {
    for (language, fixture) in [
        (
            NativeLanguage::Go,
            include_str!("fixtures/go_editor_input.jsonl"),
        ),
        (
            NativeLanguage::Json,
            include_str!("fixtures/json_editor_input.jsonl"),
        ),
        (
            NativeLanguage::LezerGrammar,
            include_str!("fixtures/lezer_editor_input.jsonl"),
        ),
        (
            NativeLanguage::Yaml,
            include_str!("fixtures/yaml_editor_input.jsonl"),
        ),
    ] {
        assert_mixed_lines_input(language, fixture);
    }
}

fn assert_mixed_lines_input(language: NativeLanguage, fixture: &str) {
    let extension = language.support().indent_on_input(Indentation::default());
    for row in fixture.lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let name = case["name"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let caret: usize = serde_json::from_value(case["caret"].clone()).unwrap();
        let mut state = EditorState::with_extensions(source, extension.clone()).unwrap();
        state
            .update(Transaction::default().with_selection(EditorSelection::single(caret, caret)))
            .unwrap();
        let mut history = History::new(state);
        let input = history
            .state()
            .replace_selection(case["typed"].as_str().unwrap())
            .unwrap()
            .with_user_event(case["event"].as_str().unwrap());
        history.update(input, (), |_| (None, ())).unwrap();
        assert_eq!(
            history.state().text().as_str(),
            case["result"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            json!(history.state().selection().main().head()),
            case["finalCaret"],
            "{name}"
        );
        assert_eq!(history.undo_depth(), 1);
        assert!(history.undo().is_some());
        assert_eq!(history.state().text().as_str(), source, "{name}");
        assert_eq!(history.state().selection().main().head(), caret, "{name}");
    }
}

fn byte_at(value: &Value, text: &str) -> usize {
    let target = usize::try_from(value.as_u64().unwrap()).unwrap();
    let mut units = 0;
    for (at, ch) in text.char_indices() {
        if target == units {
            return at;
        }
        units += ch.len_utf16();
    }
    assert_eq!(target, units);
    text.len()
}

fn positions(value: &Value, text: &str) -> Vec<usize> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|at| byte_at(at, text))
        .collect()
}

#[test]
fn reindent_on_input_matches_pinned_language_transactions() {
    let mut failures = Vec::new();
    for line in include_str!("fixtures/indent_input.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let name = case["language"].as_str().unwrap();
        let source = case["doc"].as_str().unwrap();
        let result = case["result"].as_str().unwrap();
        let language = LanguageDescription::match_name(LANGUAGES, name, false)
            .unwrap()
            .built_in()
            .unwrap();
        let unit = if case["unit"] == "\t" {
            IndentUnit::Tab
        } else {
            IndentUnit::Spaces(NonZeroUsize::new(2).unwrap())
        };
        let extension = language.support().indent_on_input(Indentation {
            unit,
            ..Indentation::default()
        });
        let mut state = EditorState::with_extensions(source, extension).unwrap();
        let ranges = positions(&case["positions"], source)
            .into_iter()
            .map(|at| SelectionRange::new(at, at))
            .collect();
        let selection = EditorSelection::new(ranges, 0).unwrap();
        state
            .update(Transaction::default().with_selection(selection))
            .unwrap();
        let mut history = History::new(state);
        let transaction = history
            .state()
            .replace_selection(case["typed"].as_str().unwrap())
            .unwrap()
            .with_user_event(case["event"].as_str().unwrap());
        history.update(transaction, (), |_| (None, ())).unwrap();
        let actual = history.state().text().as_str();
        if actual != result {
            failures.push(format!("{name} {source:?}: {actual:?} != {result:?}"));
        } else {
            let carets: Vec<_> = history
                .state()
                .selection()
                .ranges()
                .iter()
                .map(|range| range.head())
                .collect();
            assert_eq!(
                carets,
                positions(&case["carets"], result),
                "{name} {source:?}"
            );
        }
        assert_eq!(history.undo_depth(), 1);
        assert!(history.undo().is_some());
        assert_eq!(history.state().text().as_str(), source);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
