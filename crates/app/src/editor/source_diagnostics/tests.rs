use std::{sync::Mutex, time::Duration};

use gpui::{EntityInputHandler, Modifiers, TestAppContext};
use gpui_editor_codemirror::{
    Change, ChangePermission, ChangeSet, Compartment, Extension, Precedence, StateEffect,
    change_filter,
    language::{
        StreamSettings,
        javascript::{EslintFinding, EslintFix, es_lint},
    },
    language_data::{LANGUAGES, NativeLanguage},
    lint::{Action, DEFAULT_LINT_DELAY, LintConfig, LintSource, linter},
};

use super::*;
use crate::{
    document::{BlockPreview, Section, SectionKind},
    editor::bind_keys,
};

const BAD: &str = "```JSON title\r\n{\"name\": \"字\", }\r\n```\r\n\r\nOutside.";

fn enter_json(editor: &mut Editor, window: &mut Window, cx: &mut Context<Editor>) {
    enter_fence(editor, 0, window, cx);
}

fn enter_fence(editor: &mut Editor, index: usize, window: &mut Window, cx: &mut Context<Editor>) {
    let block = editor
        .markdown()
        .sections()
        .into_iter()
        .filter(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
        .nth(index)
        .unwrap();
    let range = block.source;
    editor.activate(
        Section::new(SectionKind::DraftCode, range.clone(), range),
        false,
        cx,
    );
    window.focus(&editor.focus);
}

#[gpui::test]
fn json_diagnostics_debounce_point_at_source_and_do_not_change_undo(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(BAD.into(), window, cx));
    let executor = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            assert!(!editor.open_source_diagnostics(window, cx));
            enter_json(editor, window, cx);
        });
        cx.background_executor().clone()
    });
    cx.run_until_parked();
    executor.advance_clock(DEFAULT_LINT_DELAY - Duration::from_millis(1));
    cx.run_until_parked();
    cx.update(|_, cx| assert!(editor.read(cx).source_diagnostic_marks().is_empty()));
    executor.advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    let from = BAD.find('}').unwrap();
    let pointer = cx.update(|_, cx| {
        let editor = editor.read(cx);
        let marks = editor.source_diagnostic_marks();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].range, from..from);
        assert!(marks[0].diagnostic.message.contains("property name"));
        assert_eq!(marks[0].diagnostic.source.as_deref(), Some("JSON"));
        assert_eq!(editor.source(), BAD);
        assert!(!editor.view.can_undo());
        marker_anchor(editor.layout.as_ref().unwrap(), from)
            .unwrap()
            .center()
    });
    cx.simulate_mouse_move(pointer, None, Modifiers::none());
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        let overlay = editor.source_diagnostic_tooltip(
            editor.layout.as_ref().unwrap(),
            editor.scroll.bounds(),
            window,
        );
        assert!(overlay.unwrap().text.contains("property name"));
    });
    cx.simulate_keystrokes("f8");
    cx.update(|_, cx| assert_eq!(editor.read(cx).cursor(), from));
    cx.simulate_keystrokes("cmd-shift-m");
    cx.run_until_parked();
    assert!(cx.debug_bounds("source-diagnostics-panel").is_some());
    let row = cx.debug_bounds("source-diagnostic-0").unwrap().center();
    cx.simulate_click(row, Modifiers::none());
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(!editor.diagnostic_panel_open());
        assert!(editor.focus.is_focused(window));
        assert_eq!(editor.cursor(), from);
        assert!(!editor.view.can_undo());
    });

    let comma = BAD.find(',').unwrap();
    cx.update(|_, cx| editor.update(cx, |editor, _| editor.set_selection(comma, comma + 1)));
    cx.simulate_keystrokes("backspace");
    executor.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(editor.source_diagnostic_marks().is_empty());
        assert_eq!(editor.source(), BAD.replacen(',', "", 1));
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), BAD));
    executor.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source_diagnostic_marks().len(), 1);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn keyboard_navigation_activates_lint_hover_and_tracks_accepted_updates(cx: &mut TestAppContext) {
    #[derive(serde::Deserialize)]
    struct Fixture {
        source: String,
        steps: Vec<Step>,
    }
    #[derive(serde::Deserialize)]
    struct Step {
        name: String,
        selection: [usize; 2],
        tooltip: Option<Vec<String>>,
    }
    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../../tests/fixtures/codemirror/lint_navigation_hover.jsonl"
    ))
    .unwrap();
    let first = fixture.source.find("abc").unwrap();
    let second = fixture.source.find("other").unwrap();
    let input = vec![
        Diagnostic::new(first..first + 3, Severity::Warning, "range"),
        Diagnostic::new(first..first + 2, Severity::Error, "overlap"),
        Diagnostic::new(second..second, Severity::Error, "point"),
    ];
    let source = fixture.source.clone();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source, window, cx));
    cx.update(|window, cx| {
        window.activate_window();
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            configure(
                editor,
                linter(
                    Some(LintSource::new(|_| Vec::new())),
                    LintConfig {
                        tooltip_filter: Some(Arc::new(|mut items, _| {
                            items.reverse();
                            items
                        })),
                        ..Default::default()
                    },
                )
                .append(),
                cx,
            );
            enter_fence(editor, 0, window, cx);
            editor.sync_source_diagnostics(cx);
            assert!(editor.apply_diagnostic_update(lint::set_diagnostics(input.clone())));
            editor.set_selection(0, 0);
            cx.notify();
        });
    });
    cx.run_until_parked();
    for step in fixture.steps {
        match step.name.as_str() {
            "next" | "next-from-overlap" => cx.simulate_keystrokes("f8"),
            "wrap-back-to-point" => cx.simulate_keystrokes("shift-f8"),
            "inside" | "outside" | "return" => cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    let [anchor, head] = step.selection;
                    editor.set_selection(anchor, head);
                    cx.notify();
                });
            }),
            "publication" => cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    assert!(editor.apply_diagnostic_update(lint::set_diagnostics(input.clone())));
                    cx.notify();
                });
            }),
            "again" => {
                cx.update(|_, cx| {
                    editor.update(cx, |editor, cx| {
                        editor.set_selection(0, 0);
                        cx.notify();
                    });
                });
                cx.simulate_keystrokes("f8");
            }
            "edit" => cx.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    let units = fixture.source[..first + 3].encode_utf16().count();
                    editor.replace_text_in_range(Some(units..units), " ", window, cx);
                });
            }),
            _ => panic!("unknown step: {}", step.name),
        }
        cx.run_until_parked();
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            let expected = step.tooltip.as_ref().and_then(|items| items.first());
            if step.name == "edit" {
                assert_ne!(editor.source(), fixture.source);
                assert!(editor.view.can_undo());
            } else {
                assert_eq!(
                    [editor.anchor(), editor.cursor()],
                    step.selection,
                    "{}",
                    step.name
                );
                assert_eq!(editor.source(), fixture.source);
                assert!(!editor.view.can_undo());
            }
            assert_eq!(
                editor
                    .source_diagnostic_tooltip(
                        editor.layout.as_ref().unwrap(),
                        editor.scroll.bounds(),
                        window,
                    )
                    .as_ref()
                    .map(|overlay| &overlay.text),
                expected,
                "{}",
                step.name,
            );
        });
    }
}

