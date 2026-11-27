use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    EditorState, Extension, Precedence,
    language::{
        fold::{fold_code, fold_service, foldable, folded_ranges},
        json::JsonDocument,
    },
    setup::basic_setup,
    view::keymap::KeyPlatform,
};
use serde_json::{Value, json};

type Calls = Arc<Mutex<Vec<Value>>>;
fn offset(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

#[test]
fn registered_fold_services_match_pinned_query_and_command_order() {
    for row in include_str!("fixtures/language_fold_service.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let chosen = case["chosen"].as_str().unwrap();
        let expected = case["range"].as_array().unwrap();
        let range = offset(&expected[0])..offset(&expected[1]);
        let text = case["doc"].as_str().unwrap().to_owned();
        let calls = Calls::default();
        let register = |name: &'static str| {
            let answer = (chosen == name).then(|| range.clone());
            let snapshot = text.clone();
            let calls = calls.clone();
            fold_service(move |state, start, end| {
                assert_eq!(snapshot, state.text().as_str());
                calls.lock().unwrap().push(json!([name, start, end]));
                answer.clone()
            })
        };
        let mut state = EditorState::with_extensions(
            text.as_str(),
            Extension::group([
                register("low").with_precedence(Precedence::Low),
                register("default"),
                register("high").with_precedence(Precedence::High),
                register("later"),
            ]),
        )
        .unwrap();
        let syntax = JsonDocument::parse(state.text().clone());
        let fallback = |_: &EditorState, start, end| {
            assert_eq!(chosen, "syntax");
            syntax.foldable(start, end)
        };
        assert_eq!(foldable(&state, 0, 1, fallback), Some(range));
        assert_eq!(
            json!(std::mem::take(&mut *calls.lock().unwrap())),
            case["queryCalls"]
        );
        let command = fold_code(&state, fallback);
        assert_eq!(json!(command.is_some()), case["handled"]);
        state.update(command.unwrap()).unwrap();
        assert_eq!(json!(*calls.lock().unwrap()), case["commandCalls"]);
        let folds: Vec<_> = folded_ranges(&state)
            .iter(0)
            .map(|fold| [fold.from, fold.to])
            .collect();
        assert_eq!(json!(folds), case["folded"]);
    }
}

#[test]
fn invalid_service_range_is_rejected_without_falling_through() {
    let state = EditorState::with_extensions(
        "猫\na",
        Extension::group([
            fold_service(|_, _, _| Some(1..5)).with_precedence(Precedence::High),
            fold_service(|_, _, _| {
                panic!("a lower-precedence service must not replace this result")
            }),
        ]),
    )
    .unwrap();
    assert!(foldable(&state, 0, 3, |_, _, _| panic!("must not ask syntax")).is_none());
    assert!(fold_code(&state, |_, _, _| panic!("must not ask syntax")).is_none());
    assert!(foldable(&state, 1, 3, |_, _, _| panic!("invalid input line")).is_none());
}

#[test]
fn basic_gutter_shows_and_toggles_a_service_fold_without_syntax() {
    let state = EditorState::with_extensions(
        "open\ninner\nclose",
        fold_service(|_, start, end| (start == 0 && end == 4).then_some(4..11)),
    )
    .unwrap();
    let mut editor = basic_setup::<()>(state, KeyPlatform::Mac).unwrap();
    let marker = |editor: &gpui_editor_codemirror::setup::SetupEditor<()>| {
        let slots = editor.gutters(1..=3, None).unwrap();
        let gutter = slots
            .iter()
            .find(|slot| slot.class == "cm-foldGutter")
            .unwrap();
        let [row] = gutter.rows.as_slice() else {
            panic!("expected one fold marker");
        };
        assert_eq!(row.line_number, 1);
        row.markers[0].content.clone()
    };
    assert_eq!(marker(&editor).as_deref(), Some("⌄"));
    assert!(
        editor
            .toggle_line_fold(1, None, std::time::Duration::ZERO, ())
            .unwrap()
    );
    assert_eq!(marker(&editor).as_deref(), Some("›"));
    assert!(
        editor
            .toggle_line_fold(1, None, std::time::Duration::ZERO, ())
            .unwrap()
    );
    assert_eq!(marker(&editor).as_deref(), Some("⌄"));
}
