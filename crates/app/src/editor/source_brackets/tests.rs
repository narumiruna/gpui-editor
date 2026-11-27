use super::*;
use crate::{
    document::{BlockPreview, Section},
    editor::{bind_keys, to_utf16},
};
use gpui::{EntityInputHandler, TestAppContext};
use gpui_editor_codemirror::language::wast::WastDocument;

mod config;
mod html;
mod lezer;
mod multiple;
mod python;
mod quotes;
mod render;
mod xml;

type Hit = ([usize; 2], Option<[usize; 2]>, bool);

fn wast_bracket_near(text: &Text, scope: Range<usize>, head: usize) -> Option<BracketMatch> {
    grammar_bracket_near(text, &scope, head, "()[]{}", |head| {
        WastDocument::parse(text.slice_owned(scope.clone())?).bracket_near(head)
    })
}

#[gpui::test]
fn configured_wast_close_options_use_live_mounted_data(cx: &mut TestAppContext) {
    use gpui_editor_codemirror::{
        Compartment, Extension,
        language::{
            LanguageData,
            markdown::{
                MarkdownCodeLanguages, MarkdownConfig, MarkdownParserConfig, markdown_with_parser,
            },
        },
        language_data::wast_language,
    };
    use std::sync::Arc;
    let source = "```wat\r\n(func $a)\r\n```\r\n";
    let parser = wast_language().configure(None, Some("configured"));
    let data = Compartment::new();
    let installed = data.of(parser.data(
        LanguageData::default()
            .with(
                "closeBrackets",
                CloseBracketData {
                    brackets: Some(&["["]),
                    before: Some("!"),
                    string_prefixes: &[],
                },
            )
            .with("wordChars", "-"),
    ));
    let markdown = markdown_with_parser(
        MarkdownConfig::default(),
        MarkdownParserConfig {
            code_languages: Some(MarkdownCodeLanguages::Configured {
                resolve: Arc::new(move |name| {
                    (name == "wat").then(|| MarkdownCodeParser::Wast(parser.clone()))
                }),
                fallback: None,
            }),
            ..Default::default()
        },
    );
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.into(),
            Extension::group([markdown, installed]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            activate_first_code_preview(editor, cx);
            let at = source.find("$a").unwrap();
            editor.set_selection(at, at);
            let (scope, parser) = editor.bracket_scope().unwrap();
            let (options, syntax) = editor.source_bracket_context(&scope, parser.as_ref());
            assert!(matches!(
                syntax.as_ref().map(SupportedDocument::syntax),
                Some(NativeSyntax::Wast(_))
            ));
            assert_eq!(options.brackets, ["["]);
            assert_eq!(options.before, "!");
            assert_eq!(options.word_chars, "-");
            crate::editor::one_dark_tests::configure(
                editor,
                data.reconfigure(Extension::default()),
                cx,
            );
            let (options, _) = editor.source_bracket_context(&scope, parser.as_ref());
            assert_ne!(options.brackets, ["["]);
            assert_ne!(options.before, "!");
            assert_ne!(options.word_chars, "-");
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.revision(), 0);
            assert_eq!(editor.view.undo_depth(), 0);
        })
    });
}

