use gpui::{Modifiers, MouseButton, TestAppContext, Window, px, size};
use gpui_editor_codemirror::language_data::NativeLanguage;

use super::*;
use crate::{
    document::{BlockPreview, Section, SectionKind},
    editor::bind_keys,
};

const SQL: &str = "字\r\n\r\n```sql\r\nSELECT app.items FROM app.items;\r\n```\r\n\r\nAfter.";

fn activate_code(
    editor: &mut Editor,
    position: usize,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    let fence = editor
        .markdown()
        .sections()
        .into_iter()
        .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
        .unwrap()
        .source;
    editor.activate(
        Section::new(SectionKind::DraftCode, fence.clone(), fence),
        false,
        cx,
    );
    editor.move_to(position, false, cx);
    window.focus(&editor.focus);
}

#[gpui::test]
fn syntax_indent_keys_use_projected_source_lines_and_keep_bytes(cx: &mut TestAppContext) {
    for (language, marked, keys, expected) in [
        (
            "go",
            "func f() {§\r  println(\"猫😀\")\r\n}",
            "enter",
            "func f() {\r  \r  println(\"猫😀\")\r\n}",
        ),
        (
            "go",
            "func f() {\r  use(  §  )\r}",
            "enter",
            "func f() {\r  use(\r    \r  )\r}",
        ),
        (
            "go",
            "func f() {\r  pr§intln(\"猫😀\")\r  oth§er()\r\n}",
            "cmd-enter",
            "func f() {\r  println(\"猫😀\")\r  other()\r\n  \r\n}",
        ),
        (
            "json",
            "{\r§\"猫\": [\r\n   \"😀\"\r  ]\r§}",
            "cmd-alt-\\",
            "{\r  \"猫\": [\r\n    \"😀\"\r  ]\r}",
        ),
    ] {
        let prefix = format!("Before.\r\n\r\n```{language}\r\n");
        let suffix = "\r\n```\r\n\r\nAfter.";
        let mut source = prefix.clone();
        let mut positions = Vec::new();
        for ch in marked.chars() {
            if ch == '§' {
                positions.push(source.len());
            } else {
                source.push(ch);
            }
        }
        source.push_str(suffix);
        let (anchor, head) = (*positions.last().unwrap(), positions[0]);
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_code(editor, head, window, cx);
                editor.set_selection(anchor, head);
            });
        });
        cx.simulate_keystrokes(keys);
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(
                editor.source(),
                format!("{prefix}{expected}{suffix}"),
                "{keys}"
            );
            assert_eq!(editor.view.undo_depth(), 1);
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert_eq!((editor.anchor(), editor.cursor()), (anchor, head));
        });
    }
}

#[gpui::test]
fn indent_does_not_fall_back_to_a_newline_that_joins_raw_breaks(cx: &mut TestAppContext) {
    for (source, at, keys) in [
        (
            "```go\npackage p\rvar x = 1\nvar y = 2\n```",
            "var x",
            "enter",
        ),
        ("```go\npackage p\rvar x = 1\n```", "\n```", "cmd-enter"),
    ] {
        let caret = source.find(at).unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| activate_code(editor, caret, window, cx));
        });
        cx.simulate_keystrokes(keys);
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert_eq!(editor.cursor(), caret);
            assert!(!editor.view.can_undo());
        });
    }
}

#[gpui::test]
fn typing_a_go_closer_reindents_a_lone_cr_line_without_changing_the_fence(cx: &mut TestAppContext) {
    let source = "Before.\r\n\r\n```go\r\npackage p\rfunc f() {\r  println(\"名😀\")\r    \r\n```\r\n\r\nAfter.";
    let caret = source.find("    \r\n").unwrap() + 4;
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| activate_code(editor, caret, window, cx));
    });
    cx.simulate_input("}");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source.replace("    \r\n", "}\r\n"));
        assert_eq!(editor.cursor(), caret - 3);
        assert_eq!(editor.view.undo_depth(), 1);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.cursor(), caret);
    });
}

#[gpui::test]
fn source_line_keys_do_not_move_or_duplicate_the_fences(cx: &mut TestAppContext) {
    let source = "Before\r\n\r\n```js\r\nfirst\r\n字\r\n```\r\n\r\nAfter";
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, source.find("first").unwrap(), window, cx)
        });
    });
    cx.simulate_keystrokes("alt-up");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
    cx.simulate_keystrokes("alt-down");
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).source(),
            source.replace("first\r\n字", "字\r\nfirst")
        )
    });
    cx.simulate_keystrokes("alt-shift-down");
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).source(),
            source.replace("first\r\n字", "字\r\nfirst\r\nfirst")
        )
    });
    cx.simulate_keystrokes("secondary-z secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
}

