use std::sync::Arc;

use gpui_editor_codemirror::view::{
    MarkDecoration,
    bidi::{
        BIDI_ISOLATED_RANGES, BidiIsolate, BidiSpan, Direction, bidi_spans, line_isolates,
        move_by_char, move_visually,
    },
};
use gpui_editor_codemirror::{
    Assoc, EditError, EditorSelection, EditorState, Extension, RangeSet, SelectionRange, Text,
};
use serde_json::Value;

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn direction(value: &Value) -> Direction {
    match value.as_u64().unwrap() {
        0 => Direction::Ltr,
        1 => Direction::Rtl,
        other => panic!("invalid direction {other}"),
    }
}

fn isolate(value: &Value) -> BidiIsolate {
    BidiIsolate {
        from: byte(&value[0]),
        to: byte(&value[1]),
        direction: (!value[2].is_null()).then(|| direction(&value[2])),
        inner: value[3].as_array().unwrap().iter().map(isolate).collect(),
    }
}

fn assoc(value: &Value) -> Option<Assoc> {
    match value.as_i64().unwrap() {
        -1 => Some(Assoc::Before),
        0 => None,
        1 => Some(Assoc::After),
        other => panic!("invalid cursor association {other}"),
    }
}

fn cursor(value: &Value) -> SelectionRange {
    EditorSelection::cursor(
        byte(&value[0]),
        assoc(&value[1]),
        value[2].as_u64().map(|level| u8::try_from(level).unwrap()),
        None,
    )
}

#[test]
fn spans_and_motion_match_pinned_codemirror() {
    for row in include_str!("fixtures/view_bidi.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let id = row["id"].as_str().unwrap();
        let dir = direction(&row["dir"]);
        let doc = if let Some(line) = row["text"].as_str() {
            line.to_owned()
        } else {
            let repeat = &row["text"];
            repeat[0].as_str().unwrap().repeat(byte(&repeat[1])) + repeat[2].as_str().unwrap()
        };
        let doc = Text::new(row["prefix"].as_str().unwrap().to_owned() + &doc);
        let line = doc.line(2).unwrap();
        let isolates: Vec<_> = row["isolates"]
            .as_array()
            .unwrap()
            .iter()
            .map(isolate)
            .collect();
        let spans = bidi_spans(line.text, dir, &isolates).unwrap();
        let expected = row["spans"].as_array().unwrap();
        assert_eq!(spans.len(), expected.len(), "{id}: spans = {spans:?}");
        for (&span, expected) in spans.iter().zip(expected) {
            assert_eq!(
                span,
                BidiSpan {
                    from: byte(&expected[0]),
                    to: byte(&expected[1]),
                    level: byte(&expected[2]),
                },
                "{id}: spans = {spans:?}"
            );
            assert_eq!(span.direction(), direction(&expected[3]), "{id}");
            assert_eq!(span.side(false, dir), byte(&expected[4]), "{id}");
            assert_eq!(span.side(true, dir), byte(&expected[5]), "{id}");
        }
        for lookup in row["finds"].as_array().unwrap() {
            assert_eq!(
                BidiSpan::find(
                    &spans,
                    byte(&lookup[0]),
                    (!lookup[2].is_null()).then(|| byte(&lookup[2])),
                    assoc(&lookup[1]),
                ),
                Some(byte(&lookup[3])),
                "{id}: {lookup}"
            );
        }
        for walk in row["walks"].as_array().unwrap() {
            let forward = walk["forward"].as_bool().unwrap();
            let mut at = cursor(&walk["start"]);
            for expected in walk["moves"].as_array().unwrap() {
                let step = move_visually(&line, &spans, dir, at, forward)
                    .unwrap_or_else(|| panic!("{id}: traversal ended early, expected {expected}"));
                let expected_cursor = cursor(expected);
                assert!(
                    step.selection.eq_with_assoc(&expected_cursor),
                    "{id}: got {:?}, expected {:?}",
                    step.selection,
                    expected_cursor
                );
                assert_eq!(
                    step.selection.bidi_level(),
                    expected_cursor.bidi_level(),
                    "{id}"
                );
                assert_eq!(step.moved_over, expected[3].as_str().unwrap(), "{id}");
                at = step.selection;
            }
            assert!(
                move_visually(&line, &spans, dir, at, forward).is_none(),
                "{id}"
            );
        }
    }
}

