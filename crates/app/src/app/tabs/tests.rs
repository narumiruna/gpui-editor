use gpui::{
    Entity, EntityInputHandler as _, Focusable as _, Modifiers, MouseButton, Point, TestAppContext,
    VisualTestContext, point, px, size,
};

use super::{Pixels, Shell, reorder::SCROLL_TICK};
use crate::app::{
    ActiveOperation, Intent, TabId,
    tests::{
        Fixture, replace_current, restore, selection, start, tab_paths, tab_selector, use_session,
    },
};

const LEFT: MouseButton = MouseButton::Left;

fn tab_ids(cx: &mut VisualTestContext, shell: &Entity<Shell>) -> Vec<TabId> {
    cx.update(|_, cx| shell.read(cx).tabs.iter().map(|tab| tab.id).collect())
}

fn begin_drag(cx: &mut VisualTestContext, id: TabId) {
    let source = cx.debug_bounds(tab_selector("tab", id)).unwrap().center();
    cx.simulate_mouse_down(source, LEFT, Modifiers::none());
    move_drag(cx, source + point(px(4.), px(0.)));
    cx.update(|_, cx| assert!(cx.has_active_drag()));
}

fn move_drag(cx: &mut VisualTestContext, target: Point<Pixels>) {
    cx.simulate_mouse_move(target, LEFT, Modifiers::none());
    cx.run_until_parked();
}

fn release(cx: &mut VisualTestContext, target: Point<Pixels>) {
    cx.simulate_mouse_up(target, LEFT, Modifiers::none());
    cx.run_until_parked();
    cx.update(|_, cx| assert!(!cx.has_active_drag()));
}

#[gpui::test]
fn document_tabs_reorder_without_selecting_closing_or_resetting_editors(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let first = fixture.document("First.md", "one");
    let middle = fixture.document("Middle.md", "two");
    let last = fixture.document("Last.md", "three");
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.update(|_, cx| crate::editor::bind_keys(cx));
    cx.simulate_resize(size(px(1040.), px(720.)));
    fixture.open(cx, &shell);
    for path in [&first, &middle, &last] {
        start(cx, &shell, Intent::Open(path.clone()));
    }
    let ids = tab_ids(cx, &shell);
    let [a, b, c] = ids[..] else {
        panic!("expected three tabs")
    };
    let first_editor = cx.update(|_, cx| shell.read(cx).tab(a).unwrap().editor.clone());
    let tree_tab = cx.debug_bounds("sidebar-tab-Tree").unwrap();
    let (selected, editor) = replace_current(cx, &shell, "unsaved draft");
    cx.simulate_keystrokes("shift-left");
    let draft_selection = selection(cx, &editor);
    assert_eq!(draft_selection, 12..13);

    begin_drag(cx, a);
    let target = cx
        .debug_bounds(tab_selector("close-tab", c))
        .unwrap()
        .center();
    move_drag(cx, target);
    cx.update(|_, cx| {
        let shell = shell.read(cx);
        assert_eq!(tab_paths(shell), [&first, &middle, &last]);
        let line = shell.document_insertion_line(a, target, cx).unwrap();
        assert!(line.left() > target.x);
        assert_eq!(line.size.width, px(2.));
    });
    release(cx, target);
    assert!(!cx.has_pending_prompt());
    assert_eq!(tab_ids(cx, &shell), [b, c, a]);

    begin_drag(cx, c);
    let destination = cx.debug_bounds(tab_selector("tab", b)).unwrap();
    let target = point(destination.left() + px(2.), destination.center().y);
    move_drag(cx, target);
    release(cx, target);
    assert_eq!(tab_ids(cx, &shell), [c, b, a]);

    begin_drag(cx, b);
    let target = cx
        .debug_bounds("sidebar-drop-Tree-before")
        .unwrap()
        .center();
    move_drag(cx, target);
    cx.update(|_, cx| {
        assert!(
            shell
                .read(cx)
                .document_insertion_line(b, target, cx)
                .is_none()
        )
    });
    release(cx, target);
    begin_drag(cx, c);
    let target = cx
        .debug_bounds(tab_selector("close-tab", c))
        .unwrap()
        .center();
    move_drag(cx, target);
    release(cx, target);
    assert!(!cx.has_pending_prompt());
    assert_eq!(cx.debug_bounds("sidebar-tab-Tree"), Some(tree_tab));
    assert_eq!(selection(cx, &editor), draft_selection);
    cx.update(|window, cx| {
        let shell = shell.read(cx);
        assert_eq!(tab_paths(shell), [&last, &middle, &first]);
        assert_eq!(shell.selected, Some(selected));
        assert_eq!(shell.current().unwrap().editor, editor);
        assert_eq!(shell.tab(a).unwrap().editor, first_editor);
        assert_eq!(editor.read(cx).source(), "unsaved draft");
        assert!(shell.current().unwrap().dirty(cx));
        assert!(editor.focus_handle(cx).is_focused(window));
    });

    cx.simulate_input("t!");
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "unsaved draft!"));
    use_session(cx, &shell, &fixture.session_path);
    start(cx, &shell, Intent::Save);
    start(cx, &shell, Intent::Exit);
    cx.cx.read(|cx| assert!(cx.windows().is_empty()));
    let (reopened, cx) = cx.cx.add_window_view(Shell::new);
    restore(cx, &reopened, &fixture.session_path);
    cx.update(|_, cx| {
        let shell = reopened.read(cx);
        assert_eq!(tab_paths(shell), [&last, &middle, &first]);
        assert_eq!(shell.current().unwrap().path(), Some(last.as_path()));
        assert_eq!(
            shell.current().unwrap().editor.read(cx).source(),
            "unsaved draft!"
        );
    });
}

