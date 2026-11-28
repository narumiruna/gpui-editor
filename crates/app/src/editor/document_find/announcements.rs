use std::sync::Mutex;

use gpui::{Focusable, Modifiers, TestAppContext, VisualTestContext};
use gpui_editor_codemirror::{
    ChangePermission, Extension, PHRASES, change_filter, view::update_listener,
};

use super::*;

type Speech = Arc<Mutex<Vec<(bool, Range<usize>, Vec<String>)>>>;

fn open_find<'a>(
    cx: &'a mut TestAppContext,
    source: &str,
    query: &str,
    extensions: Extension,
) -> (Entity<Editor>, &'a mut VisualTestContext, Speech) {
    let speech = Speech::default();
    let capture = speech.clone();
    let listener = update_listener(move |update| {
        let spoken: Vec<_> = update.announcements().map(str::to_owned).collect();
        if !spoken.is_empty() {
            capture.lock().unwrap().push((
                update.doc_changed(),
                update.state.selection().main().range(),
                spoken,
            ));
        }
        Ok(())
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.into(),
            Extension::group([listener, extensions]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(750.), px(400.)));
    cx.update(|window, cx| {
        super::super::bind_keys(cx);
        window.focus(&editor.read(cx).focus);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    cx.simulate_input(query);
    cx.run_until_parked();
    (editor, cx, speech)
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn find_announces_selected_matches_without_moving_focus_or_editing(cx: &mut TestAppContext) {
    let source = "😀 hit here\r\nThen hit.";
    let phrases = PHRASES.of(Arc::new(
        [
            ("current match".into(), "Found".into()),
            ("on line".into(), "at row".into()),
        ]
        .into(),
    ));
    let (editor, cx, speech) = open_find(cx, source, "hit", phrases);
    assert!(speech.lock().unwrap().is_empty());
    cx.simulate_keystrokes("enter shift-enter");
    cx.run_until_parked();
    assert_eq!(
        *speech.lock().unwrap(),
        [
            (false, 5..8, vec!["Found. 😀 hit here at row 1.".into()]),
            (false, 20..23, vec!["Found. Then hit. at row 2.".into()]),
        ]
    );
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        let field = editor.find.as_ref().unwrap().query.read(cx);
        assert!(field.focus_handle(cx).is_focused(window));
        assert_eq!(editor.selection(), 20..23);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
        assert!(!editor.view.can_undo_selection());
    });
}

#[gpui::test]
fn find_function_keys_match_pinned_search_in_both_focus_scopes(cx: &mut TestAppContext) {
    let mut rows = include_str!("../../../tests/fixtures/codemirror/setup_find_keys_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|row| row["preset"] == "basic");
    let panel = rows.next().unwrap();
    let body = rows.next().unwrap();
    assert!(rows.next().is_none());
    let source = panel["source"].as_str().unwrap();
    let (editor, cx, speech) = open_find(cx, source, "hit", Extension::default());
    for row in [&panel, &body] {
        let panel_scope = row["scope"] == "panel";
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_selection(0, 0);
                if !panel_scope {
                    window.focus(&editor.focus);
                }
                cx.notify();
            });
        });
        cx.run_until_parked();
        assert!(speech.lock().unwrap().is_empty());
        for step in row["steps"].as_array().unwrap() {
            let key = step["key"].as_str().unwrap();
            cx.simulate_keystrokes(key);
            cx.run_until_parked();
            let mut delivered = std::mem::take(&mut *speech.lock().unwrap());
            assert_eq!(delivered.len(), 1, "{} {key}", row["scope"]);
            let (changed, selection, announced) = delivered.pop().unwrap();
            assert!(!changed);
            assert_eq!(
                serde_json::json!([selection.start, selection.end]),
                step["selection"]
            );
            assert_eq!(serde_json::json!(announced), step["announcements"]);
            cx.update(|window, cx| {
                let editor = editor.read(cx);
                let field = editor.find.as_ref().unwrap().query.read(cx);
                let focused = if panel_scope {
                    field.focus_handle(cx).is_focused(window)
                } else {
                    editor.focus.is_focused(window)
                };
                assert_eq!(serde_json::json!(focused), step["focused"]);
                assert_eq!(editor.selection(), selection);
                assert_eq!(editor.source(), source);
                assert_eq!(
                    serde_json::json!(editor.view.undo_depth()),
                    row["undoDepth"]
                );
                assert!(!editor.view.can_undo_selection());
            });
        }
    }
}

