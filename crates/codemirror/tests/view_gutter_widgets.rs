use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    EditError, EditorState, Extension, Precedence, RangeSet,
    view::{
        ScrollRect,
        bidi::Direction,
        decorations::{BlockPlacement, Decoration, ReplaceSpec, WidgetSpec},
        gutters::{
            GUTTER_LINE_CLASS, GUTTER_WIDGET_CLASS, GutterConfig, GutterMarker, GutterSide,
            GutterWidget, GutterWidgetMarker, GuttersConfig, LINE_NUMBER_WIDGET_MARKER, gutter,
            gutter_covered_margins, gutter_slots, gutter_widget_rows, gutter_with_widget_marker,
            gutters, gutters_fixed, line_numbers,
        },
    },
};
use serde_json::{Value, json};

type Calls = Arc<Mutex<Vec<Value>>>;

fn marker(
    calls: &Calls,
    name: &'static str,
    accepted: &'static [&'static str],
    response: Option<&'static str>,
) -> GutterWidgetMarker {
    let calls = calls.clone();
    Arc::new(move |state, widget| {
        assert_eq!(state.text().as_str(), "猫 one\nbeta\nlast");
        let label = *widget.value::<&str>().unwrap();
        let range = widget.source();
        let kind = match widget.placement() {
            BlockPlacement::Before => 1,
            BlockPlacement::After => 2,
            BlockPlacement::Range => 3,
        };
        calls
            .lock()
            .unwrap()
            .push(json!([name, label, range.start, range.end, kind]));
        accepted.contains(&label).then(|| GutterMarker {
            element_class: format!("widget-{name}"),
            content: response.map(str::to_owned),
        })
    })
}

#[test]
fn widget_gutters_match_pinned_delivered_markers_and_callback_order() {
    let calls = Calls::default();
    let extension = Extension::group([
        gutter_with_widget_marker(
            GutterConfig {
                class: "after".into(),
                side: GutterSide::After,
                ..GutterConfig::default()
            },
            marker(&calls, "local", &["before"], Some("before")),
        ),
        line_numbers(Default::default()),
        gutter(GutterConfig {
            class: "empty".into(),
            render_empty_elements: true,
            ..GutterConfig::default()
        }),
        LINE_NUMBER_WIDGET_MARKER.of(marker(&calls, "number-1", &["after"], Some("↑"))),
        LINE_NUMBER_WIDGET_MARKER.of(marker(&calls, "number-2", &["replace"], Some("R"))),
        GUTTER_WIDGET_CLASS.of(marker(&calls, "common", &["before", "replace"], None)),
        GUTTER_WIDGET_CLASS
            .of(marker(&calls, "high", &["replace"], Some("*")))
            .with_precedence(Precedence::High),
    ]);
    let state = EditorState::with_extensions("猫 one\nbeta\nlast", extension).unwrap();
    let block = |label, position, side| {
        Decoration::widget(
            label,
            WidgetSpec {
                side,
                block: true,
                inline_order: false,
            },
        )
        .try_range(position..position)
        .unwrap()
    };
    let decorations = [
        block("before", 0, 0),
        Decoration::widget("inline", WidgetSpec::default())
            .try_range(4..4)
            .unwrap(),
        block("after", 7, 1),
        Decoration::replace(
            Some("replace"),
            ReplaceSpec {
                block: true,
                ..Default::default()
            },
        )
        .try_range(8..12)
        .unwrap(),
        block("silent", state.text().len(), 1),
    ];
    let widgets: Vec<_> = decorations
        .iter()
        .filter_map(GutterWidget::from_decoration)
        .collect();
    assert_eq!(widgets.len(), 4);
    assert!(widgets[0].value::<usize>().is_none());
    let rows = gutter_widget_rows(&state, &widgets).unwrap();
    assert!(rows.iter().all(|row| row.widget_index < 3));
    let slots: Vec<_> = gutter_slots(&state, 1..=3)
        .unwrap()
        .into_iter()
        .map(|slot| {
            let markers: Vec<_> = rows
                .iter()
                .filter(|row| row.side == slot.side && row.gutter_index == slot.index)
                .map(|row| {
                    let classes = std::iter::once("cm-gutterElement")
                        .chain(
                            row.markers
                                .iter()
                                .map(|marker| marker.element_class.as_str()),
                        )
                        .collect::<Vec<_>>()
                        .join(" ");
                    let text: String = row
                        .markers
                        .iter()
                        .filter_map(|marker| marker.content.as_deref())
                        .collect();
                    json!([classes, text])
                })
                .collect();
            json!({"class": format!("cm-gutter {}", slot.class), "rows": markers})
        })
        .collect();
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/view_gutter_widgets.json")).unwrap();
    assert_eq!(json!(*calls.lock().unwrap()), expected["calls"]);
    assert_eq!(json!(slots), expected["slots"]);
}