#[gpui::test]
fn keyboard_lint_hover_survives_pointer_motion_but_not_escape_or_blur(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(BAD.into(), window, cx));
    let clock = cx.update(|window, cx| {
        window.activate_window();
        bind_keys(cx);
        editor.update(cx, |editor, cx| enter_json(editor, window, cx));
        cx.background_executor().clone()
    });
    cx.run_until_parked();
    clock.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    let check = |cx: &mut gpui::VisualTestContext, visible| {
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            let hover = editor.keyboard_diagnostic_hover(layout, editor.scroll.bounds(), window);
            let active = lint::lint_hover_tooltip()
                .active(editor.view.state())
                .unwrap();
            assert_eq!(active.is_locked(), visible);
            assert_eq!(hover.is_some(), visible);
            if let Some(hover) = hover {
                assert!(hover.prefer_above);
                assert_eq!(
                    hover.anchor,
                    layout.bounds_at(BAD.find('}').unwrap(), false)
                );
                assert!(hover.text.contains("property name"));
            }
            assert_eq!(editor.source(), BAD);
            assert!(!editor.view.can_undo());
        });
    };
    let jump = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let start = editor.source_fence().unwrap().body.start;
                editor.set_selection(start, start);
                cx.notify();
            });
        });
        cx.simulate_keystrokes("f8");
        cx.run_until_parked();
    };
    jump(cx);
    check(cx, true);
    cx.simulate_mouse_move(point(px(1.), px(1.)), None, Modifiers::none());
    check(cx, true);
    cx.simulate_keystrokes("escape");
    check(cx, false);
    jump(cx);
    check(cx, true);
    cx.deactivate_window();
    check(cx, false);
    cx.update(|window, cx| {
        window.activate_window();
        window.focus(&editor.read(cx).focus);
    });
    cx.run_until_parked();
    check(cx, false);
}

