//! Snapshots from the SHA-512-verified `@codemirror/view` 6.44.0 and
//! `@codemirror/state` 6.7.6 tarballs in `docs/codemirror-upstream.json`.
//! The external oracle drives upstream plugins with deterministic view
//! measurements, uses actual state transactions, and converts UTF-16 offsets
//! to UTF-8 bytes. It does not compare paint or browser input routing.

use std::{num::NonZeroUsize, ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, Precedence, SelectionRange, TAB_SIZE, Transaction,
    view::{
        keymap::{KeyEvent, KeyModifiers},
        selection_layers::{
            CrosshairModifier, DrawSelectionOptions, LayerConfig, LayerKind, LayerRect,
            RectangleHit, RectangleMarker, RectangleStartEvent, RectangularSelection,
            RectangularSelectionOptions, SET_DROP_CURSOR, SelectionGeometry, SelectionPlatform,
            crosshair_cursor, draw_selection, draw_selection_markers, drop_cursor,
            drop_cursor_marker, drop_cursor_position, get_draw_selection_config, layer,
            layer_metadata, rectangular_selection, set_drop_cursor,
        },
    },
};
use serde_json::{Value, json};

fn compare(name: &str, value: Value) {
    let expected = include_str!("fixtures/view_selection_layers.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|case| case["name"] == name)
        .unwrap_or_else(|| panic!("missing fixture: {name}"));
    assert_eq!(value, expected["value"], "{name}");
}

fn config(state: &EditorState) -> Value {
    let config = get_draw_selection_config(state);
    json!([
        config.cursor_blink_rate,
        config.draw_range_cursor,
        config.ios_selection_handles
    ])
}

fn metadata(state: &EditorState) -> Value {
    Value::Array(
        layer_metadata(state)
            .into_iter()
            .map(|layer| {
                json!([
                    layer.config.class,
                    layer.config.above,
                    layer.config.update_on_doc_view_update,
                    layer.z_index
                ])
            })
            .collect(),
    )
}

#[test]
fn draw_config_and_layer_order() {
    compare("config/default", config(&EditorState::new("")));
    let base_draw = draw_selection(DrawSelectionOptions {
        cursor_blink_rate: Some(900),
        draw_range_cursor: Some(false),
        ios_selection_handles: Some(false),
    });
    let high_draw = draw_selection(DrawSelectionOptions {
        cursor_blink_rate: Some(0),
        draw_range_cursor: Some(true),
        ..Default::default()
    });
    let custom = layer(LayerConfig {
        class: Some("custom".into()),
        update_on_doc_view_update: false,
        ..Default::default()
    });
    let compartment = Compartment::new();
    let mut state = EditorState::with_extensions(
        "",
        Extension::group([
            custom.clone(),
            base_draw.clone(),
            compartment.of(high_draw).with_precedence(Precedence::High),
            custom,
            layer(LayerConfig {
                above: true,
                class: Some("float".into()),
                ..Default::default()
            }),
        ]),
    )
    .unwrap();
    compare("config/combined", config(&state));
    compare("layer/precedence", metadata(&state));
    assert_eq!(layer_metadata(&state)[0].kind, LayerKind::Cursor);
    assert_eq!(layer_metadata(&state)[1].kind, LayerKind::Selection);
    state
        .update(
            Transaction::default().with_effects([compartment.reconfigure(Extension::default())]),
        )
        .unwrap();
    compare("config/reconfigured", config(&state));
    compare("layer/reconfigured", metadata(&state));

    let conflicting = draw_selection(DrawSelectionOptions {
        ios_selection_handles: Some(true),
        ..Default::default()
    });
    let error =
        EditorState::with_extensions("", Extension::group([base_draw, conflicting.clone()]))
            .unwrap_err();
    compare(
        "config/conflict",
        json!(error == ConfigurationError::InvalidFieldData),
    );
    let result =
        state.update(Transaction::default().with_effects([compartment.reconfigure(conflicting)]));
    assert_eq!(
        result,
        Err(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        ))
    );
    compare("config/reconfigured", config(&state));
}

fn rect(left: f64, top: f64, right: f64, bottom: f64) -> LayerRect {
    LayerRect {
        left,
        top,
        right,
        bottom,
    }
}

struct Geometry;

impl SelectionGeometry for Geometry {
    fn caret_rect(&mut self, position: usize, assoc: Assoc) -> Option<LayerRect> {
        let (left, top) = match position {
            1 => (7.0, 0.0),
            2 => (14.0, 0.0),
            3 => (21.0, 0.0),
            6 => return None,
            12 => (28.0, 22.0),
            _ => panic!("unexpected caret position: {position}"),
        };
        let left = if assoc == Assoc::Before {
            left - 1.0
        } else {
            left + 1.0
        };
        Some(rect(left, top, left, top + 9.0))
    }

