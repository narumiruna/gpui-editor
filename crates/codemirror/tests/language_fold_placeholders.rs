use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, PHRASES, Transaction,
    language::fold::{
        FoldPlaceholderConfig, PrepareFoldPlaceholder, code_folding_with_placeholders, fold_effect,
        fold_json, fold_placeholder, fold_placeholder_decorations, unfold_effect,
    },
};
use serde_json::{Value, json};

#[test]
fn preparation_and_rendering_match_pinned_fold_widget_lifecycle() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let log = calls.clone();
    let prepare: PrepareFoldPlaceholder = Arc::new(move |state, range| {
        let text = state.text().slice(range.clone()).unwrap();
        log.lock().unwrap().push(json!([
            state.text().as_str(),
            [range.start, range.end],
            text
        ]));
        Arc::from(text)
    });
    let custom = |prefix| {
        code_folding_with_placeholders(FoldPlaceholderConfig {
            prepare: Some(prepare.clone()),
            render: Some(Arc::new(move |_, _, value| {
                Some(format!("{prefix}:{}", value.unwrap_or("-")).into())
            })),
            ..Default::default()
        })
    };
    let phrases = PHRASES.of(Arc::new(
        [
            ("unfold".into(), "show source".into()),
            ("folded code".into(), "hidden source".into()),
        ]
        .into(),
    ));
    let slot = Compartment::new();
    let extensions = |config| Extension::group([slot.of(config), phrases.clone()]);
    let fields = [fold_json("folds")];
    let byte = |value: &Value| value.as_u64().unwrap() as usize;
    let mut state =
        EditorState::with_extensions("猫 lead\n[old]\n", extensions(custom("first"))).unwrap();
    for row in include_str!("fixtures/language_fold_placeholders.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let phase = case["phase"].as_str().unwrap();
        let mut transaction = Transaction::default();
        if let Some(edit) = case["edit"].as_array() {
            transaction = Transaction::new(
                ChangeSet::single(Change::new(
                    byte(&edit[0])..byte(&edit[1]),
                    edit[2].as_str().unwrap(),
                ))
                .unwrap(),
            );
        }
        if let Some(span) = case["fold"].as_array() {
            let span = byte(&span[0])..byte(&span[1]);
            let effect = if phase == "unfold" {
                unfold_effect()
            } else {
                fold_effect()
            };
            transaction = transaction.with_effects([effect.of(span)]);
        }
        if let Some(head) = case["head"].as_u64() {
            transaction =
                transaction.with_selection(EditorSelection::single(head as usize, head as usize));
        }
        if phase == "renderer" {
            transaction = transaction.with_effects([slot.reconfigure(custom("second"))]);
        } else if phase == "stock_label" {
            let options = FoldPlaceholderConfig {
                text: Some("hidden".into()),
                ..Default::default()
            };
            transaction = transaction
                .with_effects([slot.reconfigure(code_folding_with_placeholders(options))]);
        }
        if phase == "restored" {
            let data = state.to_json_with_fields(&fields).unwrap();
            assert_eq!(data["folds"], json!([14, 19]));
            state =
                EditorState::from_json_with_fields(&data, extensions(custom("second")), &fields)
                    .unwrap();
        } else if phase != "before" {
            state.update(transaction).unwrap();
        }
        let decorations = fold_placeholder_decorations(&state).unwrap();
        let marks: Vec<_> = decorations
            .iter(0)
            .map(|mark| {
                let label = mark.value.widget_value().unwrap();
                assert_eq!(
                    fold_placeholder(&state, &(mark.from..mark.to)).as_ref(),
                    Some(label)
                );
                json!([
                    mark.from,
                    mark.to,
                    label.text.as_ref(),
                    label.title,
                    label.aria_label
                ])
            })
            .collect();
        assert_eq!(json!(marks), case["markers"], "{phase}");
        assert_eq!(json!(state.text().as_str()), case["doc"], "{phase}");
        assert_eq!(
            json!(std::mem::take(&mut *calls.lock().unwrap())),
            case["calls"],
            "{phase}"
        );
    }
}

#[test]
fn options_merge_independently_and_conflicts_leave_installed_folds_unchanged() {
    let text = |label: &'static str| {
        code_folding_with_placeholders(FoldPlaceholderConfig {
            text: Some(label.into()),
            ..Default::default()
        })
    };
    let prepare: PrepareFoldPlaceholder = Arc::new(|_, _| "prepared".into());
    let callbacks = code_folding_with_placeholders(FoldPlaceholderConfig {
        prepare: Some(prepare),
        render: Some(Arc::new(|_, _, prepared| {
            assert_eq!(prepared, Some("prepared"));
            None
        })),
        ..Default::default()
    });
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(
        "abc",
        Extension::group([slot.of(text("hidden")), text("hidden"), callbacks]),
    )
    .unwrap();
    let span = 0..2;
    state
        .update(Transaction::default().with_effects([fold_effect().of(span.clone())]))
        .unwrap();
    assert!(
        state
            .update(Transaction::default().with_effects([slot.reconfigure(text("different"))]))
            .is_err()
    );
    assert_eq!(
        fold_placeholder(&state, &span).unwrap().text.as_ref(),
        "hidden"
    );
}