#[gpui::test]
fn hover_filters_change_only_the_scoped_tooltip(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(BAD.into(), window, cx));
    let slot = Compartment::new();
    let from = BAD.find('}').unwrap();
    let provider = linter(
        Some(LintSource::new(move |_| {
            vec![
                Diagnostic::new(from..from, Severity::Warning, "first"),
                Diagnostic::new(from..from, Severity::Error, "second"),
            ]
        })),
        LintConfig::default(),
    );
    let config = |hide| {
        linter(
            None,
            LintConfig {
                tooltip_filter: Some(Arc::new(move |mut items, state| {
                    assert_eq!(state.text().as_str(), BAD);
                    if hide {
                        return Vec::new();
                    }
                    items.reverse();
                    items
                })),
                ..Default::default()
            },
        )
    };
    let executor = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            configure(
                editor,
                Extension::group([provider, slot.of(config(false))]).append(),
                cx,
            );
            enter_json(editor, window, cx);
        });
        cx.background_executor().clone()
    });
    cx.run_until_parked();
    executor.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    let pointer = cx.update(|_, cx| {
        let editor = editor.read(cx);
        marker_anchor(editor.layout.as_ref().unwrap(), from)
            .unwrap()
            .center()
    });
    cx.simulate_mouse_move(pointer, None, Modifiers::none());
    let check = |editor: &Editor, window: &Window, expected| {
        let layout = editor.layout.as_ref().unwrap();
        let overlay = editor.source_diagnostic_tooltip(layout, editor.scroll.bounds(), window);
        assert_eq!(
            overlay.as_ref().map(|overlay| overlay.text.as_str()),
            expected
        );
        let marks = editor.source_diagnostic_marks();
        assert_eq!(
            marks
                .iter()
                .map(|mark| mark.diagnostic.message.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(editor.source(), BAD);
        assert!(!editor.view.can_undo());
    };
    cx.update(|window, cx| check(editor.read(cx), window, Some("second")));
    for (extension, expected) in [(config(true), None), (Extension::default(), Some("first"))] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, slot.reconfigure(extension), cx)
            })
        });
        cx.run_until_parked();
        executor.advance_clock(DEFAULT_LINT_DELAY);
        cx.run_until_parked();
        cx.update(|window, cx| check(editor.read(cx), window, expected));
    }
}

#[gpui::test]
fn host_lint_adapter_fixes_only_the_active_mounted_script(cx: &mut TestAppContext) {
    for line_break in ["\r\n", "\r"] {
        check_scoped_script_fix(cx, line_break);
    }
}

fn check_scoped_script_fix(cx: &mut TestAppContext, line_break: &str) {
    let source = format!(
        "Outside é\r\n\r\n```html\r\n<script>\r\nconst x = '😀';{line_break}bad;\r\n</script>\r\n```\r\n\r\n```html\r\n<script>bad;</script>\r\n```\r\n"
    );
    let start = source.find("bad").unwrap();
    let verifier = |input: &str, _: &serde_json::Value| {
        assert!(!input.contains('\r'));
        let Some(at) = input.find("bad") else {
            return Vec::new();
        };
        let prefix = &input[..at];
        let line = prefix.bytes().filter(|&byte| byte == b'\n').count() + 1;
        let column = prefix.rsplit('\n').next().unwrap().encode_utf16().count() + 1;
        let units = prefix.encode_utf16().count();
        vec![EslintFinding {
            line,
            column,
            end_line: Some(line),
            end_column: Some(column + 3),
            severity: 2,
            message: "test finding".into(),
            rule_id: Some("fixture".into()),
            fix: Some(EslintFix {
                range: units..units + 3,
                text: "good".into(),
            }),
        }]
    };
    let adapter = es_lint(
        |state| {
            NativeLanguage::Markdown
                .support()
                .parse_with_code_languages(
                    state.text().clone(),
                    StreamSettings::default(),
                    LANGUAGES,
                )
                .unwrap()
                .javascript_regions()
        },
        verifier,
        None,
        Vec::new,
    );
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
    let executor = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            configure(
                editor,
                linter(Some(adapter), LintConfig::default()).append(),
                cx,
            );
            enter_fence(editor, 0, window, cx);
        });
        cx.background_executor().clone()
    });
    cx.run_until_parked();
    executor.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let marks = editor.source_diagnostic_marks();
            let [mark] = marks.as_slice() else {
                panic!("one scoped diagnostic")
            };
            assert_eq!(mark.range, start..start + 3);
            assert_eq!(mark.diagnostic.source.as_deref(), Some("eslint:fixture"));
            assert!(!editor.view.can_undo());
            editor.run_source_diagnostic_action(&mark.diagnostic, 0, cx);
            assert_eq!(editor.source(), source.replacen("bad", "good", 1));
            assert!(editor.view.can_undo());
            editor.run_source_diagnostic_action(&mark.diagnostic, 0, cx);
            assert_eq!(editor.source(), source.replacen("bad", "good", 1));
        })
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn stock_json_lint_counts_logical_line_breaks_but_marks_original_bytes(cx: &mut TestAppContext) {
    let source = "Before 😀\r\n\r\n```json\r\n[\r\n  \"😀\"\r\n  false\r\n]\r\n```\r\n\r\nAfter";
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    let executor = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| enter_json(editor, window, cx));
        cx.background_executor().clone()
    });
    cx.run_until_parked();
    executor.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let marks = editor.source_diagnostic_marks();
        let [mark] = marks.as_slice() else {
            panic!("expected one diagnostic");
        };
        let from = source.find("false").unwrap();
        assert_eq!(mark.range, from..from);
        assert_eq!(
            mark.diagnostic.message,
            "Expected ',' or ']' after array element in JSON at position 11 (line 3 column 3)"
        );
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn stale_json_results_do_not_publish_after_edits_or_during_ime(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(BAD.into(), window, cx));
    let executor = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| enter_json(editor, window, cx));
        cx.background_executor().clone()
    });
    cx.run_until_parked();
    executor.advance_clock(Duration::from_millis(300));
    let comma = BAD.find(',').unwrap();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_selection(comma, comma + 1);
            editor.replace_and_mark_text_in_range(None, "中", None, window, cx);
            assert!(editor.is_composing());
            assert!(!editor.open_source_diagnostics(window, cx));
        })
    });
    cx.run_until_parked();
    executor.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.source_diagnostic_marks().is_empty());
            assert_eq!(lint::diagnostic_count(editor.view.state()), 0);
            editor.replace_text_in_range(None, "", window, cx);
            assert!(!editor.is_composing());
        })
    });
    cx.run_until_parked();
    executor.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), BAD.replacen(',', "", 1));
        assert!(editor.source_diagnostic_marks().is_empty());
        assert!(!editor.diagnostics.runner.pending());
    });
}

