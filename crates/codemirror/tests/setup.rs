use std::time::Duration;

use gpui_editor_codemirror::{
    Compartment, EditorSelection, EditorState, Extension, READ_ONLY, Transaction,
    autocomplete::brackets::close_brackets,
    commands::bindings::TabFocusMode,
    language::{HighlightTag, HighlightTheme, highlighting_for},
    setup::{SetupEditor, basic_setup, minimal_setup},
    view::keymap::{KeyEvent, KeyModifiers, KeyPlatform},
};

fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn cursor(source: &str, position: usize) -> EditorState {
    let mut state = EditorState::new(source);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(position, position)))
        .unwrap();
    state
}

fn key<M: Clone>(
    editor: &mut SetupEditor<M>,
    name: &str,
    modifiers: KeyModifiers,
    now: u64,
    metadata: M,
) -> Option<M> {
    editor
        .key_event(KeyEvent::new(name, modifiers), at(now), metadata)
        .unwrap()
        .1
}

#[test]
fn basic_installs_bracket_pair_precedence_and_minimal_does_not() {
    let platform = KeyPlatform::Mac;
    let mut basic = basic_setup::<()>(cursor("()", 1), platform).unwrap();
    let mut minimal = minimal_setup::<()>(cursor("()", 1), platform);
    assert_eq!(basic.bracket_matches().len(), 1);
    assert!(minimal.bracket_matches().is_empty());
    for layer in [basic.highlight_fallback(), minimal.highlight_fallback()] {
        let style = highlighting_for(
            &[layer],
            &[HighlightTag::new("keyword")],
            None,
            HighlightTheme::Light,
        );
        assert_eq!(style.foreground.unwrap().rgba32(), 0x770088ff);
    }
    assert_eq!(
        key(&mut basic, "Backspace", KeyModifiers::default(), 1, ()),
        Some(())
    );
    assert_eq!(
        key(&mut minimal, "Backspace", KeyModifiers::default(), 1, ()),
        Some(())
    );
    assert_eq!(basic.view().state().text().as_str(), "");
    assert_eq!(minimal.view().state().text().as_str(), ")");

    basic.insert_text("[", at(1000), ()).unwrap();
    assert_eq!(basic.view().state().text().as_str(), "[]");
    assert_eq!(basic.view().state().selection().main().head(), 1);
    basic.insert_text("]", at(1100), ()).unwrap();
    assert_eq!(basic.view().state().text().as_str(), "[]");
    assert_eq!(basic.view().state().selection().main().head(), 2);
    minimal.insert_text("[", at(1000), ()).unwrap();
    assert_eq!(minimal.view().state().text().as_str(), "[)");
}

#[test]
fn close_bracket_input_follows_its_extension_without_changing_preset_keys() {
    for line in include_str!("fixtures/setup_close_brackets.jsonl").lines() {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        let position =
            |offset: &serde_json::Value| usize::try_from(offset.as_u64().unwrap()).unwrap();
        let mode = row["mode"].as_str().unwrap();
        let slot = Compartment::new();
        let extension = |enabled| {
            if enabled {
                close_brackets()
            } else {
                Extension::default()
            }
        };
        let mut state = EditorState::with_extensions(
            row["source"].as_str().unwrap(),
            slot.of(extension(matches!(mode, "initial" | "detach"))),
        )
        .unwrap();
        let selection =
            EditorSelection::single(position(&row["select"][0]), position(&row["select"][1]));
        state
            .update(Transaction::default().with_selection(selection))
            .unwrap();
        let mut editor = if row["preset"] == "basic" {
            basic_setup(state, KeyPlatform::Linux).unwrap()
        } else {
            minimal_setup(state, KeyPlatform::Linux)
        };
        if matches!(mode, "attach" | "detach") {
            let change = Transaction::default()
                .with_effects([slot.reconfigure(extension(mode == "attach"))]);
            editor
                .view_mut()
                .update(change, (), |_| (None, ()))
                .unwrap();
        }
        for (i, step) in row["steps"].as_array().unwrap().iter().enumerate() {
            let input = step["key"].as_str().unwrap();
            let now = (i as u64 + 1) * 1000;
            if input == "Backspace" {
                assert_eq!(
                    key(&mut editor, input, KeyModifiers::default(), now, ()),
                    Some(())
                );
            } else {
                assert!(editor.insert_text(input, at(now), ()).unwrap());
            }
            let state = editor.view().state();
            let selection = state.selection().main();
            assert_eq!(
                state.text().as_str(),
                step["doc"].as_str().unwrap(),
                "{} {mode} {input}",
                row["preset"]
            );
            assert_eq!(
                serde_json::json!([selection.anchor(), selection.head()]),
                step["selection"]
            );
        }
    }
}