#[gpui::test]
fn projected_line_keys_follow_display_rows_and_keep_raw_fence_bytes(cx: &mut TestAppContext) {
    let body = "甲\r😀x\r\né\n終";
    let cases = [
        ("cpp", "alt-shift-up", "甲\r😀x\r\n😀x\r\né\n終", false),
        ("java", "alt-down", "甲\ré\r\n😀x\n終", false),
        ("rust", "alt-up", "😀x\r甲\r\né\n終", false),
        ("go", "alt-up", "😀x\r甲\r\né\n終", false),
        ("json", "alt-up", "😀x\r甲\r\né\n終", false),
        ("lezer", "alt-down", "甲\ré\r\n😀x\n終", false),
        ("yaml", "alt-shift-up", "甲\r😀x\r\n😀x\r\né\n終", false),
        ("wat", "alt-shift-down", "甲\r😀x\r\n😀x\r\né\n終", true),
        ("wast", "alt-down", "甲\ré\r\n😀x\n終", false),
    ];
    for (language, key, expected, bottom_copy) in cases {
        let prefix = format!("周囲\r\n\r\n```{language}\r\n");
        let suffix = "\r\n```\r\n\r\nAfter.";
        let draft = format!("{prefix}{body}{suffix}");
        let updated = format!("{prefix}{expected}{suffix}");
        let start = prefix.len();
        let line = draft.find('😀').unwrap();
        let caret = line + '😀'.len_utf8();
        let target_line = if bottom_copy {
            updated.rfind('😀').unwrap()
        } else {
            updated.find('😀').unwrap()
        };
        let target = target_line + '😀'.len_utf8();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.clone(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| activate_code(editor, caret, window, cx));
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            assert_eq!(
                layout.row_edge(caret, false, false).unwrap().0,
                line,
                "{language}"
            );
        });
        cx.simulate_keystrokes(key);
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), updated, "{language}, {key}");
            assert_eq!(editor.cursor(), target);
            assert_eq!(
                editor.source_fence().unwrap().body,
                start..start + expected.len()
            );
            let layout = editor.layout.as_ref().unwrap();
            assert_eq!(
                layout.row_edge(target, false, false).unwrap().0,
                target_line
            );
            assert_eq!(editor.view.undo_depth(), 1);
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), draft);
            assert_eq!(editor.cursor(), caret);
            assert_eq!(
                editor.source_fence().unwrap().body,
                start..start + body.len()
            );
            assert!(!editor.view.can_undo());
        });
        cx.simulate_keystrokes("secondary-shift-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), updated);
            assert_eq!(editor.cursor(), target);
            assert_eq!(
                editor.source_fence().unwrap().body,
                start..start + expected.len()
            );
        });
    }
}

#[gpui::test]
fn projected_line_keys_keep_multiple_ranges_in_the_fence(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::SelectionRange;

    let draft = "```json\r\na\r😀b\r\nc\né\r界\r\nx\r\n```\r\n\r\nAfter.";
    let updated = "```json\r\na\rc\r\n😀b\n界\ré\r\nx\r\n```\r\n\r\nAfter.";
    let make_selection = |source: &str| {
        let emoji = source.find('😀').unwrap();
        let accent = source.find('é').unwrap();
        EditorSelection::new(
            vec![
                SelectionRange::new(emoji + '😀'.len_utf8(), emoji),
                SelectionRange::new(accent + 'é'.len_utf8(), accent + 'é'.len_utf8()),
            ],
            1,
        )
        .unwrap()
    };
    let original = make_selection(draft);
    let moved = make_selection(updated);
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, original.main().head(), window, cx);
            editor.view.set_selection(original.clone()).unwrap();
        });
    });
    cx.simulate_keystrokes("alt-down");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), updated);
        assert_eq!(editor.view.state().selection(), &moved);
        assert_eq!(editor.view.undo_depth(), 1);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), draft);
        assert_eq!(editor.view.state().selection(), &original);
        assert!(!editor.view.can_undo());
    });
    // Reject the whole command if any range is outside the editable body.
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            let outside =
                EditorSelection::new(vec![SelectionRange::new(0, 0), original.main()], 1).unwrap();
            editor.view.set_selection(outside).unwrap();
        });
    });
    cx.simulate_keystrokes("alt-shift-down");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), draft);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn projected_line_move_cannot_absorb_the_closing_fence_lf(cx: &mut TestAppContext) {
    // Moving the last line up leaves an empty line with a raw lone CR before
    // the separator of the closing fence. Only an LF suffix would absorb it.
    for ending in ["\n", "\r\n"] {
        let draft = format!("```yaml\n# a\n\rz: 1{ending}```\n\nAfter.");
        let caret = draft.find('z').unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.clone(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| activate_code(editor, caret, window, cx));
        });
        cx.simulate_keystrokes("alt-up");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            if ending == "\n" {
                assert_eq!(editor.source(), draft);
                assert!(!editor.view.can_undo());
            } else {
                assert_eq!(editor.source(), "```yaml\n# a\nz: 1\r\r\n```\n\nAfter.");
                let scope = editor.source_fence().unwrap().body;
                assert_eq!(&editor.source()[scope], "# a\nz: 1\r");
                assert_eq!(editor.view.undo_depth(), 1);
            }
        });
    }
}

