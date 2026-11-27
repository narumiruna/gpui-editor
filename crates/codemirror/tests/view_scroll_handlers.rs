use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    Assoc, EditorState, Extension, Precedence, SelectionRange,
    view::{
        CoveredMargins, EditorView, FixedScroller, ScrollBlock, ScrollMargins, ScrollOffset,
        ScrollRect, ScrollRequest, ScrollSnapshot, ScrollSnapshotGeometry, ScrollStrategy,
        ScrollTarget, ScrollView, ViewException, Viewport, bidi::Direction, covered_margins,
        exception_sink, scroll_handler, try_scroll_handler,
    },
};
use serde_json::{Value, json};

type Events = Arc<Mutex<Vec<String>>>;

fn number(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn pixel(value: &Value) -> f64 {
    value.as_f64().unwrap()
}

fn offset(value: &Value) -> ScrollOffset {
    ScrollOffset {
        x: pixel(&value["x"]),
        y: pixel(&value["y"]),
    }
}

fn rect(value: &Value) -> ScrollRect {
    ScrollRect {
        left: pixel(&value["left"]),
        top: pixel(&value["top"]),
        right: pixel(&value["right"]),
        bottom: pixel(&value["bottom"]),
    }
}

fn block(value: &Value) -> ScrollBlock {
    ScrollBlock {
        from: number(&value["from"]),
        top: pixel(&value["top"]),
    }
}

fn precedence(value: &Value) -> Precedence {
    match value["precedence"].as_str() {
        Some("highest") => Precedence::Highest,
        Some("high") => Precedence::High,
        Some("low") => Precedence::Low,
        None => Precedence::Default,
        other => panic!("unexpected precedence: {other:?}"),
    }
}

fn extensions(row: &Value, events: &Events, target: ScrollTarget) -> Extension {
    let errors = events.clone();
    let mut extensions = vec![exception_sink(move |error| {
        assert_eq!(error.context, Some("scroll handler"));
        errors
            .lock()
            .unwrap()
            .push(format!("error:{}", error.message));
    })];
    for spec in row["handlers"].as_array().unwrap() {
        let input = row["input"].clone();
        let (events, spec) = (events.clone(), spec.clone());
        let priority = precedence(&spec);
        extensions.push(
            try_scroll_handler(move |view, received| {
                assert_eq!(received, target);
                assert_eq!(view.state.text().len(), number(&input["docLength"]));
                assert_eq!(
                    view.viewport.interval(),
                    number(&input["viewport"][0])..number(&input["viewport"][1])
                );
                assert_eq!(view.focused, input["focused"].as_bool().unwrap());
                events
                    .lock()
                    .unwrap()
                    .push(spec["name"].as_str().unwrap().into());
                match spec["action"].as_str().unwrap() {
                    "false" => Ok(false),
                    "true" => Ok(true),
                    "throw" => Err(ViewException::new("handler failed")),
                    action => panic!("unexpected handler action {action}"),
                }
            })
            .with_precedence(priority),
        );
    }
    for spec in row["providers"].as_array().unwrap() {
        let (events, spec) = (events.clone(), spec.clone());
        let bounds = rect(&row["scroller"]["clip"]);
        let legacy = (!row["legacyBounds"].is_null()).then(|| rect(&row["legacyBounds"]));
        let priority = precedence(&spec);
        extensions.push(
            covered_margins(move |_, clip| {
                assert_eq!(clip, bounds);
                events
                    .lock()
                    .unwrap()
                    .push(spec["name"].as_str().unwrap().into());
                let insets = &spec["insets"];
                let edges = &spec["edges"];
                if !insets.is_null() {
                    let legacy = legacy.expect("measured legacy reference");
                    assert_ne!(legacy, bounds);
                    CoveredMargins::from_insets(
                        legacy,
                        ScrollMargins {
                            left: insets["left"].as_f64(),
                            top: insets["top"].as_f64(),
                            right: insets["right"].as_f64(),
                            bottom: insets["bottom"].as_f64(),
                        },
                    )
                } else {
                    (!edges.is_null()).then(|| CoveredMargins {
                        left: edges["left"].as_f64(),
                        top: edges["top"].as_f64(),
                        right: edges["right"].as_f64(),
                        bottom: edges["bottom"].as_f64(),
                    })
                }
            })
            .with_precedence(priority),
        );
    }
    Extension::group(extensions)
}

#[test]
fn scroll_callbacks_and_reveal_match_the_pinned_browser() {
    let mut count = 0;
    for line in include_str!("fixtures/view_scroll_handlers.jsonl").lines() {
        count += 1;
        let row: Value = serde_json::from_str(line).unwrap();
        assert!(row["error"].is_null(), "{row}");
        let name = &row["name"];
        let target_data = &row["target"];
        let target = ScrollTarget {
            range: SelectionRange::new(
                number(&target_data["anchor"]),
                number(&target_data["head"]),
            ),
            x: ScrollStrategy::Center,
            y: ScrollStrategy::Center,
            x_margin: pixel(&target_data["xMargin"]),
            y_margin: pixel(&target_data["yMargin"]),
        };
        let events = Events::default();
        let doc = (0..50)
            .map(|i| format!("{i} {}", "text ".repeat(32)))
            .collect::<Vec<_>>()
            .join("\n");
        let state = EditorState::with_extensions(doc, extensions(&row, &events, target)).unwrap();
        let request = if row["snapshot"] == true {
            let data = &row["snapshotGeometry"];
            let snapshot = ScrollSnapshot::capture(
                state.text(),
                ScrollSnapshotGeometry {
                    offset: offset(&data["offset"]),
                    padding_top: pixel(&data["paddingTop"]),
                    viewport_from: number(&data["viewportFrom"]),
                    first_viewport_block: block(&data["firstViewportBlock"]),
                },
                |height| {
                    assert_eq!(height, pixel(&data["probe"]["height"]));
                    Some(block(&data["probe"]["block"]))
                },
            )
            .unwrap();
            assert_eq!(snapshot.anchor(), target.range.head());
            ScrollRequest::Restore(snapshot)
        } else {
            assert_eq!(target_data["x"], "center");
            assert_eq!(target_data["y"], "center");
            ScrollRequest::Reveal(target)
        };
        let mut host = EditorView::<()>::new_with_scroll(state, request);
        let input = &row["input"];
        if !input.is_null() {
            assert_eq!(&input["target"], target_data);
            let interval = number(&input["viewport"][0])..number(&input["viewport"][1]);
            let viewport = Viewport::around(host.state().text(), [interval]).unwrap();
            host.set_viewport(viewport, input["focused"].as_bool().unwrap())
                .unwrap();
        }
        let measured = &row["scroller"];
        let scroller = FixedScroller {
            clip: rect(&measured["clip"]),
            offset_width: pixel(&measured["offsetWidth"]),
            offset_height: pixel(&measured["offsetHeight"]),
            scale_x: pixel(&measured["scaleX"]),
            scale_y: pixel(&measured["scaleY"]),
            scroll_x: measured["scrollX"].as_bool().unwrap(),
            scroll_y: measured["scrollY"].as_bool().unwrap(),
            direction: Direction::Ltr,
        };
        let before = offset(&row["before"]);
        let request = host.take_scroll_request().unwrap();
        let view = ScrollView {
            state: host.state(),
            viewport: host.viewport(),
            focused: host.focused(),
        };
        let mut measurements = row["measurements"].as_array().unwrap().iter();
        let planned = match request {
            ScrollRequest::Restore(snapshot) => snapshot
                .plan_restore(view.state.text(), |from| {
                    assert_eq!(from, target.range.head());
                    Some(pixel(&row["restored"]))
                })
                .unwrap(),
            ScrollRequest::Reveal(target) if view.handle_scroll(target) => before,
            ScrollRequest::Reveal(target) => {
                let delta = view
                    .plan_reveal(target, scroller, |pos, side| {
                        events.lock().unwrap().push("coords".into());
                        let measured = measurements.next().unwrap();
                        assert_eq!(pos, number(&measured["pos"]));
                        assert_eq!(
                            side,
                            match measured["side"].as_i64().unwrap() {
                                -1 => Assoc::Before,
                                1 => Assoc::After,
                                side => panic!("unexpected side {side}"),
                            }
                        );
                        Some(rect(&measured["rect"]))
                    })
                    .unwrap();
                ScrollOffset {
                    x: before.x + delta.x,
                    y: before.y + delta.y,
                }
            }
        };
        assert!(measurements.next().is_none(), "{name}");
        assert_eq!(json!(*events.lock().unwrap()), row["calls"], "{name}");
        for (axis, before, planned) in [("x", before.x, planned.x), ("y", before.y, planned.y)] {
            let expected = row["assigned"][axis].as_f64().unwrap_or(before);
            assert!((planned - expected).abs() < 1e-9, "{name}: {axis}");
        }
    }
    assert_eq!(count, 5);
}

fn simple_scroller() -> FixedScroller {
    FixedScroller {
        clip: ScrollRect {
            left: 10.0,
            top: 20.0,
            right: 110.0,
            bottom: 120.0,
        },
        offset_width: 100.0,
        offset_height: 100.0,
        scale_x: 1.0,
        scale_y: 1.0,
        scroll_x: true,
        scroll_y: true,
        direction: Direction::Ltr,
    }
}

#[test]
fn reject_nonfinite_or_crossed_bounds_but_accept_zero_extent() {
    let scroller = simple_scroller();
    for (left, valid) in [
        (f64::NAN, false),
        (f64::INFINITY, false),
        (111.0, false),
        (110.0, true),
    ] {
        let state = EditorState::with_extensions(
            "x",
            covered_margins(move |_, _| {
                Some(CoveredMargins {
                    left: Some(left),
                    ..CoveredMargins::default()
                })
            }),
        )
        .unwrap();
        let viewport = Viewport::complete(state.text());
        let result = ScrollView {
            state: &state,
            viewport: &viewport,
            focused: false,
        }
        .clip_scroller(scroller);
        let zero_width = FixedScroller {
            clip: ScrollRect {
                left,
                ..scroller.clip
            },
            ..scroller
        };
        assert_eq!(result, valid.then_some(zero_width));
    }
    assert_eq!(
        CoveredMargins::from_insets(
            scroller.clip,
            ScrollMargins {
                right: Some(f64::NAN),
                ..ScrollMargins::default()
            }
        ),
        None
    );
    let state =
        EditorState::with_extensions("x", covered_margins(|_, _| panic!("invalid clip"))).unwrap();
    let viewport = Viewport::complete(state.text());
    let view = ScrollView {
        state: &state,
        viewport: &viewport,
        focused: false,
    };
    for clip in [
        ScrollRect {
            left: f64::NEG_INFINITY,
            ..scroller.clip
        },
        ScrollRect {
            left: -f64::MAX,
            right: f64::MAX,
            ..scroller.clip
        },
        ScrollRect {
            top: 121.0,
            ..scroller.clip
        },
    ] {
        assert_eq!(view.clip_scroller(FixedScroller { clip, ..scroller }), None);
    }
    assert_eq!(
        view.plan_reveal(ScrollTarget::at(1), scroller, |_, _| None),
        None
    );
}

#[test]
fn interception_sees_clipped_valid_targets_and_continues_without_a_sink() {
    let state = EditorState::with_extensions(
        "a😀",
        Extension::group([
            try_scroll_handler(|_, _| Err(ViewException::new("no sink"))),
            scroll_handler(|view, target| {
                assert_eq!(target.range.head(), view.state.text().len());
                true
            }),
        ]),
    )
    .unwrap();
    let viewport = Viewport::complete(state.text());
    let view = ScrollView {
        state: &state,
        viewport: &viewport,
        focused: false,
    };
    assert!(view.handle_scroll(ScrollTarget::at(100)));
    assert!(!view.handle_scroll(ScrollTarget::at(2)));
}
