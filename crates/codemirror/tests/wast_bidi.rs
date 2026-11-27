//! Native isolation marks and line ordering against the pinned language and view.

use std::sync::Arc;

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditError, EditorState, Extension, Precedence, RangeSet,
    RangeValue, Text, Transaction,
    language::{
        BidiIsolateOptions, bidi_isolates, configured_bidi_isolates,
        wast::{ISOLATE, WastDocument, WastIsolation},
    },
    language_data::{NativeSyntax, attached_language_snapshot, wast_language},
    view::{
        MarkDecoration, Viewport,
        bidi::{BIDI_ISOLATED_RANGES, Direction, bidi_spans, line_isolates},
    },
};
use serde_json::{Value, json};

fn byte(value: &Value) -> usize {
    value.as_u64().unwrap() as usize
}

#[test]
fn wast_isolation_and_visual_order_match_pinned_view() {
    for row in include_str!("fixtures/wast_bidi.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let id = case["id"].as_str().unwrap();
        let text = Text::new(case["source"].as_str().unwrap());
        let parsed = WastDocument::parse(text.clone());
        let marked_nodes = parsed
            .nodes()
            .filter(|node| node.kind().isolates_direction());
        let syntax: Vec<_> = marked_nodes
            .map(|node| json!([node.kind().name(), node.range().start, node.range().end]))
            .collect();
        assert_eq!(json!(syntax), case["syntax"], "{id}: syntax");
        let visible = case["visible"]
            .as_array()
            .unwrap()
            .iter()
            .map(|range| byte(&range[0])..byte(&range[1]));
        let viewport = Viewport::around(&text, visible).unwrap();
        let direction = if case["direction"] == 0 {
            Direction::Ltr
        } else {
            Direction::Rtl
        };
        let marks = parsed
            .bidi_isolate_marks(
                &viewport,
                BidiIsolateOptions {
                    always_isolate: case["always"].as_bool().unwrap(),
                    text_direction: direction,
                    per_line_text_direction: case["perLine"].as_bool().unwrap(),
                },
            )
            .unwrap();
        let ranges: Vec<_> = marks
            .iter(0)
            .map(|mark| {
                assert_eq!(mark.value.class(), "cm-iso");
                assert_eq!(mark.value.bidi_direction(), None);
                assert!(mark.value.start_side() < 0 && mark.value.end_side() > 0);
                [mark.from, mark.to]
            })
            .collect();
        assert_eq!(json!(ranges), case["ranges"], "{id}: visible marks");
        assert_eq!(parsed.text(), &text, "{id}: original bytes");
        assert_spans(&parsed, direction, marks, &case["lines"], id);
    }
}

fn assert_spans(
    parsed: &WastDocument,
    direction: Direction,
    marks: Arc<RangeSet<MarkDecoration>>,
    lines: &Value,
    label: &str,
) {
    let state = EditorState::create_from_text(
        parsed.syntax_lines().clone(),
        None,
        BIDI_ISOLATED_RANGES
            .of(marks)
            .with_precedence(Precedence::Lowest),
    )
    .unwrap();
    for (index, expected) in lines.as_array().unwrap().iter().enumerate() {
        let number = index + 1;
        let line = state.text().line(number).unwrap();
        let isolates = line_isolates(&state, number).unwrap();
        let spans: Vec<_> = bidi_spans(line.text, direction, &isolates)
            .unwrap()
            .into_iter()
            .map(|span| [span.from, span.to, span.level])
            .collect();
        assert_eq!(json!(spans), *expected, "{label}:{number}: visual spans");
    }
}

#[test]
fn configured_node_isolation_matches_pinned_marks_and_spans() {
    let decode = |value: &Value| match value.as_str() {
        Some("auto") => Some(WastIsolation::Auto),
        Some("ltr") => Some(WastIsolation::Ltr),
        Some("rtl") => Some(WastIsolation::Rtl),
        None if value.is_null() => None,
        _ => unreachable!("unknown fixture direction {value}"),
    };
    let name_of = |direction| match direction {
        WastIsolation::Auto => "auto",
        WastIsolation::Ltr => "ltr",
        WastIsolation::Rtl => "rtl",
    };
    for line in include_str!("fixtures/wast_isolate_props.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let label = row["name"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let overrides = row["overrides"].as_object().unwrap().clone();
        let configured = wast_language()
            .with_node_props(&[ISOLATE.add(move |node| overrides.get(node.name()).map(decode))]);
        let state = EditorState::with_extensions(source, configured.extension()).unwrap();
        let snapshot = attached_language_snapshot(&state).unwrap();
        let NativeSyntax::Wast(doc) = snapshot.syntax() else {
            unreachable!()
        };
        let syntax: Vec<_> = doc
            .nodes()
            .filter_map(|node| {
                let dir = node.prop(&ISOLATE).copied().flatten()?;
                let range = node.range();
                Some(json!([
                    node.kind().name(),
                    range.start,
                    range.end,
                    name_of(dir)
                ]))
            })
            .collect();
        assert_eq!(json!(syntax), row["syntax"], "{label}: properties");
        let visible = row["visible"]
            .as_array()
            .unwrap()
            .iter()
            .map(|range| byte(&range[0])..byte(&range[1]));
        let viewport = Viewport::around(doc.text(), visible).unwrap();
        let marks = doc
            .bidi_isolate_marks(
                &viewport,
                BidiIsolateOptions {
                    always_isolate: row["always"].as_bool().unwrap(),
                    ..Default::default()
                },
            )
            .unwrap();
        let ranges: Vec<_> = marks
            .iter(0)
            .map(|mark| {
                assert_eq!(mark.value.class(), "cm-iso");
                assert!(mark.value.start_side() < 0 && mark.value.end_side() > 0);
                let direction = match mark.value.bidi_direction() {
                    Some(Direction::Ltr) => WastIsolation::Ltr,
                    Some(Direction::Rtl) => WastIsolation::Rtl,
                    None => WastIsolation::Auto,
                };
                json!([mark.from, mark.to, name_of(direction)])
            })
            .collect();
        assert_eq!(json!(ranges), row["marks"], "{label}: directed marks");
        assert_spans(doc, Direction::Ltr, marks, &row["lines"], label);
    }
}