const DRAFT: &str = "Outside.\n\n```rust\n字 first\nsecond\n```\n\n```txt\nother\n```\n";

fn configure(editor: &mut Editor, effect: StateEffect, cx: &mut Context<Editor>) {
    let active = editor.active.clone();
    editor
        .view
        .update(
            Transaction::default().with_effects([effect]),
            active.clone(),
            |_| (None, active),
        )
        .unwrap();
    cx.notify();
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

#[gpui::test]
fn configured_sources_use_precedence_delay_and_existing_presentation(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(DRAFT.into(), window, cx));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let glyph = DRAFT.find('字').unwrap();
    let second = DRAFT.find("second").unwrap();
    let source = |name, offset, delay| {
        let calls = calls.clone();
        linter(
            Some(LintSource::new(move |state| {
                assert_eq!(state.text().as_str(), DRAFT);
                calls.lock().unwrap().push(name);
                let mut result = vec![Diagnostic::new(offset..offset + 1, Severity::Warning, name)];
                if name == "high" {
                    result[0].range = glyph..glyph + '字'.len_utf8();
                    result.push(Diagnostic::new(
                        glyph + 1..glyph + 2,
                        Severity::Error,
                        "invalid byte boundaries",
                    ));
                    result.push(Diagnostic::new(0..1, Severity::Error, "outside"));
                }
                result
            })),
            LintConfig {
                delay: Some(ms(delay)),
                auto_panel: name == "high",
                marker_filter: Some(Arc::new(move |mut diagnostics, _| {
                    assert!(diagnostics.iter().all(|diagnostic| !matches!(
                        diagnostic.message.as_str(),
                        "outside" | "invalid byte boundaries"
                    )));
                    for diagnostic in &mut diagnostics {
                        diagnostic
                            .message
                            .push_str(if name == "high" { " H" } else { " L" });
                    }
                    diagnostics.push(Diagnostic::new(1..2, Severity::Error, "filter outside"));
                    diagnostics
                })),
                ..LintConfig::default()
            },
        )
    };
    let executor = cx.update(|_, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            configure(
                editor,
                Extension::group([
                    source("low", second, 120),
                    source("high", glyph, 50).with_precedence(Precedence::High),
                ])
                .append(),
                cx,
            );
            assert!(!editor.has_source_diagnostics());
        });
        cx.background_executor().clone()
    });
    cx.run_until_parked();
    executor.advance_clock(ms(500));
    assert!(calls.lock().unwrap().is_empty());
    cx.update(|window, cx| editor.update(cx, |editor, cx| enter_fence(editor, 0, window, cx)));
    cx.run_until_parked();
    executor.advance_clock(ms(119));
    cx.run_until_parked();
    assert!(calls.lock().unwrap().is_empty());
    executor.advance_clock(ms(1));
    cx.run_until_parked();
    assert_eq!(*calls.lock().unwrap(), ["high", "low"]);
    let pointer = cx.update(|window, cx| {
        let editor = editor.read(cx);
        let marks = editor.source_diagnostic_marks();
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].range, glyph..glyph + '字'.len_utf8());
        assert_eq!(marks[0].diagnostic.message, "high H L");
        assert_eq!(marks[1].diagnostic.message, "low H L");
        assert_eq!(lint::diagnostic_count(editor.view.state()), 2);
        assert!(editor.focus.is_focused(window));
        assert_eq!(editor.source(), DRAFT);
        assert!(!editor.view.can_undo());
        marker_anchor(editor.layout.as_ref().unwrap(), glyph)
            .unwrap()
            .center()
    });
    assert!(cx.debug_bounds("source-diagnostics-panel").is_some());
    cx.simulate_mouse_move(pointer, None, Modifiers::none());
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert_eq!(
            editor
                .source_diagnostic_tooltip(
                    editor.layout.as_ref().unwrap(),
                    editor.scroll.bounds(),
                    window
                )
                .unwrap()
                .text,
            "high H L"
        );
    });
    cx.simulate_keystrokes("f8");
    cx.update(|_, cx| assert_eq!(editor.read(cx).selection(), glyph..glyph + '字'.len_utf8()));
    cx.simulate_keystrokes("cmd-shift-m");
    cx.simulate_keystrokes("escape");
    assert_eq!(*calls.lock().unwrap(), ["high", "low"]);
}

