use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Compartment, EditError, EditorSelection, EditorState, Extension,
    Range, RangeSet, SelectionRange, Text, Transaction,
    language::brackets::{BracketMatchingOptions, bracket_matching},
    language_data::{cpp_extension, java_extension, json_extension, wast_extension},
    setup::{basic_setup, minimal_setup},
    view::{MarkDecoration, keymap::KeyPlatform},
};
use serde_json::{Value, json};

fn marks(set: &RangeSet<MarkDecoration>) -> Value {
    json!(
        set.iter(0)
            .map(|mark| json!([mark.from, mark.to, mark.value.class()]))
            .collect::<Vec<_>>()
    )
}

#[test]
fn preset_renders_attached_syntax_matches_from_the_same_carets() {
    let mut differences = Vec::new();
    for line in include_str!("fixtures/setup_bracket_marks.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = &row["name"];
        let source = Text::new(row["source"].as_str().unwrap());
        let language = match row["mode"].as_str().unwrap() {
            "cpp" => cpp_extension(),
            "java" => java_extension(),
            "json" => json_extension(),
            "wast" => wast_extension(),
            _ => unreachable!(),
        };
        let observed = Arc::new(Mutex::new(Vec::new()));
        let slot = Compartment::new();
        let options = &row["options"];
        let mut options_config = BracketMatchingOptions {
            after_cursor: options["afterCursor"].as_bool(),
            brackets: options["brackets"].as_str().map(Arc::from),
            max_scan_distance: serde_json::from_value(options["maxScanDistance"].clone()).unwrap(),
            ..Default::default()
        };
        if options["render"] == true {
            let observed = observed.clone();
            let source = source.clone();
            options_config.render_match = Some(Arc::new(move |hit, state| {
                assert_eq!(state.text(), &source);
                let start = [hit.start.start, hit.start.end];
                let end = hit.end.as_ref().map(|span| [span.start, span.end]);
                observed.lock().unwrap().push(json!([
                    start,
                    end,
                    hit.matched,
                    state.selection().ranges().len()
                ]));
                let label = if hit.matched {
                    "syntax-match"
                } else {
                    "syntax-miss"
                };
                vec![MarkDecoration::new(label).range(hit.start.clone()).unwrap()]
            }));
        }
        let selection = EditorSelection::new(
            serde_json::from_value::<Vec<usize>>(row["carets"].clone())
                .unwrap()
                .into_iter()
                .map(|head| SelectionRange::new(head, head))
                .collect(),
            0,
        )
        .unwrap();
        let state = EditorState::create_from_text(
            source.clone(),
            Some(selection.clone()),
            Extension::group([
                ALLOW_MULTIPLE_SELECTIONS.of(true),
                language,
                slot.of(bracket_matching(options_config)),
            ]),
        )
        .unwrap();
        let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
        for step in ["before", "after"] {
            if step == "after" {
                editor
                    .view_mut()
                    .update(
                        Transaction::default()
                            .with_effects([slot.reconfigure(Extension::default())]),
                        (),
                        |_| (None, ()),
                    )
                    .unwrap();
            }
            let revision = editor.view().revision();
            let count = observed.lock().unwrap().len();
            let _ = editor.bracket_matches();
            assert_eq!(observed.lock().unwrap().len(), count);
            let actual = marks(&editor.decorations().unwrap().bracket_matches);
            if actual != row[step] {
                differences.push(format!(
                    "{name} {step}: received {actual}; expected {}",
                    row[step]
                ));
            }
            assert_eq!(editor.view().revision(), revision);
            assert_eq!(editor.view().state().text(), &source);
            assert_eq!(editor.view().state().selection(), &selection);
        }
        if json!(*observed.lock().unwrap()) != row["callbacks"] {
            differences.push(format!(
                "{name}: renderer received {:?}; expected {}",
                *observed.lock().unwrap(),
                row["callbacks"]
            ));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}

#[test]
fn minimal_preset_draws_no_bracket_marks_without_the_extension() {
    let state = EditorState::with_extensions("{\"a\":0}", json_extension()).unwrap();
    let mut editor = minimal_setup::<()>(state, KeyPlatform::Linux);
    assert!(editor.decorations().unwrap().bracket_matches.is_empty());
    let matching = bracket_matching(Default::default());
    editor
        .view_mut()
        .update(
            Transaction::default().with_effects([matching.append()]),
            (),
            |_| (None, ()),
        )
        .unwrap();
    assert_eq!(
        marks(&editor.decorations().unwrap().bracket_matches),
        json!([[0, 1, "cm-matchingBracket"], [6, 7, "cm-matchingBracket"]])
    );
}

#[test]
fn attached_syntax_does_not_let_a_renderer_split_a_scalar() {
    let config = bracket_matching(BracketMatchingOptions {
        render_match: Some(Arc::new(|_, _| {
            vec![Range::new(10, 11, MarkDecoration::new("invalid"))]
        })),
        ..Default::default()
    });
    let source = "(module \"😀\")";
    let state =
        EditorState::with_extensions(source, Extension::group([wast_extension(), config])).unwrap();
    let editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
    assert_eq!(editor.decorations().unwrap_err(), EditError::InvalidRange);
    assert_eq!(editor.view().state().text().as_str(), source);
}
