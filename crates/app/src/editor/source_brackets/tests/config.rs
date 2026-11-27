use std::sync::Arc;

use gpui_editor_codemirror::{
    Compartment, Extension, Transaction,
    language::brackets::{BracketMatchingOptions, bracket_matching},
};

use super::*;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Options {
    brackets: Option<String>,
    max_scan_distance: Option<usize>,
    after_cursor: Option<bool>,
}

impl From<Options> for BracketMatchingOptions {
    fn from(options: Options) -> Self {
        Self {
            brackets: options.brackets.map(Arc::from),
            max_scan_distance: options.max_scan_distance,
            after_cursor: options.after_cursor,
            ..Default::default()
        }
    }
}

#[gpui::test]
fn stock_plain_feedback_counts_logical_utf16_chunks(cx: &mut TestAppContext) {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        fill: String,
        repeat: usize,
        separator: String,
        checks: Vec<(usize, Vec<[usize; 2]>)>,
    }
    const PREFIX: &str = "字 😀\r\n\r\n```text\r\n";
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../codemirror/tests/fixtures/bracket_plain_defaults.jsonl"
    ));
    for row in fixture.lines() {
        let case: Case = serde_json::from_str(row).unwrap();
        let source = format!(
            "{PREFIX}({}{})\r\n```\r\n\r\nOutside )",
            case.fill.repeat(case.repeat),
            case.separator
        );
        let origin = PREFIX.len();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                activate_first_code_preview(editor, cx);
                window.focus(&editor.focus);
                for (head, expected) in case.checks {
                    editor.set_selection(origin + head, origin + head);
                    let mut actual = Vec::new();
                    if let Some(hit) = editor.source_bracket_match() {
                        assert!(hit.matched, "{} at {head}", case.name);
                        for range in std::iter::once(hit.start).chain(hit.end) {
                            actual.push([range.start - origin, range.end - origin]);
                        }
                        actual.sort_unstable();
                    }
                    assert_eq!(actual, expected, "{} at {head}", case.name);
                }
                assert_eq!(editor.source(), source);
                assert!(!editor.view.can_undo());
            });
        });
    }
}

#[gpui::test]
fn installed_plain_pairs_and_scan_limit_control_live_feedback(cx: &mut TestAppContext) {
    replay_bracket_hits(
        cx,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../codemirror/tests/fixtures/bracket_plain_config.jsonl"
        )),
        |mode| Some(mode),
    );
}

#[gpui::test]
fn installed_javascript_bracket_options_match_pinned_hits(cx: &mut TestAppContext) {
    replay_bracket_hits(
        cx,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../codemirror/tests/fixtures/javascript_bracket_options.jsonl"
        )),
        |mode| Some(mode),
    );
}

#[gpui::test]
fn installed_css_bracket_options_match_pinned_hits(cx: &mut TestAppContext) {
    replay_bracket_hits(
        cx,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../codemirror/tests/fixtures/css_bracket_options.jsonl"
        )),
        |mode| (mode == "stylesheet").then_some("css"),
    );
}

#[gpui::test]
fn installed_grammar_bracket_options_match_pinned_hits(cx: &mut TestAppContext) {
    replay_bracket_hits(
        cx,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../codemirror/tests/fixtures/grammar_bracket_options.jsonl"
        )),
        |mode| Some(mode),
    );
}

#[gpui::test]
fn installed_go_and_rust_bracket_options_match_pinned_hits(cx: &mut TestAppContext) {
    replay_bracket_hits(
        cx,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../codemirror/tests/fixtures/go_rust_bracket_options.jsonl"
        )),
        |mode| Some(mode),
    );
}

#[gpui::test]
fn installed_java_and_python_bracket_options_match_pinned_hits(cx: &mut TestAppContext) {
    replay_bracket_hits(
        cx,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../codemirror/tests/fixtures/java_python_bracket_options.jsonl"
        )),
        |mode| Some(mode),
    );
}

