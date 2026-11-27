//! Range and mapping cases checked against `@codemirror/view` 6.44.0 and
//! `@codemirror/state` 6.7.6 without a browser. Rust uses UTF-8 byte offsets.

use std::ops::Range as ByteRange;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, EditorState, MapMode, Range, RangeValue, Text, Transaction,
    view::{
        MarkDecoration,
        decorations::{
            BlockPlacement, Decoration, DecorationKind, DecorationSet, DecorationSetError,
            LineDecoration, ReplaceSpec, WidgetSpec,
        },
    },
};

type Decor = Decoration<&'static str>;

fn widget(label: &'static str, side: i32, block: bool, inline_order: bool) -> Decor {
    Decor::widget(
        label,
        WidgetSpec {
            side,
            block,
            inline_order,
        },
    )
}

fn line() -> Decor {
    Decor::line(LineDecoration::new("line"))
}

fn label(value: &Decor) -> &str {
    match value.kind() {
        DecorationKind::Mark(mark) => mark.class(),
        DecorationKind::Line(line) => line.class(),
        _ => value.widget_value().copied().unwrap_or("hidden"),
    }
}

fn sorted_order<'a>(set: &'a DecorationSet<&'static str>) -> Vec<(&'a str, usize, usize)> {
    set.iter(0)
        .map(|range| (label(&range.value), range.from, range.to))
        .collect()
}

fn offsets<W>(set: &DecorationSet<W>) -> Vec<(usize, usize)> {
    set.iter(0).map(|range| (range.from, range.to)).collect()
}

fn change(range: ByteRange<usize>, insert: &str) -> ChangeSet {
    ChangeSet::single(Change::new(range, insert)).unwrap()
}

