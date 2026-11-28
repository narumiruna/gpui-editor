use gpui::px;
use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Compartment, EditorSelection, Extension, SelectionRange,
    Transaction, language::bidi_isolates, theme_one_dark::one_dark_theme,
};

use super::*;

type Mark = (usize, usize, bool);

#[derive(serde::Deserialize)]
struct Case {
    language: String,
    body: String,
    ranges: Vec<[usize; 2]>,
    marks: Vec<Mark>,
}

fn cases() -> impl Iterator<Item = Case> {
    // The verified @codemirror/language 6.13.1 bracket plugin. Offsets are UTF-8 bytes.
    include_str!("../multiple.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn select(
    editor: &mut Editor,
    ranges: impl IntoIterator<Item = [usize; 2]>,
    main: usize,
    cx: &mut Context<Editor>,
) {
    let ranges = ranges
        .into_iter()
        .map(|[anchor, head]| SelectionRange::new(anchor, head))
        .collect();
    editor
        .view
        .set_selection(EditorSelection::new(ranges, main).unwrap())
        .unwrap();
    cx.notify();
}

fn marks(editor: &Editor, origin: usize) -> Vec<Mark> {
    let mut marks: Vec<_> = editor
        .source_bracket_matches()
        .into_iter()
        .flat_map(|hit| {
            let matched = hit.matched;
            [Some(hit.start), hit.end]
                .into_iter()
                .flatten()
                .map(move |range| (range.start - origin, range.end - origin, matched))
        })
        .collect();
    marks.sort_unstable();
    marks
}

#[gpui::test]
fn all_empty_native_selections_match_the_pinned_decorations(cx: &mut TestAppContext) {
    for case in cases() {
        let prefix = format!("外 (prose)\r\n\r\n```{} (protected)\r\n", case.language);
        let source = format!(
            "{prefix}{}\r\n```\r\n\r\n```text\r\n(outside)\r\n```",
            case.body
        );
        let origin = prefix.len();
        let ranges = case.ranges.iter().map(|r| r.map(|pos| origin + pos));
        let header = source.find("(protected)").unwrap() + 1;
        let protected = source.find("(outside)").unwrap() + 1;
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                ALLOW_MULTIPLE_SELECTIONS.of(true),
                window,
                cx,
            )
            .unwrap()
        });
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                assert!(editor.source_bracket_matches().is_empty());
                activate_first_code_preview(editor, cx);
                window.focus(&editor.focus);
                select(editor, ranges.clone(), 0, cx);
                assert!(editor.source_syntax.is_none());
                assert_eq!(marks(editor, origin), case.marks, "{}", case.language);
                // A primary caret in the protected header must not suppress a body caret.
                select(
                    editor,
                    [[header; 2]]
                        .into_iter()
                        .chain(ranges)
                        .chain([[protected; 2]]),
                    0,
                    cx,
                );
                assert_eq!(marks(editor, origin), case.marks, "{}", case.language);
                assert_eq!(editor.source(), source);
                assert!(!editor.view.can_undo());
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            if let Some(fence) = editor.source_fence()
                && let Some(parser) = &fence.parser
            {
                assert!(
                    editor
                        .current_source_document(
                            &fence.body,
                            parser,
                            editor.source_bracket_settings()
                        )
                        .is_some()
                );
            }
            assert_eq!(
                marks(editor, origin),
                case.marks,
                "{} cached",
                case.language
            );
            assert_eq!(
                editor.layout.as_ref().unwrap().painted_bracket_quads.len(),
                case.marks.len()
            );
        });
    }
}