#[gpui::test]
fn settings_apply_to_stock_json_until_a_configured_source_replaces_it(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(BAD.into(), window, cx));
    let compartment = Compartment::new();
    let settings = LintConfig {
        delay: Some(ms(50)),
        auto_panel: true,
        ..LintConfig::default()
    };
    let executor = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            configure(
                editor,
                compartment.of(linter(None, settings.clone())).append(),
                cx,
            );
            enter_json(editor, window, cx);
        });
        cx.background_executor().clone()
    });
    cx.run_until_parked();
    executor.advance_clock(ms(49));
    cx.run_until_parked();
    cx.update(|_, cx| assert!(editor.read(cx).source_diagnostic_marks().is_empty()));
    executor.advance_clock(ms(1));
    cx.run_until_parked();
    assert!(cx.debug_bounds("source-diagnostics-panel").is_some());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(
                editor.source_diagnostic_marks()[0]
                    .diagnostic
                    .source
                    .as_deref(),
                Some("JSON")
            );
            configure(
                editor,
                compartment.reconfigure(linter(Some(LintSource::new(|_| Vec::new())), settings)),
                cx,
            );
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert!(!editor.read(cx).diagnostic_panel_open()));
    // Open forces the pending empty publication; auto-panel closes and returns focus.
    cx.simulate_keystrokes("cmd-shift-m");
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.diagnostic_panel_open());
            assert!(editor.focus.is_focused(window));
            assert!(editor.source_diagnostic_marks().is_empty());
            assert_eq!(editor.source(), BAD);
            assert!(!editor.view.can_undo());
            configure(editor, compartment.reconfigure(Extension::default()), cx);
        })
    });
    cx.run_until_parked();
    executor.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source_diagnostic_marks().len(), 1);
        assert!(!editor.diagnostic_panel_open());
    });
}

#[gpui::test]
fn configured_force_refresh_and_partial_results_follow_the_native_clock(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(DRAFT.into(), window, cx));
    let glyph = DRAFT.find('字').unwrap();
    let second = DRAFT.find("second").unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let executor = cx.update(|window, cx| {
        bind_keys(cx);
        let executor = cx.background_executor().clone();
        let fast_calls = calls.clone();
        let slow_clock = executor.clone();
        let slow_calls = calls.clone();
        editor.update(cx, |editor, cx| {
            configure(
                editor,
                Extension::group([
                    linter(
                        Some(LintSource::new(move |_| {
                            fast_calls.lock().unwrap().push("fast");
                            vec![Diagnostic::new(glyph..glyph, Severity::Error, "fast")]
                        })),
                        LintConfig {
                            needs_refresh: Some(Arc::new(move |update| {
                                update.selection_set
                                    && update.state.selection().main().head() == second
                            })),
                            ..LintConfig::default()
                        },
                    ),
                    linter(
                        Some(LintSource::asynchronous(move |_| {
                            slow_calls.lock().unwrap().push("slow");
                            let timer = slow_clock.timer(ms(400));
                            async move {
                                timer.await;
                                Ok(vec![Diagnostic::new(
                                    second..second,
                                    Severity::Warning,
                                    "slow",
                                )])
                            }
                        })),
                        LintConfig::default(),
                    ),
                ])
                .append(),
                cx,
            );
            enter_fence(editor, 0, window, cx);
        });
        executor
    });
    cx.run_until_parked();
    assert!(calls.lock().unwrap().is_empty());
    cx.simulate_keystrokes("cmd-shift-m");
    cx.run_until_parked();
    assert_eq!(*calls.lock().unwrap(), ["fast", "slow"]);
    executor.advance_clock(ms(199));
    cx.run_until_parked();
    cx.update(|_, cx| assert!(editor.read(cx).source_diagnostic_marks().is_empty()));
    executor.advance_clock(ms(1));
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(editor.read(cx).source_diagnostic_marks().len(), 1));
    executor.advance_clock(ms(200));
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(editor.read(cx).source_diagnostic_marks().len(), 2));
    cx.simulate_keystrokes("escape");
    cx.update(|_, cx| editor.update(cx, |editor, _| editor.set_selection(second - 1, second - 1)));
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    assert_eq!(calls.lock().unwrap().len(), 2);
    executor.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    assert_eq!(*calls.lock().unwrap(), ["fast", "slow", "fast", "slow"]);
    cx.update(|_, cx| assert!(!editor.read(cx).view.can_undo()));
}

