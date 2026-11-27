use super::*;

#[gpui::test]
fn xml_feedback_matches_case_sensitive_tag_names_and_delimiters(cx: &mut TestAppContext) {
    let prefix = "外 😀\r\n\r\n```xml\r\n";
    let body = "<root attr=\"[x]\">🐈<x/>\r\n</root>";
    let suffix = "\r\n```\r\n\r\n```xml\r\n<root></root>\r\n```";
    let original = format!("{prefix}{body}{suffix}");
    let start = prefix.len();
    let open = start + 1..start + 5;
    let close_at = start + body.find("</root>").unwrap();
    let close = close_at + 2..close_at + 6;
    let self_close = start + body.find("<x/>").unwrap();
    let protected = original.rfind("<root>").unwrap() + 2;
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(original.clone(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            editor.set_selection(open.start + 1, open.start + 1);
            assert_eq!(editor.source_bracket_match(), None);
            activate_first_code_preview(editor, cx);
            window.focus(&editor.focus);
            for (at, from, to) in [
                (open.start + 1, open.clone(), close.clone()),
                (close.start + 1, close.clone(), open.clone()),
                (
                    self_close + 1,
                    self_close..self_close + 1,
                    self_close + 2..self_close + 4,
                ),
            ] {
                editor.set_selection(at, at);
                assert_eq!(
                    editor.source_bracket_match(),
                    Some(BracketMatch {
                        start: from,
                        end: Some(to),
                        matched: true
                    })
                );
            }
            editor.set_selection(protected, protected);
            assert_eq!(editor.source_bracket_match(), None);
            editor.set_selection(close.start, close.end);
            editor.replace_text_in_range(None, "Root", window, cx);
            editor.set_selection(open.start + 1, open.start + 1);
            assert_eq!(editor.source(), original.replacen("</root>", "</Root>", 1));
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
            assert_eq!(editor.source(), original);
            editor.set_selection(open.start + 1, open.start + 1);
            assert_eq!(editor.source_bracket_match().unwrap().end, Some(close));
            editor.replace_and_mark_text_in_range(None, "!", None, window, cx);
            assert!(editor.is_composing());
            assert_eq!(editor.source_bracket_match(), None);
        });
    });
}
