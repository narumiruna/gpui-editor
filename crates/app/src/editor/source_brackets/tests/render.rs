use std::sync::Arc;

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Compartment, EditorSelection, Extension, SelectionRange,
    Transaction,
    language::brackets::{BracketMatchRender, BracketMatchingOptions, bracket_matching},
    theme_one_dark::one_dark_theme,
    view::MarkDecoration,
};

use super::{multiple::assert_painted, *};

const PREFIX: &str = "前 😀\r\n\r\n```text\r\n";
const SUFFIX: &str = "\r\n```\r\n\r\nOutside )";

#[gpui::test]
fn configured_render_callbacks_control_canvas_marks(cx: &mut TestAppContext) {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        source: String,
        carets: Vec<usize>,
        marks: Vec<(usize, usize, String)>,
    }
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../codemirror/tests/fixtures/bracket_render_roles.jsonl"
    ));
    let origin = PREFIX.len();
    for line in fixture.lines() {
        let case: Case = serde_json::from_str(line).unwrap();
        let source = format!("{PREFIX}{}{SUFFIX}", case.source);
        let render = renderer({
            let source = source.clone();
            let mode = case.name.clone();
            Arc::new(move |hit, state| {
                assert_eq!(state.text().as_str(), source);
                assert!(hit.matched);
                let (class, range) = match mode.as_str() {
                    "startOnly" => ("cm-nonmatchingBracket", hit.start.clone()),
                    "interior" => (
                        "cm-matchingBracket",
                        hit.start.end..hit.end.as_ref().unwrap().start,
                    ),
                    "empty" => return Vec::new(),
                    "unstyled" => ("host-custom", hit.start.clone()),
                    _ => unreachable!(),
                };
                vec![MarkDecoration::new(class).range(range).unwrap()]
            })
        });
        let slot = Compartment::new();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                Extension::group([
                    ALLOW_MULTIPLE_SELECTIONS.of(true),
                    render,
                    slot.of(Extension::default()),
                ]),
                window,
                cx,
            )
            .unwrap()
        });
        let expected: Vec<_> = case
            .marks
            .iter()
            .filter_map(|(from, to, class)| {
                let matched = match class.as_str() {
                    "cm-matchingBracket" => true,
                    "cm-nonmatchingBracket" => false,
                    "host-custom" => return None,
                    _ => unreachable!(),
                };
                Some((*from, *to, matched))
            })
            .collect();
        for themed in [false, true] {
            cx.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    if themed {
                        let active = editor.active.clone();
                        editor
                            .view
                            .update(
                                Transaction::default()
                                    .with_effects([slot.reconfigure(one_dark_theme())]),
                                active.clone(),
                                |_| (None, active),
                            )
                            .unwrap();
                    } else {
                        activate_first_code_preview(editor, cx);
                        window.focus(&editor.focus);
                        let selection = EditorSelection::new(
                            case.carets
                                .iter()
                                .map(|&head| SelectionRange::new(origin + head, origin + head))
                                .collect(),
                            0,
                        )
                        .unwrap();
                        editor.view.set_selection(selection).unwrap();
                    }
                    cx.notify();
                });
            });
            cx.run_until_parked();
            cx.update(|_, cx| {
                let editor = editor.read(cx);
                assert_painted(editor, origin, &expected, themed);
                assert_eq!(editor.source(), source);
                assert!(!editor.view.can_undo());
            });
        }
    }
}

#[gpui::test]
fn renderer_marks_must_stay_on_valid_bytes_in_the_active_body(cx: &mut TestAppContext) {
    let source = format!("{PREFIX}(😀){SUFFIX}");
    let origin = PREFIX.len();
    for invalid in [0..'前'.len_utf8(), origin + 2..origin + 3] {
        let extension = renderer(Arc::new(move |hit, _| {
            let mark = MarkDecoration::new("cm-matchingBracket");
            vec![
                mark.clone().range(hit.start.clone()).unwrap(),
                mark.range(invalid.clone()).unwrap(),
            ]
        }));
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(source.clone(), extension, window, cx).unwrap()
        });
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                activate_first_code_preview(editor, cx);
                editor.set_selection(origin + 1, origin + 1);
                window.focus(&editor.focus);
                assert!(editor.source_bracket_match().is_some());
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_painted(editor, origin, &[], false);
            assert_eq!(editor.source(), source);
        });
    }
}

fn renderer(render_match: BracketMatchRender) -> Extension {
    bracket_matching(BracketMatchingOptions {
        render_match: Some(render_match),
        ..Default::default()
    })
}
