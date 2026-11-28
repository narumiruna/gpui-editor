use gpui_editor_codemirror::language::brackets::{BracketMatchingOptions, bracket_matching};

use super::{config::Options, *};

type Mark = (usize, usize, bool);

#[gpui::test]
fn installed_html_bracket_options_follow_the_pinned_mount_probes(cx: &mut TestAppContext) {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        source: String,
        head: usize,
        options: Options,
        marks: Vec<(usize, usize, String)>,
    }
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/codemirror/html_bracket_options.jsonl"
    ));
    for line in fixture.lines() {
        let case: Case = serde_json::from_str(line).unwrap();
        if case.name.starts_with("custom-") {
            continue;
        }
        let expected = case
            .marks
            .into_iter()
            .map(|(from, to, mark)| (from, to, mark == "M"))
            .collect::<Vec<_>>();
        check_configured_html(
            cx,
            &case.name,
            &case.source,
            case.head,
            case.options.into(),
            &expected,
        );
    }
}

#[gpui::test]
fn configured_html_matching_does_not_pair_across_mounts(cx: &mut TestAppContext) {
    for source in [
        "<script>/*«*/</script><script>/*»*/</script>",
        "<button onclick=\"/*«*/\"></button><button onclick=\"/*»*/\"></button>",
    ] {
        let start = source.find('«').unwrap();
        check_configured_html(
            cx,
            "independent mounts",
            source,
            start,
            BracketMatchingOptions {
                brackets: Some("«»".into()),
                ..Default::default()
            },
            &[(start, start + '«'.len_utf8(), false)],
        );
    }
}

fn check_configured_html(
    cx: &mut TestAppContext,
    name: &str,
    body: &str,
    head: usize,
    options: BracketMatchingOptions,
    expected: &[Mark],
) {
    const PREFIX: &str = "外 😀\r\n\r\n```html\r\n";
    const SUFFIX: &str = "\r\n```\r\n\r\n```html\r\n<script>/*»*/</script>\r\n```";
    let source = format!("{PREFIX}{body}{SUFFIX}");
    let origin = PREFIX.len();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source.clone(), bracket_matching(options), window, cx)
            .unwrap()
    });
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            activate_first_code_preview(editor, cx);
            window.focus(&editor.focus);
            editor.set_selection(origin + head, origin + head);
            let mut marks = Vec::new();
            if let Some(hit) = editor.source_bracket_match() {
                for range in std::iter::once(hit.start).chain(hit.end) {
                    marks.push((range.start - origin, range.end - origin, hit.matched));
                }
                marks.sort_unstable();
            }
            assert_eq!(marks, expected, "{name}");
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert_eq!(
                editor.layout.as_ref().unwrap().painted_bracket_quads.len(),
                expected.len(),
                "{name}"
            );
            let outside = source.rfind('»').unwrap();
            editor.set_selection(outside, outside);
            assert_eq!(editor.source_bracket_match(), None, "{name}");
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());
        });
    });
}

#[gpui::test]
fn html_mounted_brackets_stay_in_their_syntax_regions(cx: &mut TestAppContext) {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        source: String,
        checks: Vec<(usize, Option<Hit>)>,
    }
    const PREFIX: &str = "外 😀\r\n\r\n```html\r\n";
    const SUFFIX: &str = "\r\n```\r\n\r\n```html\r\n<script>f(1)</script>\r\n```";
    let fixtures = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/codemirror/html_mounted_brackets.jsonl"
    ));
    for row in fixtures.lines() {
        let case: Case = serde_json::from_str(row).unwrap();
        let source = format!("{PREFIX}{}{SUFFIX}", case.source);
        let origin = PREFIX.len();
        let protected = source.rfind("f(1)").unwrap() + 2;
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                activate_first_code_preview(editor, cx);
                window.focus(&editor.focus);
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                let range = |[start, end]: [usize; 2]| origin + start..origin + end;
                for (head, expected) in case.checks {
                    editor.set_selection(origin + head, origin + head);
                    let expected = expected.map(|(start, end, matched)| BracketMatch {
                        start: range(start),
                        end: end.map(range),
                        matched,
                    });
                    assert_eq!(
                        editor.source_bracket_match(),
                        expected,
                        "{} at {head}",
                        case.name
                    );
                }
                editor.set_selection(protected, protected);
                assert!(editor.source_bracket_match().is_none());
                assert_eq!(editor.source(), source);
                assert!(!editor.view.can_undo());
            });
        });
    }
}

#[gpui::test]
fn html_feedback_uses_tag_names_and_stays_inside_the_active_fence(cx: &mut TestAppContext) {
    let prefix = "外 😀\r\n\r\n```html\r\n";
    let body = "<section title=\"(x)\">\r\n  <em>猫</em>\r\n</section>\r\n<script>f(1)</script>";
    let suffix = "\r\n```\r\n\r\n```html\r\n<section></section>\r\n```";
    let source = format!("{prefix}{body}{suffix}");
    let offset = prefix.len();
    let open = offset + 1..offset + 8;
    let close_at = offset + body.find("</section>").unwrap();
    let close = close_at + 2..close_at + 9;
    let mount_at = offset + body.find("f(1)").unwrap();
    let script_start = offset + body.find("<script>").unwrap();
    let angle = offset + body.find('>').unwrap();
    let protected = source.rfind("<section>").unwrap() + 3;
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            editor.set_selection(open.start + 1, open.start + 1);
            assert_eq!(editor.source_bracket_match(), None);
            activate_first_code_preview(editor, cx);
            window.focus(&editor.focus);
            for (head, start, end) in [
                (open.start + 1, open.clone(), close.clone()),
                (close.start + 1, close.clone(), open.clone()),
                (open.start, offset..offset + 1, angle..angle + 1),
                (
                    mount_at,
                    mount_at - 1..mount_at,
                    script_start..script_start + 1,
                ),
                (
                    mount_at + 4,
                    mount_at + 3..mount_at + 4,
                    mount_at + 1..mount_at + 2,
                ),
                (
                    mount_at + 2,
                    mount_at + 1..mount_at + 2,
                    mount_at + 3..mount_at + 4,
                ),
            ] {
                editor.set_selection(head, head);
                assert_eq!(
                    editor.source_bracket_match(),
                    Some(BracketMatch {
                        start,
                        end: Some(end),
                        matched: true
                    })
                );
            }
            editor.set_selection(open.start, open.end);
            assert_eq!(editor.source_bracket_match(), None);
            editor.set_selection(protected, protected);
            assert_eq!(editor.source_bracket_match(), None);
            editor.set_selection(close.start, close.end);
            editor.replace_text_in_range(None, "article", window, cx);
        });
    });
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            assert_eq!(
                editor.source(),
                source.replacen("</section>", "</article>", 1)
            );
            editor.set_selection(open.start + 1, open.start + 1);
            assert_eq!(
                editor.source_bracket_match(),
                Some(BracketMatch {
                    start: open.clone(),
                    end: None,
                    matched: false
                })
            );
        });
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), source);
            editor.set_selection(open.start + 1, open.start + 1);
            assert_eq!(editor.source_bracket_match().unwrap().end, Some(close));
            editor.replace_and_mark_text_in_range(None, "A", None, window, cx);
            assert!(editor.is_composing());
            assert_eq!(editor.source_bracket_match(), None);
        });
    });
}
