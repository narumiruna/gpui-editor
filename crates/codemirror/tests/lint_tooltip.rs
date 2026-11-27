use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorState, Extension,
    Transaction,
    lint::{
        Diagnostic, DiagnosticFilter, LintConfig, LintGutterConfig, Severity, diagnostic_count,
        lint_gutter, lint_gutter_markers, lint_gutter_tooltip_at, lint_hover_tooltip,
        lint_tooltip_at, linter, set_diagnostics,
    },
    view::tooltips::{HoverSession, HoverSide, HoverTarget, Tooltip, active_tooltips},
};
use serde_json::{Value, json};

#[test]
fn tooltip_filter_order_and_segment_edges_match_pinned_browser() {
    let oracle = include_str!("fixtures/lint_tooltip.jsonl");
    let example: Value = serde_json::from_str(oracle.lines().next().unwrap()).unwrap();
    let source = example["source"].as_str().unwrap();
    let slot = Compartment::new();
    let mut state = EditorState::with_extensions(
        source,
        Extension::group([
            linter(
                None,
                LintConfig {
                    marker_filter: Some(Arc::new(|mut diagnostics, _| {
                        diagnostics.retain(|diagnostic| diagnostic.message != "invisible");
                        diagnostics
                    })),
                    ..Default::default()
                },
            ),
            slot.of(Extension::default()),
        ]),
    )
    .unwrap();
    state
        .update(set_diagnostics(vec![
            Diagnostic::new(3..6, Severity::Error, "outer"),
            Diagnostic::new(4..5, Severity::Warning, "inner"),
            Diagnostic::new(5..5, Severity::Info, "point"),
            Diagnostic::new(8..10, Severity::Hint, "tail"),
            Diagnostic::new(0..2, Severity::Error, "invisible"),
        ]))
        .unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let filter = |name: &'static str| -> DiagnosticFilter {
        let calls = calls.clone();
        Arc::new(move |mut diagnostics, state| {
            assert_eq!(state.text().as_str(), "é abc\r\nxy");
            let messages: Vec<_> = diagnostics
                .iter()
                .map(|diagnostic| &diagnostic.message)
                .collect();
            let logical_units = state.text().as_str().encode_utf16().count() - 1;
            calls
                .lock()
                .unwrap()
                .push(json!([name, messages, logical_units]));
            match name {
                "add" => diagnostics.iter_mut().for_each(|d| d.message.push('!')),
                "reverse" => diagnostics.reverse(),
                "clear" => diagnostics.clear(),
                _ => unreachable!(),
            }
            diagnostics
        })
    };
    let (add, reverse, clear) = (filter("add"), filter("reverse"), filter("clear"));
    for row in oracle.lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let inputs = case["configuration"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| {
                let callback = match name.as_str().unwrap() {
                    "add" => add.clone(),
                    "reverse" => reverse.clone(),
                    "clear" => clear.clone(),
                    name => panic!("unexpected filter {name}"),
                };
                linter(
                    None,
                    LintConfig {
                        tooltip_filter: Some(callback),
                        ..Default::default()
                    },
                )
            });
        state
            .update(
                Transaction::default().with_effects([slot.reconfigure(Extension::group(inputs))]),
            )
            .unwrap();
        for request in case["outcomes"].as_array().unwrap() {
            let position = request["at"].as_u64().unwrap().try_into().unwrap();
            let side = if request["side"] == -1 {
                Assoc::Before
            } else {
                Assoc::After
            };
            let result = lint_tooltip_at(&state, position, side).unwrap();
            assert_eq!(result.is_some(), request["active"] == true, "{request}");
            let messages = result
                .into_iter()
                .flat_map(|result| {
                    assert!((result.range.start..=result.range.end).contains(&position));
                    result
                        .diagnostics
                        .into_iter()
                        .map(|diagnostic| diagnostic.message)
                })
                .collect::<Vec<_>>();
            assert_eq!(json!(messages), request["messages"], "{request}");
            assert_eq!(
                json!(std::mem::take(&mut *calls.lock().unwrap())),
                request["calls"],
                "{request}"
            );
            assert_eq!(json!(diagnostic_count(&state)), case["markerCount"]);
        }
    }
    assert_eq!(
        lint_tooltip_at(&state, 1, Assoc::After).unwrap_err(),
        EditError::InvalidRange
    );
}