#[gpui::test]
fn line_keys_without_projection_still_treat_lone_cr_as_content(cx: &mut TestAppContext) {
    let draft = "```js\r\na\rb\r\nc\r\n```\r\n";
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, draft.find('b').unwrap(), window, cx)
        });
    });
    cx.simulate_keystrokes("alt-down");
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).source(), "```js\r\nc\r\na\rb\r\n```\r\n");
    });
}

#[gpui::test]
fn copying_the_only_body_line_uses_the_fence_line_ending(cx: &mut TestAppContext) {
    let source = "```js\r\nfirst\r\n```\r\n";
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, source.find("first").unwrap(), window, cx)
        });
    });
    cx.simulate_keystrokes("alt-shift-down");
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).source(),
            "```js\r\nfirst\r\nfirst\r\n```\r\n"
        )
    });
}

#[gpui::test]
fn enter_uses_syntax_and_crlf_in_a_single_line_fence(cx: &mut TestAppContext) {
    let source = "```json\r\n{   }\r\n```\r\n";
    let start = source.find('{').unwrap();
    let at = start + 2;
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| activate_code(editor, at, window, cx));
    });
    cx.simulate_keystrokes("enter");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), "```json\r\n{\r\n  \r\n}\r\n```\r\n");
        assert_eq!(editor.cursor(), start + 5);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
}

#[gpui::test]
fn wast_and_lezer_enter_match_the_pinned_delimiter_gap_command(cx: &mut TestAppContext) {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        source: String,
        head: usize,
        after: String,
        position: usize,
    }
    let wast = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../codemirror/tests/fixtures/wast_newline.jsonl"
    ));
    let lezer = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../codemirror/tests/fixtures/lezer_state.jsonl"
    ));
    let cases = wast
        .lines()
        .flat_map(|line| [("wat", line), ("wast", line)])
        .chain(lezer.lines().map(|line| ("lezer", line)));
    const SUFFIX: &str = "\r\n```\r\n\r\nAfter 😀.";
    for (language, line) in cases {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        if row["kind"].as_str().is_some_and(|kind| kind != "enter") {
            continue;
        }
        let case: Case = serde_json::from_value(row).unwrap();
        let prefix = format!("前 😀\r\n\r\n```{language}\r\n");
        let source = format!("{prefix}{}{SUFFIX}", case.source);
        let head = prefix.len() + case.head;
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| activate_code(editor, head, window, cx));
        });
        cx.simulate_keystrokes("enter");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(
                editor.source(),
                format!("{prefix}{}{SUFFIX}", case.after),
                "{} ({language})",
                case.name
            );
            assert_eq!(
                editor.cursor(),
                prefix.len() + case.position,
                "{} ({language})",
                case.name
            );
            assert_eq!(editor.view.undo_depth(), 1);
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert_eq!(editor.cursor(), head);
            assert!(!editor.view.can_undo());
        });
    }
}

#[gpui::test]
fn blank_line_uses_end_of_source_line_and_keeps_the_fences(cx: &mut TestAppContext) {
    let source = "```js\r\nfunction f() {\r\n}\r\n```\r\n";
    let at = source.find("function").unwrap() + 3;
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| activate_code(editor, at, window, cx));
    });
    cx.simulate_keystrokes("cmd-enter");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source.replace("{\r\n", "{\r\n  \r\n"));
        assert_eq!(editor.cursor(), source.find('{').unwrap() + 5);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
}

#[gpui::test]
fn indent_shortcut_changes_only_selected_source_lines(cx: &mut TestAppContext) {
    let source = "```json\r\n{\r\n\"字\": 1\r\n}\r\n```\r\n";
    let start = source.find('{').unwrap();
    let end = source.find('}').unwrap() + 1;
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, start, window, cx);
            editor.set_selection(start, end);
        });
    });
    cx.simulate_keystrokes("cmd-alt-\\");
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).source(),
            source.replace("\"字\"", "  \"字\"")
        );
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
}

