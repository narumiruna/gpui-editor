//! Behavior checked with the actual `codemirror` 6.0.2 presets in Chrome.

use std::{
    ops::Range,
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Assoc, Compartment, ConfigurationError, EditorSelection,
    EditorState, Extension, SelectionRange, StateField, Transaction,
    language_data::{NativeLanguage, json_extension},
    setup::{
        SetupEditor, SetupKeyContext, SetupKeyResult, basic_setup, basic_setup_with_language,
        minimal_setup, try_minimal_setup,
    },
    view::{
        Viewport,
        decorations::DecorationKind,
        keymap::{KeyEvent, KeyModifiers, KeyPlatform},
        selection_layers::{
            LayerKind, LayerRect, RectangleHit, RectangleStartEvent, RectangularSelection,
            SelectionGeometry, SelectionPlatform, draw_selection_markers, drop_cursor_marker,
            layer_metadata, set_drop_cursor,
        },
        update_listener,
    },
};

fn setup_source(source: &str, anchor: usize, head: usize) -> EditorState {
    let mut state =
        EditorState::with_extensions(source, ALLOW_MULTIPLE_SELECTIONS.of(false)).unwrap();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(anchor, head)))
        .unwrap();
    state
}

fn key(
    editor: &mut SetupEditor<()>,
    event: KeyEvent<'_>,
    context: SetupKeyContext<'_>,
) -> SetupKeyResult<()> {
    editor
        .key_event_with(event, Duration::from_secs(1), (), context)
        .unwrap()
}

