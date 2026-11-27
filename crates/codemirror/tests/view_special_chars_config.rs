use std::sync::Arc;

use gpui_editor_codemirror::{
    Compartment, ConfigurationError, EditError, EditorState, Extension, PHRASES, Transaction,
    setup::{basic_setup, try_minimal_setup},
    view::{
        SpecialCharConfig, Viewport, keymap::KeyPlatform, special_char_appearance,
        special_char_decorations, special_char_extension,
    },
};
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};

fn pattern(source: &str) -> Option<Arc<Regex>> {
    Some(Arc::new(Regex::new(source).unwrap()))
}

#[test]
fn configured_special_chars_match_pinned_browser_projections() {
    let case_insensitive = || {
        Some(Arc::new(
            RegexBuilder::new("[y]")
                .case_insensitive(true)
                .build()
                .unwrap(),
        ))
    };
    for row in include_str!("fixtures/view_special_chars_config.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let name = case["name"].as_str().unwrap();
        let add = |source| {
            special_char_extension(SpecialCharConfig {
                add_special_chars: pattern(source),
                ..Default::default()
            })
        };
        let config = match name {
            "inactive" => Extension::default(),
            "add_to_stock" => add("Y"),
            "replace_and_add" => Extension::group([
                special_char_extension(SpecialCharConfig {
                    special_chars: pattern("字X|x"),
                    render: Some(Arc::new(|character, description, fallback| {
                        assert!(description.starts_with("Visible control "));
                        (character == '字').then(|| format!("「{fallback}」"))
                    })),
                    ..Default::default()
                }),
                add("Y"),
            ]),
            "replace_with_flags" => special_char_extension(SpecialCharConfig {
                special_chars: case_insensitive(),
                ..Default::default()
            }),
            "combined_rebuilds_flags" => special_char_extension(SpecialCharConfig {
                special_chars: case_insensitive(),
                add_special_chars: pattern("x"),
                ..Default::default()
            }),
            _ => panic!("Unrecognized case {name}"),
        };
        let state = EditorState::with_extensions(
            case["doc"].as_str().unwrap(),
            Extension::group([
                config,
                PHRASES.of(Arc::new(
                    [("Control character".into(), "Visible control".into())].into(),
                )),
            ]),
        )
        .unwrap();
        let set = special_char_decorations(&state, &Viewport::complete(state.text())).unwrap();
        let mut marks = Vec::new();
        let mut appearances = Vec::new();
        for range in set.iter(0) {
            let item = *range.value.widget_value().unwrap();
            marks.push(json!([range.from, range.to, u32::from(item.character)]));
            let appearance = special_char_appearance(&state, item).unwrap();
            appearances.push(json!([appearance.glyph, appearance.description]));
        }
        assert_eq!(json!(marks), case["marks"], "{name}");
        assert_eq!(json!(appearances), case["appearance"], "{name}");
    }
}

#[test]
fn conflict_rejects_reconfiguration_and_removal_disables_marks() {
    let config = Compartment::new();
    let pattern = pattern("x+");
    let option = SpecialCharConfig {
        special_chars: pattern.clone(),
        ..Default::default()
    };
    let mut state = EditorState::with_extensions(
        "xx\t\0",
        config.of(Extension::group([
            special_char_extension(option.clone()),
            special_char_extension(option),
        ])),
    )
    .unwrap();
    let inspect = |state: &EditorState| {
        special_char_decorations(state, &Viewport::complete(state.text()))
            .unwrap()
            .iter(0)
            .count()
    };
    assert_eq!(inspect(&state), 1);
    let conflicting = Extension::group([
        special_char_extension(SpecialCharConfig {
            special_chars: pattern,
            ..Default::default()
        }),
        special_char_extension(SpecialCharConfig {
            special_chars: Some(Arc::new(Regex::new("x+").unwrap())),
            ..Default::default()
        }),
    ]);
    assert_eq!(
        state.update(Transaction::default().with_effects([config.reconfigure(conflicting)])),
        Err(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        ))
    );
    assert_eq!(inspect(&state), 1);
    state
        .update(Transaction::default().with_effects([config.reconfigure(Extension::default())]))
        .unwrap();
    assert_eq!(inspect(&state), 0);
}

#[test]
fn presets_compose_caller_match_options_with_stock_enablement() {
    let state = EditorState::with_extensions(
        "x\0",
        special_char_extension(SpecialCharConfig {
            special_chars: pattern("x"),
            ..Default::default()
        }),
    )
    .unwrap();
    let basic = basic_setup::<()>(state.clone(), KeyPlatform::Mac).unwrap();
    let minimal = try_minimal_setup::<()>(state, KeyPlatform::Mac).unwrap();
    for preset in [basic, minimal] {
        let decorations = preset.decorations().unwrap().special_chars;
        let [item] = decorations.iter(0).collect::<Vec<_>>().try_into().unwrap();
        assert_eq!(item.value.widget_value().unwrap().character, 'x');
    }
}
