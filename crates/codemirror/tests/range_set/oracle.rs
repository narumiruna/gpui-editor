use serde_json::{Value as Json, json};

use super::{
    Change, ChangeDesc, ChangeSet, Difference, Event, RangeSet, RangeSetUpdate, Recorder, Value,
    compare, entry, mark, point, set, sides,
};

fn fixture() -> Json {
    serde_json::from_str(include_str!("../fixtures/range_set_oracle.json")).unwrap()
}

fn iter(sets: &[&RangeSet<Value>], from: usize) -> Json {
    RangeSet::iter_sets(sets, from)
        .map(|(rank, range)| json!([rank, range.from, range.to, range.value.name]))
        .collect()
}

fn spans(sets: &[&RangeSet<Value>], from: usize, to: usize, min: Option<usize>) -> Json {
    let mut recorder = Recorder::default();
    let open = RangeSet::spans(sets, from, to, &mut recorder, min);
    let events: Json = recorder
        .0
        .into_iter()
        .map(|event| match event {
            Event::Span(a, b, active, open) => json!(["span", a, b, active, open]),
            Event::Point(a, b, value, active, open, rank) => {
                json!(["point", a, b, value, active, open, rank])
            }
        })
        .collect();
    json!({ "events": events, "open": open })
}

fn diff(
    old: &[&RangeSet<Value>],
    new: &[&RangeSet<Value>],
    changes: &ChangeDesc,
    min: Option<usize>,
) -> Json {
    compare(old, new, changes, min)
        .into_iter()
        .map(|event| match event {
            Difference::Range(a, b, old, new) => json!(["range", a, b, old, new]),
            Difference::Point(a, b, old, new) => json!(["point", a, b, old, new]),
            Difference::Bound(pos) => json!(["bound", pos]),
        })
        .collect()
}

#[test]
fn maps_pinned_range_geometry_without_inserted_text() {
    use gpui_editor_codemirror::{MapMode, RangeValue};

    #[derive(Clone)]
    struct MappedValue<'a> {
        name: &'a str,
        sides: (i32, i32),
        point: bool,
        mode: MapMode,
    }

    impl RangeValue for MappedValue<'_> {
        fn start_side(&self) -> i32 {
            self.sides.0
        }
        fn end_side(&self) -> i32 {
            self.sides.1
        }
        fn is_point(&self) -> bool {
            self.point
        }
        fn map_mode(&self) -> MapMode {
            self.mode
        }
    }

    for line in include_str!("../fixtures/range_set_description.jsonl").lines() {
        let row: Json = serde_json::from_str(line).unwrap();
        let offset = |value: &Json| usize::try_from(value.as_u64().unwrap()).unwrap();
        let side = |value: &Json| i32::try_from(value.as_i64().unwrap()).unwrap();
        let desc = ChangeDesc::from_json(&row["sections"]).unwrap();
        assert_eq!(desc.to_json(), row["sections"]);
        let ranges = row["ranges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|range| {
                MappedValue {
                    name: range[2].as_str().unwrap(),
                    sides: (side(&range[3]), side(&range[4])),
                    point: range[5].as_bool().unwrap(),
                    mode: match range[6].as_str().unwrap() {
                        "Simple" => MapMode::Simple,
                        "TrackBefore" => MapMode::TrackBefore,
                        "TrackDel" => MapMode::TrackDel,
                        "TrackAfter" => MapMode::TrackAfter,
                        unknown => panic!("unknown mapping mode {unknown}"),
                    },
                }
                .range(offset(&range[0]), offset(&range[1]))
            })
            .collect();
        let before = RangeSet::of(ranges, true).unwrap();
        let after = before.map_desc(&desc).unwrap();
        let actual: Json = after
            .iter(0)
            .map(|range| json!([range.from, range.to, range.value.name]))
            .collect();
        assert_eq!(actual, row["mapped"], "{}", row["name"]);
        assert_eq!(before.len(), row["ranges"].as_array().unwrap().len());
    }
}

