//! Deliver accepted view announcements through AppKit, without changing focus.

use std::{cell::RefCell, rc::Rc};

use gpui::{App, AsyncWindowContext, Global, Window};
use gpui_editor_codemirror::view::{ViewPlugin, ViewUpdate};
use objc2::{MainThreadMarker, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{
    NSAccessibilityAnnouncementKey, NSAccessibilityAnnouncementRequestedNotification,
    NSAccessibilityPostNotificationWithUserInfo, NSAccessibilityPriorityKey,
    NSAccessibilityPriorityLevel, NSApplication,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString};

struct Delivery(Rc<dyn Fn(&str)>);

impl Global for Delivery {}

pub(crate) fn install_native_delivery(cx: &mut App) {
    cx.set_global(Delivery(Rc::new(post_announcement)));
}

pub(super) struct AnnouncementPlugin {
    window: AsyncWindowContext,
    pending: Rc<RefCell<Vec<String>>>,
    deliver: Rc<dyn Fn(&str)>,
}

impl AnnouncementPlugin {
    pub(super) fn new(window: &Window, cx: &App) -> Option<Self> {
        Some(Self {
            window: window.to_async(cx),
            pending: Rc::default(),
            deliver: cx.try_global::<Delivery>()?.0.clone(),
        })
    }
}

impl ViewPlugin for AnnouncementPlugin {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        let mut pending = self.pending.borrow_mut();
        let was_empty = pending.is_empty();
        pending.extend(update.announcements().map(str::to_owned));
        if !was_empty || pending.is_empty() {
            return;
        }
        // Drain in dispatch order after GPUI releases the current update. A
        // destroyed editor drops its queue before the task can deliver it.
        let queue = Rc::downgrade(&self.pending);
        let deliver = self.deliver.clone();
        self.window
            .spawn(async move |cx| {
                let Some(queue) = queue.upgrade() else {
                    return;
                };
                let messages = std::mem::take(&mut *queue.borrow_mut());
                let active = cx
                    .update(|window, _| window.is_window_active())
                    .unwrap_or(false);
                if active {
                    for message in messages {
                        deliver(&message);
                    }
                }
            })
            .detach();
    }
}

fn post_announcement(message: &str) {
    let Some(main) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(main);
    let info = announcement_info(message);
    // SAFETY: The SDK requires the application element on the main thread
    // and an NSString and NSNumber under the respective user-info keys.
    unsafe {
        NSAccessibilityPostNotificationWithUserInfo(
            &app,
            NSAccessibilityAnnouncementRequestedNotification,
            Some(&info),
        );
    }
}

fn announcement_info(message: &str) -> Retained<NSDictionary<NSString, AnyObject>> {
    let text = NSString::from_str(message);
    let priority = NSNumber::new_isize(NSAccessibilityPriorityLevel::Low.0);
    // SAFETY: These are NSString keys provided by AppKit.
    let keys = unsafe { [NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey] };
    NSDictionary::from_slices(&keys, &[&*text, &*priority])
}

#[cfg(test)]
mod tests;