#[gpui::test]
fn css_bracket_feedback_uses_the_active_stylesheet(cx: &mut TestAppContext) {
    const PREFIX: &str = "前 😀\r\n\r\n```css\r\n";
    let body = r#"a { --v: f(")"); }"#;
    let source = format!("{PREFIX}{body}\r\n```\r\n\r\nOutside )");
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
    cx.update(|window, cx| {
        let open = PREFIX.len() + body.find('(').unwrap();
        let close = PREFIX.len() + body.rfind(')').unwrap();
        editor.update(cx, |editor, cx| {
            activate_first_code_preview(editor, cx);
            editor.set_selection(open + 1, open + 1);
            window.focus(&editor.focus);
            assert_eq!(
                editor.source_bracket_match(),
                Some(BracketMatch {
                    start: open..open + 1,
                    end: Some(close..close + 1),
                    matched: true,
                })
            );
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(
            editor
                .read(cx)
                .layout
                .as_ref()
                .unwrap()
                .painted_bracket_quads
                .len(),
            2
        )
    });
}

#[test]
fn wast_brackets_match_the_pinned_oracle() {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        source: String,
        repeat: Option<(String, usize)>,
        checks: Vec<(usize, Option<Hit>)>,
    }

    for row in include_str!("wast_brackets.jsonl").lines() {
        let case: Case = serde_json::from_str(row).unwrap();
        let source = match case.repeat {
            Some((unit, count)) => case.source.replacen('$', &unit.repeat(count), 1),
            None => case.source,
        };
        let text = Text::new(source);
        for (head, expected) in case.checks {
            let range = |[from, to]: [usize; 2]| from..to;
            let expected = expected.map(|(start, end, matched)| BracketMatch {
                start: range(start),
                end: end.map(range),
                matched,
            });
            assert_eq!(
                wast_bracket_near(&text, 0..text.len(), head),
                expected,
                "{} at byte {head}",
                case.name
            );
        }
    }
}

#[test]
fn wast_bracket_bounds_keep_byte_edges_and_scope() {
    let text = Text::new(")字\r\n((");
    let scope = 1..text.len() - 1;
    for head in [0, 1, 2, 5, text.len()] {
        assert_eq!(wast_bracket_near(&text, scope.clone(), head), None);
    }
    let inside = wast_bracket_near(&text, scope.clone(), scope.end).unwrap();
    assert_eq!(inside.start, scope.end - 1..scope.end);
    assert_eq!(inside.end, None);
}

#[gpui::test]
fn wast_bracket_feedback_uses_the_current_fence_body(cx: &mut TestAppContext) {
    for language in ["wat", "wast"] {
        let prefix = format!("字\r\n\r\n```{language}\r\n");
        let body = ";; 😀 )\r\n(module (data (i32.const 0) \"字 )\"))";
        let suffix = "\r\n```\r\n\r\nOutside )";
        let source = format!("{prefix}{body}{suffix}");
        let open = prefix.len() + body.find("(module").unwrap();
        let close = prefix.len() + body.len() - 1;
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                editor.set_selection(open + 1, open + 1);
                assert!(editor.source_bracket_match().is_none());
                let range = editor
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                    .unwrap()
                    .source;
                editor.activate(
                    Section::new(SectionKind::DraftCode, range.clone(), range),
                    false,
                    cx,
                );
                editor.move_to(open + 1, false, cx);
                assert_eq!(
                    editor.source_bracket_match(),
                    Some(BracketMatch {
                        start: open..open + 1,
                        end: Some(close..close + 1),
                        matched: true,
                    })
                );
                editor.move_to(open, true, cx);
                assert!(editor.source_bracket_match().is_none());
                editor.move_to(close + 1, false, cx);
                assert_eq!(
                    editor.source_bracket_match(),
                    Some(BracketMatch {
                        start: close..close + 1,
                        end: Some(open..open + 1),
                        matched: true,
                    })
                );
                window.focus(&editor.focus);
            });
        });
        cx.simulate_keystrokes("backspace");
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                assert_eq!(editor.source(), format!("{}{suffix}", &source[..close]));
                editor.move_to(open + 1, false, cx);
                assert_eq!(
                    editor.source_bracket_match(),
                    Some(BracketMatch {
                        start: open..open + 1,
                        end: None,
                        matched: false,
                    })
                );
            });
        });
        cx.simulate_keystrokes("cmd-z");
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                assert_eq!(editor.source(), source);
                editor.move_to(open + 1, false, cx);
                assert_eq!(
                    editor.source_bracket_match().unwrap().end,
                    Some(close..close + 1)
                );
                editor.replace_and_mark_text_in_range(None, "(", None, window, cx);
                assert!(editor.is_composing());
                assert!(editor.source_bracket_match().is_none());
                assert_eq!(
                    editor.source(),
                    format!("{}({}", &source[..open + 1], &source[open + 1..])
                );
            });
        });
    }
}

#[gpui::test]
fn wast_quote_input_matches_the_pinned_oracle(cx: &mut TestAppContext) {
    #[derive(serde::Deserialize)]
    struct Snapshot {
        body: String,
        selection: [usize; 2],
        hit: Option<Hit>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        before: Snapshot,
        after: Vec<Snapshot>,
    }

    cx.update(bind_keys);
    for row in include_str!("wast_quotes.jsonl").lines() {
        let case: Case = serde_json::from_str(row).unwrap();
        for language in ["wat", "wast"] {
            let prefix = format!("字 😀\r\n\r\n```{language}\r\n");
            let suffix = "\r\n```\r\n\r\nOutside \" )";
            let start = prefix.len();
            let source = format!("{prefix}{}{suffix}", case.before.body);
            let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source, window, cx));
            let check = |editor: &Editor, expected: &Snapshot| {
                let name = &case.name;
                assert_eq!(
                    editor.source(),
                    format!("{prefix}{}{suffix}", expected.body),
                    "{language}: {name} text"
                );
                assert_eq!(
                    editor.source_fence().unwrap().body,
                    start..start + expected.body.len(),
                    "{language}: {name} scope"
                );
                assert_eq!(
                    [editor.anchor(), editor.cursor()],
                    expected.selection.map(|pos| start + pos),
                    "{language}: {name} selection"
                );
                let range = |[from, to]: [usize; 2]| start + from..start + to;
                assert_eq!(
                    editor.source_bracket_match(),
                    expected.hit.map(|(from, to, matched)| BracketMatch {
                        start: range(from),
                        end: to.map(range),
                        matched,
                    }),
                    "{language}: {name} hit"
                );
            };
            cx.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    activate_first_code_preview(editor, cx);
                    let [anchor, head] = case.before.selection.map(|pos| start + pos);
                    editor.set_selection(anchor, head);
                    window.focus(&editor.focus);
                });
                check(editor.read(cx), &case.before);
            });
            for step in &case.after {
                cx.simulate_input("\"");
                cx.update(|_, cx| check(editor.read(cx), step));
            }
            cx.simulate_keystrokes("cmd-z");
            cx.update(|_, cx| {
                let editor = editor.read(cx);
                check(editor, &case.before);
                assert!(!editor.view.can_undo(), "{language}: {} undo", case.name);
            });
        }
    }
}

