use std::time::Duration;

use gpui_editor_codemirror::view::keymap::{
    KeyBinding, KeyEvent, KeyModifiers, KeyPlatform, KeyResult, KeySequence, Keymap, KeymapError,
};

const NONE: KeyModifiers = KeyModifiers {
    alt: false,
    control: false,
    meta: false,
    shift: false,
};
const CTRL: KeyModifiers = KeyModifiers {
    control: true,
    ..NONE
};

fn event(name: &str) -> KeyEvent<'_> {
    KeyEvent::new(name, NONE)
}

#[test]
fn scopes_platform_overrides_and_shift_commands() {
    let bindings = [KeyBinding {
        mac: Some("Mod-ArrowLeft".into()),
        windows: Some("Ctrl-Home".into()),
        shift: Some("select"),
        scope: "editor panel".into(),
        ..KeyBinding::new("Home", "move")
    }];
    let mut sequence = KeySequence::default();
    for (platform, name, modifiers) in [
        (KeyPlatform::Linux, "Home", NONE),
        (
            KeyPlatform::Mac,
            "ArrowLeft",
            KeyModifiers { meta: true, ..NONE },
        ),
        (KeyPlatform::Windows, "Home", CTRL),
    ] {
        let map = Keymap::new(platform, bindings.clone()).unwrap();
        for scope in ["editor", "panel", "unknown"] {
            for shift in [false, true] {
                let mut output = None;
                let result = map.dispatch(
                    scope,
                    KeyEvent::new(name, KeyModifiers { shift, ..modifiers }),
                    Duration::ZERO,
                    &mut sequence,
                    |id| {
                        output = Some(*id);
                        true
                    },
                );
                let exists = scope != "unknown";
                assert_eq!(result.handled, exists);
                assert_eq!(
                    output,
                    exists.then_some(if shift { "select" } else { "move" })
                );
            }
        }
    }
}

#[test]
fn priority_any_dispatch_duplicate_commands_and_prevention() {
    let map = Keymap::new(
        KeyPlatform::Linux,
        [
            KeyBinding {
                any: Some("first"),
                ..KeyBinding::default()
            },
            KeyBinding::new("Ctrl-ArrowLeft", "second"),
            KeyBinding {
                any: Some("middle"),
                prevent_default: true,
                stop_propagation: true,
                ..KeyBinding::new("Control-ArrowLeft", "second")
            },
            KeyBinding::new("c-ArrowLeft", "last"),
        ],
    )
    .unwrap();
    let mut sequence = KeySequence::default();
    for (name, expected, handled) in [
        (
            "ArrowLeft",
            vec!["first", "second", "middle", "last", "middle"],
            true,
        ),
        ("ArrowRight", vec!["first", "middle"], false),
    ] {
        let mut calls = Vec::new();
        let result = map.dispatch(
            "editor",
            KeyEvent::new(name, CTRL),
            Duration::ZERO,
            &mut sequence,
            |id| {
                calls.push(*id);
                false
            },
        );
        assert_eq!(calls, expected);
        assert_eq!(
            result,
            KeyResult {
                handled,
                stop_propagation: handled
            }
        );
    }
    let mut calls = Vec::new();
    let result = map.dispatch(
        "editor",
        KeyEvent::new("ArrowLeft", CTRL),
        Duration::ZERO,
        &mut sequence,
        |id| {
            calls.push(*id);
            *id == "second"
        },
    );
    assert_eq!(calls, ["first", "second"]);
    assert_eq!(
        result,
        KeyResult {
            handled: true,
            stop_propagation: true
        }
    );
}

#[test]
fn modifier_fallback_handles_layout_cases_and_shifted_printable_keys() {
    let bindings = [
        KeyBinding::new("Ctrl-!", "literal"),
        KeyBinding::new("Ctrl-Shift-1", "base"),
        KeyBinding::new("Alt-3", "base-alt"),
        KeyBinding::new("Ctrl-Alt-3", "base-altgr"),
        KeyBinding::new("Shift-X", "shift"),
        KeyBinding::new("Shift-Space", "space"),
        KeyBinding::new("Ctrl--", "dash"),
    ];
    let mut sequence = KeySequence::default();
    for platform in [KeyPlatform::Linux, KeyPlatform::Windows, KeyPlatform::Mac] {
        let map = Keymap::new(platform, bindings.clone()).unwrap();
        let ctrl_shift = KeyModifiers {
            shift: true,
            ..CTRL
        };
        let alt = KeyModifiers { alt: true, ..NONE };
        let shift = KeyModifiers {
            shift: true,
            ..NONE
        };
        for (key, expected) in [
            (
                KeyEvent {
                    base_name: Some("1"),
                    shifted_base_name: Some("!"),
                    ..KeyEvent::new("!", ctrl_shift)
                },
                Some("literal"),
            ),
            (
                KeyEvent {
                    base_name: Some("1"),
                    shifted_base_name: Some("!"),
                    ..KeyEvent::new("¡", ctrl_shift)
                },
                Some("base"),
            ),
            (
                KeyEvent {
                    base_name: Some("3"),
                    ..KeyEvent::new("£", alt)
                },
                (platform != KeyPlatform::Mac).then_some("base-alt"),
            ),
            (
                KeyEvent {
                    base_name: Some("3"),
                    ..KeyEvent::new(
                        "£",
                        KeyModifiers {
                            control: true,
                            ..alt
                        },
                    )
                },
                (platform != KeyPlatform::Windows).then_some("base-altgr"),
            ),
            (KeyEvent::new("X", shift), Some("shift")),
            (KeyEvent::new(" ", shift), Some("space")),
            (KeyEvent::new("-", CTRL), Some("dash")),
        ] {
            let mut output = None;
            let result = map.dispatch("editor", key, Duration::ZERO, &mut sequence, |id| {
                output = Some(*id);
                true
            });
            assert_eq!(output, expected, "{platform:?}: {key:?}");
            assert_eq!(result.handled, expected.is_some());
        }
    }
}