#[test]
fn exact_marker_ties_use_the_same_heap_order_as_the_pinned_gutter() {
    let sets = |group, labels: &[char]| -> Vec<_> {
        labels
            .iter()
            .map(|label| {
                let marker = GutterMarker::class(format!("widget-{group}-{label}"));
                Arc::new(RangeSet::of(vec![marker.at(0)], false).unwrap())
            })
            .collect()
    };
    let local = sets("local", &['a', 'b', 'c', 'd']);
    let shared = sets("shared", &['a', 'b', 'c', 'd', 'e']);
    let mut extensions = vec![gutter(GutterConfig {
        markers: Some(Arc::new(move |_| local.clone())),
        ..GutterConfig::default()
    })];
    extensions.extend(shared.into_iter().map(|set| GUTTER_LINE_CLASS.of(set)));
    let state =
        EditorState::with_extensions("猫 one\nbeta\nlast", Extension::group(extensions)).unwrap();
    let slots = gutter_slots(&state, 1..=3).unwrap();
    assert_eq!(slots[0].rows.len(), 1);
    let markers: Vec<_> = slots[0].rows[0]
        .markers
        .iter()
        .map(|marker| &marker.element_class)
        .collect();
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/view_gutter_widgets.json")).unwrap();
    assert_eq!(json!(markers), expected["ties"]);
}

#[test]
fn fixed_gutters_cover_physical_edges_in_both_directions() {
    use gpui_editor_codemirror::{Compartment, Transaction};

    let expected: Value =
        serde_json::from_str(include_str!("fixtures/view_gutter_widgets.json")).unwrap();
    let bounds = |left, right| ScrollRect {
        left,
        right,
        top: 0.,
        bottom: 80.,
    };
    for case in expected["cover"].as_array().unwrap() {
        let direction = if case["direction"] == "ltr" {
            Direction::Ltr
        } else {
            Direction::Rtl
        };
        let before = if direction == Direction::Ltr {
            bounds(10., 25.)
        } else {
            bounds(280., 300.)
        };
        let after = (case["columns"] == "both").then(|| {
            if direction == Direction::Ltr {
                bounds(280., 300.)
            } else {
                bounds(10., 25.)
            }
        });
        let policy = Compartment::new();
        let mut extensions = vec![policy.of(gutters(GuttersConfig::default()))];
        if case["columns"] != "none" {
            extensions.push(line_numbers(Default::default()));
        }
        if after.is_some() {
            extensions.push(gutter(GutterConfig {
                side: GutterSide::After,
                ..GutterConfig::default()
            }));
        }
        let mut state =
            EditorState::with_extensions("猫 one\nbeta\nlast", Extension::group(extensions))
                .unwrap();
        let options = Extension::group(case["options"].as_array().unwrap().iter().map(|fixed| {
            gutters(GuttersConfig {
                fixed: fixed.as_bool().unwrap(),
            })
        }));
        state
            .update(Transaction::default().with_effects([policy.reconfigure(options)]))
            .unwrap();
        assert_eq!(
            json!(gutters_fixed(&state).unwrap()),
            case["fixed"],
            "{case}"
        );
        let edges = gutter_covered_margins(&state, direction, before, after)
            .unwrap()
            .map(|edges| [edges.left, edges.right]);
        let expected = case["edges"]
            .as_array()
            .map(|edges| [edges[0].as_f64(), edges[1].as_f64()]);
        assert_eq!(edges, expected, "{case}");
        assert!(
            gutter_covered_margins(&state, direction, bounds(25., 10.), after)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn widget_gutters_skip_unrepresented_widgets_and_validate_byte_ranges() {
    let state = EditorState::with_extensions("猫", line_numbers(Default::default())).unwrap();
    let hidden = Decoration::<()>::replace(
        None,
        ReplaceSpec {
            block: true,
            ..Default::default()
        },
    )
    .try_range(0..3)
    .unwrap();
    assert!(GutterWidget::from_decoration(&hidden).is_none());
    let invalid = Decoration::widget(
        (),
        WidgetSpec {
            block: true,
            ..Default::default()
        },
    )
    .try_range(1..1)
    .unwrap();
    assert_eq!(
        gutter_widget_rows(&state, &[GutterWidget::from_decoration(&invalid).unwrap()]),
        Err(EditError::InvalidRange)
    );
}
