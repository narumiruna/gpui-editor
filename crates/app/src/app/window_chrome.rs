//! Native title bar and window controls.

use gpui::{Div, Pixels, TitlebarOptions, Window, div, px};
#[cfg(target_os = "macos")]
use gpui::{MouseButton, point, prelude::*};
#[cfg(target_os = "macos")]
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
#[cfg(target_os = "macos")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub(super) const TOP_ROW_HEIGHT: Pixels = px(36.);

#[cfg(target_os = "macos")]
pub(super) fn titlebar() -> TitlebarOptions {
    TitlebarOptions {
        title: None,
        appears_transparent: true,
        traffic_light_position: Some(point(px(10.), px(12.))),
    }
}

#[cfg(target_os = "windows")]
pub(super) fn titlebar() -> TitlebarOptions {
    TitlebarOptions {
        title: Some("GPUI Editor".into()),
        appears_transparent: false,
        traffic_light_position: None,
    }
}

pub(super) fn controls_width(window: &Window) -> Pixels {
    if !cfg!(target_os = "macos") || window.is_fullscreen() {
        px(0.)
    } else {
        px(78.)
    }
}

#[cfg(target_os = "windows")]
pub(super) fn drag_region() -> Div {
    div()
}

#[cfg(target_os = "macos")]
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