#[gpui::test]
fn configured_work_is_invalidated_by_edits_reconfiguration_fence_switch_and_ime(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(DRAFT.into(), window, cx));
    let compartment = Compartment::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (executor, registration) = cx.update(|window, cx| {
        bind_keys(cx);
        let executor = cx.background_executor().clone();
        let clock = executor.clone();
        let calls = calls.clone();
        let registration = linter(
            Some(LintSource::asynchronous(move |state| {
                let at = state.selection().main().head();
                let mut calls = calls.lock().unwrap();
                calls.push(at);
                let run = calls.len();
                let timer = clock.timer(ms(100));
                async move {
                    timer.await;
                    Ok(vec![Diagnostic::new(
                        at..at,
                        Severity::Error,
                        format!("run {run}"),
                    )])
                }
            })),
            LintConfig {
                delay: Some(ms(20)),
                ..LintConfig::default()
            },
        );
        editor.update(cx, |editor, cx| {
            configure(editor, compartment.of(registration.clone()).append(), cx);
            enter_fence(editor, 0, window, cx);
            let start = editor.source_fence().unwrap().body.start;
            editor.set_selection(start, start);
        });
        (executor, registration)
    });
    cx.run_until_parked();
    executor.advance_clock(ms(20));
    cx.run_until_parked();
    assert_eq!(calls.lock().unwrap().len(), 1);
    cx.simulate_input("x");
    cx.run_until_parked();
    executor.advance_clock(ms(20));
    cx.run_until_parked();
    assert_eq!(calls.lock().unwrap().len(), 2);
    executor.advance_clock(ms(80));
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(lint::diagnostic_count(editor.view.state()), 0);
            // Both transactions can occur before the next render. The prior run
            // must be invalid even though the registrations end up identical.
            configure(editor, compartment.reconfigure(Extension::default()), cx);
            configure(editor, compartment.reconfigure(registration), cx);
        })
    });
    cx.run_until_parked();
    executor.advance_clock(ms(20));
    cx.run_until_parked();
    assert_eq!(calls.lock().unwrap().len(), 3);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(lint::diagnostic_count(editor.view.state()), 0);
            enter_fence(editor, 1, window, cx);
            let start = editor.source_fence().unwrap().body.start;
            editor.set_selection(start, start);
        })
    });
    cx.run_until_parked();
    executor.advance_clock(ms(20));
    cx.run_until_parked();
    assert_eq!(calls.lock().unwrap().len(), 4);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.replace_and_mark_text_in_range(None, "中", None, window, cx);
            assert!(editor.is_composing());
            assert!(!editor.open_source_diagnostics(window, cx));
        })
    });
    cx.run_until_parked();
    executor.advance_clock(ms(200));
    cx.run_until_parked();
    assert_eq!(calls.lock().unwrap().len(), 4);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(lint::diagnostic_count(editor.view.state()), 0);
            editor.replace_text_in_range(None, "", window, cx);
            assert!(!editor.is_composing());
        })
    });
    cx.run_until_parked();
    executor.advance_clock(ms(20));
    cx.run_until_parked();
    executor.advance_clock(ms(100));
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let marks = editor.source_diagnostic_marks();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].diagnostic.message, "run 5");
        assert_eq!(marks[0].range.start, editor.source().find("other").unwrap());
        assert_eq!(editor.source(), DRAFT.replacen('字', "x字", 1));
    });
}

