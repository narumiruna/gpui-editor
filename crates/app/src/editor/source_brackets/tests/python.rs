use super::*;

const PREFIX: &str = "字 😀\r\n\r\n```python\r\n";
const SUFFIX: &str = "\r\n```\r\n\r\n```python\r\nProtected = \"字\"\r\n```";

#[gpui::test]
fn python_quote_input_matches_pinned_editor_state(cx: &mut TestAppContext) {
    #[derive(serde::Deserialize)]
    struct Snapshot {
        body: String,
        selection: [usize; 2],
    }
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        before: Snapshot,
        after: Vec<(String, Snapshot)>,
    }

    cx.update(bind_keys);
    for row in include_str!("../python_quotes.jsonl").lines() {
        let case: Case = serde_json::from_str(row).unwrap();
        let origin = PREFIX.len();
        let source = format!("{PREFIX}{}{SUFFIX}", case.before.body);
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source, window, cx));
        let check = |editor: &Editor, expected: &Snapshot| {
            let name = &case.name;
            assert_eq!(
                editor.source(),
                format!("{PREFIX}{}{SUFFIX}", expected.body),
                "{name}: text"
            );
            assert_eq!(
                editor.source_fence().unwrap().body,
                origin..origin + expected.body.len(),
                "{name}: fence body"
            );
            assert_eq!(
                [editor.anchor(), editor.cursor()],
                expected.selection.map(|pos| origin + pos),
                "{name}: selection"
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

        let mut previous = &case.before;
        let mut undo = Vec::new();
        for (key, step) in &case.after {
            if key == "\x08" {
                cx.simulate_keystrokes("backspace");
            } else {
                cx.simulate_input(key);
            }
            cx.update(|_, cx| check(editor.read(cx), step));
            if step.body != previous.body {
                undo.push(previous);
            }
            previous = step;
        }
        for previous in undo.into_iter().rev() {
            cx.simulate_keystrokes("cmd-z");
            cx.update(|_, cx| check(editor.read(cx), previous));
        }
        cx.update(|_, cx| assert!(!editor.read(cx).view.can_undo(), "{}", case.name));
    }
}

#[gpui::test]
fn python_quote_input_preserves_composition_and_protected_fences(cx: &mut TestAppContext) {
    let source = format!("{PREFIX}\"😀 \"{SUFFIX}");
    let position = PREFIX.len();
    let composed = format!("{}\"{}", &source[..position], &source[position..]);
    let protected = source.find("Protected").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
    cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_first_code_preview(editor, cx);
            editor.move_to(position, false, cx);
            window.focus(&editor.focus);
            editor.replace_text_in_range(
                Some(to_utf16(&source, protected..protected + 1)),
                "\"",
                window,
                cx,
            );
            assert_eq!(editor.source(), source);
            assert!(!editor.view.can_undo());

            editor.replace_and_mark_text_in_range(None, "\"", None, window, cx);
            assert!(editor.is_composing());
            assert_eq!(editor.source(), composed);
            editor.replace_text_in_range(None, "\"", window, cx);
            assert!(!editor.is_composing());
            assert_eq!(editor.source(), composed);
            assert_eq!(editor.cursor(), position + 1);
        });
    });
    cx.simulate_keystrokes("cmd-z");
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}