#[test]
fn mapped_and_rebuilt_values_keep_their_pinned_identity() {
    let oracle: Json =
        serde_json::from_str(include_str!("../fixtures/range_set_value_identity.json")).unwrap();
    let tracked = |mut range: super::Range<Value>| {
        range.value.identity_only = true;
        range.value.start = 0;
        range.value.end = 0;
        set(vec![range])
    };
    let old_mark = tracked(mark(0, 10, "wide"));
    let old_point = tracked(point(0, 10, "point"));
    let edit = ChangeDesc::new(10, vec![(4..6, 2)]).unwrap();
    let mapped_mark = old_mark.map_desc(&edit).unwrap();
    let mapped_point = old_point.map_desc(&edit).unwrap();
    let joined = RangeSet::join(&[&old_mark, &RangeSet::empty()]);
    let rebuilt = old_mark
        .update(RangeSetUpdate {
            filter: Some(&mut |_| true),
            ..Default::default()
        })
        .unwrap();
    let other_mark = tracked(mark(0, 10, "wide"));
    let other_point = tracked(point(0, 10, "point"));
    let plain = unchanged(10);
    for (name, before, after, changes) in [
        ("mappedMark", &old_mark, &mapped_mark, &edit),
        ("mappedPoint", &old_point, &mapped_point, &edit),
        (
            "joinedAndMapped",
            &old_mark,
            &joined.map_desc(&edit).unwrap(),
            &edit,
        ),
        ("rebuilt", &old_mark, &rebuilt, &plain),
        ("distinctMark", &old_mark, &other_mark, &edit),
        ("distinctPoint", &old_point, &other_point, &plain),
    ] {
        assert_eq!(
            diff(&[before], &[after], changes, None),
            oracle[name],
            "{name}"
        );
    }
    assert_eq!(
        json!(RangeSet::eq(&[&old_point], &[&mapped_point], 0, Some(10))),
        oracle["equalMappedPoint"]
    );
    assert_eq!(
        json!(RangeSet::eq(&[&old_point], &[&other_point], 0, Some(10))),
        oracle["equalDistinctPoint"]
    );

    let value = old_point.iter(0).next().unwrap().value.clone();
    let shared = std::sync::Arc::new(value.clone());
    let of = |value| RangeSet::of(vec![super::Range::new(0, 10, value)], false).unwrap();
    let first = of(shared.clone());
    let second = of(shared);
    let independent = of(std::sync::Arc::new(value));
    assert_eq!(
        json!(RangeSet::eq(&[&first], &[&second], 0, Some(10))),
        oracle["equalSharedValue"]
    );
    assert_eq!(
        json!(RangeSet::eq(&[&first], &[&independent], 0, Some(10))),
        oracle["equalDistinctPoint"]
    );
}

fn unchanged(length: usize) -> ChangeDesc {
    ChangeDesc::new(length, vec![]).unwrap()
}