#[gpui::test]
fn standalone_native_diagnostics_require_an_explicit_source_and_keep_original_bytes(
    cx: &mut TestAppContext,
) {
    const SOURCE: &str = "字 first\r\nnext\r\n";
    let next = SOURCE.find("next").unwrap();
    let end = SOURCE.len();
    let middle = SOURCE.find('\n').unwrap();
    let slot = Compartment::new();
    let settings = LintConfig {
        delay: Some(ms(30)),
        marker_filter: Some(Arc::new(move |diagnostics, _| {
            assert_eq!(diagnostics.len(), 3);
            diagnostics
        })),
        ..Default::default()
    };
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = calls.clone();
    let provider = LintSource::new(move |state| {
        observed
            .lock()
            .unwrap()
            .push(state.text().as_str().to_owned());
        let mut first = Diagnostic::new(0..'字'.len_utf8(), Severity::Error, "first");
        first.actions.push(Action::new("Must not run", |_, _| {
            panic!("standalone diagnostic action")
        }));
        vec![
            first,
            Diagnostic::new(next..next, Severity::Warning, "next line"),
            Diagnostic::new(end..end, Severity::Info, "end of file"),
            Diagnostic::new(1..2, Severity::Error, "inside UTF-8"),
            Diagnostic::new(middle..middle, Severity::Error, "inside CRLF"),
            Diagnostic::new(4..middle, Severity::Error, "split CRLF endpoint"),
            Diagnostic::new(end..end + 1, Severity::Error, "past EOF"),
        ]
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::with_native_extensions(
            SOURCE.into(),
            Extension::group([
                slot.of(linter(None, settings.clone())),
                change_filter(|_, _| ChangePermission::Reject),
            ]),
            window,
            cx,
        )
        .unwrap();
        editor.set_path(Some("invalid.json".into()), None);
        editor
    });
    let clock = cx.executor();
    cx.run_until_parked();
    clock.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    let before = cx.update(|window, cx| {
        bind_keys(cx);
        window.activate_window();
        editor.update(cx, |editor, cx| {
            window.focus(&editor.focus);
            assert!(!editor.open_source_diagnostics(window, cx));
            assert_eq!(lint::diagnostic_count(editor.view.state()), 0);
            configure(
                editor,
                slot.reconfigure(linter(Some(provider), settings)),
                cx,
            );
            editor.sync_source_diagnostics(cx);
            assert!(calls.lock().unwrap().is_empty());
            assert!(!editor.replace(0..0, "rejected", cx));
            editor.view.state().selection().clone()
        })
    });
    cx.run_until_parked();
    clock.advance_clock(ms(29));
    cx.run_until_parked();
    assert!(calls.lock().unwrap().is_empty());
    clock.advance_clock(ms(1));
    cx.run_until_parked();
    assert_eq!(*calls.lock().unwrap(), [SOURCE]);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let marks = editor.source_diagnostic_marks();
            let ranges: Vec<_> = marks.iter().map(|mark| mark.range.clone()).collect();
            assert_eq!(ranges, [0..'字'.len_utf8(), next..next, end..end]);
            assert_eq!(lint::diagnostic_count(editor.view.state()), 3);
            editor.run_source_diagnostic_action(&marks[0].diagnostic, 0, cx);
            assert_eq!(editor.source(), SOURCE);
            assert_eq!(editor.view.state().selection(), &before);
            assert_eq!(
                (
                    editor.view.revision(),
                    editor.view.undo_depth(),
                    editor.view.redo_depth()
                ),
                (0, 0, 0)
            );
            assert!(!editor.view.can_undo_selection() && !editor.view.can_redo_selection());
            assert!(editor.focus.is_focused(window));
        });
    });
    cx.simulate_keystrokes("f8");
    cx.update(|_, cx| assert_eq!(editor.read(cx).selection(), 0..'字'.len_utf8()));
    cx.simulate_keystrokes("f8");
    cx.update(|_, cx| assert_eq!(editor.read(cx).selection(), next..next));
    cx.simulate_keystrokes("f8");
    cx.update(|_, cx| assert_eq!(editor.read(cx).selection(), end..end));
    cx.simulate_keystrokes("shift-f8");
    cx.update(|_, cx| assert_eq!(editor.read(cx).selection(), next..next));
    cx.simulate_keystrokes("cmd-shift-m");
    cx.run_until_parked();
    assert!(cx.debug_bounds("source-diagnostics-panel").is_some());
    let row = cx.debug_bounds("source-diagnostic-0").unwrap().center();
    cx.simulate_click(row, Modifiers::none());
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.selection(), 0..'字'.len_utf8());
        assert!(editor.focus.is_focused(window));
        assert_eq!(editor.source(), SOURCE);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn standalone_native_diagnostics_reject_stale_path_text_and_registration_results(
    cx: &mut TestAppContext,
) {
    let slot = Compartment::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new("字\r\n".into(), window, cx));
    let clock = cx.executor();
    let provider_clock = clock.clone();
    let observed = calls.clone();
    let registration = linter(
        Some(LintSource::asynchronous(move |state| {
            let snapshot = state.text().clone();
            let mut calls = observed.lock().unwrap();
            calls.push(snapshot.as_str().to_owned());
            let run = calls.len();
            let timer = provider_clock.timer(ms(100));
            async move {
                timer.await;
                Ok(vec![Diagnostic::new(
                    0..0,
                    Severity::Error,
                    format!("run {run}: {}", snapshot.as_str()),
                )])
            }
        })),
        LintConfig {
            delay: Some(ms(20)),
            ..Default::default()
        },
    );
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_path(Some("original.txt".into()), None);
            configure(editor, slot.of(registration.clone()).append(), cx);
            editor.sync_source_diagnostics(cx);
        });
    });
    let mut wait = ms(20);
    for (i, change) in ["path", "restored path", "edit", "restored registration"]
        .into_iter()
        .enumerate()
    {
        let path_change = matches!(change, "path" | "restored path");
        cx.run_until_parked();
        clock.advance_clock(wait);
        cx.run_until_parked();
        assert_eq!(calls.lock().unwrap().len(), i + 1);
        clock.advance_clock(ms(90));
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                match change {
                    "path" => editor.set_path(Some("other.js".into()), None),
                    "restored path" => {
                        editor.set_path(Some("other.js".into()), None);
                        editor.set_path(Some("original.txt".into()), None);
                        assert!(editor.live_diagnostic_scope().is_some());
                    }
                    "edit" => assert!(editor.replace(0..0, "x", cx)),
                    "restored registration" => {
                        configure(editor, slot.reconfigure(Extension::default()), cx);
                        configure(editor, slot.reconfigure(registration.clone()), cx);
                    }
                    _ => unreachable!(),
                }
                assert!(editor.current_diagnostic_scope().is_none());
                // Let path-stale work reach its publication check before we
                // explicitly sync the new scope or request another frame.
                if !path_change {
                    editor.sync_source_diagnostics(cx);
                    cx.notify();
                }
            });
        });
        clock.advance_clock(ms(10));
        cx.run_until_parked();
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                assert_eq!(lint::diagnostic_count(editor.view.state()), 0, "{change}");
                if path_change {
                    editor.sync_source_diagnostics(cx);
                    cx.notify();
                }
            });
        });
        wait = if path_change { ms(20) } else { ms(10) };
    }
    clock.advance_clock(wait);
    cx.run_until_parked();
    assert_eq!(
        *calls.lock().unwrap(),
        ["字\r\n", "字\r\n", "字\r\n", "x字\r\n", "x字\r\n"]
    );
    clock.advance_clock(ms(100));
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let marks = editor.source_diagnostic_marks();
            assert_eq!(marks.len(), 1);
            assert_eq!(marks[0].diagnostic.message, "run 5: x字\r\n");
            configure(editor, slot.reconfigure(Extension::default()), cx);
        });
    });
    cx.run_until_parked();
    clock.advance_clock(DEFAULT_LINT_DELAY);
    cx.run_until_parked();
    assert_eq!(calls.lock().unwrap().len(), 5);
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(!editor.has_source_diagnostics());
        assert_eq!(lint::diagnostic_count(editor.view.state()), 0);
        assert_eq!(editor.source(), "x字\r\n");
        assert_eq!((editor.view.revision(), editor.view.undo_depth()), (1, 1));
    });
}