#[test]
fn presets_project_stock_special_chars_and_basic_marks_from_current_state() {
    let state = setup_source("one\none\n\u{200b}", 0, 3);
    let mut basic = basic_setup::<()>(state.clone(), KeyPlatform::Linux).unwrap();
    let mut minimal = minimal_setup::<()>(state, KeyPlatform::Linux);
    assert!(basic.view().state().allows_multiple_selections());
    assert!(!minimal.view().state().allows_multiple_selections());
    let mut slots = basic.gutters(1..=3, None).unwrap();
    assert_eq!(
        slots
            .iter()
            .map(|slot| slot.class.as_str())
            .collect::<Vec<_>>(),
        ["cm-lineNumbers", "cm-foldGutter"]
    );
    let numbers = slots.remove(0);
    assert_eq!(
        numbers
            .rows
            .iter()
            .map(|row| row.markers[0].content.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["1", "2", "3"]
    );
    assert_eq!(
        numbers.rows[0].markers[1].element_class,
        "cm-activeLineGutter"
    );
    assert_eq!(
        slots[0].spacer.as_ref().unwrap().content.as_deref(),
        Some("›")
    );
    assert!(minimal.gutters(1..=3, None).unwrap().is_empty());

    let basic_marks = basic.decorations().unwrap();
    let minimal_marks = minimal.decorations().unwrap();
    for marks in [&basic_marks, &minimal_marks] {
        let mut special = marks.special_chars.iter(0);
        let only = special.next().unwrap();
        assert_eq!(only.from..only.to, 8..11);
        assert_eq!(only.value.widget_value().unwrap().placeholder(), '•');
        assert!(special.next().is_none());
    }
    let active = basic_marks.active_lines.iter(0).next().unwrap();
    assert_eq!(active.from, 0);
    assert!(
        matches!(active.value.kind(), DecorationKind::Line(line) if line.class() == "cm-activeLine")
    );
    let matched: Vec<_> = basic_marks
        .selection_matches
        .iter(0)
        .map(|mark| (mark.from..mark.to, mark.value.class()))
        .collect();
    assert_eq!(matched, [(4..7, "cm-selectionMatch")]);
    assert!(minimal_marks.active_lines.iter(0).next().is_none());
    assert!(minimal_marks.selection_matches.is_empty());

    let viewport = Viewport::around(basic.view().state().text(), std::iter::once(0..3)).unwrap();
    basic.view_mut().set_viewport(viewport, true).unwrap();
    assert!(basic.decorations().unwrap().selection_matches.is_empty());
    let multi = EditorSelection::new(
        vec![SelectionRange::new(0, 0), SelectionRange::new(4, 4)],
        1,
    )
    .unwrap();
    basic.view_mut().set_selection(multi.clone()).unwrap();
    minimal.view_mut().set_selection(multi).unwrap();
    assert_eq!(
        minimal.view().state().selection().ranges(),
        [SelectionRange::new(4, 4)]
    );
    assert_eq!(
        basic
            .decorations()
            .unwrap()
            .active_lines
            .iter(0)
            .map(|line| line.from)
            .collect::<Vec<_>>(),
        [0, 4]
    );
    assert!(basic.decorations().unwrap().selection_matches.is_empty());
}

#[test]
fn basic_projects_and_invokes_fold_state_for_gutter_and_platform_keys() {
    for (platform, modifiers) in [
        (
            KeyPlatform::Mac,
            KeyModifiers {
                meta: true,
                alt: true,
                ..KeyModifiers::default()
            },
        ),
        (
            KeyPlatform::Linux,
            KeyModifiers {
                control: true,
                shift: true,
                ..KeyModifiers::default()
            },
        ),
    ] {
        let mut editor =
            basic_setup::<()>(setup_source("{\n  \"x\": 1\n}\n", 0, 0), platform).unwrap();
        let syntax = NativeLanguage::Json
            .parse(editor.view().state().text().clone())
            .unwrap();
        let label = |editor: &SetupEditor<()>| {
            editor.gutters(1..=1, Some(&syntax)).unwrap()[1].rows[0].markers[0]
                .content
                .clone()
        };
        assert_eq!(label(&editor).as_deref(), Some("⌄"));
        let context = || SetupKeyContext {
            syntax: Some(&syntax),
            ..SetupKeyContext::default()
        };
        let fold_event = |base, shifted| {
            let mut event = KeyEvent::new(if modifiers.shift { shifted } else { base }, modifiers);
            event.base_name = Some(base);
            event
        };
        assert!(
            key(&mut editor, fold_event("[", "{"), context())
                .key
                .handled
        );
        assert_eq!(label(&editor).as_deref(), Some("›"));
        let decorations = editor.decorations().unwrap();
        let placeholder = decorations.fold_placeholders.iter(0).next().unwrap();
        assert_eq!(placeholder.from..placeholder.to, 1..11);
        assert_eq!(placeholder.value.widget_value(), Some(&"…"));
        assert!(
            editor
                .toggle_line_fold(1, None, Duration::from_secs(2), ())
                .unwrap()
        );
        assert_eq!(label(&editor).as_deref(), Some("⌄"));
        assert!(
            editor
                .toggle_line_fold(1, Some(&syntax), Duration::from_secs(3), ())
                .unwrap()
        );
        assert_eq!(label(&editor).as_deref(), Some("›"));
        assert!(
            key(&mut editor, fold_event("]", "}"), context())
                .key
                .handled
        );
        assert_eq!(label(&editor).as_deref(), Some("⌄"));
        assert!(
            editor
                .decorations()
                .unwrap()
                .fold_placeholders
                .iter(0)
                .next()
                .is_none()
        );
        assert_eq!(editor.view().state().selection().main().head(), 0);

        let all = KeyModifiers {
            control: true,
            alt: true,
            ..KeyModifiers::default()
        };
        assert!(
            key(&mut editor, KeyEvent::new("[", all), context())
                .key
                .handled
        );
        assert_eq!(label(&editor).as_deref(), Some("›"));
        assert!(
            key(&mut editor, KeyEvent::new("]", all), context())
                .key
                .handled
        );
        assert_eq!(label(&editor).as_deref(), Some("⌄"));
    }
}

#[test]
fn fold_keys_use_projected_bounds_and_announce_logical_lines() {
    let source = "[\r  {\r    \"x\": 1\r  }\r]\r\n";
    let anchor = source.find('{').unwrap();
    let selection = EditorSelection::single(anchor, anchor);
    let messages = Arc::new(Mutex::new(Vec::new()));
    let state = EditorState::create_from_text(
        gpui_editor_codemirror::Text::new(source),
        Some(selection.clone()),
        update_listener({
            let messages = Arc::clone(&messages);
            move |update| {
                messages
                    .lock()
                    .unwrap()
                    .extend(update.announcements().map(str::to_owned));
                Ok(())
            }
        }),
    )
    .unwrap();
    let syntax = NativeLanguage::Json.parse(state.text().clone()).unwrap();
    let mut setup = basic_setup::<()>(state, KeyPlatform::Mac).unwrap();
    let modifiers = KeyModifiers {
        meta: true,
        alt: true,
        ..Default::default()
    };
    let context = || SetupKeyContext {
        syntax: Some(&syntax),
        ..Default::default()
    };
    assert!(
        key(&mut setup, KeyEvent::new("[", modifiers), context())
            .key
            .handled
    );
    let folds = setup.configured_fold_placeholders().unwrap();
    let inner = folds.iter(0).next().unwrap();
    assert_eq!(inner.from..inner.to, anchor + 1..source.find('}').unwrap());
    assert!(
        key(&mut setup, KeyEvent::new("]", modifiers), context())
            .key
            .handled
    );
    assert!(
        setup
            .configured_fold_placeholders()
            .unwrap()
            .iter(0)
            .next()
            .is_none()
    );
    assert_eq!(
        *messages.lock().unwrap(),
        ["Folded lines 2 to 4.", "Unfolded lines 2 to 4."]
    );
    assert_eq!(setup.view().state().selection(), &selection);
    assert_eq!(setup.view().state().text().as_str(), source);
    assert_eq!(setup.view().undo_depth(), 0);
}

#[test]
fn native_language_reindents_basic_input_in_the_same_history_step() {
    let state = setup_source("{\n    ", 6, 6);
    let mut basic = basic_setup_with_language(
        state.clone(),
        KeyPlatform::Linux,
        NativeLanguage::Json.support(),
    )
    .unwrap();
    let mut minimal = minimal_setup(state, KeyPlatform::Linux);
    basic.insert_text("}", Duration::ZERO, 4).unwrap();
    minimal.insert_text("}", Duration::ZERO, 5).unwrap();
    assert_eq!(basic.view().state().text().as_str(), "{\n}");
    assert_eq!(minimal.view().state().text().as_str(), "{\n    }");
    assert_eq!(basic.view_mut().undo(), Some(4));
    assert_eq!(basic.view().state().text().as_str(), "{\n    ");
}

#[test]
fn setup_tracks_installed_json_for_input_indentation_and_history() {
    for line in include_str!("fixtures/setup_attached_indent.jsonl").lines() {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        let source = row["source"].as_str().unwrap();
        let mode = row["mode"].as_str().unwrap();
        let event = row["event"].as_str().unwrap();
        let slot = Compartment::new();
        let extension = |install| {
            if install {
                json_extension()
            } else {
                Extension::default()
            }
        };
        let mut state = EditorState::with_extensions(
            source,
            slot.of(extension(matches!(mode, "initial" | "detach"))),
        )
        .unwrap();
        let end = source.len();
        state
            .update(Transaction::default().with_selection(EditorSelection::single(end, end)))
            .unwrap();
        let mut editor = if row["preset"] == "basic" {
            basic_setup(state, KeyPlatform::Linux).unwrap()
        } else {
            minimal_setup(state, KeyPlatform::Linux)
        };
        if matches!(mode, "attach" | "detach") {
            editor
                .view_mut()
                .update(
                    Transaction::default()
                        .with_effects([slot.reconfigure(extension(mode == "attach"))]),
                    (),
                    |_| (None, ()),
                )
                .unwrap();
        }
        if event == "input.type" {
            editor.insert_text("}", Duration::ZERO, ()).unwrap();
        } else {
            let input = editor
                .view()
                .state()
                .replace_selection("}")
                .unwrap()
                .with_user_event(event);
            editor.view_mut().update(input, (), |_| (None, ())).unwrap();
        }
        let check = |editor: &SetupEditor<()>, step: &str, expected: &str| {
            let view = editor.view();
            let state = view.state();
            let context = format!("{} {mode} {event}, {step}", row["preset"]);
            assert_eq!(state.text().as_str(), expected, "{context}");
            assert_eq!(
                serde_json::json!(state.selection().main().head()),
                row[step]["head"],
                "{context}"
            );
            assert_eq!(
                serde_json::json!([view.undo_depth(), view.redo_depth()]),
                row[step]["history"],
                "{context}"
            );
        };
        let result = row["result"].as_str().unwrap();
        check(&editor, "applied", result);
        assert_eq!(editor.view_mut().undo(), Some(()));
        check(&editor, "undone", source);
        assert_eq!(editor.view_mut().redo(), Some(()));
        check(&editor, "redone", result);
    }
}

#[test]
fn minimal_keeps_the_original_state_if_a_field_rejects_setup() {
    let field = StateField::new(
        |_| Ok(()),
        |_, update| {
            if update.transaction().effects().is_empty() {
                Ok(())
            } else {
                Err(ConfigurationError::InvalidFieldData)
            }
        },
    );
    let state = EditorState::with_extensions("one", field.extension()).unwrap();
    assert!(try_minimal_setup::<()>(state.clone(), KeyPlatform::Linux).is_err());
    let mut editor = minimal_setup::<()>(state, KeyPlatform::Linux);
    assert!(layer_metadata(editor.view().state()).is_empty());
    editor.insert_text("a", Duration::ZERO, ()).unwrap();
    assert_eq!(editor.view().state().text().as_str(), "aone");
}

struct MeasuredSelection;

impl SelectionGeometry for MeasuredSelection {
    fn caret_rect(&mut self, position: usize, _: Assoc) -> Option<LayerRect> {
        Some(LayerRect {
            left: position as f64,
            right: position as f64,
            top: 0.,
            bottom: 20.,
        })
    }

    fn selection_rects(&mut self, range: Range<usize>) -> Vec<LayerRect> {
        vec![LayerRect {
            left: range.start as f64,
            right: range.end as f64,
            top: 0.,
            bottom: 20.,
        }]
    }
}

#[test]
fn presets_measure_selection_and_basic_tracks_the_pointer_policies() {
    let state = setup_source("one\ntwo", 0, 3);
    let mut basic = basic_setup::<()>(state.clone(), KeyPlatform::Linux).unwrap();
    let mut minimal = minimal_setup::<()>(state, KeyPlatform::Linux);
    for editor in [&basic, &minimal] {
        let state = editor.view().state();
        assert_eq!(
            layer_metadata(state)
                .iter()
                .map(|layer| layer.kind)
                .collect::<Vec<_>>(),
            [LayerKind::Cursor, LayerKind::Selection]
        );
        let marks =
            draw_selection_markers(state, SelectionPlatform::Default, &mut MeasuredSelection)
                .unwrap();
        assert_eq!(marks.cursors[0].class, "cm-cursor cm-cursor-primary");
        assert_eq!(marks.cursors[0].left, 3.);
        assert_eq!(marks.selection[0].class, "cm-selectionBackground");
        assert_eq!(marks.selection[0].width, Some(3.));
    }
    let alt = KeyModifiers {
        alt: true,
        ..KeyModifiers::default()
    };
    let event = RectangleStartEvent {
        button: 0,
        modifiers: alt,
    };
    let start = RectangleHit {
        position: 1,
        line_end_column: None,
    };
    assert!(
        RectangularSelection::start(minimal.view().state(), event, start)
            .unwrap()
            .is_none()
    );
    let drag = RectangularSelection::start(basic.view().state(), event, start)
        .unwrap()
        .unwrap();
    let end = RectangleHit {
        position: 6,
        line_end_column: None,
    };
    let selection = drag
        .selection(basic.view().state(), Some(end), false)
        .unwrap();
    assert_eq!(
        selection
            .ranges()
            .iter()
            .map(|range| range.range())
            .collect::<Vec<_>>(),
        [1..2, 5..6]
    );
    basic.view_mut().set_selection(selection).unwrap();
    assert_eq!(basic.view().state().selection().ranges().len(), 2);

    assert!(
        set_drop_cursor(minimal.view().state(), Some(4))
            .unwrap()
            .is_none()
    );
    let effect = set_drop_cursor(basic.view().state(), Some(4))
        .unwrap()
        .unwrap();
    basic
        .view_mut()
        .update(Transaction::default().with_effects([effect]), (), |_| {
            (None, ())
        })
        .unwrap();
    let marker = drop_cursor_marker(basic.view().state(), |pos| {
        MeasuredSelection.caret_rect(pos, Assoc::After)
    })
    .unwrap();
    assert_eq!((marker.class.as_str(), marker.left), ("cm-dropCursor", 4.));
    assert!(minimal.crosshair_cursor_mut().is_none());
    let crosshair = basic.crosshair_cursor_mut().unwrap();
    assert!(crosshair.key_down(KeyEvent::new("Alt", alt)));
    assert!(crosshair.is_down());
    assert!(crosshair.key_up(KeyEvent::new("Alt", KeyModifiers::default())));
    assert!(!crosshair.is_down());
}