#[gpui::test]
fn toggle_comment_uses_the_native_fence_language_and_undo(cx: &mut TestAppContext) {
    for (language, line, commented) in [
        ("js", "const 字 = 1", "// const 字 = 1"),
        ("css", "p { color: red; }", "/* p { color: red; } */"),
        (
            "wat",
            "(module\r  (func $é)\r  (memory 1))",
            "(module\r  ;; (func $é)\r  (memory 1))",
        ),
        (
            "html",
            "<script>\r\nconst 字 = 1\r\n</script>",
            "<script>\r\n// const 字 = 1\r\n</script>",
        ),
        (
            "jinja",
            "<script>\r\nconst 字 = {{ count }};\r\n</script>",
            "<script>\r\n// const 字 = {{ count }};\r\n</script>",
        ),
    ] {
        let original = format!("Outside\r\n\r\n```{language}\r\n{line}\r\n```\r\n");
        let end = original.replace(line, commented);
        let start = original.find(line).unwrap()
            + line
                .find("const")
                .or_else(|| line.find("(func"))
                .unwrap_or(0);
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::new(original.clone(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_code(editor, start + 2, window, cx)
            });
        });
        cx.simulate_keystrokes("cmd-/");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), end));
        cx.simulate_keystrokes("cmd-/");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), original));
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), end));
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), original));
    }
}

#[gpui::test]
fn mounted_wast_metadata_does_not_make_a_fence_editable(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::{
        Assoc,
        commands::CommentTokens,
        language::markdown::{
            MarkdownCodeLanguages, MarkdownConfig, MarkdownParserConfig, markdown_with_parser,
        },
    };
    let source = "Before 😀\r\n\r\n```wat\r\n(module)\r\n```\r\n\r\nAfter.";
    let body_end = source.find("(module)").unwrap() + "(module)".len();
    let closing_fence = source.rfind("```").unwrap();
    let extension = markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Resolve(Arc::new(|name| {
                (name == "wat").then_some(NativeLanguage::Wast)
            }))),
            ..Default::default()
        },
    );
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source.into(), extension, window, cx).unwrap()
    });
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let tokens = editor.view.state().language_data_at::<CommentTokens>(
                "commentTokens",
                body_end,
                Assoc::After,
            );
            assert_eq!(tokens[0].line, Some(";;"));
            editor.set_selection(body_end, body_end);
            assert!(!editor.toggle_source_comment(cx));
            activate_code(editor, body_end - 1, window, cx);
            editor.set_selection(closing_fence, closing_fence);
            assert!(!editor.toggle_source_comment(cx));
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());
        })
    });
}

#[gpui::test]
fn parent_syntax_selection_respects_draft_and_composition_in_sql(cx: &mut TestAppContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(SQL.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            assert!(!editor.select_source_parent(cx));
            activate_code(editor, SQL.find("items").unwrap() + 1, window, cx);
        });
    });
    for expected in ["items", "app.items", "SELECT app.items FROM app.items;"] {
        cx.simulate_keystrokes("cmd-i");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(&editor.source()[editor.selection()], expected);
            assert_eq!(editor.source(), SQL);
            assert!(!editor.view.can_undo());
        });
    }
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(!editor.select_source_parent(cx));
            let at = SQL.find("items").unwrap() + 1;
            editor.marked = Some(at..at);
            editor.set_selection(at, at);
            assert!(!editor.select_source_parent(cx));
            assert_eq!(editor.cursor(), at);
            editor.finish_composition();
        });
    });
}

#[gpui::test]
fn parent_syntax_selection_expands_from_a_mount_to_its_host(cx: &mut TestAppContext) {
    let draft =
        "😀\n\n```html\n<div><script>const result = run(input);</script></div>\n```\n\nUntouched.";
    let position = draft.find("input").unwrap() + 2;
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| activate_code(editor, position, window, cx));
    });
    for expected in [
        "input",
        "(input)",
        "run(input)",
        "const result = run(input);",
        "<script>const result = run(input);</script>",
        "<div><script>const result = run(input);</script></div>",
    ] {
        cx.simulate_keystrokes("cmd-i");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(&editor.source()[editor.selection()], expected);
            assert!(editor.view.state().selection().main().undirectional());
            assert_eq!(editor.source(), draft);
            assert!(!editor.view.can_undo());
        });
    }
    cx.update(|_, cx| editor.update(cx, |editor, cx| assert!(!editor.select_source_parent(cx))));
}