#[gpui::test]
fn replace_all_announces_match_count_on_the_edit_not_on_undo(cx: &mut TestAppContext) {
    let source = "😀 aa\r\n";
    let phrases = PHRASES.of(Arc::new(
        [("replaced $ matches".into(), "Changed $1 results".into())].into(),
    ));
    let (editor, cx, speech) = open_find(cx, source, "a", phrases);
    click(cx, "document-find-show-replacement");
    click(cx, "document-find-replace-all");
    assert_eq!(
        *speech.lock().unwrap(),
        [(true, 0..0, vec!["Changed 2 results.".into()])]
    );
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), "😀 \r\n");
        assert_eq!(editor.view.undo_depth(), 1);
    });
    cx.simulate_keystrokes("escape secondary-z");
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.view.undo_depth(), 0);
    });
    assert_eq!(speech.lock().unwrap().len(), 1);
}

#[gpui::test]
fn replace_next_announces_replacement_then_following_match_in_start_state(cx: &mut TestAppContext) {
    let (editor, cx, speech) = open_find(cx, "😀 cat cat", "cat", Extension::default());
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    speech.lock().unwrap().clear();
    click(cx, "document-find-show-replacement");
    cx.simulate_input("longer");
    click(cx, "document-find-replace");
    assert_eq!(
        *speech.lock().unwrap(),
        [
            (true, 11..11, vec!["replaced match on line 1.".into()]),
            (
                false,
                12..15,
                vec!["current match. 😀 cat cat on line 1.".into()]
            ),
        ]
    );
    click(cx, "document-find-replace");
    assert_eq!(
        speech.lock().unwrap()[2..],
        [(true, 18..18, vec!["replaced match on line 1.".into()])]
    );
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "😀 longer longer"));
}

#[gpui::test]
fn markdown_rejections_do_not_announce_replacements(cx: &mut TestAppContext) {
    let source = "term outside\n\n```rust\nterm\n```\n";
    let (editor, cx, speech) = open_find(cx, source, "term", Extension::default());
    cx.simulate_keystrokes("shift-enter");
    cx.run_until_parked();
    cx.update(|_, cx| {
        let start = source.rfind("term").unwrap();
        assert_eq!(editor.read(cx).selection(), start..start + "term".len());
    });
    speech.lock().unwrap().clear();
    click(cx, "document-find-show-replacement");
    cx.simulate_input("word");
    click(cx, "document-find-replace");
    click(cx, "document-find-replace-all");
    assert!(speech.lock().unwrap().is_empty());
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert!(!editor.view.can_undo());
    });
}

#[gpui::test]
fn native_filter_rejections_do_not_announce_replacements(cx: &mut TestAppContext) {
    let source = "x x";
    let filter = change_filter(|_, transaction| {
        if transaction.changes().is_empty() {
            ChangePermission::Allow
        } else {
            ChangePermission::Reject
        }
    });
    let (editor, cx, speech) = open_find(cx, source, "x", filter);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    speech.lock().unwrap().clear();
    click(cx, "document-find-show-replacement");
    cx.simulate_input("y");
    click(cx, "document-find-replace");
    click(cx, "document-find-replace-all");
    assert!(speech.lock().unwrap().is_empty());
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.source(), source);
        assert_eq!(editor.selection(), 0..1);
        assert!(!editor.view.can_undo());
    });
}