fn add(old: &RangeSet<Value>, range: super::Range<Value>) -> RangeSet<Value> {
    old.update(RangeSetUpdate {
        add: vec![range],
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn oracle_matches_the_archive_pin() {
    let inventory: Json =
        serde_json::from_str(include_str!("../../../../docs/codemirror-upstream.json")).unwrap();
    let state = inventory["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pkg| pkg["name"] == "@codemirror/state")
        .unwrap();
    let source = &fixture()["source"];
    assert_eq!(source["version"], state["version"]);
    assert_eq!(source["integrity"], state["integrity"]);
}

#[test]
fn tied_point_traversal_matches_the_layer_heap() {
    let tied = set(["first", "second", "third", "fourth"]
        .map(|name| point(1, 3, name))
        .to_vec());
    let cover = set(vec![mark(0, 4, "cover")]);
    let joined = RangeSet::join(&[&tied, &cover]);
    let empty = RangeSet::empty();
    let empty_points = set(["a", "b", "c", "d"]
        .map(|name| sides(point(1, 1, name), 1, 2))
        .to_vec());
    let oracle = &fixture()["ties"];
    assert_eq!(iter(&[&tied], 0), oracle["iter"]);
    let single: Json = tied
        .iter(0)
        .map(|range| json!([0, range.from, range.to, range.value.name]))
        .collect();
    assert_eq!(single, oracle["iter"]);
    assert_eq!(iter(&[&joined], 2), oracle["join"]);
    assert_eq!(spans(&[&empty, &tied], 2, 3, None), oracle["spans"]);
    assert_eq!(diff(&[], &[&tied], &unchanged(4), None), oracle["diff"]);
    assert_eq!(spans(&[&empty_points], 1, 1, None), oracle["empty"]);
}

#[test]
fn layer_precedence_and_point_filters_match_the_oracle() {
    let low = set(vec![
        mark(0, 12, "wide"),
        mark(0, 12, "equal"),
        sides(point(2, 8, "main"), 0, 1),
        point(4, 8, "shadowed"),
        sides(point(8, 8, "before"), -1, -1),
        point(8, 8, "after"),
    ]);
    let high = set(vec![
        mark(1, 12, "above"),
        sides(point(2, 8, "loser"), 0, 1),
        point(6, 10, "cross"),
    ]);
    let empty = RangeSet::empty();
    let sets = [&low, &empty, &high];
    let deeper = set(vec![mark(0, 4, "cover"), point(1, 3, "deep")]);
    let joined = RangeSet::join(&[&deeper, &empty]);
    let oracle = &fixture()["precedence"];
    let mut between = Vec::new();
    low.between(2, 8, |range| {
        between.push(entry(range));
        true
    });
    assert_eq!(json!(between), oracle["between"]);
    assert_eq!(iter(&sets, 3), oracle["iter"]);
    assert_eq!(spans(&sets, 0, 12, None), oracle["spans"]);
    assert_eq!(spans(&sets, 3, 8, None), oracle["clipped"]);
    assert_eq!(spans(&[&deeper], 0, 4, Some(0)), oracle["lowerLayer"]);
    assert_eq!(
        diff(&[], &[&deeper], &unchanged(4), Some(0)),
        oracle["skipSet"]
    );
    assert_eq!(
        diff(&[], &[&joined], &unchanged(4), Some(0)),
        oracle["joinedSet"]
    );
}

#[test]
fn comparison_skips_aligned_shared_chunks_but_not_rebuilt_chunks() {
    let shared = set(vec![mark(0, 10, "shared"), point(11, 11, "widget")]);
    let extra = set(vec![mark(2, 4, "extra")]);
    let updated = add(&shared, mark(12, 14, "add"));
    let rebuilt = shared
        .update(RangeSetUpdate {
            add: vec![mark(12, 14, "add")],
            filter_range: Some(11..=11),
            filter: Some(&mut |_| true),
            ..Default::default()
        })
        .unwrap();
    let shift = ChangeSet::single(Change::new(0..0, "++")).unwrap();
    let later = set(vec![mark(3, 10, "later"), point(11, 11, "widget")]);
    let mapped = add(&later.map(&shift), mark(13, 15, "extra"));
    let layered = set(vec![mark(0, 10, "top"), mark(1, 9, "lower")]);
    let atomic = set(vec![point(1, 5, "atom")]);
    let across = set(vec![mark(0, 7, "across")]);
    let full = set((0..250).map(|i| point(i, i, "same")).collect());
    let full_updated = add(&full, point(500, 500, "extra"));
    let oracle = &fixture()["reuse"];
    let text = unchanged(16);
    assert_eq!(
        diff(&[&shared], &[&shared, &extra], &text, None),
        oracle["unchanged"]
    );
    assert_eq!(
        diff(&[&shared], &[&updated, &extra], &text, None),
        oracle["updated"]
    );
    assert_eq!(
        diff(&[&shared], &[&rebuilt, &extra], &text, None),
        oracle["rebuilt"]
    );
    assert_eq!(
        diff(&[&later], &[&mapped], &shift.desc(16).unwrap(), None),
        oracle["shifted"]
    );
    assert_eq!(
        diff(&[&layered], &[&layered, &extra], &text, None),
        oracle["lowerChunk"]
    );
    assert_eq!(
        diff(&[&atomic], &[&atomic, &across], &text, None),
        oracle["atomicChunk"]
    );
    assert_eq!(
        json!(RangeSet::eq(&[&full], &[&full_updated], 0, Some(100))),
        oracle["equal"]
    );
    assert_eq!(
        json!(RangeSet::eq(&[&full], &[&full.clone()], 0, Some(100))),
        oracle["cloned"]
    );
}

#[test]
fn points_clip_to_query_edges_and_unchanged_diff_gaps() {
    let old = set(vec![
        mark(0, 12, "keep"),
        sides(point(2, 9, "old"), -1, 1),
        point(9, 9, "edge"),
    ]);
    let changes =
        ChangeSet::new(vec![Change::new(3..6, "+++++"), Change::new(9..11, "+")]).unwrap();
    let altered = old
        .map(&changes)
        .update(RangeSetUpdate {
            add: vec![sides(point(8, 12, "new"), -1, 2)],
            filter: Some(&mut |range| range.value.name != "old"),
            ..Default::default()
        })
        .unwrap();
    let oracle = &fixture()["clipping"];
    assert_eq!(spans(&[&old], 0, 2, None), oracle["left"]);
    assert_eq!(spans(&[&old], 4, 7, None), oracle["both"]);
    assert_eq!(spans(&[&old], 9, 9, None), oracle["right"]);
    assert_eq!(
        diff(&[&old], &[&altered], &changes.desc(12).unwrap(), None),
        oracle["diff"]
    );
}