#[gpui::test]
fn bidi_source_motion_and_pointer_hits_use_the_painted_visual_stops(cx: &mut TestAppContext) {
    let draft = "```txt\r\nab אב cd\r\nnext\r\n```\r\n\r\nOutside.";
    let line = draft.find("ab ").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| activate_code(editor, line + 3, window, cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.layout.as_mut().unwrap().set_bidi_test_geometry(
                line,
                px(80.),
                [0, 1, 2, 5, 3, 7, 8, 9]
                    .into_iter()
                    .enumerate()
                    .map(|(x, byte)| (byte, px(x as f32 * 10.))),
            );
            let caret = |editor: &Editor| {
                let selection = editor.view.state().selection().main();
                editor
                    .layout
                    .as_ref()
                    .unwrap()
                    .bounds_at_selection(selection, editor.upstream)
                    .unwrap()
            };
            let before = caret(editor);
            editor.move_horizontal(true, false, cx);
            assert_eq!(editor.cursor(), line + 5);
            assert_eq!(editor.view.state().selection().main().bidi_level(), Some(1));
            let middle = caret(editor);
            editor.move_horizontal(true, false, cx);
            assert_eq!(editor.cursor(), line + 7);
            let end = caret(editor);
            assert!(before.left() < middle.left() && middle.left() < end.left());
            // The two visual ends of this RTL group share one document byte.
            assert!(editor.run_source_command(SourceCommand::CursorGroupLeft, cx));
            assert_eq!(editor.cursor(), line + 7);
            assert_eq!(editor.view.state().selection().main().bidi_level(), Some(1));
            assert_eq!(caret(editor).left(), before.left());
            assert!(editor.run_source_command(SourceCommand::CursorGroupLeft, cx));
            assert_eq!(editor.cursor(), line);
            assert!(editor.run_source_command(SourceCommand::CursorGroupLeft, cx));
            assert_eq!(editor.cursor(), line, "motion stays inside the fence body");
            editor.mouse_down(
                &gpui::MouseDownEvent {
                    button: MouseButton::Left,
                    position: gpui::point(end.left() - px(0.5), end.center().y),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );
            assert_eq!(editor.cursor(), line + 3);
            assert_eq!(editor.view.state().selection().main().bidi_level(), Some(1));
            assert_eq!(caret(editor).left(), end.left());
            let item = editor
                .layout
                .as_ref()
                .unwrap()
                .item_at(middle.center())
                .unwrap();
            assert_eq!(item.x_spans(line + 1..line + 5).count(), 2);
            editor.move_horizontal(false, true, cx);
            assert_eq!(editor.anchor(), line + 3);
            assert_eq!(editor.cursor(), line + 5);
            editor.move_to(line + "ab אב cd".len(), false, cx);
            editor.move_horizontal(true, false, cx);
            assert_eq!(editor.cursor(), draft.find("next").unwrap());
            assert_eq!(editor.source(), draft);
            assert!(!editor.view.can_undo());
        });
    });
}

#[gpui::test]
fn vertical_motion_preserves_the_measured_bidi_column(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::Assoc;

    let draft = "```txt\r\nab אב cd\r\nab אב cd\r\n```";
    let first = draft.find("ab ").unwrap();
    let second = draft.rfind("ab ").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            activate_code(editor, first + 3, window, cx)
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let initial = EditorSelection::cursor(first + 3, Some(Assoc::After), Some(1), None);
            editor
                .view
                .set_selection(EditorSelection::new(vec![initial], 0).unwrap())
                .unwrap();
            let layout = editor.layout.as_mut().unwrap();
            for line in [first, second] {
                layout.set_bidi_test_geometry(
                    line,
                    px(80.),
                    [0, 1, 2, 5, 3, 7, 8, 9]
                        .into_iter()
                        .enumerate()
                        .map(|(x, byte)| (byte, px(x as f32 * 10.))),
                );
            }
            let x = layout.bounds_at_selection(initial, false).unwrap().left();
            editor.move_vertical(true, false, cx);
            let main = editor.view.state().selection().main();
            assert_eq!(main.head(), second + 7);
            assert_eq!(main.bidi_level(), Some(0));
            let next = editor
                .layout
                .as_ref()
                .unwrap()
                .bounds_at_selection(main, editor.upstream)
                .unwrap();
            assert_eq!(next.left(), x);
            assert!(main.goal_column().is_some());
            assert_eq!(editor.source(), draft);
        });
    });
}