fn tick(cx: &mut VisualTestContext) {
    cx.cx.executor().advance_clock(SCROLL_TICK);
    cx.run_until_parked();
}

fn offset(cx: &mut VisualTestContext, shell: &Entity<Shell>) -> Pixels {
    cx.update(|_, cx| shell.read(cx).tab_scroll.offset().x)
}

#[gpui::test]
fn document_tab_drag_scrolls_at_both_edges_without_more_mouse_moves(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let paths: Vec<_> = (0..7)
        .map(|i| fixture.document(&format!("Document {i:02}.md"), "text"))
        .collect();
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.simulate_resize(size(px(560.), px(360.)));
    fixture.open(cx, &shell);
    for path in &paths {
        start(cx, &shell, Intent::Open(path.clone()));
    }
    let ids = tab_ids(cx, &shell);
    let selected = *ids.last().unwrap();
    let strip = cx.debug_bounds("document-tabs").unwrap();
    let left = point(strip.left() + px(4.), strip.center().y);
    let right = point(strip.right() - px(4.), strip.center().y);
    let far_right = cx.update(|_, cx| -shell.read(cx).tab_scroll.max_offset().width);
    assert!(far_right < px(-24.), "tabs must overflow the test window");
    assert_eq!(offset(cx, &shell), far_right);

    begin_drag(cx, selected);
    move_drag(cx, left);
    for _ in 0..3 {
        let before = offset(cx, &shell);
        tick(cx);
        assert!(offset(cx, &shell) > before);
    }
    let outside = left + point(px(0.), strip.size.height);
    move_drag(cx, outside);
    let parked = offset(cx, &shell);
    tick(cx);
    tick(cx);
    assert_eq!(offset(cx, &shell), parked);
    assert_eq!(tab_ids(cx, &shell), ids);

    move_drag(cx, left);
    for _ in 0..100 {
        if offset(cx, &shell) == px(0.) {
            break;
        }
        tick(cx);
    }
    assert_eq!(offset(cx, &shell), px(0.));
    release(cx, left);
    assert_eq!(
        tab_ids(cx, &shell),
        [&ids[ids.len() - 1..], &ids[..ids.len() - 1]].concat()
    );

    begin_drag(cx, selected);
    move_drag(cx, right);
    // Release while more scrolling is possible, not at the limit.
    let before = offset(cx, &shell);
    tick(cx);
    assert!(offset(cx, &shell) < before);
    assert!(offset(cx, &shell) > far_right);
    release(cx, right);
    let after_release = offset(cx, &shell);
    tick(cx);
    tick(cx);
    assert_eq!(offset(cx, &shell), after_release);
    assert_ne!(tab_ids(cx, &shell)[0], selected);
    cx.update(|_, cx| assert_eq!(shell.read(cx).selected, Some(selected)));
}