#[test]
fn multistroke_prefixes_expire_and_wrong_key_is_consumed() {
    let map = Keymap::new(
        KeyPlatform::Linux,
        [KeyBinding::new("Ctrl-k x Enter", "open")],
    )
    .unwrap();
    let mut sequence = KeySequence::default();
    let mut separate = KeySequence::default();
    let mut output = Vec::new();
    let mut dispatch = |scope, key, ms, seq: &mut KeySequence| {
        map.dispatch(scope, key, Duration::from_millis(ms), seq, |id| {
            output.push(*id);
            true
        })
        .handled
    };
    let start = KeyEvent::new("k", CTRL);
    assert!(dispatch("editor", start, 100, &mut sequence));
    assert!(!dispatch("editor", event("x"), 200, &mut separate));
    assert!(!dispatch("panel", event("x"), 300, &mut sequence));
    assert!(!dispatch("editor", event("Control"), 400, &mut sequence));
    assert!(dispatch("editor", event("x"), 4099, &mut sequence));
    assert!(dispatch("editor", event("Enter"), 8098, &mut sequence));
    assert!(!dispatch("editor", event("Enter"), 8099, &mut sequence));

    assert!(dispatch("editor", start, 10000, &mut sequence));
    assert!(dispatch("editor", event("y"), 10001, &mut sequence));
    assert!(!dispatch("editor", event("x"), 10002, &mut sequence));
    assert!(dispatch("editor", start, 20000, &mut sequence));
    assert!(!dispatch("editor", event("x"), 24000, &mut sequence));
    assert_eq!(output, ["open"]);
}

#[test]
fn prefix_any_and_prevention_only_bindings() {
    let map = Keymap::new(
        KeyPlatform::Other,
        [
            KeyBinding {
                any: Some("any"),
                ..KeyBinding::default()
            },
            KeyBinding::new("Ctrl-k Space", "insert"),
            KeyBinding {
                key: Some("Tab".into()),
                prevent_default: true,
                ..KeyBinding::default()
            },
        ],
    )
    .unwrap();
    let mut sequence = KeySequence::default();
    for (key, expected) in [
        (KeyEvent::new("k", CTRL), vec![]),
        (event(" "), vec!["any", "insert"]),
        (event("Tab"), vec!["any"]),
    ] {
        let mut calls = Vec::new();
        let result = map.dispatch("editor", key, Duration::ZERO, &mut sequence, |id| {
            calls.push(*id);
            *id == "insert"
        });
        assert!(result.handled);
        assert!(!result.stop_propagation);
        assert_eq!(calls, expected);
    }
}

#[test]
fn keymap_scope_names_match_pinned_space_delimiting_and_empty_default() {
    use serde_json::{Value, json};

    for row in include_str!("fixtures/keymap_scope.jsonl").lines() {
        let expected: Value = serde_json::from_str(row).unwrap();
        let at = |key| expected[key].as_str().unwrap();
        let map = Keymap::new(
            KeyPlatform::Other,
            [KeyBinding {
                scope: at("bindingScope").into(),
                any: Some("decline"),
                stop_propagation: true,
                ..KeyBinding::new("ArrowLeft", "move")
            }],
        )
        .unwrap();
        let mut calls = Vec::new();
        let result = map.dispatch(
            at("eventScope"),
            event(at("key")),
            Duration::ZERO,
            &mut KeySequence::default(),
            |id| {
                calls.push(*id);
                *id == "move"
            },
        );
        assert_eq!(json!(calls), expected["calls"], "{row}");
        assert_eq!(json!(result.handled), expected["handled"], "{row}");
        assert_eq!(json!(result.stop_propagation), expected["stopped"], "{row}");
    }
}

#[test]
fn invalid_modifiers_and_prefix_conflicts_are_rejected() {
    assert!(matches!(
        Keymap::new(KeyPlatform::Mac, [KeyBinding::new("Super-a", 0)]),
        Err(KeymapError::UnknownModifier(name)) if name == "Super"
    ));
    assert!(matches!(
        Keymap::new(
            KeyPlatform::Mac,
            [
                KeyBinding::new("Mod-x x", 0),
                KeyBinding {
                    scope: "panel".into(),
                    ..KeyBinding::new("Cmd-x", 1)
                }
            ]
        ),
        Err(KeymapError::ConflictingPrefix(_))
    ));
}
