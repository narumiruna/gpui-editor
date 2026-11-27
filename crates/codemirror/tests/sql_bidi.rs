//! The fixture runs the pinned JavaScript SQL grammar, `bidiIsolates`, and
//! `EditorView.bidiSpans`. See `fixtures/sql_bidi.mjs`.

use std::ops::Range;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, EditorState, Precedence, RangeValue, Text,
    language::sql::{SqlBidiIsolate, SqlBidiIsolateOptions, SqlDialect, SqlDocument},
    view::{
        Viewport,
        bidi::{BIDI_ISOLATED_RANGES, BidiIsolate, Direction, bidi_spans, line_isolates},
    },
};
use serde_json::{Value, json};

fn usize_at(value: &Value) -> usize {
    value.as_u64().unwrap() as usize
}

fn range(value: &Value) -> Range<usize> {
    usize_at(&value[0])..usize_at(&value[1])
}

fn isolate_json(isolate: &BidiIsolate) -> Value {
    assert_eq!(isolate.direction, None);
    json!([
        isolate.from,
        isolate.to,
        isolate.inner.iter().map(isolate_json).collect::<Vec<_>>()
    ])
}

#[test]
fn pinned_sql_properties_marks_line_clipping_and_bidi_spans() {
    for line in include_str!("fixtures/sql_bidi.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let id = case["id"].as_str().unwrap();
        let input = &case["input"];
        let source = input.as_str().map_or_else(
            || {
                format!(
                    "{}{}{}",
                    input[0].as_str().unwrap(),
                    "a".repeat(usize_at(&input[1])),
                    input[2].as_str().unwrap(),
                )
            },
            str::to_owned,
        );
        let dialect = SqlDialect::from_export(case["dialect"].as_str().unwrap()).unwrap();
        let document = SqlDocument::parse_with_dialect(Text::new(&source), dialect);
        assert!(!document.is_recovered(), "{id}");
        let syntax: Vec<_> = document
            .nodes()
            .filter_map(|node| match node.kind().bidi_isolate()? {
                SqlBidiIsolate::Auto => {
                    let span = node.range();
                    Some(json!([node.kind().name(), span.start, span.end]))
                }
            })
            .collect();
        assert_eq!(json!(syntax), case["syntax"], "{id}: syntax properties");
        let direction = match usize_at(&case["direction"]) {
            0 => Direction::Ltr,
            1 => Direction::Rtl,
            _ => unreachable!("invalid fixture direction"),
        };
        let marks = document
            .bidi_isolate_marks(
                &Viewport::around(
                    document.text(),
                    case["visible"].as_array().unwrap().iter().map(range),
                )
                .unwrap(),
                SqlBidiIsolateOptions {
                    always_isolate: case["always"].as_bool().unwrap(),
                    text_direction: direction,
                    per_line_text_direction: case["perLine"].as_bool().unwrap(),
                },
            )
            .unwrap();
        let ranges: Vec<_> = marks
            .iter(0)
            .map(|range| {
                assert_eq!(range.value.class(), "cm-iso", "{id}");
                assert_eq!(range.value.bidi_direction(), None, "{id}");
                assert!(range.value.start_side() < 0, "{id}: inclusive start");
                assert!(range.value.end_side() > 0, "{id}: inclusive end");
                [range.from, range.to]
            })
            .collect();
        assert_eq!(json!(ranges), case["ranges"], "{id}: visible marks");
        let state = EditorState::with_extensions(
            &source,
            BIDI_ISOLATED_RANGES
                .of(marks)
                .with_precedence(Precedence::Lowest),
        )
        .unwrap();
        for (index, expected) in case["lines"].as_array().unwrap().iter().enumerate() {
            let number = index + 1;
            let line = state.text().line(number).unwrap();
            let isolates = line_isolates(&state, number).unwrap();
            let clipped = isolates.iter().map(isolate_json).collect::<Vec<_>>();
            assert_eq!(json!(clipped), expected["isolated"], "{id}:{number}");
            let order = |isolates: &[BidiIsolate]| {
                bidi_spans(line.text, direction, isolates)
                    .unwrap()
                    .iter()
                    .map(|span| [span.from, span.to, span.level])
                    .collect::<Vec<_>>()
            };
            assert_eq!(json!(order(&isolates)), expected["spans"], "{id}:{number}");
            assert_eq!(json!(order(&[])), expected["plain"], "{id}:{number} plain");
        }
    }
}

#[test]
fn refreshed_marks_use_the_latest_parse_and_viewport() {
    let language = SqlDialect::StandardSql.language();
    let document = language.parse(Text::new("SELECT 'אב';"));
    let changes = ChangeSet::new(vec![Change::new(0..0, " /* גד */ ")]).unwrap();
    let edited = language.reparse(&document, &changes).unwrap();
    let marks = edited
        .bidi_isolate_marks(
            &Viewport::complete(edited.text()),
            SqlBidiIsolateOptions::default(),
        )
        .unwrap();
    assert_eq!(
        marks
            .iter(0)
            .map(|r| edited.text().slice(r.from..r.to).unwrap())
            .collect::<Vec<_>>(),
        ["/* גד */", "'אב'"]
    );

    let short_document = SqlDocument::parse(Text::new("'א'"));
    let foreign_viewport = Viewport::around(&Text::new("xxxx"), std::iter::once(2..4)).unwrap();
    assert_eq!(
        short_document
            .bidi_isolate_marks(&foreign_viewport, SqlBidiIsolateOptions::default())
            .unwrap_err(),
        EditError::InvalidRange,
    );
}