#[gpui::test]
fn document_drop_uses_painted_tab_geometry_before_the_next_scroll_frame(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let paths: Vec<_> = (0..7)
        .map(|i| fixture.document(&format!("Note {i:02}.md"), ""))
        .collect();
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.simulate_resize(size(px(560.), px(360.)));
    fixture.open(cx, &shell);
    for path in &paths {
        start(cx, &shell, Intent::Open(path.clone()));
    }
    cx.update(|window, cx| {
        shell.read(cx).tab_scroll.set_offset(point(px(0.), px(0.)));
        window.refresh();
    });
    cx.run_until_parked();
    let ids = tab_ids(cx, &shell);
    begin_drag(cx, ids[0]);
    let target = cx.update(|_, cx| {
        shell
            .read(cx)
            .tab_scroll
            .bounds_for_item(2)
            .unwrap()
            .center()
            - point(px(2.), px(0.))
    });
    assert!(cx.debug_bounds("document-tabs").unwrap().contains(&target));
    move_drag(cx, target);
    cx.update(|_, cx| {
        // Model a scroll tick after the strip painted but before mouse-up is delivered.
        shell
            .read(cx)
            .tab_scroll
            .set_offset(point(px(-12.), px(0.)));
    });
    release(cx, target);
    assert_eq!(tab_ids(cx, &shell)[..3], [ids[1], ids[0], ids[2]]);
}

#[gpui::test]
fn document_drag_cancels_on_escape_other_button_or_document_operation(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (shell, cx) = cx.add_window_view(Shell::new);
    fixture.open(cx, &shell);
    for name in ["first.md", "second.md"] {
        start(cx, &shell, Intent::Open(fixture.document(name, "")));
    }
    let ids = tab_ids(cx, &shell);
    let target = cx
        .debug_bounds(tab_selector("close-tab", ids[1]))
        .unwrap()
        .center();
    for cancel in ["escape", "right button", "save"] {
        begin_drag(cx, ids[0]);
        move_drag(cx, target);
        match cancel {
            "escape" => cx.simulate_keystrokes("escape"),
            "right button" => cx.simulate_mouse_up(target, MouseButton::Right, Modifiers::none()),
            _ => start(cx, &shell, Intent::Save),
        }
        release(cx, target);
        assert_eq!(tab_ids(cx, &shell), ids, "{cancel}");
    }
}

#[gpui::test]
fn document_tab_drop_rechecks_busy_developer_and_marked_text_guards(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let first = fixture.document("First.md", "first");
    let second = fixture.document("Second.md", "second");
    let (shell, cx) = cx.add_window_view(Shell::new);
    cx.simulate_resize(size(px(1040.), px(720.)));
    fixture.open(cx, &shell);
    for path in [&first, &second] {
        start(cx, &shell, Intent::Open(path.clone()));
    }
    let ids = tab_ids(cx, &shell);
    let dest = cx
        .debug_bounds(tab_selector("close-tab", ids[1]))
        .unwrap()
        .center();
    let editor = cx.update(|_, cx| shell.read(cx).current().unwrap().editor.clone());

    begin_drag(cx, ids[0]);
    move_drag(cx, dest);
    cx.update(|_, cx| {
        shell.update(cx, |shell, cx| {
            shell.active = Some(ActiveOperation::Other);
            cx.notify();
        })
    });
    release(cx, dest);
    assert_eq!(tab_ids(cx, &shell), ids);
    cx.update(|_, cx| {
        shell.update(cx, |shell, cx| {
            shell.active = None;
            cx.notify();
        })
    });
    cx.run_until_parked();

    begin_drag(cx, ids[0]);
    move_drag(cx, dest);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.replace_and_mark_text_in_range(Some(0..6), "ㄓ", None, window, cx);
        })
    });
    release(cx, dest);
    assert_eq!(tab_ids(cx, &shell), ids);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.is_composing());
            assert_eq!(editor.source(), "ㄓ");
            editor.unmark_text(window, cx);
        });
        assert_eq!(
            tab_paths(shell.read(cx)),
            [first.as_path(), second.as_path()]
        );
    });
    cx.run_until_parked();

    begin_drag(cx, ids[0]);
    move_drag(cx, dest);
    cx.update(|window, cx| shell.update(cx, |shell, cx| shell.developer_preferences(window, cx)));
    release(cx, dest);
    assert_eq!(tab_ids(cx, &shell), ids);
    cx.update(|_, cx| assert_eq!(shell.read(cx).selected, Some(ids[1])));
    assert!(!cx.has_pending_prompt());
}
