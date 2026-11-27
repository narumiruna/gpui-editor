use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use gpui_editor_codemirror::{
    EditorState, Extension, PHRASES, Transaction,
    language::fold::{
        FOLD_GUTTER_CLASS, FoldCommand, FoldGutterClick, FoldGutterConfig, FoldGutterMarker,
        FoldGutterState, code_folding, fold_effect, fold_gutter, fold_keymap, fold_service,
        folded_ranges, toggle_fold, unfold_effect,
    },
    view::{
        ANNOUNCE,
        gutters::{GutterMarker, gutter_slots},
    },
};
use serde_json::{Value, json};

fn byte(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn span(value: &Value) -> Range<usize> {
    byte(&value[0])..byte(&value[1])
}

fn state(case: &Value, extension: Extension) -> EditorState {
    let phrases = PHRASES.of(Arc::new(
        [
            ("Folded lines".into(), "Collapsed".into()),
            ("Unfolded lines".into(), "Expanded".into()),
            ("to".into(), "through".into()),
            ("Fold line".into(), "Hide source".into()),
            ("Unfold line".into(), "Show source".into()),
        ]
        .into(),
    ));
    let region = span(&case["region"]);
    let service = fold_service(move |_, from, _| (from == 0).then(|| region.clone()));
    EditorState::with_extensions(
        case["doc"].as_str().unwrap(),
        Extension::group([extension, phrases, service]),
    )
    .unwrap()
}

fn effects(transaction: &Transaction) -> Value {
    json!(
        transaction
            .effects()
            .iter()
            .filter_map(|effect| {
                if let Some(range) = effect.get(fold_effect()) {
                    Some(json!(["fold", [range.start, range.end]]))
                } else if let Some(range) = effect.get(unfold_effect()) {
                    Some(json!(["unfold", [range.start, range.end]]))
                } else {
                    effect
                        .get(&ANNOUNCE)
                        .map(|message| json!(["announce", message]))
                }
            })
            .collect::<Vec<_>>()
    )
}

#[test]
fn command_announcements_and_effect_order_match_the_pinned_view() {
    for row in include_str!("fixtures/language_folding_controls.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        if case["kind"] != "command" {
            continue;
        }
        let mut state = state(&case, code_folding());
        let name = case["name"].as_str().unwrap();
        if name.contains("unfold") {
            state
                .update(
                    Transaction::default().with_effects([fold_effect().of(span(&case["region"]))]),
                )
                .unwrap();
        }
        let fallback = |_: &EditorState, _, _| panic!("must not ask syntax on this line");
        let transaction = match name {
            "fold" => FoldCommand::FoldCode.run(&state, fallback),
            "unfold" => FoldCommand::UnfoldCode.run(&state, fallback),
            "toggle_fold" | "toggle_unfold" => toggle_fold(&state, fallback),
            "fold_all" => FoldCommand::FoldAll.run(&state, |_, _, _| None),
            "unfold_all" => FoldCommand::UnfoldAll.run(&state, fallback),
            _ => panic!("Unexpected command {name}"),
        };
        assert_eq!(json!(transaction.is_some()), case["handled"], "{name}");
        assert_eq!(effects(&transaction.unwrap()), case["effects"], "{name}");
    }
}

#[test]
fn fold_keymap_matches_the_pinned_order_and_platform_overrides() {
    let expected = include_str!("fixtures/language_folding_controls.jsonl")
        .lines()
        .map(|row| serde_json::from_str::<Value>(row).unwrap())
        .find(|row| row["kind"] == "keymap")
        .unwrap();
    let bindings: Vec<_> = fold_keymap()
        .into_iter()
        .map(|binding| {
            let command = match binding.run.unwrap() {
                FoldCommand::FoldCode => "foldCode",
                FoldCommand::UnfoldCode => "unfoldCode",
                FoldCommand::FoldAll => "foldAll",
                FoldCommand::UnfoldAll => "unfoldAll",
            };
            assert!(binding.windows.is_none() && binding.linux.is_none());
            json!([binding.key, binding.mac, command])
        })
        .collect();
    assert_eq!(json!(bindings), expected["bindings"]);
}

fn marker_json(marker: FoldGutterMarker) -> Value {
    json!([
        marker.marker.content,
        marker.title,
        marker.marker.element_class
    ])
}

fn markers(state: &EditorState, config: &FoldGutterConfig) -> Value {
    let slots = gutter_slots(state, 1..=state.text().line_count()).unwrap();
    let [slot] = slots.as_slice() else {
        panic!("Expected only a fold gutter");
    };
    let [row] = slot.rows.as_slice() else {
        panic!("Expected one fold marker");
    };
    assert_eq!(slot.class, FOLD_GUTTER_CLASS);
    assert_eq!(row.from, 0);
    let spacer = config.spacer(state);
    let marker = config.marker(state, 0..4, |_, _, _| None).unwrap();
    assert_eq!(slot.spacer.as_ref(), Some(&spacer.marker));
    assert_eq!(row.markers, std::slice::from_ref(&marker.marker));
    json!([marker_json(spacer), marker_json(marker)])
}

#[test]
fn configured_gutter_markers_and_clicks_follow_the_pinned_view() {
    for row in include_str!("fixtures/language_folding_controls.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        if case["kind"] != "gutter" {
            continue;
        }
        let intercept = Arc::new(AtomicBool::new(false));
        let handler = intercept.clone();
        let mut config = FoldGutterConfig {
            open_text: "collapse".into(),
            closed_text: "expand".into(),
            handle_click: Some(Arc::new(move |_, _| handler.load(Ordering::Relaxed))),
            ..FoldGutterConfig::default()
        };
        if case["custom"] == true {
            config.render_marker = Some(Arc::new(|_, kind| GutterMarker {
                element_class: "custom-fold".into(),
                content: Some(
                    match kind {
                        FoldGutterState::Open => "expanded",
                        FoldGutterState::Closed => "collapsed",
                    }
                    .into(),
                ),
            }));
        }
        let mut state = state(&case, fold_gutter(config.clone()));
        assert_eq!(markers(&state, &config), case["before"]);
        for step in case["steps"].as_array().unwrap() {
            let name = step["name"].as_str().unwrap();
            intercept.store(name == "intercept", Ordering::Relaxed);
            let result = config.click(&state, span(&step["line"]), |_, _, _| None);
            if name == "intercept" {
                assert!(matches!(result, FoldGutterClick::Handled));
                assert_eq!(step["effects"], json!([]));
            } else {
                let FoldGutterClick::Dispatch(transaction) = result else {
                    panic!("No transaction for {name}");
                };
                assert_eq!(effects(&transaction), step["effects"]);
                state.update(transaction).unwrap();
            }
            let folds: Vec<_> = folded_ranges(&state)
                .iter(0)
                .map(|fold| [fold.from, fold.to])
                .collect();
            assert_eq!(json!(folds), step["folds"]);
            assert_eq!(markers(&state, &config), step["markers"]);
            assert_eq!(state.selection().main().head(), 0);
            assert_eq!(state.text().as_str(), case["doc"].as_str().unwrap());
        }
    }
}