#[test]
fn document_motion_crosses_crlf_empty_and_rtl_lines() {
    // `EditorView.moveByChar` from the pinned 6.44.0 archive. The oracle uses
    // RTL for line two and maps normalized UTF-16 offsets to original bytes.
    let text = Text::new("na\r\nאב\r\n\r\nx😀");
    let order = |line: &gpui_editor_codemirror::Line<'_>| {
        let direction = if line.number == 2 {
            Direction::Rtl
        } else {
            Direction::Ltr
        };
        Ok((direction, bidi_spans(line.text, direction, &[])?))
    };
    let record = |range: SelectionRange| (range.head(), range.assoc(), range.bidi_level());
    let before = Some(Assoc::Before);
    let after = Some(Assoc::After);
    for (forward, start, expected) in [
        (
            true,
            EditorSelection::cursor(0, before, None, None),
            [
                (1, before, Some(0)),
                (2, after, None),
                (4, before, None),
                (6, before, Some(1)),
                (8, after, None),
                (10, before, None),
                (12, before, None),
                (13, before, Some(0)),
                (17, after, None),
            ],
        ),
        (
            false,
            EditorSelection::cursor(17, after, None, None),
            [
                (13, after, Some(0)),
                (12, before, None),
                (10, after, None),
                (8, after, None),
                (6, after, Some(1)),
                (4, before, None),
                (2, after, None),
                (1, after, Some(0)),
                (0, before, None),
            ],
        ),
    ] {
        let mut at = start;
        for row in expected {
            at = move_by_char(&text, at, forward, order, None).unwrap();
            assert_eq!(record(at), row);
        }
        assert_eq!(
            record(move_by_char(&text, at, forward, order, None).unwrap()),
            record(at)
        );
    }
    let mut seen = Vec::new();
    let mut letters = |cluster: &str| {
        seen.push(cluster.to_owned());
        cluster == "\n" || cluster.chars().all(|ch| ch.is_ascii_lowercase())
    };
    let start = EditorSelection::cursor(0, before, None, None);
    let group = move_by_char(&text, start, true, order, Some(&mut letters)).unwrap();
    assert_eq!(record(group), (4, before, None));
    assert_eq!(seen, ["n", "a", "\n", "ב"]);
}

#[test]
fn decoration_sources_layer_and_clip_isolates_by_line() {
    // Measured from the pinned EditorView.bidiSpans with two overlapping
    // bidiIsolatedRanges sources. One auto-direction mark crosses CRLF.
    let rtl = MarkDecoration::new("").with_bidi_direction(Some(Direction::Rtl));
    let ltr = MarkDecoration::new("").with_bidi_direction(Some(Direction::Ltr));
    let first = RangeSet::of(vec![rtl.range(2..17).unwrap()], false).unwrap();
    let second = RangeSet::of(
        vec![
            ltr.range(7..10).unwrap(),
            MarkDecoration::new("").range(9..20).unwrap(),
        ],
        false,
    )
    .unwrap();
    let state = EditorState::with_extensions(
        "A אב 12 Z\r\nx גד y",
        Extension::group([
            BIDI_ISOLATED_RANGES.of(Arc::new(first)),
            BIDI_ISOLATED_RANGES.of(Arc::new(second)),
        ]),
    )
    .unwrap();
    for (line_number, expected) in [
        (
            1,
            vec![(0, 2, 0), (2, 7, 1), (7, 9, 2), (9, 10, 1), (10, 11, 2)],
        ),
        (2, vec![(4, 7, 1), (1, 4, 1), (0, 1, 2), (7, 8, 0)]),
    ] {
        let line = state.text().line(line_number).unwrap();
        let isolates = line_isolates(&state, line_number).unwrap();
        let spans = bidi_spans(line.text, Direction::Ltr, &isolates).unwrap();
        assert_eq!(
            spans
                .iter()
                .map(|span| (span.from, span.to, span.level))
                .collect::<Vec<_>>(),
            expected
        );
    }
    assert_eq!(line_isolates(&state, 3), Err(EditError::InvalidRange));
    let invalid = RangeSet::of(vec![MarkDecoration::new("").range(1..3).unwrap()], false).unwrap();
    let state =
        EditorState::with_extensions("😀", BIDI_ISOLATED_RANGES.of(Arc::new(invalid))).unwrap();
    assert_eq!(line_isolates(&state, 1), Err(EditError::InvalidRange));
}

#[test]
fn deeply_nested_isolates_are_rejected() {
    let mut inner = Vec::new();
    for _ in 0..65 {
        inner = vec![BidiIsolate {
            from: 0,
            to: 2,
            direction: None,
            inner,
        }];
    }
    assert_eq!(
        bidi_spans("א", Direction::Ltr, &inner),
        Err(EditError::InvalidRange)
    );
}

#[test]
fn invalid_utf8_coordinates_are_rejected() {
    let iso = |from, to| BidiIsolate {
        from,
        to,
        direction: Some(Direction::Rtl),
        inner: vec![],
    };
    for isolates in [
        vec![iso(2, 3)],
        vec![iso(1, 6)],
        vec![iso(3, 3)],
        vec![iso(1, 5), iso(3, 5)],
        vec![iso(3, 5), iso(1, 3)],
        vec![BidiIsolate {
            inner: vec![iso(3, 5)],
            ..iso(1, 3)
        }],
    ] {
        assert_eq!(
            bidi_spans("aאב", Direction::Ltr, &isolates),
            Err(EditError::InvalidRange)
        );
    }
    let text = Text::new("x\nאב");
    let line = text.line(2).unwrap();
    let spans = bidi_spans(line.text, Direction::Ltr, &[]).unwrap();
    for head in [0, 3, 7] {
        let start = EditorSelection::cursor(head, None, None, None);
        assert!(move_visually(&line, &spans, Direction::Ltr, start, true).is_none());
    }
}
