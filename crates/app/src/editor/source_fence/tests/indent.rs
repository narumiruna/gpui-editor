use std::sync::Mutex;

use gpui_editor_codemirror::{
    Assoc, ChangePermission, StateField, change_filter,
    language::{
        INDENT_UNIT, IndentDecision, IndentStyle, indent_service,
        wast::{INDENT, WastIndentStrategy},
    },
    language_data::json_extension,
};
use serde_json::{Value, json};

use super::*;

#[gpui::test]
fn configured_wast_indent_commands_match_pinned_host_callbacks(cx: &mut TestAppContext) {
    for line in
        include_str!("../../../../../codemirror/tests/fixtures/wast_fence_indent_commands.jsonl")
            .lines()
    {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let body = row["body"].as_str().unwrap();
        let selection = fixture_range(&row["selection"]);
        let expected = fixture_range(&row["expected"]);
        let first = source.find("  (func").unwrap();
        let last = source.find("  (data").unwrap();
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let observe = recorded.clone();
        let field = StateField::new(|_| Ok("parent"), |value, _| Ok(*value));
        let installed = field.extension();
        let strategy: WastIndentStrategy = Arc::new(move |cx| {
            let host = cx.state().expect("current parent state");
            assert_eq!(
                host.context().field(&field).unwrap().as_deref(),
                Some(&"parent")
            );
            assert!(host.text().as_str().ends_with("Tail."));
            let next = cx.continue_indent();
            let lines = cx.line_context();
            let pos = lines.position();
            let before = if pos >= last {
                lines.line_indent(first, Assoc::After)?
            } else {
                0
            };
            let range = cx.node().range();
            assert!(host.text().slice(range.clone()).unwrap().starts_with('('));
            let simulated = lines.simulated_break();
            observe.lock().unwrap().push(json!({
                "range": [range.start, range.end], "pos": pos, "unit": lines.unit(), "next": next,
                "before": before, "head": host.selection().main().head(), "simulated": simulated,
                "double": simulated.is_some_and(|at| lines.line_at(at, Assoc::Before).unwrap().range.is_empty()),
            }));
            Some(lines.unit() + 1 + next.unwrap_or(0) + before)
        });
        let wast = wast_language()
            .with_node_props(&[INDENT.add(move |row| row.is("App").then(|| strategy.clone()))]);
        let bindings = Extension::group([
            installed,
            markdown_parser(wast.clone(), Some(wast)),
            INDENT_UNIT.of(IndentStyle::parse("   ").unwrap()),
            if name == "selection" {
                json_extension()
            } else {
                Extension::default()
            },
            if name == "service" {
                indent_service(|_, _| IndentDecision::Column(9))
            } else {
                Extension::default()
            },
        ]);
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::with_native_extensions(source.to_owned(), bindings, window, cx).unwrap()
        });
        cx.update(|window, cx| {
            bind_keys(cx);
            editor.update(cx, |editor, cx| {
                activate_host_fence(editor, body, cx);
                assert!(editor.source_fence().is_some());
                editor.set_selection(selection.start, selection.end);
                window.focus(&editor.focus);
            });
        });
        cx.run_until_parked();
        cx.simulate_keystrokes(match name {
            "enter" | "service" => "enter",
            "blank" => "cmd-enter",
            "selection" => "cmd-alt-\\",
            _ => unreachable!(),
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), row["result"].as_str().unwrap(), "{name}");
            let main = editor.view.state().selection().main();
            assert_eq!(
                (main.anchor(), main.head()),
                (expected.start, expected.end),
                "{name}"
            );
            assert_eq!(json!(editor.view.undo_depth()), row["depth"], "{name}");
            assert_eq!(json!(*recorded.lock().unwrap()), row["calls"], "{name}");
        });
        cx.simulate_keystrokes("secondary-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source, "{name}");
            let main = editor.view.state().selection().main();
            assert_eq!(
                (main.anchor(), main.head()),
                (selection.start, selection.end),
                "{name}"
            );
            assert_eq!((editor.view.undo_depth(), editor.view.redo_depth()), (0, 1));
        });
        cx.simulate_keystrokes("secondary-shift-z");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), row["result"].as_str().unwrap(), "{name}");
            assert_eq!((editor.view.undo_depth(), editor.view.redo_depth()), (1, 0));
        });
    }
}

#[gpui::test]
fn rejected_mounted_wast_enter_does_not_retry_literal_lf(cx: &mut TestAppContext) {
    const BODY: &str = "(module\r  (func \"字\" ())\r\n)";
    let source = format!("前 😀\r\n\r\n```wat\r\n{BODY}\r\n```\r\n");
    let head = source.find("()").unwrap() + 1;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = calls.clone();
    let filter = change_filter(move |_, transaction| {
        let mut reject = false;
        for (_, insert) in transaction.changes().changes() {
            reject |= insert.contains("\r\n");
            captured.lock().unwrap().push(insert.to_owned());
        }
        if reject {
            ChangePermission::Reject
        } else {
            ChangePermission::Allow
        }
    });
    let strict = wast_language().configure(Some(true), None);
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.clone(),
            Extension::group([markdown_parser(strict, None), filter]),
            window,
            cx,
        )
        .unwrap()
    });
    let original = cx.update(|window, cx| {
        bind_keys(cx);
        editor.update(cx, |editor, cx| {
            activate_host_fence(editor, BODY, cx);
            editor.set_selection(head, head);
            window.focus(&editor.focus);
            markdown_snapshot(editor.view.state()).unwrap()
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.cursor(), head);
        assert_eq!(
            (
                editor.view.revision(),
                editor.view.undo_depth(),
                editor.view.redo_depth()
            ),
            (0, 0, 0)
        );
        assert!(Arc::ptr_eq(
            &original,
            &markdown_snapshot(editor.view.state()).unwrap()
        ));
    });
    let attempts = calls.lock().unwrap();
    assert_eq!(attempts.len(), 1);
    assert!(attempts[0].starts_with("\r\n"));
}
