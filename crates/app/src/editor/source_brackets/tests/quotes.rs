use super::*;

#[gpui::test]
fn java_quote_input_matches_the_pinned_editor(cx: &mut TestAppContext) {
    check_quotes(
        cx,
        "java",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../codemirror/tests/fixtures/java_quotes.jsonl"
        )),
    );
}

#[gpui::test]
fn rust_quote_input_matches_the_pinned_editor(cx: &mut TestAppContext) {
    check_quotes(
        cx,
        "rust",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../codemirror/tests/fixtures/rust_quotes.jsonl"
        )),
    );
}

#[gpui::test]
fn go_quote_input_matches_the_pinned_editor(cx: &mut TestAppContext) {
    check_quotes(
        cx,
        "go",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../codemirror/tests/fixtures/go_quotes.jsonl"
        )),
    );
}

#[gpui::test]
fn javascript_family_quote_input_matches_the_pinned_editor(cx: &mut TestAppContext) {
    check_quotes(
        cx,
        "js",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../codemirror/tests/fixtures/javascript_quotes.jsonl"
        )),
    );
}

#[gpui::test]
fn xml_quote_input_matches_the_pinned_editor(cx: &mut TestAppContext) {
    check_quotes(
        cx,
        "xml",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../codemirror/tests/fixtures/xml_quotes.jsonl"
        )),
    );
}

#[gpui::test]
fn cpp_quote_input_matches_the_pinned_editor(cx: &mut TestAppContext) {
    check_quotes(
        cx,
        "cpp",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../codemirror/tests/fixtures/cpp_quotes.jsonl"
        )),
    );
}

fn check_quotes(cx: &mut TestAppContext, language: &str, fixture: &str) {
    #[derive(serde::Deserialize)]
    struct Case {
        mode: Option<String>,
        name: String,
        source: String,
        head: usize,
        key: String,
        after: String,
        position: usize,
    }
    cx.update(bind_keys);
    for row in fixture.lines() {
        let case: Case = serde_json::from_str(row).unwrap();
        let mode = case.mode.as_deref().unwrap_or(language);
        let prefix = format!("前 😀\r\n\r\n```{mode}\r\n");
        let suffix = format!("\r\n```\r\n\r\n```{mode}\r\nprotectedValue \"字\"\r\n```");
        let origin = prefix.len();
        let source = format!("{prefix}{}{suffix}", case.source);
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                activate_first_code_preview(editor, cx);
                editor.set_selection(origin + case.head, origin + case.head);
                window.focus(&editor.focus);
            });
        });
        cx.simulate_input(&case.key);
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(
                editor.source(),
                format!("{prefix}{}{suffix}", case.after),
                "{}",
                case.name
            );
            assert_eq!(
                editor.selection(),
                origin + case.position..origin + case.position,
                "{}",
                case.name
            );
            assert!(editor.view.can_undo(), "{}", case.name);
        });
        cx.simulate_keystrokes("cmd-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source, "{}", case.name);
            assert_eq!(editor.cursor(), origin + case.head, "{}", case.name);
            assert!(!editor.view.can_undo(), "{}", case.name);
        });
    }
}