fn replay_bracket_hits(
    cx: &mut TestAppContext,
    fixture: &str,
    fence_language: impl Fn(&str) -> Option<&str>,
) {
    #[derive(serde::Deserialize)]
    struct Case {
        mode: Option<String>,
        name: String,
        source: String,
        options: Options,
        checks: Vec<(usize, Option<Hit>)>,
    }
    for line in fixture.lines() {
        let case: Case = serde_json::from_str(line).unwrap();
        let Some(language) = fence_language(case.mode.as_deref().unwrap_or("text")) else {
            continue;
        };
        let prefix = format!("前 😀\r\n\r\n```{language}\r\n");
        let origin = prefix.len();
        let source = format!("{prefix}{}\r\n```\r\n\r\nOutside > )»,", case.source);
        let options = case.options.into();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(source.clone(), bracket_matching(options), window, cx)
                .unwrap()
        });
        let &(paint_head, paint_hit) = case
            .checks
            .iter()
            .find(|(_, hit)| hit.is_some())
            .unwrap_or(&case.checks[0]);
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                activate_first_code_preview(editor, cx);
                window.focus(&editor.focus);
                let range = |[start, end]: [usize; 2]| origin + start..origin + end;
                for (head, hit) in case.checks {
                    editor.set_selection(origin + head, origin + head);
                    assert_eq!(
                        editor.source_bracket_match(),
                        hit.map(|(start, end, matched)| BracketMatch {
                            start: range(start),
                            end: end.map(range),
                            matched,
                        }),
                        "{} at {head}",
                        case.name
                    );
                }
                let paint_head = origin + paint_head;
                editor.set_selection(paint_head, paint_head);
                cx.notify();
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            let painted = &editor.layout.as_ref().unwrap().painted_bracket_quads;
            assert_eq!(
                painted.len(),
                paint_hit.map_or(0, |(_, end, _)| 1 + usize::from(end.is_some())),
                "{}",
                case.name
            );
            assert!(editor.focus.is_focused(window));
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());
        });
    }
}

#[gpui::test]
fn installed_json_bracket_options_match_pinned_marks(cx: &mut TestAppContext) {
    type Mark = (usize, usize, bool);
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        source: String,
        options: Options,
        checks: Vec<(usize, Vec<Mark>)>,
    }
    const PREFIX: &str = "前 😀\r\n\r\n```json\r\n";
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../codemirror/tests/fixtures/json_bracket_options.jsonl"
    ));
    for line in fixture.lines() {
        let case: Case = serde_json::from_str(line).unwrap();
        let source = format!("{PREFIX}{}\r\n```\r\n\r\nOutside 】 » )", case.source);
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                bracket_matching(case.options.into()),
                window,
                cx,
            )
            .unwrap()
        });
        let (paint_head, paint_marks) = case
            .checks
            .iter()
            .find(|(_, marks)| !marks.is_empty())
            .unwrap();
        let paint_head = PREFIX.len() + paint_head;
        let painted_count = paint_marks.len();
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                activate_first_code_preview(editor, cx);
                window.focus(&editor.focus);
                for (head, expected) in case.checks {
                    editor.set_selection(PREFIX.len() + head, PREFIX.len() + head);
                    let mut marks = Vec::new();
                    if let Some(hit) = editor.source_bracket_match() {
                        for range in std::iter::once(hit.start).chain(hit.end) {
                            marks.push((
                                range.start - PREFIX.len(),
                                range.end - PREFIX.len(),
                                hit.matched,
                            ));
                        }
                        marks.sort_unstable();
                    }
                    assert_eq!(marks, expected, "{} at {head}", case.name);
                }
                editor.set_selection(paint_head, paint_head);
                cx.notify();
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let painted = &editor.layout.as_ref().unwrap().painted_bracket_quads;
            assert_eq!(painted.len(), painted_count, "{}", case.name);
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());
        });
    }
}

#[gpui::test]
fn removing_plain_bracket_options_restores_stock_paint(cx: &mut TestAppContext) {
    check_removal(cx, "text", "<x> (p)", "<>", 0);
}

#[gpui::test]
fn removing_javascript_bracket_options_restores_stock_paint(cx: &mut TestAppContext) {
    check_removal(cx, "js", "/*«x»*/; (p)", "«»", 2);
}