#[test]
fn direction_only_reconfiguration_changes_marks_and_preserves_old_output() {
    let source = "(data \"ABC אב\")\r\n";
    let slot = Compartment::new();
    let stock = wast_language();
    let configured = stock.with_node_props(&[
        ISOLATE.add(|node| (node.name() == "String").then_some(Some(WastIsolation::Rtl)))
    ]);
    let mut state = EditorState::with_extensions(source, slot.of(stock.extension())).unwrap();
    let snapshot = attached_language_snapshot(&state).unwrap();
    let marks = |syntax: &NativeSyntax| {
        let NativeSyntax::Wast(doc) = syntax else {
            unreachable!()
        };
        doc.bidi_isolate_marks(
            &Viewport::complete(doc.text()),
            BidiIsolateOptions {
                always_isolate: true,
                ..Default::default()
            },
        )
        .unwrap()
    };
    let old = marks(snapshot.syntax());
    state
        .update(Transaction::default().with_effects([slot.reconfigure(configured.extension())]))
        .unwrap();
    let new = marks(attached_language_snapshot(&state).unwrap().syntax());
    assert_eq!(old.len(), 1);
    assert_eq!(new.len(), 1);
    let old_mark = old.iter(0).next().unwrap();
    let new_mark = new.iter(0).next().unwrap();
    assert_eq!((old_mark.from, old_mark.to), (new_mark.from, new_mark.to));
    assert_eq!(old_mark.value.bidi_direction(), None);
    assert_eq!(new_mark.value.bidi_direction(), Some(Direction::Rtl));
    assert!(!RangeValue::eq(&old_mark.value, &new_mark.value));
    let retained = marks(snapshot.syntax());
    assert!(RangeValue::eq(
        &old_mark.value,
        &retained.iter(0).next().unwrap().value
    ));
    assert_eq!(state.text().as_str(), source);
    assert_eq!(state.selection().main().head(), 0);
}

#[test]
fn isolation_requests_are_opt_in_and_reconfigure_without_edits() {
    let slot = Compartment::new();
    let mut state =
        EditorState::with_extensions("(data \"אב\")", slot.of(Extension::default())).unwrap();
    assert_eq!(configured_bidi_isolates(&state), None);
    for (extension, expected) in [
        (bidi_isolates(false), Some(false)),
        (
            Extension::group([bidi_isolates(false), bidi_isolates(true)]),
            Some(true),
        ),
        (Extension::default(), None),
    ] {
        state
            .update(Transaction::default().with_effects([slot.reconfigure(extension)]))
            .unwrap();
        assert_eq!(configured_bidi_isolates(&state), expected);
        assert_eq!(state.text().as_str(), "(data \"אב\")");
        assert_eq!(state.selection().main().head(), 0);
    }
}

#[test]
fn edited_wast_snapshots_refresh_isolates_and_check_byte_boundaries() {
    let parsed = WastDocument::parse(Text::new("(data \"אב\")\r;; مرحبا"));
    let changes = ChangeSet::single(Change::new(0..0, ";; 🦀\r\n")).unwrap();
    let updated = parsed.reparse(&changes).unwrap();
    let marks = updated
        .bidi_isolate_marks(
            &Viewport::complete(updated.text()),
            BidiIsolateOptions {
                always_isolate: true,
                ..Default::default()
            },
        )
        .unwrap();
    let marked: Vec<_> = marks
        .iter(0)
        .map(|mark| updated.text().slice(mark.from..mark.to).unwrap())
        .collect();
    assert_eq!(marked, [";; 🦀", "\"אב\"", ";; مرحبا"]);
    let unrelated = Text::new("     ");
    let inside_crab = Viewport::around(&unrelated, std::iter::once(4..5)).unwrap();
    assert_eq!(
        updated
            .bidi_isolate_marks(&inside_crab, BidiIsolateOptions::default())
            .unwrap_err(),
        EditError::InvalidRange,
    );
}