#[gpui::test]
fn horizontal_source_commands_keep_all_ranges(cx: &mut TestAppContext) {
    let draft = "```txt\r\na🦀c\r\nxyz\r\n```\r\nOutside.";
    let first = draft.find("a🦀").unwrap() + 1;
    let second = draft.find("xyz").unwrap() + 1;
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, first, window, cx);
            let selection = EditorSelection::new(
                vec![
                    EditorSelection::cursor(first, None, None, None),
                    EditorSelection::cursor(second, None, None, None),
                ],
                1,
            )
            .unwrap();
            editor.view.set_selection(selection).unwrap();
        });
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("right");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let selection = editor.view.state().selection();
        assert_eq!(selection.main_index(), 1);
        assert_eq!(
            selection
                .ranges()
                .iter()
                .map(|range| range.head())
                .collect::<Vec<_>>(),
            [first + 4, second + 1]
        );
    });
    cx.simulate_keystrokes("shift-left");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let selection = editor.view.state().selection();
        assert_eq!(selection.main_index(), 1);
        assert_eq!(
            selection
                .ranges()
                .iter()
                .map(|range| range.range())
                .collect::<Vec<_>>(),
            [first..first + 4, second..second + 1]
        );
        assert_eq!(editor.source(), draft);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn group_keys_use_source_word_characters_for_each_selection(cx: &mut TestAppContext) {
    let draft = "```js\r\nfoo$bar = 1;\r\nδ$end = 2;\r\n```";
    let first = draft.find("foo$bar").unwrap();
    let second = draft.find("δ$end").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, second, window, cx);
            editor
                .view
                .set_selection(
                    EditorSelection::new(
                        vec![
                            EditorSelection::cursor(first, None, None, None),
                            EditorSelection::cursor(second, None, None, None),
                        ],
                        1,
                    )
                    .unwrap(),
                )
                .unwrap();
        });
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("alt-shift-right");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let selection = editor.view.state().selection();
        assert_eq!(selection.main_index(), 1);
        assert_eq!(
            selection
                .ranges()
                .iter()
                .map(|range| &draft[range.range()])
                .collect::<Vec<_>>(),
            ["foo$bar", "δ$end"]
        );
        assert_eq!(editor.source(), draft);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn row_boundary_and_select_line_keys_use_the_active_fence_layout(cx: &mut TestAppContext) {
    let draft = format!("```txt\r\n    {}last\r\nnext\r\n```", "abcd ".repeat(30));
    let head = draft.find("last").unwrap() - 2;
    let first = draft.find("    abcd").unwrap();
    let second = draft.find("next").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.clone(), window, cx));
    cx.simulate_resize(size(px(270.), px(540.)));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| activate_code(editor, head, window, cx));
    });
    cx.run_until_parked();
    let (start, first_end) = cx.update(|_, cx| {
        let layout = editor.read(cx).layout.as_ref().unwrap();
        (
            layout.row_edge(head, false, false).unwrap().0,
            layout.row_edge(first, false, true).unwrap().0,
        )
    });
    assert!(first + 4 < start && start < head);
    cx.simulate_keystrokes("cmd-left");
    cx.update(|_, cx| assert_eq!(editor.read(cx).cursor(), start));
    cx.simulate_keystrokes("cmd-left");
    cx.update(|_, cx| assert_eq!(editor.read(cx).cursor(), first + 4));
    cx.simulate_keystrokes("cmd-left");
    cx.update(|_, cx| assert_eq!(editor.read(cx).cursor(), first));
    cx.simulate_keystrokes("cmd-shift-right");
    cx.update(|_, cx| assert_eq!(editor.read(cx).selection(), first..first_end));
    cx.simulate_keystrokes("ctrl-l");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.selection(), first..second);
        assert!(editor.view.state().selection().main().undirectional());
        assert_eq!(editor.source(), draft);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn select_and_indent_follow_the_displayed_lone_cr_lines(cx: &mut TestAppContext) {
    let draft = "```json\r\n{\r\n  \"猫\": true,\r  \"later\": false\r\n}\r\n```";
    let first = draft.find("  \"猫\"").unwrap();
    let second = draft.find("  \"later\"").unwrap();
    let caret = draft.find('猫').unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| activate_code(editor, caret, window, cx));
    });
    cx.simulate_keystrokes("ctrl-l");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.selection(), first..second);
        assert!(!editor.view.can_undo());
    });
    cx.simulate_keystrokes("cmd-]");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(
            editor.source(),
            format!("{}  {}", &draft[..first], &draft[first..])
        );
        assert_eq!(editor.view.undo_depth(), 1);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), draft);
        assert_eq!(editor.selection(), first..second);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn indent_keys_use_the_configured_unit_and_preserve_crlf(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::language::{INDENT_UNIT, IndentStyle};

    let draft = "```txt\r\n猫\r\nnext\r\n```";
    let first = draft.find('猫').unwrap();
    let next = draft.find("next").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, first, window, cx);
            let style = INDENT_UNIT.of(IndentStyle::parse("\t").unwrap());
            let transaction = Transaction::default().with_effects([style.append()]);
            let active = editor.active.clone();
            editor
                .view
                .update(transaction, active.clone(), |_| (None, active))
                .unwrap();
            editor.set_selection(first, next);
        });
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-]");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "```txt\r\n\t猫\r\nnext\r\n```"));
    cx.simulate_keystrokes("cmd-[");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), draft));
    cx.simulate_keystrokes("secondary-z secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), draft);
        assert_eq!(editor.selection(), first..next);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn delete_to_visual_row_edges_stays_inside_the_fence(cx: &mut TestAppContext) {
    let draft = format!("```txt\r\n{}last\r\nnext\r\n```", "abcd ".repeat(30));
    let head = draft.find("last").unwrap() - 2;
    let body = draft.find("abcd").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.clone(), window, cx));
    cx.simulate_resize(size(px(270.), px(540.)));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| activate_code(editor, head, window, cx));
    });
    cx.run_until_parked();
    let start = cx.update(|_, cx| {
        let layout = editor.read(cx).layout.as_ref().unwrap();
        layout.row_edge(head, false, false).unwrap().0
    });
    assert!(body < start && start < head);
    cx.simulate_keystrokes("cmd-backspace");
    let mut expected = draft.clone();
    expected.replace_range(start..head, "");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), expected);
        assert_eq!(editor.cursor(), start);
    });
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    let end = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), draft);
        assert_eq!(editor.cursor(), head);
        editor
            .layout
            .as_ref()
            .unwrap()
            .row_edge(head, false, true)
            .unwrap()
            .0
    });
    cx.simulate_keystrokes("cmd-delete");
    expected = draft.clone();
    assert!(end > head);
    expected.replace_range(head..end, "");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), expected));
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| editor.update(cx, |editor, cx| editor.move_to(body, false, cx)));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-backspace");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), draft);
        assert_eq!(editor.cursor(), body);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn native_vertical_commands_keep_multiple_carets_and_their_columns(cx: &mut TestAppContext) {
    let draft = "```txt\r\nabcdefgh\r\nX\r\n12345678\r\nabcdefgh\r\n```\r\n\r\nOutside.";
    let start = draft.find("abcdefgh").unwrap() + 4;
    let short = draft.find('X').unwrap() + 1;
    let third = draft.find("12345678").unwrap() + 4;
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| activate_code(editor, start, window, cx));
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-alt-down");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let selection = editor.view.state().selection();
        assert_eq!(
            selection
                .ranges()
                .iter()
                .map(|range| range.head())
                .collect::<Vec<_>>(),
            [start, short]
        );
        assert_eq!(selection.main().head(), short);
        assert!(selection.main().goal_column().is_some());
        assert_eq!(editor.source(), draft);
    });
    cx.simulate_keystrokes("down");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let selection = editor.view.state().selection();
        assert_eq!(
            selection
                .ranges()
                .iter()
                .map(|range| range.head())
                .collect::<Vec<_>>(),
            [short, third]
        );
        assert_eq!(selection.main().head(), third);
        assert_eq!(editor.source(), draft);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn native_page_command_preserves_caret_height_and_fence_scope(cx: &mut TestAppContext) {
    let body: String = (0..80)
        .map(|line| format!("line {line:02} abcdefgh\n"))
        .collect();
    let draft = format!("```txt\n{body}```\n\nOutside.");
    let start = draft.find("line 03").unwrap() + 2;
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.clone(), window, cx));
    cx.simulate_resize(size(px(700.), px(380.)));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| activate_code(editor, start, window, cx));
    });
    cx.run_until_parked();
    let before = cx.update(|_, cx| {
        let editor = editor.read(cx);
        let caret = editor
            .layout
            .as_ref()
            .unwrap()
            .bounds_at(start, false)
            .unwrap();
        caret.top() - editor.scroll.bounds().top()
    });
    assert!(before > px(0.));
    cx.simulate_keystrokes("pagedown");
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let head = cx.update(|_, cx| {
        let editor = editor.read(cx);
        let head = editor.cursor();
        assert!(head > draft.find("line 08").unwrap());
        assert!(head < draft.find("Outside").unwrap());
        let caret = editor
            .layout
            .as_ref()
            .unwrap()
            .bounds_at(head, false)
            .unwrap();
        let after = caret.top() - editor.scroll.bounds().top();
        assert!(
            (after - before).abs() < px(4.),
            "before {before:?}; after {after:?}"
        );
        head
    });
    cx.simulate_keystrokes("shift-pageup");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.anchor(), head);
        assert!(editor.cursor() < head);
        assert!(editor.cursor() >= editor.source_fence().unwrap().body.start);
        assert_eq!(editor.source(), draft);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn option_drag_selects_one_column_range_per_source_line(cx: &mut TestAppContext) {
    let draft = "```txt\r\nabcdefgh\r\nZ\r\nabcdefgh\r\n```\r\n\r\nOutside.";
    let first = draft.find("abcdefgh").unwrap();
    let middle = draft.find('Z').unwrap() + 1;
    let last = draft.rfind("abcdefgh").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, first + 2, window, cx)
        });
    });
    cx.run_until_parked();
    let (start, end) = cx.update(|_, cx| {
        let layout = editor.read(cx).layout.as_ref().unwrap();
        let at = |offset| layout.bounds_at(offset, false).unwrap().center();
        (at(first + 2), at(last + 4))
    });
    let alt = Modifiers {
        alt: true,
        ..Default::default()
    };
    cx.simulate_mouse_down(start, MouseButton::Left, alt);
    cx.simulate_mouse_move(end, MouseButton::Left, alt);
    cx.simulate_mouse_up(end, MouseButton::Left, alt);
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let ranges: Vec<_> = editor
            .view
            .state()
            .selection()
            .ranges()
            .iter()
            .map(|range| range.range())
            .collect();
        assert_eq!(
            ranges,
            [first + 2..first + 4, middle..middle, last + 2..last + 4]
        );
        assert!(editor.rectangle.is_none());
        assert_eq!(editor.source(), draft);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn typing_reindents_a_mounted_language_in_one_undo_step(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;

    let source = "Before.\r\n\r\n```html\r\n<script>\r\nif (ok) {\r\n      \r\n</script>\r\n```\r\n\r\nAfter.";
    let expected = source.replace("      \r\n", "}\r\n");
    let start = source.find("      ").unwrap() + 6;
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, start, window, cx);
            editor.replace_text_in_range(None, "}", window, cx);
            assert_eq!(editor.source(), expected);
            assert_eq!(editor.cursor(), start - 5);
        });
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.cursor(), start);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn typing_a_closer_reindents_a_lone_cr_line_without_changing_the_fence(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;

    for (language, body) in [
        ("c", "void f() {\r\n  call(\"名😀\");\r    "),
        ("cpp", "void f() {\r\n  call(\"名😀\");\r    "),
        ("java", "class A {\r\n  String n = \"名😀\";\r    "),
        ("rust", "fn f() {\r\n  call(\"名😀\");\r    "),
        ("json", "{\r\n  \"名😀\": true\r    "),
        ("lezer", "@top Main {\r\n  Word\r    "),
        ("yaml", "root: {\r\n  v: \"名😀\"\r    "),
    ] {
        let source = format!("Before.\r\n\r\n```{language}\r\n{body}\r\n```\r\n\r\nAfter.");
        let expected = source.replace("\r    \r\n", "\r}\r\n");
        let start = source.find("    ").unwrap() + 4;
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_code(editor, start, window, cx);
                editor.replace_text_in_range(None, "}", window, cx);
                assert_eq!(editor.source(), expected, "{language}");
                assert_eq!(editor.cursor(), start - 3);
            });
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert_eq!(editor.cursor(), start);
            assert!(!editor.view.can_undo());
        });
    }
}