#[test]
fn preset_history_restores_metadata_and_accepts_platform_redo() {
    for (platform, modifies, redo_key, shifted) in [
        (
            KeyPlatform::Mac,
            KeyModifiers {
                meta: true,
                ..Default::default()
            },
            "z",
            true,
        ),
        (
            KeyPlatform::Linux,
            KeyModifiers {
                control: true,
                ..Default::default()
            },
            "z",
            true,
        ),
        (
            KeyPlatform::Windows,
            KeyModifiers {
                control: true,
                ..Default::default()
            },
            "y",
            false,
        ),
    ] {
        let mut editor = minimal_setup(EditorState::new(""), platform);
        editor.insert_text("字", at(1), 5).unwrap();
        editor.insert_text("x", at(400), 6).unwrap();
        assert_eq!(editor.view().state().text().as_str(), "字x");
        assert_eq!(editor.view().undo_depth(), 1);
        assert_eq!(editor.view().redo_depth(), 0);
        assert_eq!(key(&mut editor, "z", modifies, 450, 7), Some(5));
        assert_eq!(editor.view().state().text().as_str(), "");
        assert_eq!(editor.view().undo_depth(), 0);
        assert_eq!(editor.view().redo_depth(), 1);
        let mut redo = KeyEvent::new(
            if shifted { "Z" } else { redo_key },
            KeyModifiers {
                shift: shifted,
                ..modifies
            },
        );
        redo.base_name = Some(redo_key);
        assert_eq!(editor.key_event(redo, at(451), 8).unwrap().1, Some(6));
        assert_eq!(editor.view().state().text().as_str(), "字x");
    }
}

#[test]
fn default_bindings_keep_line_edits_and_selection_usable() {
    let mut editor = minimal_setup::<()>(cursor("one\r\ntwo\r\nthree", 5), KeyPlatform::Mac);
    let alt = KeyModifiers {
        alt: true,
        ..Default::default()
    };
    assert_eq!(key(&mut editor, "ArrowUp", alt, 10, ()), Some(()));
    assert_eq!(editor.view().state().text().as_str(), "two\r\none\r\nthree");
    assert_eq!(
        key(
            &mut editor,
            "ArrowDown",
            KeyModifiers { shift: true, ..alt },
            30,
            ()
        ),
        Some(())
    );
    assert_eq!(
        editor.view().state().text().as_str(),
        "two\r\ntwo\r\none\r\nthree"
    );
    assert_eq!(
        key(
            &mut editor,
            "a",
            KeyModifiers {
                meta: true,
                ..Default::default()
            },
            40,
            ()
        ),
        Some(())
    );
    let state = editor.view().state();
    assert_eq!(state.selection().main().range(), 0..state.text().len());
}

#[test]
fn setup_dispatches_selection_history_and_updates_its_tab_focus_policy() {
    let mut editor = minimal_setup::<()>(cursor("abc", 0), KeyPlatform::Linux);
    let ctrl = KeyModifiers {
        control: true,
        ..KeyModifiers::default()
    };
    key(&mut editor, "ArrowRight", KeyModifiers::default(), 1, ());
    assert_eq!(editor.view().state().selection().main().head(), 1);
    assert_eq!(editor.view().undo_depth(), 0);
    assert_eq!(key(&mut editor, "u", ctrl, 2, ()), Some(()));
    assert_eq!(editor.view().state().selection().main().head(), 0);
    let alt = KeyModifiers {
        alt: true,
        ..KeyModifiers::default()
    };
    assert_eq!(key(&mut editor, "u", alt, 3, ()), Some(()));
    assert_eq!(editor.view().state().selection().main().head(), 1);

    let (result, data) = editor
        .key_event(KeyEvent::new("m", ctrl), at(4), ())
        .unwrap();
    assert!(result.handled);
    assert!(data.is_none());
    assert_eq!(editor.tab_focus_mode(), TabFocusMode::Persistent);
    let (result, _) = editor
        .key_event(KeyEvent::new("Tab", KeyModifiers::default()), at(5), ())
        .unwrap();
    assert!(!result.handled);
    key(&mut editor, "m", ctrl, 6, ());
    assert_eq!(editor.tab_focus_mode(), TabFocusMode::Disabled);
    key(&mut editor, "Escape", KeyModifiers::default(), 7, ());
    assert_eq!(
        editor.tab_focus_mode(),
        TabFocusMode::TemporaryUntil(at(2007))
    );
}

#[test]
fn enabled_types_and_backward_deletion_respect_read_only_state() {
    let state = EditorState::with_extensions("()", READ_ONLY.of(true)).unwrap();
    let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
    editor
        .view_mut()
        .set_selection(EditorSelection::single(1, 1))
        .unwrap();
    assert!(!editor.insert_text("a", at(50), ()).unwrap());
    let (key, result) = editor
        .key_event(
            KeyEvent::new("Backspace", KeyModifiers::default()),
            at(55),
            (),
        )
        .unwrap();
    assert!(key.handled);
    assert!(result.is_none());
    assert_eq!(editor.view().state().text().as_str(), "()");
}