fn check_removal(cx: &mut TestAppContext, mode: &str, body: &str, brackets: &str, head: usize) {
    let prefix = format!("前 😀\r\n\r\n```{mode}\r\n");
    let source = format!("{prefix}{body}\r\n```\r\n\r\nOutside >»");
    let custom = prefix.len() + head;
    let stock = prefix.len() + body.rfind('(').unwrap();
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.clone(),
            slot.of(bracket_matching(BracketMatchingOptions {
                brackets: Some(Arc::from(brackets)),
                ..Default::default()
            })),
            window,
            cx,
        )
        .unwrap()
    });
    for installed in [true, false] {
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                if installed {
                    activate_first_code_preview(editor, cx);
                    window.focus(&editor.focus);
                    editor.set_selection(custom, custom);
                } else {
                    remove_options(editor, &slot);
                }
                assert_eq!(editor.selection(), custom..custom);
                assert_eq!(editor.source_bracket_match().is_some(), installed);
                cx.notify();
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let painted = &editor.layout.as_ref().unwrap().painted_bracket_quads;
            assert_eq!(painted.len(), if installed { 2 } else { 0 });
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());
        });
    }
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor.set_selection(stock, stock);
            assert!(editor.source_bracket_match().is_some_and(|hit| hit.matched));
        });
    });
}

#[gpui::test]
fn installed_after_cursor_rule_controls_live_bracket_feedback(cx: &mut TestAppContext) {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Case {
        language: String,
        source: String,
        before_only: Vec<(usize, Option<Hit>)>,
        both: Vec<(usize, Option<Hit>)>,
    }
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../codemirror/tests/fixtures/bracket_after_cursor.jsonl"
    ));
    for line in fixture.lines() {
        let case: Case = serde_json::from_str(line).unwrap();
        let prefix = format!("前 😀\r\n\r\n```{}\r\n", case.language);
        let origin = prefix.len();
        let source = format!("{prefix}{}\r\n```\r\n\r\nOutside )", case.source);
        let slot = Compartment::new();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                slot.of(bracket_matching(BracketMatchingOptions {
                    after_cursor: Some(false),
                    ..Default::default()
                })),
                window,
                cx,
            )
            .unwrap()
        });
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                activate_first_code_preview(editor, cx);
                window.focus(&editor.focus);
            });
        });
        let (&(paint_head, _), &(_, Some(paint_hit))) = case
            .before_only
            .iter()
            .zip(&case.both)
            .find(|(before, both)| before.1.is_none() && both.1.is_some())
            .unwrap()
        else {
            unreachable!()
        };
        for (both, checks) in [(false, case.before_only), (true, case.both)] {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    if both {
                        remove_options(editor, &slot);
                    }
                    let range = |[start, end]: [usize; 2]| origin + start..origin + end;
                    for (head, hit) in checks {
                        editor.set_selection(origin + head, origin + head);
                        assert_eq!(
                            editor.source_bracket_match(),
                            hit.map(|(start, end, matched)| {
                                BracketMatch {
                                    start: range(start),
                                    end: end.map(range),
                                    matched,
                                }
                            }),
                            "{} at {head}, both={both}",
                            case.language
                        );
                    }
                    editor.set_selection(origin + paint_head, origin + paint_head);
                    cx.notify();
                });
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                let editor = editor.read(cx);
                let painted = &editor.layout.as_ref().unwrap().painted_bracket_quads;
                assert_eq!(
                    painted.len(),
                    if both {
                        1 + usize::from(paint_hit.1.is_some())
                    } else {
                        0
                    }
                );
                assert!(editor.focus.is_focused(window));
                assert_eq!(editor.source(), source);
                assert!(!editor.view.can_undo());
            });
        }
    }
}

fn remove_options(editor: &mut Editor, slot: &Compartment) {
    let active = editor.active.clone();
    editor
        .view
        .update(
            Transaction::default().with_effects([slot.reconfigure(Extension::default())]),
            active.clone(),
            |_| (None, active),
        )
        .unwrap();
}