#[gpui::test]
fn source_syntax_reuses_the_active_draft_after_typing_and_undo(cx: &mut TestAppContext) {
    let input = format!(
        "```json\r\n[\r\n{}  1\r\n]\r\n```\r\nOutside.",
        "  [\"字\", true],\r\n".repeat(40)
    );
    let parser = gpui_editor_codemirror::language::markdown::MarkdownCodeParser::Native(
        NativeLanguage::Json,
    );
    let position = input.rfind('1').unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(input.clone(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, position, window, cx);
            let scope = editor.source_fence().unwrap().body;
            let document = editor
                .source_document(&scope, &parser, StreamSettings::default())
                .unwrap();
            assert_eq!(document.reuse_stats().nodes_reused, 0);
            let settings = StreamSettings::default();
            assert!(
                editor
                    .current_source_document(&scope, &parser, settings)
                    .is_some()
            );
            // This same-size edit invalidates the borrowed snapshot without
            // losing the prefix seed used to refresh it.
            assert!(editor.replace(position..position + 1, "2", cx));
            let scope = editor.source_fence().unwrap().body;
            assert!(
                editor
                    .current_source_document(&scope, &parser, settings)
                    .is_none()
            );
            let document = editor
                .source_document(&scope, &parser, StreamSettings::default())
                .unwrap();
            assert!(document.reuse_stats().nodes_reused > 100);
            assert!(document.text().as_str().ends_with("  2\r\n]"));
            assert_eq!(
                editor
                    .current_source_document(&scope, &parser, settings)
                    .unwrap()
                    .text()
                    .as_str(),
                editor.source().get(scope).unwrap()
            );
        });
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert_eq!(editor.source(), &input);
            let scope = editor.source_fence().unwrap().body;
            let document = editor
                .source_document(&scope, &parser, StreamSettings::default())
                .unwrap();
            assert!(document.reuse_stats().nodes_reused > 100);
            assert!(document.text().as_str().ends_with("  1\r\n]"));
        })
    });
}

#[gpui::test]
fn parent_syntax_selection_expands_all_ranges_in_one_draft(cx: &mut TestAppContext) {
    let draft = "```json\n{\"hi\": true, \"name\": \"Ada\"}\n```";
    let flag = draft.find("true").unwrap();
    let name = draft.find("Ada").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(draft.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_code(editor, flag + 2, window, cx);
            let selection = EditorSelection::new(
                vec![
                    EditorSelection::cursor(flag + 2, None, None, None),
                    EditorSelection::cursor(name + 1, None, None, None),
                ],
                1,
            )
            .unwrap();
            editor.view.set_selection(selection).unwrap();
        });
    });
    cx.simulate_keystrokes("cmd-i");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let selection = editor.view.state().selection();
        assert_eq!(selection.main_index(), 1);
        assert_eq!(
            selection
                .ranges()
                .iter()
                .map(|range| &editor.source()[range.range()])
                .collect::<Vec<_>>(),
            ["true", "\"Ada\""]
        );
        assert_eq!(editor.source(), draft);
        assert!(!editor.view.can_undo());
    });
}