#[gpui::test]
fn wast_quote_input_preserves_ime_and_protected_markdown(cx: &mut TestAppContext) {
    let source = "字 😀\r\n\r\n```wat\r\n(data \"字\")\r\n```\r\n\r\n```wast\r\n(func)\r\n```";
    let position = source.find('"').unwrap();
    let protected = source.find("(func)").unwrap();
    let composed = format!("{}\"{}", &source[..position], &source[position..]);
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_first_code_preview(editor, cx);
            editor.move_to(position, false, cx);
            window.focus(&editor.focus);
            let protected = to_utf16(source, protected..protected + 1);
            editor.replace_text_in_range(Some(protected), "\"", window, cx);
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());
            editor.replace_and_mark_text_in_range(None, "\"", None, window, cx);
            assert!(editor.is_composing());
            assert!(editor.source_bracket_match().is_none());
            assert_eq!(editor.source(), composed);
            editor.replace_text_in_range(None, "\"", window, cx);
            assert!(!editor.is_composing());
            assert_eq!(editor.source(), composed);
        });
    });
    cx.simulate_keystrokes("cmd-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}

fn activate_first_code_preview(editor: &mut Editor, cx: &mut Context<Editor>) {
    let range = editor
        .markdown()
        .sections()
        .into_iter()
        .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
        .unwrap()
        .source;
    editor.activate(
        Section::new(SectionKind::DraftCode, range.clone(), range),
        false,
        cx,
    );
}

#[gpui::test]
fn json_brackets_use_language_data_and_do_not_cross_syntax_classes(cx: &mut TestAppContext) {
    let body = r#"{"value":"abc    (字) }","nested":[0]}"#;
    let source = format!("```json\r\n{body}\r\n```\r\n\r\nOutside.");
    let initial = source.clone();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(initial, window, cx));
    let start = "```json\r\n".len();
    let inside = start + body.find("    ").unwrap() + 1;
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            let range = editor
                .markdown()
                .sections()
                .into_iter()
                .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                .unwrap()
                .source;
            editor.activate(
                Section::new(SectionKind::DraftCode, range.clone(), range),
                false,
                cx,
            );
            editor.move_to(start + 1, false, cx);
            let hit = editor.source_bracket_match().unwrap();
            let end = start + body.len();
            assert!(hit.matched);
            assert_eq!(hit.start, start..start + 1);
            assert_eq!(hit.end, Some(end - 1..end));
            let open = start + body.find('(').unwrap();
            let close = start + body.find(')').unwrap();
            editor.move_to(open, false, cx);
            assert_eq!(
                editor.source_bracket_match(),
                Some(BracketMatch {
                    start: open..open + 1,
                    end: Some(close..close + 1),
                    matched: true,
                })
            );
            editor.move_to(inside, false, cx);
            window.focus(&editor.focus);
        });
    });
    cx.simulate_input("\"");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(
            editor.source(),
            format!("{}\"{}", &source[..inside], &source[inside..])
        );
    });
    cx.simulate_keystrokes("cmd-z");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.move_to(start + body.len(), false, cx)
        })
    });
    cx.simulate_input("(");
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).source(),
            source.replacen(body, &format!("{body}("), 1)
        )
    });
    cx.simulate_keystrokes("cmd-z");
    cx.simulate_input("[");
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).source(),
            source.replacen(body, &format!("{body}[]"), 1)
        )
    });
    cx.simulate_keystrokes("backspace");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
}