#[gpui::test]
fn java_feedback_paints_syntax_pairs_for_multiple_carets(cx: &mut TestAppContext) {
    let body = r#"class A { void f(){ log(")"); } }"#;
    let prefix = "Outside é\r\n\r\n```java (protected)\r\n";
    let source = format!("{prefix}{body}\r\n```\r\n\r\nOutside )");
    let origin = prefix.len();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.clone(),
            ALLOW_MULTIPLE_SELECTIONS.of(true),
            window,
            cx,
        )
        .unwrap()
    });
    let call = body.find("log(").unwrap() + 3;
    let end = body.find("\");").unwrap() + 1;
    let class = body.find('{').unwrap();
    let class_end = body.rfind('}').unwrap();
    let expected = [
        (class, class + 1, true),
        (call, call + 1, true),
        (end, end + 1, true),
        (class_end, class_end + 1, true),
    ];
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            activate_first_code_preview(editor, cx);
            window.focus(&editor.focus);
            select(
                editor,
                [prefix.find('(').unwrap(), origin + class, origin + call].map(|pos| [pos; 2]),
                0,
                cx,
            );
            assert_eq!(marks(editor, origin), expected);
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_painted(editor, origin, &expected, false);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn native_feedback_keeps_literal_and_comment_brackets_separate_from_code(cx: &mut TestAppContext) {
    for (language, body, [first, second], expected) in [
        (
            "rust",
            r#"fn main() { log(")"); }"#,
            [16, 18],
            [(15, 16, true), (17, 18, false), (19, 20, true)],
        ),
        (
            "go",
            "package p\r\nfunc f() { println(/* 🦀 ) */ \"界)\"); }",
            [30, 39],
            [(29, 30, true), (38, 39, false), (49, 50, true)],
        ),
        (
            "python",
            "# 界😀\r\nrun(\r\n    # )\r\n    \"é)\",\r\n)",
            [15, 24],
            [(14, 15, true), (23, 24, false), (38, 39, true)],
        ),
    ] {
        let prefix = format!("外 😀\r\n\r\n```{language} (protected)\r\n");
        let source = format!("{prefix}{body}\r\n```\r\n\r\nOutside )");
        let origin = prefix.len();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                ALLOW_MULTIPLE_SELECTIONS.of(true),
                window,
                cx,
            )
            .unwrap()
        });
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                activate_first_code_preview(editor, cx);
                window.focus(&editor.focus);
                select(
                    editor,
                    [prefix.find('(').unwrap(), origin + first, origin + second].map(|at| [at; 2]),
                    0,
                    cx,
                );
                assert_eq!(marks(editor, origin), expected, "{language}");
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_painted(editor, origin, &expected, false);
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());
        });
    }
}

#[gpui::test]
fn javascript_family_feedback_uses_pinned_delimiters_in_live_fences(cx: &mut TestAppContext) {
    #[derive(serde::Deserialize)]
    struct Case {
        mode: String,
        name: String,
        source: String,
        checks: Vec<(usize, Option<Hit>)>,
    }
    let rows: Vec<Case> = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/codemirror/javascript_brackets.jsonl"
    ))
    .lines()
    .map(|row| serde_json::from_str(row).unwrap())
    .collect();
    for (mode, name, heads) in [
        ("js", "call with string", [4, 6]),
        ("ts", "type and value groups", [33, 47]),
        ("jsx", "fragment boundary", [3, 7]),
        ("tsx", "type parameters and JSX escapes", [28, 33]),
        (
            "ts",
            "type parameters, nested type arguments, and casts",
            [11, 57],
        ),
        ("jsx", "fragment boundary", [1, 14]),
        ("js", "template interpolation boundary", [9, 26]),
    ] {
        let case = rows
            .iter()
            .find(|row| row.mode == mode && row.name == name)
            .unwrap();
        let mut expected = Vec::new();
        for head in heads {
            let (_, Some((start, end, matched))) =
                case.checks.iter().find(|(pos, _)| *pos == head).unwrap()
            else {
                panic!("missing pinned {mode} match at {head}");
            };
            expected.push((start[0], start[1], *matched));
            if let Some([from, to]) = end {
                expected.push((*from, *to, *matched));
            }
        }
        expected.sort_unstable();
        let prefix = format!("字 😀\r\n\r\n```{mode} (protected)\r\n");
        let origin = prefix.len();
        let source = format!("{prefix}{}\r\n```\r\n\r\nOutside )", case.source);
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(
                source.clone(),
                ALLOW_MULTIPLE_SELECTIONS.of(true),
                window,
                cx,
            )
            .unwrap()
        });
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                activate_first_code_preview(editor, cx);
                window.focus(&editor.focus);
                select(
                    editor,
                    [[prefix.find('(').unwrap(); 2]]
                        .into_iter()
                        .chain(heads.map(|head| [origin + head; 2])),
                    0,
                    cx,
                );
                assert_eq!(marks(editor, origin), expected, "{mode}");
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_painted(editor, origin, &expected, false);
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());
        });
    }
}

