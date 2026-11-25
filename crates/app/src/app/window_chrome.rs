//! Native traffic lights and dragging, embedded in the existing top row.

use gpui::{Div, MouseButton, Pixels, Window, div, prelude::*, px};
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub(super) const TOP_ROW_HEIGHT: Pixels = px(36.);

pub(super) fn controls_width(window: &Window) -> Pixels {
    if window.is_fullscreen() {
        px(0.)
    } else {
        px(78.)
    }
}

pub(super) fn drag_region() -> Div {
    div().on_mouse_down(MouseButton::Left, |event, window, _| {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };
        // GPUI dispatches this on the main thread while the AppKit mouse-down
        // event and the view in the borrowed handle are still live.
        unsafe {
            let native: Option<Retained<AnyObject>> =
                msg_send![handle.ns_view.cast::<AnyObject>().as_ptr(), window];
            let Some(native) = native else { return };
            if event.click_count == 2 {
                window.zoom_window();
            } else {
                let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
                let event: Option<Retained<AnyObject>> = msg_send![&*app, currentEvent];
                if let Some(event) = event {
                    let _: () = msg_send![&*native, performWindowDragWithEvent: &*event];
                }
            }
        }
    })
}