#[test]
fn lint_hover_dismissal_and_mapping_follow_the_pinned_transactions() {
    for line in include_str!("fixtures/lint_hover_hiding.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let source = case["source"].as_str().unwrap();
        let hover = lint_hover_tooltip();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let configs = case["config"].as_array().unwrap().iter().map(|input| {
            let name = input.as_str().unwrap().to_owned();
            let calls = calls.clone();
            linter(
                None,
                LintConfig {
                    hide_on: Some(Arc::new(move |update, range| {
                        calls.lock().unwrap().push(json!([
                            name,
                            range.start,
                            range.end,
                            update.start().text().as_str(),
                            !update.changes().is_empty()
                        ]));
                        match name.as_str() {
                            "defer" => None,
                            "keep" => Some(false),
                            "hide" => Some(true),
                            _ => unreachable!(),
                        }
                    })),
                    ..Default::default()
                },
            )
        });
        let mut state = EditorState::with_extensions(
            source,
            Extension::group([linter(None, Default::default()), Extension::group(configs)]),
        )
        .unwrap();
        let publish = || set_diagnostics(vec![Diagnostic::new(3..6, Severity::Error, "text")]);
        state.update(publish()).unwrap();
        let mut session = HoverSession::default();
        let hit = HoverTarget {
            pos: 4,
            side: HoverSide::After,
        };
        let request = session.activate_hover(&state, hit, Some(hover), None)[0];
        let candidate = lint_tooltip_at(&state, hit.pos, Assoc::After)
            .unwrap()
            .unwrap();
        let mut tip = Tooltip::new(candidate.range.start);
        tip.end = Some(candidate.range.end);
        tip.above = true;
        let activate = session.complete(&state, request, vec![tip]).unwrap();
        state
            .update(Transaction::default().with_effects([activate]))
            .unwrap();
        let change =
            |range, text| Transaction::new(ChangeSet::single(Change::new(range, text)).unwrap());
        let update = match case["operation"].as_str().unwrap() {
            "prefix" => change(0..0, "X"),
            "touch" => change(3..3, "X"),
            "publish" => publish(),
            "delete" => change(2..4, ""),
            _ => unreachable!(),
        };
        state.update(update).unwrap();
        let active = active_tooltips(&state)
            .into_iter()
            .map(|active| [Some(active.tooltip.pos), active.tooltip.end])
            .collect::<Vec<_>>();
        assert_eq!(json!(active), case["tooltip"], "{line}");
        assert_eq!(json!(*calls.lock().unwrap()), case["calls"], "{line}");
    }
}

#[test]
fn gutter_tooltip_filter_and_delay_do_not_change_the_marker() {
    for line in include_str!("fixtures/lint_gutter_tooltip.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let source = case["source"].as_str().unwrap();
        let name = case["name"].as_str().unwrap();
        let tooltip_filter: Option<DiagnosticFilter> = match name {
            "filtered" => Some(Arc::new(|mut diagnostics, _| {
                diagnostics.retain(|d| d.message != "first");
                diagnostics
            })),
            "empty" => Some(Arc::new(|_, _| Vec::new())),
            _ => None,
        };
        let gutter = lint_gutter(LintGutterConfig {
            marker_filter: Some(Arc::new(|mut diagnostics, _| {
                diagnostics.retain(|d| d.message != "skip");
                diagnostics
            })),
            tooltip_filter,
            hover_time: (name == "filtered").then_some(Duration::from_millis(45)),
        });
        if case["conflict"] == true {
            let inputs = Extension::group(std::iter::once(gutter).chain([100, 200].map(|ms| {
                lint_gutter(LintGutterConfig {
                    hover_time: Some(Duration::from_millis(ms)),
                    ..Default::default()
                })
            })));
            assert_eq!(
                EditorState::with_extensions(source, inputs).unwrap_err(),
                ConfigurationError::InvalidFieldData
            );
            continue;
        }
        let mut state = EditorState::with_extensions(source, gutter).unwrap();
        state
            .update(set_diagnostics(vec![
                Diagnostic::new(3..6, Severity::Error, "first"),
                Diagnostic::new(4..5, Severity::Warning, "second"),
                Diagnostic::new(8..10, Severity::Hint, "tail"),
                Diagnostic::new(8..8, Severity::Error, "skip"),
            ]))
            .unwrap();
        let marks = lint_gutter_markers(&state).unwrap();
        let actual = marks.iter(0).map(|marker| {
            let severity = match marker.value.severity() {
                Severity::Error => "error", Severity::Warning => "warning", Severity::Info => "info", Severity::Hint => "hint",
            };
            let tooltip = lint_gutter_tooltip_at(&state, marker.from).unwrap();
            let delay = tooltip.as_ref().map(|t| t.delay.as_millis());
            let messages: Vec<_> = tooltip.into_iter().flat_map(|t| t.diagnostics.into_iter().map(|d| d.message)).collect();
            json!({"line": marker.from, "severity": severity, "delay": delay, "messages": messages})
        }).collect::<Vec<_>>();
        assert_eq!(json!(actual), case["marks"], "{line}");
        assert!(lint_gutter_tooltip_at(&state, 2).unwrap().is_none());
        assert_eq!(
            lint_gutter_tooltip_at(&state, 1).unwrap_err(),
            EditError::InvalidRange
        );
        assert_eq!(state.text().as_str(), source);
    }
}
