use gpui_editor_codemirror::{
    Assoc, Change, ChangeDesc, ChangeSet, EditorSelection, SelectionRange, Text,
    view::{
        FixedScroller, SCROLL_SNAPSHOT, ScrollBlock, ScrollDelta, ScrollOffset, ScrollRect,
        ScrollSnapshot, ScrollSnapshotGeometry, ScrollStrategy, ScrollTarget, bidi::Direction,
    },
};
use serde_json::Value;

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

fn block(value: &Value) -> ScrollBlock {
    ScrollBlock {
        from: number(&value["from"]),
        top: pixel(&value["top"]),
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

fn strategy(value: &Value) -> ScrollStrategy {
    match value.as_str().unwrap() {
        "nearest" => ScrollStrategy::Nearest,
        "start" => ScrollStrategy::Start,
        "center" => ScrollStrategy::Center,
        "end" => ScrollStrategy::End,
        _ => panic!("invalid strategy {value}"),
    }
}

fn side(value: &Value) -> Option<Assoc> {
    match value.as_i64().unwrap() {
        -1 => Some(Assoc::Before),
        0 => None,
        1 => Some(Assoc::After),
        _ => panic!("invalid side {value}"),
    }
}

fn document(rows: usize) -> Text {
    Text::new(
        (0..rows)
            .map(|i| format!("row{i} a😀β {}", "line ".repeat(24)))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn cases(kind: &str) -> impl Iterator<Item = Value> {
    include_str!("fixtures/view_scroll_snapshot.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |value| value["kind"] == kind)
}

#[test]
fn snapshots_match_capture_mapping_and_raw_restore_assignments_in_chrome() {
    let mut count = 0;
    for row in cases("snapshot") {
        count += 1;
        let name = &row["name"];
        let mut text = document(number(&row["rows"]));
        let geometry = &row["geometry"];
        let probe = &row["probe"];
        let snapshot = ScrollSnapshot::capture(
            &text,
            ScrollSnapshotGeometry {
                offset: offset(&geometry["offset"]),
                padding_top: pixel(&geometry["paddingTop"]),
                viewport_from: number(&geometry["viewportFrom"]),
                first_viewport_block: block(&geometry["firstViewportBlock"]),
            },
            |height| {
                assert_eq!(height, pixel(&probe["height"]), "{name}");
                Some(block(&probe["block"]))
            },
        )
        .unwrap();
        assert_eq!(snapshot.anchor(), number(&row["anchor"]), "{name}");

        let edits = row["edits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                Change::new(
                    number(&e["from"])..number(&e["to"]),
                    e["insert"].as_str().unwrap(),
                )
            })
            .collect();
        let changes = ChangeSet::new(edits).unwrap();
        let desc = changes.desc(text.len()).unwrap();
        let effect = SCROLL_SNAPSHOT.of(snapshot).map(&desc).unwrap();
        let mapped = *effect.get(&SCROLL_SNAPSHOT).unwrap();
        assert_eq!(mapped.anchor(), number(&row["mapped"]), "{name}");
        changes.apply_to(&mut text).unwrap();

        let restore = &row["restore"];
        if !restore["rows"].is_null() {
            text = document(number(&restore["rows"]));
        }
        let planned = mapped
            .plan_restore(&text, |anchor| {
                assert_eq!(anchor, number(&restore["anchor"]), "{name}");
                Some(pixel(&restore["top"]))
            })
            .unwrap();
        assert_eq!(planned, offset(&restore["assigned"]), "{name}");
    }
    assert_eq!(count, 7);
}

#[test]
fn fixed_scroller_reveal_matches_chrome_before_clamp_and_rounding() {
    let mut count = 0;
    for row in cases("reveal") {
        count += 1;
        let name = &row["name"];
        let text = document(number(&row["rows"]));
        let target = &row["target"];
        let (anchor, head) = (number(&target["anchor"]), number(&target["head"]));
        let range = if anchor == head {
            EditorSelection::cursor(head, side(&target["assoc"]), None, None)
        } else {
            SelectionRange::new(anchor, head)
        };
        let target = ScrollTarget {
            range,
            x: strategy(&target["x"]),
            y: strategy(&target["y"]),
            x_margin: pixel(&target["xMargin"]),
            y_margin: pixel(&target["yMargin"]),
        };
        let enclosure = &row["scroller"];
        let scroller = FixedScroller {
            clip: rect(&enclosure["clip"]),
            offset_width: pixel(&enclosure["offsetWidth"]),
            offset_height: pixel(&enclosure["offsetHeight"]),
            scale_x: pixel(&enclosure["scaleX"]),
            scale_y: pixel(&enclosure["scaleY"]),
            scroll_x: enclosure["scrollX"].as_bool().unwrap(),
            scroll_y: enclosure["scrollY"].as_bool().unwrap(),
            direction: match enclosure["direction"].as_str().unwrap() {
                "ltr" => Direction::Ltr,
                "rtl" => Direction::Rtl,
                _ => unreachable!(),
            },
        };
        let mut measured = row["measurements"].as_array().unwrap().iter();
        let actual = target
            .plan_reveal(&text, scroller, |pos, assoc| {
                let measured = measured.next().unwrap();
                assert_eq!(pos, number(&measured["pos"]), "{name}");
                assert_eq!(Some(assoc), side(&measured["side"]), "{name}");
                (!measured["rect"].is_null()).then(|| rect(&measured["rect"]))
            })
            .unwrap();
        assert!(measured.next().is_none(), "{name}");
        let expected = |axis: &str| {
            row["assigned"][axis]
                .as_f64()
                .map_or(0.0, |value| value - pixel(&row["before"][axis]))
        };
        assert!(
            (actual.x - expected("x")).abs() <= 1e-9,
            "{name}: {actual:?}"
        );
        assert!(
            (actual.y - expected("y")).abs() <= 1e-9,
            "{name}: {actual:?}"
        );
    }
    assert_eq!(count, 11);
}

#[test]
fn snapshots_reject_invalid_geometry_and_interior_byte_positions() {
    let text = Text::new("a😀β");
    let valid = ScrollSnapshotGeometry {
        offset: ScrollOffset { x: -3.0, y: 2.0 },
        padding_top: 4.0,
        viewport_from: 0,
        first_viewport_block: ScrollBlock { from: 0, top: 0.0 },
    };
    for geometry in [
        ScrollSnapshotGeometry {
            offset: ScrollOffset {
                x: f64::NAN,
                ..valid.offset
            },
            ..valid
        },
        ScrollSnapshotGeometry {
            padding_top: -1.0,
            ..valid
        },
        ScrollSnapshotGeometry {
            viewport_from: 2,
            ..valid
        },
        ScrollSnapshotGeometry {
            first_viewport_block: ScrollBlock {
                from: 0,
                top: f64::INFINITY,
            },
            ..valid
        },
    ] {
        assert_eq!(ScrollSnapshot::capture(&text, geometry, |_| panic!()), None);
    }
    for invalid in [
        None,
        Some(ScrollBlock { from: 2, top: 5.0 }),
        Some(ScrollBlock {
            from: 5,
            top: f64::NEG_INFINITY,
        }),
    ] {
        assert_eq!(ScrollSnapshot::capture(&text, valid, |_| invalid), None);
    }
    let snapshot =
        ScrollSnapshot::capture(&text, valid, |_| Some(ScrollBlock { from: 5, top: 0.0 })).unwrap();
    assert_eq!(snapshot.plan_restore(&text, |_| None), None);
    assert_eq!(snapshot.plan_restore(&text, |_| Some(f64::INFINITY)), None);
    assert_eq!(snapshot.map(&ChangeDesc::new(4, vec![]).unwrap()), None);
    // A length-only synthetic change description can produce an interior byte.
    let malformed = ChangeDesc::new(text.len(), vec![(0..0, 1)]).unwrap();
    let mapped = snapshot.map(&malformed).unwrap();
    assert_eq!(mapped.anchor(), 6);
    assert_eq!(mapped.plan_restore(&text, |_| panic!()), None);
}

#[test]
fn reveal_rejects_invalid_inputs_and_can_use_the_head_alone() {
    let text = Text::new("a😀β");
    let clip = ScrollRect {
        left: 0.0,
        top: 0.0,
        right: 100.0,
        bottom: 100.0,
    };
    let scroller = FixedScroller {
        clip,
        offset_width: 100.0,
        offset_height: 100.0,
        scale_x: 1.0,
        scale_y: 1.0,
        scroll_x: true,
        scroll_y: true,
        direction: Direction::Ltr,
    };
    let target = ScrollTarget::new(SelectionRange::new(1, 5));
    for invalid in [
        FixedScroller {
            offset_height: -0.5,
            ..scroller
        },
        FixedScroller {
            scale_y: 0.0,
            ..scroller
        },
        FixedScroller {
            scale_x: f64::INFINITY,
            ..scroller
        },
        FixedScroller {
            clip: ScrollRect {
                left: 101.0,
                ..clip
            },
            ..scroller
        },
        FixedScroller {
            clip: ScrollRect {
                left: -f64::MAX,
                right: f64::MAX,
                ..clip
            },
            ..scroller
        },
    ] {
        assert_eq!(target.plan_reveal(&text, invalid, |_, _| panic!()), None);
    }
    let invalid_target = ScrollTarget {
        x_margin: f64::NAN,
        ..target
    };
    assert_eq!(
        invalid_target.plan_reveal(&text, scroller, |_, _| panic!()),
        None
    );
    assert_eq!(
        ScrollTarget::at(2).plan_reveal(&text, scroller, |_, _| panic!()),
        None
    );
    assert_eq!(target.plan_reveal(&text, scroller, |_, _| None), None);
    assert_eq!(
        target.plan_reveal(&text, scroller, |_, _| {
            Some(ScrollRect {
                top: f64::NAN,
                ..clip
            })
        }),
        None
    );

    let head = ScrollRect {
        left: 50.0,
        top: 110.0,
        right: 50.0,
        bottom: 125.0,
    };
    assert_eq!(
        target.plan_reveal(&text, scroller, |pos, _| (pos == 5).then_some(head)),
        Some(ScrollDelta { x: 0.0, y: 30.0 })
    );
    assert_eq!(
        ScrollTarget::at(40).plan_reveal(&text, scroller, |pos, _| {
            assert_eq!(pos, text.len());
            Some(head)
        }),
        Some(ScrollDelta { x: 0.0, y: 30.0 })
    );
    let tiny_scale = FixedScroller {
        scale_y: f64::MIN_POSITIVE,
        ..scroller
    };
    assert_eq!(
        target.plan_reveal(&text, tiny_scale, |pos, _| (pos == 5).then_some(head)),
        None
    );
}