    fn selection_rects(&mut self, range: Range<usize>) -> Vec<LayerRect> {
        match (range.start, range.end) {
            (1, 3) => vec![rect(7.0, 0.0, 21.0, 9.0)],
            (9, 12) => vec![rect(7.0, 22.0, 28.0, 31.0)],
            (1, 7) => vec![rect(7.0, 0.0, 21.0, 9.0), rect(35.0, 0.0, 49.0, 9.0)],
            (0, 3) => vec![],
            _ => panic!("unexpected selection range: {range:?}"),
        }
    }
}

fn number(value: f64) -> Value {
    if value.fract() == 0.0 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

fn marker(marker: &RectangleMarker) -> Value {
    json!([
        marker.class,
        number(marker.left),
        number(marker.top),
        marker.width.map(number),
        number(marker.height)
    ])
}

fn markers(markers: &[RectangleMarker]) -> Value {
    Value::Array(markers.iter().map(marker).collect())
}

#[test]
fn selection_markers_use_native_measurements() {
    assert!(
        draw_selection_markers(
            &EditorState::new(""),
            SelectionPlatform::Default,
            &mut Geometry
        )
        .is_none()
    );
    for (name, platform, range, handles) in [
        ("draw/default", SelectionPlatform::Default, None, None),
        (
            "draw/no-range-cursors",
            SelectionPlatform::Default,
            Some(false),
            None,
        ),
        ("draw/ios-handles", SelectionPlatform::Ios, None, None),
        ("draw/ios-native", SelectionPlatform::Ios, None, Some(false)),
    ] {
        let mut state = EditorState::with_extensions(
            "abcd\nxy\nabcdef",
            draw_selection(DrawSelectionOptions {
                draw_range_cursor: range,
                ios_selection_handles: handles,
                ..Default::default()
            }),
        )
        .unwrap();
        select(
            &mut state,
            vec![
                SelectionRange::new(3, 1),
                EditorSelection::cursor(6, Some(Assoc::Before), None, None),
                SelectionRange::new(9, 12),
            ],
            0,
        );
        let projected = draw_selection_markers(&state, platform, &mut Geometry).unwrap();
        compare(
            name,
            json!([markers(&projected.cursors), markers(&projected.selection)]),
        );
    }
    for (name, range) in [
        ("marker/empty", SelectionRange::new(2, 2)),
        ("marker/fragments", SelectionRange::new(1, 7)),
        ("marker/outside", SelectionRange::new(0, 3)),
    ] {
        compare(
            name,
            markers(&RectangleMarker::for_range(&mut Geometry, "focus", range)),
        );
    }

    let mut state = EditorState::with_extensions(
        "abcd",
        draw_selection(DrawSelectionOptions {
            draw_range_cursor: Some(false),
            ..Default::default()
        }),
    )
    .unwrap();
    select(
        &mut state,
        vec![SelectionRange::new(1, 1), SelectionRange::new(3, 3)],
        1,
    );
    let cursors = draw_selection_markers(&state, SelectionPlatform::Ios, &mut Geometry)
        .unwrap()
        .cursors;
    assert_eq!(cursors[0].class, "cm-cursor cm-cursor-secondary");
    assert_eq!(cursors[1].class, "cm-cursor cm-cursor-primary");
}

fn describe(selection: &EditorSelection) -> Value {
    json!([
        selection.main_index(),
        selection
            .ranges()
            .iter()
            .map(|range| [range.anchor(), range.head()])
            .collect::<Vec<_>>()
    ])
}

fn hit(position: usize, line_end_column: Option<usize>) -> RectangleHit {
    RectangleHit {
        position,
        line_end_column,
    }
}

fn alt() -> RectangleStartEvent {
    RectangleStartEvent {
        button: 0,
        modifiers: KeyModifiers {
            alt: true,
            ..Default::default()
        },
    }
}

fn select(state: &mut EditorState, ranges: Vec<SelectionRange>, main: usize) {
    state
        .update(Transaction::default().with_selection(EditorSelection::new(ranges, main).unwrap()))
        .unwrap();
}

fn rectangle_state(text: impl Into<String>) -> EditorState {
    EditorState::with_extensions(
        text,
        rectangular_selection(RectangularSelectionOptions::default()),
    )
    .unwrap()
}

fn start(state: &EditorState, hit: RectangleHit) -> RectangularSelection {
    RectangularSelection::start(state, alt(), hit)
        .unwrap()
        .unwrap()
}

#[test]
fn rectangle_drag_on_unicode_and_crlf() {
    let mut state = rectangle_state("abcd\r\n\twide\r\n\r\n😀éZ!\r\nxyz\r\nlast");
    select(
        &mut state,
        vec![SelectionRange::new(1, 2), SelectionRange::new(32, 32)],
        1,
    );
    let enabled = |event| {
        RectangularSelection::start(&state, event, hit(3, None))
            .unwrap()
            .is_some()
    };
    compare(
        "rectangle/default-filter",
        json!([
            enabled(RectangleStartEvent::default()),
            enabled(RectangleStartEvent { button: 1, ..alt() }),
            enabled(alt())
        ]),
    );
    let drag = start(&state, hit(28, None));
    compare(
        "rectangle/backward-unicode-multiple",
        describe(&drag.selection(&state, Some(hit(1, None)), true).unwrap()),
    );

    let state = rectangle_state("abcd\n\twide\n\n😀éZ!\nxyz\nlast");
    let drag = start(&state, hit(4, Some(6)));
    compare(
        "rectangle/virtual-end",
        describe(
            &drag
                .selection(&state, Some(hit(25, Some(2))), false)
                .unwrap(),
        ),
    );
}

#[test]
fn configured_filter_and_tab_size() {
    let filter = RectangularSelectionOptions {
        event_filter: Some(Arc::new(|event| event.modifiers.control)),
    };
    let state = EditorState::with_extensions(
        "abz\n\txy",
        Extension::group([
            rectangular_selection(filter),
            TAB_SIZE.of(NonZeroUsize::new(2).unwrap()),
        ]),
    )
    .unwrap();
    let event = RectangleStartEvent {
        button: 1,
        modifiers: KeyModifiers {
            control: true,
            ..Default::default()
        },
    };
    assert!(
        RectangularSelection::start(&state, alt(), hit(2, None))
            .unwrap()
            .is_none()
    );
    let active = RectangularSelection::start(&state, event, hit(2, None))
        .unwrap()
        .unwrap();
    let result = active.selection(&state, Some(hit(6, None)), false).unwrap();
    assert_eq!(describe(&result), json!([0, [[2, 3], [5, 6]]]));
    assert_eq!(
        RectangularSelection::start(&state, event, hit(100, None)).unwrap_err(),
        EditError::InvalidRange
    );
    assert!(
        RectangularSelection::start(&EditorState::new(""), event, hit(0, None))
            .unwrap()
            .is_none()
    );
}

#[test]
fn remap_drag_anchor_and_previous_selection() {
    let mut state = rectangle_state("xx\nabcd\nxy\nlast");
    select(&mut state, vec![SelectionRange::new(14, 14)], 0);
    let mut drag = start(&state, hit(5, None));
    let changes = ChangeSet::single(Change::new(0..4, "new\n\n")).unwrap();
    let desc = changes.desc(state.text().len()).unwrap();
    let previous = state.clone();
    state.update(Transaction::new(changes)).unwrap();
    drag.update(&previous, &state, &desc).unwrap();
    compare(
        "rectangle/mapped-drag",
        describe(
            &drag
                .selection(&state, Some(hit(11, Some(3))), true)
                .unwrap(),
        ),
    );
    assert_eq!(
        drag.selection(&state, None, false).unwrap(),
        EditorSelection::single(15, 15)
    );
}

#[test]
fn raw_offset_fallback_uses_logical_utf16_and_checks_output() {
    let state = rectangle_state("abc\nxy\nlonger text");
    let draw = start(&state, hit(3, None));
    compare(
        "rectangle/no-reference",
        describe(&draw.selection(&state, Some(hit(5, None)), false).unwrap()),
    );
    let state = rectangle_state("abc\r\nxy\r\nlonger text");
    let draw = start(&state, hit(4, None));
    assert_eq!(
        draw.selection(&state, Some(hit(6, None)), false).unwrap(),
        EditorSelection::single(1, 11)
    );

    let state = rectangle_state(format!("{}\n{}", "x".repeat(2001), "z".repeat(4500)));
    let draw = start(&state, hit(2001, Some(55)));
    compare(
        "rectangle/long-offset",
        describe(
            &draw
                .selection(&state, Some(hit(2002, None)), false)
                .unwrap(),
        ),
    );
    let mut state = rectangle_state(format!("{}\n{}", "x".repeat(2010), "z".repeat(2030)));
    select(&mut state, vec![SelectionRange::new(1, 1)], 0);
    let draw = start(&state, hit(2001, None));
    compare(
        "rectangle/no-raw-ranges",
        describe(
            &draw
                .selection(&state, Some(hit(2011, None)), false)
                .unwrap(),
        ),
    );

    for text in ["abc", "abc\nx😀y"] {
        let state = rectangle_state(text);
        let draw = start(&state, hit(3, None));
        let endpoint = if text == "abc" { 3 } else { 4 };
        let result = draw.selection(&state, Some(hit(endpoint, None)), false);
        assert_eq!(result, Err(EditError::InvalidSelection));
    }
}

#[test]
fn crosshair_modifier_reconciles_keys_and_pointer() {
    for (name, modifier, held) in [
        (
            "Alt",
            CrosshairModifier::Alt,
            KeyModifiers {
                alt: true,
                ..Default::default()
            },
        ),
        (
            "Control",
            CrosshairModifier::Control,
            KeyModifiers {
                control: true,
                ..Default::default()
            },
        ),
        (
            "Shift",
            CrosshairModifier::Shift,
            KeyModifiers {
                shift: true,
                ..Default::default()
            },
        ),
        (
            "Meta",
            CrosshairModifier::Meta,
            KeyModifiers {
                meta: true,
                ..Default::default()
            },
        ),
    ] {
        let mut pointer = crosshair_cursor(modifier);
        let empty = KeyModifiers::default();
        let mut observations = Vec::new();
        for step in 0..6 {
            let changed = match step {
                0 => pointer.key_down(KeyEvent::new(name, empty)),
                1 => pointer.key_down(KeyEvent::new("x", held)),
                2 => pointer.key_up(KeyEvent::new("x", held)),
                3 => pointer.key_up(KeyEvent::new(name, held)),
                4 => pointer.pointer_move(held),
                5 => pointer.pointer_move(empty),
                _ => unreachable!(),
            };
            observations.push([pointer.is_down(), changed]);
        }
        compare(&format!("crosshair/{name}"), json!(observations));
        assert!(pointer.pointer_move(held));
        assert!(pointer.key_up(KeyEvent::new("x", empty)));
    }
}

#[test]
fn drop_cursor_maps_and_projects_without_accepting_a_drop() {
    let mut state = EditorState::with_extensions("ab😀cd", drop_cursor()).unwrap();
    let mut dispatches = 0;
    let tracked = |state: &EditorState, n| json!([drop_cursor_position(state), n]);
    compare("drop/initial", tracked(&state, dispatches));
    for pos in [6, 6] {
        if let Some(effect) = set_drop_cursor(&state, Some(pos)).unwrap() {
            state
                .update(Transaction::default().with_effects([effect]))
                .unwrap();
            dispatches += 1;
        }
    }
    compare("drop/duplicate", tracked(&state, dispatches));
    // No host effect for a drag leaving one child for another.
    compare("drop/internal-leave", tracked(&state, dispatches));
    for (range, inserted, name) in [
        (6..6, "!", "drop/insert-at-target"),
        (1..7, "界", "drop/replace-over-target"),
    ] {
        state
            .update(Transaction::new(
                ChangeSet::single(Change::new(range, inserted)).unwrap(),
            ))
            .unwrap();
        compare(name, tracked(&state, dispatches));
    }
    let visible = drop_cursor_marker(&state, |pos| {
        assert_eq!(pos, 1);
        Some(rect(9.0, 2.0, 9.0, 7.0))
    })
    .unwrap();
    compare("drop/marker", marker(&visible));
    compare(
        "drop/missing",
        json!(
            drop_cursor_marker(&state, |_| None)
                .is_some()
                .then_some("visible")
        ),
    );
    state
        .update(
            Transaction::new(ChangeSet::single(Change::new(0..0, "z")).unwrap())
                .with_effects([SET_DROP_CURSOR.of(Some(2)), SET_DROP_CURSOR.of(Some(6))]),
        )
        .unwrap();
    compare("drop/last-effect", tracked(&state, dispatches));
    let clear = set_drop_cursor(&state, None).unwrap().unwrap();
    state
        .update(Transaction::default().with_effects([clear]))
        .unwrap();
    dispatches += 1;
    compare("drop/cleared", tracked(&state, dispatches));
    assert!(drop_cursor_marker(&state, |_| unreachable!()).is_none());
}

#[test]
fn drop_effects_map_before_insertions_and_reject_invalid_positions() {
    let mut state = EditorState::with_extensions("éx", drop_cursor()).unwrap();
    let changes = ChangeSet::single(Change::new(2..2, "!")).unwrap();
    let desc = changes.desc(state.text().len()).unwrap();
    for (original, expected) in [(Some(2), Some(2)), (Some(3), Some(4)), (None, None)] {
        assert_eq!(
            SET_DROP_CURSOR
                .of(original)
                .map(&desc)
                .unwrap()
                .get(&SET_DROP_CURSOR),
            Some(&expected)
        );
    }
    assert!(
        set_drop_cursor(&EditorState::new("é"), Some(0))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        set_drop_cursor(&state, Some(1)).unwrap_err(),
        EditError::InvalidRange
    );
    let invalid = Transaction::default().with_effects([SET_DROP_CURSOR.of(Some(20))]);
    assert_eq!(
        state.update(invalid),
        Err(EditError::Configuration(
            ConfigurationError::InvalidFieldData
        ))
    );
    assert_eq!(drop_cursor_position(&state), None);
}
