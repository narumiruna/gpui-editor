use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Compartment, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, Facet, LINE_SEPARATOR, Precedence, SelectionRange, StateField, Text,
};
use serde_json::{Value, json};

fn oracle() -> Value {
    serde_json::from_str(include_str!("fixtures/state_create.json")).unwrap()
}

fn line(separator: &str) -> Extension {
    LINE_SEPARATOR.of(separator.to_owned())
}

fn selection() -> EditorSelection {
    EditorSelection::new(
        vec![SelectionRange::new(0, 0), SelectionRange::new(5, 3)],
        1,
    )
    .unwrap()
}

fn describe(state: &EditorState) -> Value {
    let text = state.text();
    json!({
        "text": text.as_str(),
        "lines": text.to_lines(),
        "line_break": state.line_break(),
        "display": state.slice_doc(0..text.len()).unwrap(),
        "multiple": state.allows_multiple_selections(),
        "selection": state.selection().to_json(),
    })
}

#[test]
fn create_matches_pinned_defaults_and_explicit_policy() {
    let manifest: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let pin = manifest["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pin| pin["name"] == "@codemirror/state")
        .unwrap();
    let expected = oracle();
    for key in ["name", "version", "integrity"] {
        assert_eq!(expected["pin"][key], pin[key]);
    }

    for (case, source, extension) in [
        ("empty", "", Extension::default()),
        ("default_lines", "a\r\nb\rc\nd", Extension::default()),
        ("empty_separator", "a\r\nb\rc\nd", line("")),
    ] {
        let state = EditorState::create(source, None, extension).unwrap();
        let mut actual = describe(&state);
        assert_eq!(actual["multiple"], true, "{case}");
        actual["multiple"] = json!(false); // The native default retains multiple ranges.
        assert_eq!(actual, expected[case], "{case}");
    }

    let native = EditorState::create("abcdef", Some(selection()), Extension::default()).unwrap();
    assert_eq!(native.selection(), &selection());
    let single = EditorState::create(
        "abcdef",
        Some(selection()),
        ALLOW_MULTIPLE_SELECTIONS.of(false),
    )
    .unwrap();
    assert_eq!(describe(&single), expected["default_multiple"]);
    let conflict = EditorState::create(
        "abcdef",
        Some(selection()),
        Extension::group([
            ALLOW_MULTIPLE_SELECTIONS
                .of(false)
                .with_precedence(Precedence::High),
            ALLOW_MULTIPLE_SELECTIONS.of(true),
        ]),
    )
    .unwrap();
    assert_eq!(describe(&conflict), expected["allow_conflict"]);

    let mixed = EditorState::create("a\nb\r\nc", None, line("\r\n")).unwrap();
    assert_eq!(mixed.text().as_str(), expected["unmatched_lf"]["text"]);
    assert_eq!(expected["unmatched_lf"]["lines"], json!(["a\nb", "c"]));
    assert_eq!(mixed.text().to_lines(), ["a", "b", "c"]);
}

#[test]
fn create_resolves_precedence_and_runs_callbacks_in_pinned_order() {
    let events = Arc::new(Mutex::new(Vec::<Value>::new()));
    let setting = Facet::new({
        let events = events.clone();
        move |inputs: &[&&str]| {
            let labels = inputs.iter().map(|&&value| value).collect::<Vec<_>>();
            if !labels.is_empty() {
                let mut event = vec![json!("static")];
                event.extend(labels.iter().map(|label| json!(label)));
                events.lock().unwrap().push(json!(event));
            }
            labels.join(",")
        }
    });
    let dynamic = Facet::new({
        let events = events.clone();
        move |inputs: &[&String]| {
            let value = inputs.first().map(|value| (*value).clone());
            if let Some(value) = &value {
                events
                    .lock()
                    .unwrap()
                    .push(json!(["dynamic-combine", value]));
            }
            value
        }
    });
    let second = StateField::new(
        {
            let (events, setting) = (events.clone(), setting.reader());
            move |state| {
                events
                    .lock()
                    .unwrap()
                    .push(json!(["second", &*state.facet(&setting)?]));
                Ok("second")
            }
        },
        |_, _| panic!("creation must not run field updates"),
    );
    let first = StateField::<&str>::new(
        |_| panic!("default initializer ran"),
        |_, _| panic!("creation must not run field updates"),
    );
    let high = setting.of("high");
    let slot = Compartment::new();
    let content = Extension::group([
        high.clone(),
        line("::"),
        first.init({
            let events = events.clone();
            move |state| {
                events.lock().unwrap().push(json!([
                    "first",
                    state.text().as_str(),
                    state.selection().to_json(),
                    state.facet(&LINE_SEPARATOR)?.as_deref(),
                ]));
                Ok("first")
            }
        }),
    ])
    .with_precedence(Precedence::High);
    let state = EditorState::create(
        "aa::b|c::",
        Some(selection()),
        Extension::group([
            dynamic.compute_with_dependencies(
                |deps| {
                    deps.field(&first);
                    deps.field(&second);
                },
                {
                    let (events, first, second) = (events.clone(), first.reader(), second.reader());
                    move |state| {
                        events
                            .lock()
                            .unwrap()
                            .push(json!(["dynamic-compute", state.selection().ranges().len()]));
                        Ok(format!(
                            "{}:{}",
                            state.field(&first)?.unwrap(),
                            state.field(&second)?.unwrap()
                        ))
                    }
                },
            ),
            second.extension(),
            setting.of("low"),
            high,
            line("|"),
            first.init(|_| panic!("lower precedence initializer ran")),
            slot.of(content),
            ALLOW_MULTIPLE_SELECTIONS.of(false),
        ]),
    )
    .unwrap();
    assert!(state.compartment(&slot).is_some());
    let mut actual = describe(&state);
    actual["dynamic"] = json!(&*state.context().facet(&dynamic).unwrap());
    actual["events"] = json!(*events.lock().unwrap());
    assert_eq!(actual, oracle()["callbacks"]);
}

#[test]
fn create_checks_all_endpoints_before_initializing_fields() {
    let events = Arc::new(Mutex::new(Vec::<&str>::new()));
    let setting = Facet::new({
        let events = events.clone();
        move |inputs: &[&bool]| {
            if !inputs.is_empty() {
                events.lock().unwrap().push("static");
            }
        }
    });
    let field = StateField::<()>::new(|_| panic!("invalid selection reached field"), |_, _| Ok(()));
    let extensions = Extension::group([
        setting.of(true),
        field.extension(),
        ALLOW_MULTIPLE_SELECTIONS.of(false),
    ]);
    // Byte 4 was valid in the input, but not after CRLF normalization.
    let past_end = EditorSelection::new(
        vec![SelectionRange::new(0, 0), SelectionRange::new(4, 4)],
        0,
    )
    .unwrap();
    assert_eq!(
        EditorState::create("a\r\nb", Some(past_end), extensions.clone()).unwrap_err(),
        EditError::InvalidSelection
    );
    let expected = oracle();
    assert_eq!(expected["invalid_selection"]["error"], "RangeError");
    assert_eq!(
        json!(*events.lock().unwrap()),
        expected["invalid_selection"]["events"]
    );

    // A non-main endpoint inside a UTF-8 scalar must not disappear before validation.
    let interior = EditorSelection::new(
        vec![SelectionRange::new(0, 0), SelectionRange::new(5, 2)],
        0,
    )
    .unwrap();
    assert_eq!(
        EditorState::create_from_text(Text::new("😀x"), Some(interior), extensions).unwrap_err(),
        EditError::InvalidSelection
    );
    let valid = EditorState::create(
        "😀\r\nx",
        Some(EditorSelection::single(6, 4)),
        Extension::default(),
    )
    .unwrap();
    assert_eq!(valid.selection().main(), SelectionRange::new(6, 4));
}

#[test]
fn create_from_text_keeps_bytes_and_shares_storage() {
    let text = Text::from_lines(&["a|b", "c"]);
    let created = EditorState::create_from_text(
        text.clone(),
        Some(EditorSelection::single(2, 2)),
        Extension::group([line("|"), ALLOW_MULTIPLE_SELECTIONS.of(false)]),
    )
    .unwrap();
    let mut actual = describe(&created);
    actual["same_text"] = json!(std::ptr::eq(text.as_str(), created.text().as_str()));
    assert_eq!(actual, oracle()["persistent"]);

    let source = "😀|b\r\nc\rd".repeat(4096);
    let text = Text::new(source.clone());
    let direct = EditorState::from_text(text.clone());
    let field = StateField::new(|state| Ok(state.text().clone()), |text, _| Ok(text.clone()));
    let created = EditorState::create_from_text(
        text.clone(),
        None,
        Extension::group([line("|"), field.extension()]),
    )
    .unwrap();
    let field_text = created.context().field(&field).unwrap().unwrap();
    assert_eq!(created.text().as_str(), source);
    for shared in [direct.text(), created.text(), &field_text] {
        assert!(std::ptr::eq(text.as_str(), shared.as_str()));
    }
}

#[test]
fn create_rejects_computed_initial_policies_without_running_them() {
    let expected = EditError::Configuration(ConfigurationError::DynamicInitialPolicy);
    let dynamic_separator = LINE_SEPARATOR.compute(|_| panic!("computed initial separator"));
    let mixed = Extension::group([
        line("|").with_precedence(Precedence::Highest),
        dynamic_separator,
    ]);
    assert_eq!(
        EditorState::create("a|b", None, mixed).unwrap_err(),
        expected
    );
    let dynamic_selection =
        ALLOW_MULTIPLE_SELECTIONS.compute(|_| panic!("computed initial selection policy"));
    assert_eq!(
        EditorState::create_from_text(Text::new("abcdef"), Some(selection()), dynamic_selection)
            .unwrap_err(),
        expected
    );

    let text = "a|b\r\nc";
    let extensions = LINE_SEPARATOR.compute(move |state| {
        assert_eq!(state.text().as_str(), text);
        Ok("|".to_owned())
    });
    let legacy = EditorState::with_extensions(text, extensions.clone()).unwrap();
    let persistent = EditorState::create_from_text(Text::new(text), None, extensions).unwrap();
    for state in [legacy, persistent] {
        assert_eq!(state.text().as_str(), text);
        assert_eq!(state.line_break(), "|");
    }
    assert_eq!(EditorState::new(text).text().as_str(), text);
}