#[gpui::test]
fn standalone_native_diagnostics_wait_for_an_editable_pane(cx: &mut TestAppContext) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = calls.clone();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            "alpha\r\n".into(),
            linter(
                Some(LintSource::new(move |state| {
                    observed
                        .lock()
                        .unwrap()
                        .push(state.text().as_str().to_owned());
                    vec![Diagnostic::new(0..0, Severity::Warning, "alpha")]
                })),
                LintConfig {
                    delay: Some(ms(20)),
                    ..Default::default()
                },
            ),
            window,
            cx,
        )
        .unwrap()
    });
    let clock = cx.executor();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.has_source_diagnostics()); // Markdown prose.
            editor.review_changes("beta\r\n", window, cx);
            assert!(editor.is_reviewing_changes());
            editor.set_path(Some("sample.txt".into()), None);
            assert!(!editor.has_source_diagnostics());
        });
    });
    clock.advance_clock(ms(20));
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.dismiss_merge_review(cx);
            assert!(editor.has_source_diagnostics());
            editor.set_input_enabled(false, cx);
            assert!(!editor.has_source_diagnostics());
        });
    });
    clock.advance_clock(ms(20));
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_input_enabled(true, cx);
            editor.replace_and_mark_text_in_range(Some(0..0), "字", None, window, cx);
            assert!(editor.is_composing());
            assert!(!editor.has_source_diagnostics());
        });
    });
    clock.advance_clock(ms(20));
    cx.run_until_parked();
    assert!(calls.lock().unwrap().is_empty());
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.unmark_text(window, cx);
            assert!(editor.has_source_diagnostics());
        });
    });
    cx.run_until_parked();
    clock.advance_clock(ms(20));
    cx.run_until_parked();
    assert_eq!(*calls.lock().unwrap(), ["字alpha\r\n"]);
    cx.update(|_, cx| assert_eq!(editor.read(cx).source_diagnostic_marks().len(), 1));
}

#[gpui::test]
fn diagnostic_action_cannot_replace_text_outside_the_source_fence(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(BAD.into(), window, cx));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            enter_json(editor, window, cx);
            editor.sync_source_diagnostics(cx);
            let comma = BAD.find(',').unwrap();
            let mut fix = Diagnostic::new(comma..comma + 1, Severity::Error, "Extra comma");
            fix.actions.push(Action::new("Remove fence", |_, _| {
                Ok(Transaction::new(ChangeSet::single(Change::new(0..3, ""))?))
            }));
            fix.actions.push(Action::new("Select fence", |_, _| {
                Ok(Transaction::default()
                    .with_selection(gpui_editor_codemirror::EditorSelection::single(0, 3)))
            }));
            fix.actions.push(Action::new("Remove comma", |_, range| {
                Ok(Transaction::new(ChangeSet::single(Change::new(range, ""))?))
            }));
            assert!(editor.apply_diagnostic_update(lint::set_diagnostics(vec![fix])));
            let mark = editor.source_diagnostic_marks().pop().unwrap();
            let selection = editor.selection();
            editor.run_source_diagnostic_action(&mark.diagnostic, 0, cx);
            editor.run_source_diagnostic_action(&mark.diagnostic, 1, cx);
            assert_eq!(editor.source(), BAD);
            assert_eq!(editor.selection(), selection);
            assert!(!editor.view.can_undo());
            editor.run_source_diagnostic_action(&mark.diagnostic, 2, cx);
            assert_eq!(editor.source(), BAD.replacen(',', "", 1));
            assert!(editor.view.can_undo());
        })
    });
}
