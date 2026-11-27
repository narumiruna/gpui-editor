//! Pinned default EditorState indentation and raw V8 lint from `fixtures/json_oracle.mjs`.

use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, Extension, Precedence, Text, Transaction,
    commands::{
        Indentation, indent_selection_with_projection, insert_newline_and_indent_with_projection,
    },
    language::{
        IndentDecision, IndentOptions, StreamSettings, get_indentation, indent_service,
        json::{JsonDocument, json_parse_linter},
    },
    language_data::NativeSyntax,
};
use serde_json::{Value, json};

const FIXTURES: &str = include_str!("fixtures/json_cr_indent.jsonl");

#[test]
fn uses_editor_lines_for_indentation_but_raw_text_for_lint() {
    for line in FIXTURES.lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let name = case["name"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let doc = JsonDocument::parse(Text::new(source));
        assert_eq!(doc.text().as_str(), source, "{name}");

        check_indents(&doc, name, &case["indents"]);

        let lint: Vec<_> = json_parse_linter(doc.text())
            .iter()
            .map(|error| json!([error.range.start, error.range.end, "error", error.message]))
            .collect();
        assert_eq!(json!(lint), case["lint"], "raw lint: {name}");
        if name == "missing separator between mixed endings" {
            assert_ne!(json!(lint), case["editorLint"], "{name}");
        }
    }
}

// Pinned EditorState edits and indentation commands from fixtures/json_edit_indent.mjs.
#[test]
fn indentation_and_commands_match_during_edits_and_repairs() {
    for line in include_str!("fixtures/json_edit_indent.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let steps = case["steps"].as_array().unwrap();
        let mut syntax = NativeSyntax::Json(JsonDocument::parse(Text::new(
            steps[0]["source"].as_str().unwrap(),
        )));
        for step in steps {
            let NativeSyntax::Json(doc) = &mut syntax else {
                unreachable!();
            };
            let name = format!("{}/{}", case["name"], step["name"]);
            if let Some(change) = step.get("change") {
                let (from, to, insert): (usize, usize, String) =
                    serde_json::from_value(change.clone()).unwrap();
                let changes = ChangeSet::single(Change::new(from..to, insert)).unwrap();
                *doc = doc.reparse(&changes).unwrap();
                let fresh = JsonDocument::parse(doc.text().clone());
                check_indents(&fresh, &name, &step["indents"]);
            }
            let source = step["source"].as_str().unwrap();
            assert_eq!(doc.text().as_str(), source, "{name}");
            check_indents(doc, &name, &step["indents"]);

            let mut state = EditorState::new(source);
            let end = source.len();
            state
                .update(Transaction::default().with_selection(EditorSelection::single(end, end)))
                .unwrap();
            let edit = insert_newline_and_indent_with_projection(
                &state,
                doc.syntax_lines(),
                Indentation::default(),
                |pos, settings, options| doc.indentation(pos, settings, options),
                |_| None,
                "\n",
            )
            .unwrap()
            .unwrap();
            state.update(edit).unwrap();
            let result = state.text().as_str();
            let suffix = result.strip_prefix(source).unwrap();
            let indent = suffix
                .strip_prefix("\r\n")
                .or_else(|| suffix.strip_prefix(['\r', '\n']));
            assert_eq!(
                indent,
                step["enter"].as_str().unwrap().strip_prefix('\n'),
                "{name}"
            );
            assert_eq!(state.selection().main().head(), result.len(), "{name}");

            check_selection(&syntax, &name, step["selection"].as_str().unwrap(), false);
            if let Some(wanted) = step["serviceSelection"].as_str() {
                check_selection(&syntax, &name, wanted, true);
            }
        }
    }
}

fn check_selection(syntax: &NativeSyntax, name: &str, wanted: &str, with_services: bool) {
    let NativeSyntax::Json(doc) = syntax else {
        unreachable!();
    };
    let source = doc.text().as_str();
    let mut state = if with_services {
        let second_line = doc.syntax_lines().line(2).unwrap().range.start;
        let at_second_line = |column| {
            indent_service(move |_, cx| {
                if cx.position() == second_line {
                    IndentDecision::Column(column)
                } else {
                    IndentDecision::Defer
                }
            })
        };
        EditorState::with_extensions(
            source,
            Extension::group([
                at_second_line(3),
                at_second_line(7).with_precedence(Precedence::High),
            ]),
        )
        .unwrap()
    } else {
        EditorState::new(source)
    };
    state
        .update(Transaction::default().with_selection(EditorSelection::single(0, source.len())))
        .unwrap();
    let edit = indent_selection_with_projection(
        &state,
        doc.syntax_lines(),
        Indentation::default(),
        |pos, _, options| get_indentation(&state, pos, options, Some(syntax)).unwrap(),
    )
    .unwrap()
    .unwrap();
    state.update(edit).unwrap();
    assert_eq!(
        state.text().as_str(),
        wanted,
        "{name}, services: {with_services}"
    );
}

fn check_indents(doc: &JsonDocument, name: &str, queries: &Value) {
    for query in queries.as_array().unwrap() {
        let (pos, regular, split, double, overridden): (usize, usize, usize, usize, usize) =
            serde_json::from_value(query.clone()).unwrap();
        let line_start = doc.syntax_lines().line_at(pos).unwrap().range.start;
        let override_line = |from| (from == line_start).then_some(7);
        for (expected, options) in [
            (regular, IndentOptions::default()),
            (
                split,
                IndentOptions {
                    simulate_break: Some(pos),
                    ..IndentOptions::default()
                },
            ),
            (
                double,
                IndentOptions {
                    simulate_break: Some(pos),
                    simulate_double_break: true,
                    ..IndentOptions::default()
                },
            ),
            (
                overridden,
                IndentOptions {
                    override_indentation: Some(&override_line),
                    ..IndentOptions::default()
                },
            ),
        ] {
            assert_eq!(
                doc.indentation(pos, StreamSettings::default(), options),
                Some(expected),
                "{name} byte {pos}"
            );
        }
    }
}
