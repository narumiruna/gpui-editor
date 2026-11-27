//! Oracle snapshots from stock `@codemirror/view` 6.44.0 and
//! `@codemirror/state` 6.7.6 in an external Node/jsdom run. The oracle hashes
//! the registry tarballs before use and converts UTF-16 offsets to UTF-8
//! bytes. DOM comparisons cover slot order, classes, text, and spacer samples.

use std::sync::Arc;

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Change, ChangeSet, Compartment, ConfigurationError, EditError,
    EditorSelection, EditorState, Extension, MapMode, Precedence, Range, RangeValue,
    SelectionRange, StateField, Transaction,
    view::gutters::{
        GUTTER_LINE_CLASS, GutterConfig, GutterMarker, GutterMarkerSet, GutterSide, GutterSlot,
        LINE_NUMBER_MARKERS, LineNumberConfig, gutter, gutter_slots, highlight_active_line_gutter,
        line_numbers,
    },
};
use serde_json::{Value, json};

fn fixture(case: &str) -> Value {
    include_str!("fixtures/view_gutters.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|row| row["case"] == case)
        .unwrap()
}

fn offset(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn label(class: &str, content: &str) -> GutterMarker {
    GutterMarker {
        element_class: class.into(),
        content: Some(content.into()),
    }
}

fn marker_set(ranges: impl IntoIterator<Item = Range<GutterMarker>>) -> Arc<GutterMarkerSet> {
    Arc::new(GutterMarkerSet::of(ranges.into_iter().collect(), true).unwrap())
}

fn class_set<const N: usize>(entries: [(usize, &str); N]) -> Arc<GutterMarkerSet> {
    marker_set(entries.map(|(from, class)| GutterMarker::class(class).at(from)))
}

fn fixture_set(points: &Value) -> Arc<GutterMarkerSet> {
    marker_set(points.as_array().unwrap().iter().map(|item| {
        Range::new(
            offset(&item[0]),
            offset(&item[1]),
            GutterMarker::class(item[2].as_str().unwrap()),
        )
    }))
}

fn change(spec: &Value) -> Change {
    Change::new(
        offset(&spec[0])..offset(&spec[1]),
        spec[2].as_str().unwrap(),
    )
}

fn element(markers: &[GutterMarker]) -> Value {
    let classes: Vec<_> = std::iter::once("cm-gutterElement")
        .chain(
            markers
                .iter()
                .map(|marker| marker.element_class.as_str())
                .filter(|class| !class.is_empty()),
        )
        .collect();
    let content: String = markers
        .iter()
        .filter_map(|marker| marker.content.as_deref())
        .collect();
    json!([classes.join(" "), content])
}

fn snapshot(state: &EditorState) -> Value {
    let slots = gutter_slots(state, 1..=state.text().line_count()).unwrap();
    Value::Array(slots.iter().map(slot_snapshot).collect())
}

fn slot_snapshot(slot: &GutterSlot) -> Value {
    let side = match slot.side {
        GutterSide::Before => "before",
        GutterSide::After => "after",
    };
    let class = if slot.class.is_empty() {
        "cm-gutter".into()
    } else {
        format!("cm-gutter {}", slot.class)
    };
    json!({
        "side": side,
        "index": slot.index,
        "class": class,
        "spacer": slot.spacer.as_ref().map(|marker| element(std::slice::from_ref(marker))),
        "rows": slot.rows.iter().map(|row| element(&row.markers)).collect::<Vec<_>>(),
    })
}

#[test]
fn oracle_used_the_pinned_tarballs() {
    let manifest: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let oracle = fixture("pins");
    for name in ["view", "state"] {
        let expected = manifest["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|package| package["name"] == format!("@codemirror/{name}"))
            .unwrap();
        assert_eq!(oracle[name]["version"], expected["version"]);
        assert_eq!(oracle[name]["integrity"], expected["integrity"]);
    }
}

#[test]
fn matches_stock_marker_mapping() {
    let trace = fixture("mapping");
    let text = trace["doc"].as_str().unwrap();
    let points = fixture_set(&trace["points"]);
    let marker = &points.iter(0).next().unwrap().value;
    assert!(marker.is_point());
    assert_eq!((marker.start_side(), marker.end_side()), (-1, -1));
    assert_eq!(marker.map_mode(), MapMode::TrackBefore);
    for edit in trace["edits"].as_array().unwrap() {
        let changes = ChangeSet::new(
            edit["changes"]
                .as_array()
                .unwrap()
                .iter()
                .map(change)
                .collect(),
        )
        .unwrap();
        let mapped = points.map(&changes);
        let rows: Vec<_> = mapped
            .iter(0)
            .map(|range| json!([range.from, range.to, range.value.element_class]))
            .collect();
        assert_eq!(json!(rows), edit["mapped"], "{}", edit["name"]);
        let mut state = EditorState::new(text);
        state.update(Transaction::new(changes)).unwrap();
        assert!(
            mapped
                .iter(0)
                .all(|range| state.text().line_at(range.from).is_some())
        );
    }
}

fn generic_gutter() -> Extension {
    let first = marker_set([
        label("local-a", "A").at(0),
        label("local-b", "B").at(0),
        GutterMarker::class("ignored").at(5),
        Range::new(6, 9, GutterMarker::class("span")),
    ]);
    let second = marker_set([
        label("local-c", "C").at(0),
        GutterMarker::class("local-d").at(6),
    ]);
    gutter(GutterConfig {
        class: "custom".into(),
        markers: Some(Arc::new(move |_| vec![first.clone(), second.clone()])),
        spacer: Some(Arc::new(|_| label("sizing", "WW"))),
        line_marker: Some(Arc::new(|_, line, others| {
            let classes: Vec<_> = others
                .iter()
                .map(|marker| marker.element_class.as_str())
                .collect();
            (!classes.is_empty() || line.number == 5).then(|| {
                label(
                    "computed",
                    &format!("{}:{}|", line.number, classes.join(",")),
                )
            })
        })),
        ..GutterConfig::default()
    })
}

fn simple_gutter(class: &str, side: GutterSide, render_empty_elements: bool) -> Extension {
    gutter(GutterConfig {
        class: class.into(),
        side,
        render_empty_elements,
        ..GutterConfig::default()
    })
}

fn custom_format() -> LineNumberConfig {
    LineNumberConfig {
        format_number: Some(Arc::new(|number, state| {
            format!("{number}/{}", state.text().line_count())
        })),
    }
}

fn active_heads(state: &EditorState) -> Value {
    let sets = state.context().facet(&GUTTER_LINE_CLASS).unwrap();
    let starts: Vec<_> = sets
        .iter()
        .flat_map(|set| set.iter(0))
        .filter(|range| range.value.element_class == "cm-activeLineGutter")
        .map(|range| range.from)
        .collect();
    json!(starts)
}

#[test]
fn matches_stock_slot_and_marker_order_through_reconfiguration() {
    let trace = fixture("render");
    let config = Compartment::new();
    let before = GutterSide::Before;
    let after = GutterSide::After;
    let sparse_after = simple_gutter("sparse-after", after, false);
    let generic = generic_gutter();
    let numbers = line_numbers(LineNumberConfig::default());
    let duplicate = simple_gutter("duplicate", before, false);
    let dense_after = simple_gutter("dense-after", after, true).with_precedence(Precedence::High);
    let mut state = EditorState::with_extensions(
        trace["source"].as_str().unwrap(),
        Extension::group([
            ALLOW_MULTIPLE_SELECTIONS.of(true),
            GUTTER_LINE_CLASS.of(marker_set([
                GutterMarker::class("shared").at(0),
                GutterMarker::class("same").at(6),
                GutterMarker::class("same").at(6),
                label("overrides-all", "X").at(11),
            ])),
            GUTTER_LINE_CLASS
                .of(class_set([(0, "priority"), (6, "priority")]))
                .with_precedence(Precedence::High),
            highlight_active_line_gutter(),
            highlight_active_line_gutter(),
            GUTTER_LINE_CLASS.of(class_set([(9, "not-a-line-start")])),
            LINE_NUMBER_MARKERS.of(marker_set([
                GutterMarker::class("number-class").at(0),
                label("number-content", "N").at(6),
                label("silent-number", "").at(10),
            ])),
            config.of(Extension::group([
                sparse_after.clone(),
                generic.clone(),
                line_numbers(custom_format()),
                duplicate.clone(),
                duplicate,
                simple_gutter("duplicate", before, false),
                simple_gutter("dense", before, true).with_precedence(Precedence::Highest),
                dense_after.clone(),
                numbers.clone(),
            ])),
        ]),
    )
    .unwrap();
    let selection = EditorSelection::new(
        vec![
            SelectionRange::new(0, 0),
            SelectionRange::new(4, 4),
            SelectionRange::new(6, 10),
            SelectionRange::new(13, 11),
        ],
        2,
    )
    .unwrap();
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    assert_eq!(active_heads(&state), trace["heads"]);
    assert_eq!(snapshot(&state), trace["first"]);

    state
        .update(Transaction::default().with_selection(EditorSelection::single(13, 13)))
        .unwrap();
    assert_eq!(active_heads(&state), trace["newHeads"]);
    assert_eq!(snapshot(&state), trace["selected"]);

    state
        .update(
            Transaction::default().with_effects([config.reconfigure(Extension::group([
                sparse_after,
                generic,
                numbers.with_precedence(Precedence::Highest),
                dense_after,
            ]))]),
        )
        .unwrap();
    assert_eq!(snapshot(&state), trace["reconfigured"]);
}

#[test]
fn matches_stock_formatter_and_spacer_rules() {
    let trace = fixture("sizing");
    let custom = custom_format();
    let mut state = EditorState::with_extensions(
        "\n".repeat(8),
        Extension::group([line_numbers(custom.clone()), line_numbers(custom.clone())]),
    )
    .unwrap();
    assert_eq!(snapshot(&state), trace["sizing"][0]);
    let insert = ChangeSet::single(Change::new(8..8, "\n")).unwrap();
    state.update(Transaction::new(insert)).unwrap();
    assert_eq!(snapshot(&state), trace["sizing"][1]);
    let delete = ChangeSet::single(Change::new(0..9, "")).unwrap();
    state.update(Transaction::new(delete)).unwrap();
    assert_eq!(snapshot(&state), trace["sizing"][2]);

    let conflict = EditorState::with_extensions(
        "",
        Extension::group([line_numbers(custom), line_numbers(custom_format())]),
    );
    assert_eq!(
        json!(matches!(
            conflict,
            Err(ConfigurationError::InvalidFieldData)
        )),
        trace["conflict"]
    );
    let classes_only = EditorState::with_extensions(
        "",
        Extension::group([
            highlight_active_line_gutter(),
            GUTTER_LINE_CLASS.of(class_set([(0, "solo")])),
        ]),
    )
    .unwrap();
    assert_eq!(snapshot(&classes_only), trace["noSlots"]);
}

#[test]
fn matches_stock_render_after_state_field_mapping() {
    let trace = fixture("mapped-render");
    let initial = fixture_set(&trace["points"]);
    let field = StateField::new(
        move |_| Ok((*initial).clone()),
        |markers, update| Ok(markers.map(update.transaction().changes())),
    );
    let from_field = field.clone();
    let mut state = EditorState::with_extensions(
        trace["doc"].as_str().unwrap(),
        Extension::group([
            field.extension(),
            gutter(GutterConfig {
                class: "mapped".into(),
                markers: Some(Arc::new(move |state| {
                    vec![state.context().field(&from_field).unwrap().unwrap()]
                })),
                ..GutterConfig::default()
            }),
        ]),
    )
    .unwrap();
    assert_eq!(snapshot(&state), trace["first"]);
    for step in trace["steps"].as_array().unwrap() {
        state
            .update(Transaction::new(
                ChangeSet::single(change(&step["change"])).unwrap(),
            ))
            .unwrap();
        assert_eq!(state.text().as_str(), step["doc"].as_str().unwrap());
        assert_eq!(snapshot(&state), step["rows"]);
    }
}

#[test]
fn projects_only_requested_logical_lines_using_native_byte_offsets() {
    let state = EditorState::with_extensions(
        "😀\r\nab\r\n",
        Extension::group([
            line_numbers(LineNumberConfig::default()),
            GUTTER_LINE_CLASS.of(class_set([(6, "second"), (9, "not-start"), (10, "last")])),
        ]),
    )
    .unwrap();
    let slots = gutter_slots(&state, 2..=3).unwrap();
    let rows = &slots[0].rows;
    assert_eq!((rows[0].line_number, rows[0].from), (2, 6));
    assert_eq!((rows[1].line_number, rows[1].from), (3, 10));
    assert_eq!(
        rows[0].markers,
        [GutterMarker::text("2"), GutterMarker::class("second")]
    );
    assert_eq!(
        rows[1].markers,
        [GutterMarker::text("3"), GutterMarker::class("last")]
    );
    assert_eq!(gutter_slots(&state, 0..=1), Err(EditError::InvalidRange));
    assert_eq!(gutter_slots(&state, 1..=4), Err(EditError::InvalidRange));
    let last = state.text().line_count();
    let empty = gutter_slots(&state, last + 1..=last).unwrap();
    assert!(empty[0].rows.is_empty());
    assert_eq!(empty[0].spacer, Some(GutterMarker::text("9")));
}
