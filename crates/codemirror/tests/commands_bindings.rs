//! The catalog fixture enumerates the actual exports of `@codemirror/commands`
//! 6.11.2 using function identity. The tab-focus fixture records synthetic
//! keydowns in a real EditorView 6.44.0 in headless Chrome 158.0.8127.37 with a
//! controlled clock. The commands, view, language, and state tarballs matched
//! the SHA-512 digests in `docs/codemirror-upstream.json` before use.

use std::time::Duration;

use gpui_editor_codemirror::{
    commands::bindings::{
        CommandId, TabFocusMode, default_keymap, emacs_style_keymap, history_keymap,
        indent_with_tab, standard_keymap, temporarily_set_tab_focus_mode, toggle_tab_focus_mode,
    },
    view::keymap::{KeyBinding, KeyEvent, KeyModifiers, KeyPlatform, KeySequence, Keymap},
};
use serde_json::{Value, json};

fn records(fixture: &str) -> impl Iterator<Item = Value> + '_ {
    fixture
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn fields(catalog: &str, binding: KeyBinding<CommandId>) -> Value {
    let KeyBinding {
        key,
        mac,
        windows,
        linux,
        scope,
        run,
        shift,
        any,
        prevent_default,
        stop_propagation,
    } = binding;
    let mut fields = json!({"catalog": catalog});
    for (field, value) in [
        ("key", key),
        ("mac", mac),
        ("win", windows),
        ("linux", linux),
    ] {
        if let Some(value) = value {
            fields[field] = json!(value);
        }
    }
    for (field, value) in [("run", run), ("shift", shift), ("any", any)] {
        if let Some(value) = value {
            fields[field] = json!(value.upstream_name());
        }
    }
    if scope != "editor" {
        fields["scope"] = json!(scope);
    }
    if prevent_default {
        fields["preventDefault"] = json!(true);
    }
    if stop_propagation {
        fields["stopPropagation"] = json!(true);
    }
    fields
}

#[test]
fn all_catalog_rows_match_exports_in_order() {
    let actual: Vec<_> = [
        ("standardKeymap", standard_keymap()),
        ("defaultKeymap", default_keymap()),
        ("emacsStyleKeymap", emacs_style_keymap()),
        ("historyKeymap", history_keymap()),
        ("indentWithTab", vec![indent_with_tab()]),
    ]
    .into_iter()
    .flat_map(|(name, bindings)| {
        bindings
            .into_iter()
            .map(move |binding| fields(name, binding))
    })
    .collect();
    let expected: Vec<_> = records(include_str!("fixtures/commands_bindings.jsonl")).collect();
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(actual, &expected, "row {}", index + 1);
    }
}

fn dispatch(
    platform: KeyPlatform,
    bindings: Vec<KeyBinding<CommandId>>,
    event: KeyEvent<'_>,
) -> (Vec<CommandId>, bool) {
    let mut commands = Vec::new();
    let result = Keymap::new(platform, bindings).unwrap().dispatch(
        "editor",
        event,
        Duration::ZERO,
        &mut KeySequence::default(),
        |&command| {
            commands.push(command);
            false
        },
    );
    (commands, result.handled)
}

#[test]
fn dispatch_retains_visual_identities_and_distinct_emacs_prevention() {
    use CommandId::*;
    use KeyPlatform::*;

    let control = KeyModifiers {
        control: true,
        ..KeyModifiers::default()
    };
    let ctrl_b = KeyEvent::new("b", control);
    assert_eq!(
        dispatch(Mac, standard_keymap(), ctrl_b),
        (vec![CursorCharLeft], false)
    );
    for platform in [Mac, Linux, Windows, Other] {
        assert_eq!(
            dispatch(platform, emacs_style_keymap(), ctrl_b),
            (vec![CursorCharLeft], true)
        );
    }
    assert_eq!(dispatch(Linux, standard_keymap(), ctrl_b), (vec![], false));
    assert_eq!(
        dispatch(
            Windows,
            default_keymap(),
            KeyEvent::new("ArrowRight", control)
        ),
        (vec![CursorGroupRight], true),
    );
    let cmd = KeyModifiers {
        meta: true,
        ..KeyModifiers::default()
    };
    assert_eq!(
        dispatch(Mac, default_keymap(), KeyEvent::new("Backspace", cmd)),
        (vec![DeleteLineBoundaryBackward], true),
    );
    assert_eq!(
        dispatch(Mac, default_keymap(), KeyEvent::new("ArrowLeft", cmd)),
        (vec![CursorLineBoundaryLeft], true),
    );
    assert_eq!(
        dispatch(
            Mac,
            default_keymap(),
            KeyEvent::new("Home", KeyModifiers::default())
        ),
        (vec![CursorLineBoundaryBackward], true),
    );
}

