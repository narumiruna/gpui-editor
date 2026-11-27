use gpui::TestAppContext;
use gpui_editor_codemirror::{EditError, Transaction, transaction_filter, view::ANNOUNCE};

use super::*;
use crate::editor::{Editor, bind_keys};

#[test]
fn appkit_payload_keeps_literal_text_and_low_priority() {
    let message = "<a>字 😀</a>";
    let info = announcement_info(message);
    // SAFETY: Both keys are NSString constants from the linked AppKit SDK.
    let (announcement_key, priority_key) =
        unsafe { (NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey) };
    let text = info
        .objectForKey(announcement_key)
        .unwrap()
        .downcast::<NSString>()
        .unwrap();
    let priority = info
        .objectForKey(priority_key)
        .unwrap()
        .downcast::<NSNumber>()
        .unwrap();
    assert_eq!(text.to_string(), message);
    assert_eq!(priority.integerValue(), NSAccessibilityPriorityLevel::Low.0);
}

#[gpui::test]
fn deletion_speaks_only_after_an_accepted_nonempty_selection(cx: &mut TestAppContext) {
    let spoken = Rc::new(RefCell::new(Vec::<String>::new()));
    let (editor, cx) = cx.add_window_view(|window, cx| {
        cx.set_global(Delivery(Rc::new({
            let spoken = spoken.clone();
            move |message| spoken.borrow_mut().push(message.to_owned())
        })));
        Editor::new("abcdef".into(), window, cx)
    });
    cx.update(|window, cx| {
        bind_keys(cx);
        window.activate_window();
        editor.update(cx, |editor, _| {
            window.focus(&editor.focus);
            editor.set_selection(2, 4);
        });
    });
    cx.simulate_keystrokes("backspace");
    assert_eq!(*spoken.borrow(), ["Selection deleted"]);
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "abef"));
    cx.simulate_keystrokes("backspace");
    assert_eq!(*spoken.borrow(), ["Selection deleted"]);
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), "aef"));
}

#[gpui::test]
fn active_editor_delivers_only_accepted_announcements_in_order(cx: &mut TestAppContext) {
    let spoken = Rc::new(RefCell::new(Vec::<String>::new()));
    let (editor, cx) = cx.add_window_view(|window, cx| {
        cx.set_global(Delivery(Rc::new({
            let spoken = spoken.clone();
            move |message| spoken.borrow_mut().push(message.to_owned())
        })));
        Editor::with_native_extensions(
            "Draft".into(),
            transaction_filter(|_, transaction| {
                Ok(if transaction.is_user_event("test.filtered") {
                    Transaction::default()
                } else {
                    transaction
                })
            }),
            window,
            cx,
        )
        .unwrap()
    });
    cx.update(|window, cx| {
        window.activate_window();
        editor.update(cx, |editor, _| {
            let commit = |messages: &[&str]| {
                Transaction::default()
                    .with_effects(messages.iter().map(|text| ANNOUNCE.of((*text).to_owned())))
            };
            assert_eq!(
                editor.view.update_checked(
                    commit(&["Rejected"]),
                    None,
                    |_| Err(EditError::InvalidRange),
                    |_| (None, None),
                ),
                Err(EditError::InvalidRange)
            );
            for transaction in [
                commit(&["Filtered"]).with_user_event("test.filtered"),
                commit(&["<left> 😀", "repeat"]),
                commit(&["repeat", "last"]),
            ] {
                editor
                    .view
                    .update(transaction, None, |_| (None, None))
                    .unwrap();
            }
            assert!(spoken.borrow().is_empty());
            assert_eq!(editor.source(), "Draft");
            assert!(!editor.view.can_undo());
        });
    });
    cx.run_until_parked();
    assert_eq!(*spoken.borrow(), ["<left> 😀", "repeat", "repeat", "last"]);
    cx.deactivate_window();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor
                .view
                .update(
                    Transaction::default().with_effects([ANNOUNCE.of("Inactive".to_owned())]),
                    None,
                    |_| (None, None),
                )
                .unwrap();
            cx.notify();
        });
    });
    cx.run_until_parked();
    assert_eq!(*spoken.borrow(), ["<left> 😀", "repeat", "repeat", "last"]);
    cx.update(|window, cx| {
        window.activate_window();
        editor.update(cx, |editor, _| {
            editor
                .view
                .update(
                    Transaction::default().with_effects([ANNOUNCE.of("Closing".to_owned())]),
                    None,
                    |_| (None, None),
                )
                .unwrap();
        });
        window.remove_window();
    });
    cx.run_until_parked();
    assert_eq!(*spoken.borrow(), ["<left> 😀", "repeat", "repeat", "last"]);
}
