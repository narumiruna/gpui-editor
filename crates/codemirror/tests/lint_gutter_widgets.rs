use std::sync::Arc;

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditError, EditorState, Extension, Range, Transaction,
    lint::{
        Diagnostic, LintConfig, LintGutterConfig, Severity, diagnostic_count, lint_gutter,
        lint_gutter_markers, lint_gutter_tooltip, lint_gutter_widget_marker, linter,
        set_diagnostics,
    },
    setup::basic_setup,
    view::{
        decorations::{Decoration, ReplaceSpec},
        gutters::{GutterWidget, gutter_slots, gutter_widget_rows},
        keymap::KeyPlatform,
    },
};
use serde_json::{Value, json};

fn byte(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

#[test]
fn gutter_grouping_uses_default_logical_lines_and_removes_its_slot() {
    let slot = Compartment::new();
    let source = "猫\rsecond";
    let state =
        EditorState::with_extensions(source, slot.of(lint_gutter(Default::default()))).unwrap();
    let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
    editor
        .view_mut()
        .update(
            set_diagnostics(vec![Diagnostic::new(
                4..4,
                Severity::Warning,
                "second line",
            )]),
            (),
            |_| (None, ()),
        )
        .unwrap();
    let markers = lint_gutter_markers(editor.view().state()).unwrap();
    let at = markers.iter(0).next().unwrap();
    assert_eq!(at.from, 4);
    let slots = editor.gutters(1..=2, None).unwrap();
    let slot_rows = &slots
        .iter()
        .find(|marks| marks.class == "cm-gutter-lint")
        .unwrap()
        .rows;
    let warnings: Vec<_> = slot_rows
        .iter()
        .filter(|row| {
            row.markers
                .iter()
                .any(|marker| marker.element_class == "cm-lint-marker cm-lint-marker-warning")
        })
        .map(|row| row.line_number)
        .collect();
    assert_eq!(warnings, [2]);
    assert_eq!(editor.view().state().text().as_str(), source);
    editor
        .view_mut()
        .update(
            Transaction::default().with_effects([slot.reconfigure(Extension::default())]),
            (),
            |_| (None, ()),
        )
        .unwrap();
    assert!(
        editor
            .gutters(1..=2, None)
            .unwrap()
            .iter()
            .all(|slot| slot.class != "cm-gutter-lint")
    );
    assert!(
        lint_gutter_tooltip(editor.view().state(), &at.value)
            .unwrap()
            .is_none()
    );
}

#[test]
fn gutter_publication_and_edit_mapping_match_the_pinned_state() {
    let mut rows = include_str!("fixtures/lint_gutter_mapping.jsonl").lines();
    let start: Value = serde_json::from_str(rows.next().unwrap()).unwrap();
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(
        start["source"].as_str().unwrap(),
        Extension::group([
            linter(
                None,
                LintConfig {
                    marker_filter: Some(Arc::new(|_, _| Vec::new())),
                    ..Default::default()
                },
            ),
            slot.of(Extension::default()),
        ]),
    )
    .unwrap();
    for line in rows {
        let case: Value = serde_json::from_str(line).unwrap();
        let value = &case["value"];
        let tx = match case["op"].as_str().unwrap() {
            "configure" => {
                let extension = match value.as_str().unwrap() {
                    "off" => Extension::default(),
                    "all" => lint_gutter(Default::default()),
                    "without errors" => lint_gutter(LintGutterConfig {
                        marker_filter: Some(Arc::new(|mut diagnostics, _| {
                            diagnostics.retain(|d| d.severity != Severity::Error);
                            diagnostics
                        })),
                        ..Default::default()
                    }),
                    other => panic!("unknown configuration {other}"),
                };
                Transaction::default().with_effects([slot.reconfigure(extension)])
            }
            "publish" => set_diagnostics(
                value
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|d| {
                        let severity = match d[2].as_str().unwrap() {
                            "hint" => Severity::Hint,
                            "info" => Severity::Info,
                            "warning" => Severity::Warning,
                            "error" => Severity::Error,
                            other => panic!("unknown severity {other}"),
                        };
                        Diagnostic::new(byte(&d[0])..byte(&d[1]), severity, d[3].as_str().unwrap())
                    })
                    .collect(),
            ),
            "edit" => Transaction::new(
                ChangeSet::single(Change::new(
                    byte(&value[0])..byte(&value[1]),
                    value[2].as_str().unwrap(),
                ))
                .unwrap(),
            ),
            other => panic!("unknown operation {other}"),
        };
        state.update(tx).unwrap();
        let markers = lint_gutter_markers(&state).map(|markers| {
            markers
                .iter(0)
                .map(|m| {
                    let messages: Vec<_> =
                        m.value.diagnostics().iter().map(|d| &d.message).collect();
                    json!([
                        m.from,
                        format!("{:?}", m.value.severity()).to_lowercase(),
                        messages
                    ])
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(json!(markers), case["markers"], "{line}");
        assert_eq!(diagnostic_count(&state), 0);
        assert_eq!(state.text().as_str(), case["raw"].as_str().unwrap());
    }
}

#[test]
fn installed_gutter_and_widget_groups_match_the_pinned_view() {
    for line in include_str!("fixtures/lint_gutter_widgets.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let source = case["source"].as_str().unwrap();
        let config = lint_gutter(LintGutterConfig {
            tooltip_filter: Some(Arc::new(|mut diagnostics, state| {
                assert_eq!(state.text().as_str(), "é\r\nfirst\r\nmiddle\nlast\nend");
                diagnostics.retain(|d| d.message != "inside-error");
                diagnostics
            })),
            ..Default::default()
        });
        let mut state = EditorState::with_extensions(
            source,
            Extension::group([config, lint_gutter(Default::default())]),
        )
        .unwrap();
        let diagnostics = case["lineStarts"]
            .as_array()
            .unwrap()
            .iter()
            .map(byte)
            .zip([
                (Severity::Info, "outside"),
                (Severity::Error, "start"),
                (Severity::Warning, "inside-warning"),
                (Severity::Error, "inside-error"),
                (Severity::Hint, "end"),
            ])
            .map(|(at, (severity, message))| Diagnostic::new(at..at, severity, message))
            .collect();
        state.update(set_diagnostics(diagnostics)).unwrap();
        let slots = gutter_slots(&state, 1..=5).unwrap();
        let [slot] = slots.as_slice() else {
            panic!("one stock lint gutter")
        };
        assert_eq!(slot.class, "cm-gutter-lint");
        let markers = lint_gutter_markers(&state).unwrap();
        let widget_range = byte(&case["widget"][0])..byte(&case["widget"][1]);
        let replacement = Decoration::replace(
            Some("widget"),
            ReplaceSpec {
                block: true,
                ..Default::default()
            },
        );
        let mut labels = Vec::new();
        let mut groups = Vec::new();
        if case["replace"] == true {
            let block = replacement.try_range(widget_range.clone()).unwrap();
            let widget = GutterWidget::from_decoration(&block).unwrap();
            let combined = lint_gutter_widget_marker(&state, &widget).unwrap().unwrap();
            let rows = gutter_widget_rows(&state, &[widget]).unwrap();
            let [row] = rows.as_slice() else {
                panic!("one aggregate widget row")
            };
            assert_eq!(row.gutter_index, slot.index);
            assert_eq!(row.side, slot.side);
            labels.push(slot.rows[0].markers[0].element_class.as_str());
            labels.push(row.markers[0].element_class.as_str());
            labels.push(slot.rows[4].markers[0].element_class.as_str());
            assert_eq!(json!(labels), case["markers"]);
            let filtered = lint_gutter_tooltip(&state, &combined).unwrap().unwrap();
            assert_eq!(
                filtered
                    .diagnostics
                    .iter()
                    .map(|d| d.message.as_str())
                    .collect::<Vec<_>>(),
                ["inside-warning"]
            );
            assert_eq!(combined.severity(), Severity::Error);
            groups.extend([
                markers.iter(0).next().unwrap().value.diagnostics(),
                combined.diagnostics(),
                markers
                    .iter(widget_range.end)
                    .next()
                    .unwrap()
                    .value
                    .diagnostics(),
            ]);
            assert_eq!(
                json!(
                    groups
                        .iter()
                        .map(|group| group.iter().map(|d| d.message.as_str()).collect::<Vec<_>>())
                        .collect::<Vec<_>>()
                ),
                json!(&case["groups"].as_array().unwrap()[..3])
            );
            let invalid = Range::new(1, widget_range.end, replacement.clone());
            let invalid = GutterWidget::from_decoration(&invalid).unwrap();
            assert_eq!(
                lint_gutter_widget_marker(&state, &invalid).unwrap_err(),
                EditError::InvalidRange
            );
            let empty = replacement.try_range(4..4).unwrap();
            assert!(
                lint_gutter_widget_marker(&state, &GutterWidget::from_decoration(&empty).unwrap())
                    .unwrap()
                    .is_none()
            );
        } else {
            labels.extend(
                slot.rows
                    .iter()
                    .map(|row| row.markers[0].element_class.as_str()),
            );
            assert_eq!(json!(labels), case["markers"]);
            let groups = markers
                .iter(0)
                .map(|mark| {
                    mark.value
                        .diagnostics()
                        .iter()
                        .map(|d| d.message.as_str())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert_eq!(json!(groups), case["groups"]);
        }
        assert_eq!(state.text().as_str(), source);
    }
}