#[test]
fn dispatch_uses_platform_overrides_and_opt_in_bindings() {
    use CommandId::*;
    use KeyPlatform::*;

    let shift_ctrl = KeyModifiers {
        shift: true,
        control: true,
        ..KeyModifiers::default()
    };
    let ctrl_shift_z = KeyEvent {
        base_name: Some("z"),
        ..KeyEvent::new("Z", shift_ctrl)
    };
    assert_eq!(
        dispatch(Linux, history_keymap(), ctrl_shift_z),
        (vec![Redo], true)
    );
    assert_eq!(
        dispatch(Windows, history_keymap(), ctrl_shift_z),
        (vec![], false)
    );
    assert_eq!(
        dispatch(Linux, default_keymap(), ctrl_shift_z),
        (vec![], false)
    );
    assert_eq!(
        dispatch(Mac, default_keymap(), KeyEvent::new("A", shift_ctrl)),
        (vec![ToggleBlockComment], false),
    );
    assert_eq!(
        dispatch(
            Linux,
            default_keymap(),
            KeyEvent::new(
                "A",
                KeyModifiers {
                    alt: true,
                    shift: true,
                    ..KeyModifiers::default()
                }
            )
        ),
        (vec![ToggleBlockComment], false),
    );

    let tab = KeyEvent::new(
        "Tab",
        KeyModifiers {
            shift: true,
            ..KeyModifiers::default()
        },
    );
    assert_eq!(dispatch(Mac, default_keymap(), tab), (vec![], false));
    assert_eq!(
        dispatch(Mac, vec![indent_with_tab()], tab),
        (vec![IndentLess], false)
    );
}

#[test]
fn tab_focus_matches_pinned_view_key_handling() {
    let keymap = Keymap::new(
        KeyPlatform::Mac,
        [
            indent_with_tab(),
            KeyBinding::new("x", CommandId::TransposeChars),
        ]
        .into_iter()
        .chain(default_keymap()),
    )
    .unwrap();
    for case in records(include_str!("fixtures/commands_tab_focus.jsonl")) {
        let name = case["name"].as_str().unwrap();
        let mut mode = TabFocusMode::default();
        let mut sequence = KeySequence::default();
        for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            let now = Duration::from_millis(step["at"].as_u64().unwrap());
            if let Some(command) = step["command"].as_str() {
                let handled = match command {
                    "toggle" => toggle_tab_focus_mode(&mut mode),
                    "temporary" => temporarily_set_tab_focus_mode(&mut mode, now),
                    _ => panic!("unknown command {command}"),
                };
                assert_eq!(handled, step["handled"], "{name} step {index}");
                continue;
            }

            let key = step["key"].as_str().unwrap();
            let shortcut = step["shortcut"] == true;
            let event = KeyEvent::new(
                key,
                KeyModifiers {
                    alt: shortcut,
                    shift: shortcut || step["shift"] == true,
                    ..KeyModifiers::default()
                },
            );
            let mut calls = Vec::new();
            let prevented = if mode.bypass_key_bindings(event, now) {
                false
            } else {
                let result = keymap.dispatch("editor", event, now, &mut sequence, |command| {
                    match command {
                        CommandId::IndentMore => calls.push("tab"),
                        CommandId::IndentLess => calls.push("shift-tab"),
                        CommandId::TransposeChars => calls.push("x"),
                        CommandId::SimplifySelection => {
                            calls.push("escape");
                            return step["intercept"] == true;
                        }
                        CommandId::ToggleTabFocusMode => return toggle_tab_focus_mode(&mut mode),
                        other => panic!("unexpected binding {other:?}"),
                    }
                    true
                });
                if !result.handled {
                    mode.handle_unhandled_key(event, now);
                }
                result.handled
            };
            assert_eq!(prevented, step["prevented"], "{name} step {index}");
            assert_eq!(json!(calls), step["calls"], "{name} step {index}");
        }
    }
}