#[gpui::test]
fn close_bracket_settings_follow_the_mounted_language(cx: &mut TestAppContext) {
    for (language, body, before, input, inserted) in [
        (
            "html",
            "<script>const s = </script>",
            "const s = ",
            "`",
            "``",
        ),
        (
            "jinja",
            "{% if visible %}<p> </p>{% endif %}",
            "<p>",
            "(",
            "(",
        ),
        (
            "liquid",
            "{% if visible %}<p> </p>{% endif %}",
            "<p>",
            "{",
            "{}",
        ),
    ] {
        let source = format!("字\r\n\r\n```{language}\r\n{body}\r\n```\r\n\r\nOutside.");
        let position = source.find(before).unwrap() + before.len();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                let range = editor
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                    .unwrap()
                    .source;
                editor.activate(
                    Section::new(SectionKind::DraftCode, range.clone(), range),
                    false,
                    cx,
                );
                editor.move_to(position, false, cx);
                window.focus(&editor.focus);
            });
        });
        cx.simulate_input(input);
        let mut expected = source.clone();
        expected.insert_str(position, inserted);
        cx.update(|_, cx| {
            assert_eq!(editor.read(cx).source(), expected);
            assert_eq!(editor.read(cx).cursor(), position + 1);
        });
        cx.simulate_keystrokes("cmd-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), source));
    }
}

#[gpui::test]
fn template_percent_brace_is_one_input_edit_inside_the_draft(cx: &mut TestAppContext) {
    for language in ["jinja", "liquid", "text"] {
        let draft = format!("字\r\n\r\n```{language}\r\n<p>{{}}</p>\r\n```\r\n\r\nOutside.");
        let position = draft.find("{}").unwrap() + 1;
        let initial = draft.clone();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(initial, window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                let range = editor
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                    .unwrap()
                    .source;
                editor.activate(
                    Section::new(SectionKind::DraftCode, range.clone(), range),
                    false,
                    cx,
                );
                editor.move_to(position, false, cx);
                window.focus(&editor.focus);
            });
        });
        cx.simulate_input("%");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let expected = if language == "text" { "{%}" } else { "{%%}" };
            assert_eq!(editor.source(), draft.replacen("{}", expected, 1));
            assert_eq!(editor.cursor(), position + 1);
        });
        cx.simulate_keystrokes("cmd-z");
        cx.update(|_, cx| assert_eq!(editor.read(cx).source(), draft));
    }
}

#[gpui::test]
fn source_tag_input_stays_in_the_draft_and_undo_is_atomic(cx: &mut TestAppContext) {
    for (language, body, typed, expected) in [
        ("HTM", "<section", ">", "<section></section>"),
        ("HTM", "<section>\r\n<", "/", "<section>\r\n</section>"),
        (
            "vue",
            "<template><Widget",
            ">",
            "<template><Widget></Widget>",
        ),
        ("angular", "<main><widget", ">", "<main><widget></widget>"),
        (
            "jinja",
            "{% if visible %}<strong",
            ">",
            "{% if visible %}<strong></strong>",
        ),
        ("liquid", "{{ title }}<em", ">", "{{ title }}<em></em>"),
        (
            "php",
            "<?php echo 1; ?><div",
            ">",
            "<?php echo 1; ?><div></div>",
        ),
        (
            "XML",
            "<root>\r\n  <élément",
            ">",
            "<root>\r\n  <élément></élément>",
        ),
        ("XML", "<root>字<", "/", "<root>字</root>"),
        (
            "jsx",
            "const name = '字';\r\n<div",
            ">",
            "const name = '字';\r\n<div></div>",
        ),
        (
            "tsx",
            "const node: UI = <Container>字<",
            "/",
            "const node: UI = <Container>字</Container>",
        ),
    ] {
        let prefix = format!("字\r\n\r\n```{language}\r\n");
        let source = format!("{prefix}{body}\r\n```\r\n\r\nOutside.");
        let initial = source.clone();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(initial, window, cx));
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                let range = editor
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                    .unwrap()
                    .source;
                editor.activate(
                    Section::new(SectionKind::DraftCode, range.clone(), range),
                    false,
                    cx,
                );
                editor.move_to(prefix.len() + body.len(), false, cx);
                window.focus(&editor.focus);
            });
        });
        cx.simulate_input(typed);
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source.replacen(body, expected, 1));
            let expected_head = if typed == ">" {
                body.len() + 1
            } else {
                expected.len()
            };
            assert_eq!(editor.cursor(), prefix.len() + expected_head);
        });
        cx.simulate_keystrokes("cmd-z");
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                assert_eq!(editor.source(), source);
                assert!(!editor.view.can_undo());
                editor.replace_and_mark_text_in_range(None, typed, None, window, cx);
                assert!(editor.is_composing());
                assert_eq!(
                    editor.source(),
                    source.replacen(body, &format!("{body}{typed}"), 1)
                );
            })
        });
    }
}
