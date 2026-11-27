use super::*;
use gpui::Modifiers;
use gpui_editor_codemirror::language::lezer_grammar::LezerDocument;

const PREFIX: &str = "字 😀\r\n\r\n```lezer\r\n";
const SUFFIX: &str = "\r\n```\r\n\r\nOutside }";

fn source_hit(origin: usize, (from, to, matched): Hit) -> BracketMatch {
    let range = |[from, to]: [usize; 2]| origin + from..origin + to;
    BracketMatch {
        start: range(from),
        end: to.map(range),
        matched,
    }
}

#[test]
fn lezer_brackets_match_pinned_editor_state() {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        source: String,
        checks: Vec<(usize, Option<Hit>)>,
    }
    for row in include_str!("../lezer_brackets.jsonl").lines() {
        let case: Case = serde_json::from_str(row).unwrap();
        let origin = PREFIX.len();
        let scope = origin..origin + case.source.len();
        let text = Text::new(format!("{PREFIX}{}{SUFFIX}", case.source));
        let body = LezerDocument::parse(text.slice_owned(scope.clone()).unwrap());
        let bracket_at = |head| {
            grammar_bracket_near(&text, &scope, head, "()[]{}", |head| {
                body.bracket_near(head)
            })
        };
        for (head, hit) in case.checks {
            assert_eq!(
                bracket_at(origin + head),
                hit.map(|hit| source_hit(origin, hit)),
                "{} at byte {head}",
                case.name
            );
        }
        assert_eq!(bracket_at(0), None);
        assert_eq!(bracket_at(text.len()), None);
    }
}

#[gpui::test]
fn lezer_quote_input_matches_pinned_editor_state(cx: &mut TestAppContext) {
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
        after: Vec<(String, Snapshot)>,
    }
    cx.update(bind_keys);
    for row in include_str!("../lezer_quotes.jsonl").lines() {
        let case: Case = serde_json::from_str(row).unwrap();
        let origin = PREFIX.len();
        let source = format!("{PREFIX}{}{SUFFIX}", case.before.body);
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source, window, cx));
        let check = |editor: &Editor, expected: &Snapshot| {
            let name = &case.name;
            assert_eq!(
                editor.source(),
                format!("{PREFIX}{}{SUFFIX}", expected.body),
                "{name} text"
            );
            assert_eq!(
                editor.source_fence().unwrap().body,
                origin..origin + expected.body.len(),
                "{name} fence body"
            );
            assert_eq!(
                [editor.anchor(), editor.cursor()],
                expected.selection.map(|pos| origin + pos),
                "{name} selection"
            );
            assert_eq!(
                editor.source_bracket_match(),
                expected.hit.map(|hit| source_hit(origin, hit)),
                "{name} hit"
            );
        };
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                activate_first_code_preview(editor, cx);
                let [anchor, head] = case.before.selection.map(|pos| origin + pos);
                editor.set_selection(anchor, head);
                window.focus(&editor.focus);
            });
            check(editor.read(cx), &case.before);
        });
        for (typed, snapshot) in &case.after {
            cx.simulate_input(typed);
            cx.update(|_, cx| check(editor.read(cx), snapshot));
        }
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            check(editor, &case.before);
            assert!(!editor.view.can_undo(), "{} undo", case.name);
        });
    }
}

#[gpui::test]
fn lezer_bracket_hits_follow_pointer_edit_undo_and_ime(cx: &mut TestAppContext) {
    let body = "@detectDelim\r\nRoot[p=\"[\"] {\r\n  (\"😀}\")\r\n}";
    let source = format!("{PREFIX}{body}{SUFFIX}\r\n\r\n```lezer\r\nProtected {{ }}\r\n```");
    let origin = PREFIX.len();
    let open = origin + body.find('{').unwrap();
    let close = origin + body.len() - 1;
    let literal_open = origin + body.find("\"[\"").unwrap() + 1;
    let literal_close = origin + body.find('}').unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            editor.set_selection(close + 1, close + 1);
            assert!(editor.source_bracket_match().is_none());
            activate_first_code_preview(editor, cx);
            window.focus(&editor.focus);
        });
    });
    cx.run_until_parked();
    let [literal_pointer, structural_pointer] = cx.update(|_, cx| {
        let layout = editor.read(cx).layout.as_ref().unwrap();
        [literal_close + 1, close + 1].map(|at| layout.bounds_at(at, false).unwrap().center())
    });
    cx.simulate_click(literal_pointer, Modifiers::none());
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.cursor(), literal_close + 1);
        assert_eq!(
            editor.source_bracket_match(),
            Some(BracketMatch {
                start: literal_close..literal_close + 1,
                end: Some(literal_open..literal_open + 1),
                matched: false,
            })
        );
    });
    cx.simulate_click(structural_pointer, Modifiers::none());
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.cursor(), close + 1);
        assert_eq!(
            editor.source_bracket_match(),
            Some(BracketMatch {
                start: close..close + 1,
                end: Some(open..open + 1),
                matched: true,
            })
        );
    });
    cx.simulate_keystrokes("backspace");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(
                editor.source(),
                format!("{}{}", &source[..close], &source[close + 1..])
            );
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
    cx.simulate_keystrokes("secondary-z");
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());
            editor.move_to(open + 1, false, cx);
            assert_eq!(
                editor.source_bracket_match().unwrap().end,
                Some(close..close + 1)
            );
            let protected = source.find("Protected").unwrap();
            let protected = to_utf16(&source, protected..protected + 1);
            editor.replace_text_in_range(Some(protected), "\"", window, cx);
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());

            editor.replace_and_mark_text_in_range(None, "\"", None, window, cx);
            assert!(editor.is_composing());
            assert!(editor.source_bracket_match().is_none());
            let composed = format!("{}\"{}", &source[..open + 1], &source[open + 1..]);
            assert_eq!(editor.source(), composed);
            editor.replace_text_in_range(None, "\"", window, cx);
            assert!(!editor.is_composing());
            assert_eq!(editor.source(), composed);
        });
    });
    cx.simulate_keystrokes("secondary-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}