#[test]
fn point_order_distinguishes_block_inline_and_line() {
    let text = Text::new("a\nbc\nde");
    let decoration = [
        widget("block-after", 1, true, false),
        widget("inline-clamped", i32::MIN, false, false),
        widget("mixed-before", -2, true, true),
        line(),
        widget("inline-default", 0, false, false),
        widget("mixed-after", 2, true, true),
        widget("block-before", -1, true, false),
        widget("inline-after", 1, false, false),
        widget("inline-after-clamped", i32::MAX, false, false),
    ];
    let ranges = decoration
        .iter()
        .map(|d| d.try_range(2..2).unwrap())
        .collect();
    let set = DecorationSet::new(&text, ranges, true).unwrap();
    let sorted = set
        .iter(2)
        .map(|range| {
            let value = &range.value;
            assert!(value.is_point());
            assert_eq!(value.start_side(), value.end_side());
            (label(value), value.start_side(), value.block_placement())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        sorted,
        [
            ("block-before", -400_000_001, Some(BlockPlacement::Before)),
            ("line", -200_000_000, None),
            ("inline-clamped", -100_010_000, None),
            ("mixed-before", -100_000_002, Some(BlockPlacement::Before)),
            ("inline-default", -100_000_000, None),
            ("inline-after", 100_000_001, None),
            ("mixed-after", 100_000_002, Some(BlockPlacement::After)),
            ("inline-after-clamped", 100_010_000, None),
            ("block-after", 300_000_001, Some(BlockPlacement::After)),
        ]
    );
    assert_eq!(
        DecorationSet::new(
            &text,
            decoration
                .iter()
                .map(|d| d.try_range(2..2).unwrap())
                .collect(),
            false,
        )
        .unwrap_err(),
        DecorationSetError::UnsortedRanges,
    );
}

#[test]
fn points_track_the_correct_side_of_changed_content() {
    let text = Text::new("a\nbc\nde");
    let points = [
        (widget("before", 0, true, false), MapMode::TrackBefore),
        (line(), MapMode::TrackBefore),
        (widget("left", 0, false, false), MapMode::TrackDel),
        (widget("right", 1, false, false), MapMode::TrackDel),
        (widget("after", 1, true, false), MapMode::TrackAfter),
    ];
    let set = DecorationSet::new(
        &text,
        points
            .into_iter()
            .map(|(d, track)| {
                assert_eq!(d.map_mode(), track);
                d.try_range(2..2).unwrap()
            })
            .collect(),
        false,
    )
    .unwrap();
    assert_eq!(
        sorted_order(&set.map(&change(2..2, "x\n"))),
        [
            ("before", 2, 2),
            ("line", 2, 2),
            ("left", 2, 2),
            ("right", 4, 4),
            ("after", 4, 4),
        ]
    );
    assert_eq!(
        sorted_order(&set.map(&change(1..2, "X"))),
        [("left", 2, 2), ("right", 2, 2), ("after", 2, 2)]
    );
    assert_eq!(
        sorted_order(&set.map(&change(2..3, "X"))),
        [
            ("before", 2, 2),
            ("line", 2, 2),
            ("left", 2, 2),
            ("right", 2, 2),
        ]
    );
    assert!(set.map(&change(1..3, "")).as_range_set().is_empty());
}

#[test]
fn marks_and_replacements_follow_edge_defaults_and_overrides() {
    let text = Text::new("a\nbc\nde");
    let inserts = ChangeSet::new(vec![Change::new(2..2, "X"), Change::new(4..4, "Y")]).unwrap();
    let block = ReplaceSpec {
        block: true,
        ..ReplaceSpec::default()
    };
    let cases = [
        (
            Decor::mark(MarkDecoration::new("mark")),
            (500_000_000, -600_000_000),
            (3, 5),
            false,
        ),
        (
            Decor::mark(MarkDecoration::new("mark").with_inclusive_edges(true, true)),
            (-1, 1),
            (2, 6),
            true,
        ),
        (
            Decor::replace(None, ReplaceSpec::default()),
            (499_999_999, -599_999_999),
            (3, 5),
            false,
        ),
        (
            Decor::replace(None, block),
            (-300_000_001, 200_000_001),
            (2, 6),
            true,
        ),
        (
            Decor::replace(
                None,
                ReplaceSpec {
                    inclusive: Some(true),
                    inclusive_end: Some(false),
                    ..ReplaceSpec::default()
                },
            ),
            (-2, -599_999_999),
            (2, 5),
            false,
        ),
        (
            Decor::replace(
                Some("replacement"),
                ReplaceSpec {
                    inclusive_start: Some(false),
                    ..block
                },
            ),
            (499_999_999, 200_000_001),
            (3, 6),
            false,
        ),
        (
            Decor::replace(
                None,
                ReplaceSpec {
                    inclusive: Some(false),
                    ..block
                },
            ),
            (499_999_999, -599_999_999),
            (3, 5),
            false,
        ),
    ];
    for (value, sides, inserted, survives_deletion) in cases {
        assert_eq!((value.start_side(), value.end_side()), sides);
        assert_eq!(
            value.is_point(),
            !matches!(value.kind(), DecorationKind::Mark(_))
        );
        let set = DecorationSet::new(&text, vec![value.try_range(2..4).unwrap()], false).unwrap();
        assert_eq!(offsets(&set.map(&inserts)), [inserted]);
        let deleted = set.map(&change(2..4, ""));
        if survives_deletion {
            assert_eq!(offsets(&deleted), [(2, 2)]);
        } else {
            assert!(deleted.as_range_set().is_empty());
        }
        assert_eq!(offsets(&set.map(&change(2..4, "X"))), [(2, 3)]);
    }
}

#[test]
fn validates_shape_and_utf8_boundaries() {
    let text = Text::new("é\n😀");
    let small = 2;
    let large = 3;
    let mark = Decor::mark(MarkDecoration::new("mark"));
    let point = widget("widget", 0, false, false);
    let hidden = Decor::replace(None, ReplaceSpec::default());
    let block = Decor::replace(
        None,
        ReplaceSpec {
            block: true,
            ..ReplaceSpec::default()
        },
    );
    for invalid in [
        mark.try_range(2..2),
        mark.try_range(large..small),
        line().try_range(2..3),
        point.try_range(2..3),
        hidden.try_range(2..2),
        block.try_range(large..small),
    ] {
        assert_eq!(invalid.unwrap_err(), EditError::InvalidRange);
    }
    for range in [
        Range::new(0, 0, mark), // Public Range fields cannot bypass set validation.
        Range::new(2, 3, point),
        Range::new(2, 3, line()),
        Range::new(2, 2, hidden.clone()),
        hidden.try_range(3..8).unwrap(),
        hidden.try_range(1..3).unwrap(),
        hidden.try_range(3..5).unwrap(),
    ] {
        assert_eq!(
            DecorationSet::new(&text, vec![range], false).unwrap_err(),
            DecorationSetError::InvalidRange,
        );
    }

    assert_eq!(block.block_placement(), Some(BlockPlacement::Range));
    let at = block.try_range(3..3).unwrap();
    assert_eq!(at.from, at.to);
    let half = Decor::replace(
        None,
        ReplaceSpec {
            inclusive_end: Some(true),
            ..ReplaceSpec::default()
        },
    );
    assert!(half.try_range(3..3).is_ok());
    let mid_line = widget("block", 0, true, false).try_range(2..2).unwrap();
    assert_eq!(
        mid_line.value.block_placement(),
        Some(BlockPlacement::Before)
    );
    assert!(DecorationSet::new(&text, vec![mid_line, at], false).is_ok());
}

#[test]
fn transaction_mapping_preserves_typed_widget_identity() {
    #[derive(Debug, PartialEq)]
    struct Widget(&'static str);

    let mut state = EditorState::new("é\n😀z");
    let widget = Decoration::widget(Widget("shared"), WidgetSpec::default());
    let equivalent = Decoration::widget(Widget("shared"), WidgetSpec::default());
    assert!(!widget.same_instance(&equivalent));
    assert!(RangeValue::eq(&widget, &equivalent));
    assert!(!RangeValue::eq(
        &widget,
        &Decoration::widget(Widget("other"), WidgetSpec::default())
    ));
    let set = DecorationSet::new(
        state.text(),
        vec![
            Decoration::line(LineDecoration::new("line"))
                .try_range(3..3)
                .unwrap(),
            Decoration::mark(MarkDecoration::new("mark"))
                .try_range(3..7)
                .unwrap(),
            widget.try_range(7..7).unwrap(),
            Decoration::replace(None, ReplaceSpec::default())
                .try_range(7..8)
                .unwrap(),
        ],
        false,
    )
    .unwrap();
    let txn = Transaction::new(
        ChangeSet::new(vec![Change::new(0..0, "漢"), Change::new(7..7, "é")]).unwrap(),
    );
    state.update(txn.clone()).unwrap();
    assert_eq!(state.text().as_str(), "漢é\n😀éz");
    let set = set.map(txn.changes());
    assert_eq!(offsets(&set), [(6, 6), (6, 10), (10, 10), (12, 13)]);
    let mapped = &set.iter(10).nth(1).unwrap().value;
    assert_eq!(mapped.widget_value(), Some(&Widget("shared")));
    assert!(widget.same_instance(mapped));
    assert!(std::ptr::eq(
        widget.widget_value().unwrap(),
        mapped.widget_value().unwrap(),
    ));
}