#[gpui::test]
fn feedback_excludes_fence_headers_and_does_not_cross_html_mounts(cx: &mut TestAppContext) {
    type ScopeCase<'a> = (&'a str, &'a str, &'a [[usize; 2]], &'a [Mark]);
    for (language, body, ranges, expected) in [
        ("text (protected)", "", &[[0, 0]][..], &[][..]),
        (
            "",
            "(x)",
            &[[0, 0], [3, 3]],
            &[(0, 1, true), (0, 1, true), (2, 3, true), (2, 3, true)],
        ),
        (
            "html",
            "<script>f(</script>\r\n<style>x)</style>",
            &[[10, 10], [30, 30]],
            &[(9, 10, false), (29, 30, false)],
        ),
    ] as [ScopeCase<'_>; 3]
    {
        let prefix = format!("```{language}\r\n");
        let separator = if body.is_empty() { "" } else { "\r\n" };
        let source = format!("{prefix}{body}{separator}```\r\n\r\nOutside )");
        let origin = prefix.len();
        let outside = source.len();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(source, ALLOW_MULTIPLE_SELECTIONS.of(true), window, cx)
                .unwrap()
        });
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                activate_first_code_preview(editor, cx);
                if let Some(header) = prefix.find('(') {
                    select(editor, [[header + 1; 2], [origin; 2]], 0, cx);
                } else {
                    select(
                        editor,
                        ranges
                            .iter()
                            .map(|r| r.map(|pos| origin + pos))
                            .chain([[outside; 2]]),
                        0,
                        cx,
                    );
                }
                assert_eq!(marks(editor, origin), expected);
            });
        });
    }
}

pub(super) fn assert_painted(editor: &Editor, origin: usize, expected: &[Mark], installed: bool) {
    let layout = editor.layout.as_ref().unwrap();
    assert_eq!(layout.theme.view.is_some(), installed);
    let mut expected_quads = Vec::new();
    for &(start, end, matched) in expected {
        let range = origin + start..origin + end;
        let color = rgba(if installed {
            0xbad0_f847
        } else if matched {
            0x328c_8252
        } else {
            0xbb55_5544
        });
        for row in &layout.rows {
            for item in row.items.iter().filter(|item| item.is_text()) {
                for x in item.x_spans(range.clone()) {
                    assert!(x.end > x.start);
                    expected_quads.push((
                        Bounds::new(
                            layout.origin + point(x.start, row.y),
                            size(x.end - x.start, row.height),
                        ),
                        color.into(),
                    ));
                }
            }
        }
    }
    assert_eq!(expected_quads.len(), expected.len());
    for quad in &layout.painted_bracket_quads {
        let index = expected_quads
            .iter()
            .position(|expected| *expected == (quad.bounds, quad.background))
            .expect("submitted bracket quad");
        expected_quads.swap_remove(index);
    }
    assert!(expected_quads.is_empty(), "missing bracket quads");
}

#[gpui::test]
fn focused_canvas_submits_native_brackets_and_hides_them_during_ime(cx: &mut TestAppContext) {
    let case = cases().find(|case| case.body.contains("אב")).unwrap();
    let prefix = "```json\r\n";
    let source = format!("{prefix}{}\r\n```", case.body);
    let origin = prefix.len();
    let theme = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.clone(),
            Extension::group([
                ALLOW_MULTIPLE_SELECTIONS.of(true),
                bidi_isolates(true),
                theme.of(Extension::default()),
            ]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(580.), px(390.)));
    let revision = cx.update(|window, cx| {
        bind_keys(cx);
        window.activate_window();
        editor.update(cx, |editor, cx| {
            activate_first_code_preview(editor, cx);
            window.focus(&editor.focus);
            select(
                editor,
                case.ranges.iter().map(|r| r.map(|pos| origin + pos)),
                1,
                cx,
            );
            editor.view.revision()
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert_painted(editor.read(cx), origin, &case.marks, false));

    // A nonempty primary must not suppress secondary matched and mismatched roles.
    let mixed = [&case.marks[..2], &case.marks[4..]].concat();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let mut ranges: Vec<_> = case
                .ranges
                .iter()
                .map(|r| r.map(|pos| origin + pos))
                .collect();
            ranges[1][1] += 1;
            select(editor, ranges, 1, cx);
            let active = editor.active.clone();
            editor
                .view
                .update(
                    Transaction::default().with_effects([theme.reconfigure(one_dark_theme())]),
                    active.clone(),
                    |_| (None, active),
                )
                .unwrap();
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_painted(editor, origin, &mixed, true);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.view.revision(), revision);
        assert!(!editor.view.can_undo());
    });

    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    cx.update(|_, cx| assert_painted(editor.read(cx), origin, &[], true));
    cx.update(|window, cx| window.focus(&editor.read(cx).focus));
    cx.run_until_parked();
    cx.update(|_, cx| assert_painted(editor.read(cx), origin, &mixed, true));

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.replace_and_mark_text_in_range(None, "3", None, window, cx);
            assert!(editor.is_composing());
            assert_eq!(editor.source(), source.replacen("[1]", "[3]", 1));
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert_painted(editor.read(cx), origin, &[], true);
        editor.update(cx, |editor, cx| {
            editor.replace_text_in_range(None, "3", window, cx);
            assert!(!editor.is_composing());
        });
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}
