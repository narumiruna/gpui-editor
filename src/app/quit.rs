//! Route AppKit quit requests (including the Dock menu) through the GPUI save guard.

use std::{cell::RefCell, ffi::c_ulong, mem};

use gpui::{App, AsyncApp};
use objc2::{
    MainThreadMarker, class, ffi, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel,
};

use super::Quit;

struct QuitGuard {
    app: AsyncApp,
    approved: bool,
}

thread_local! {
    static GUARD: RefCell<Option<QuitGuard>> = const { RefCell::new(None) };
}

pub(super) fn install(cx: &App) -> Result<(), &'static str> {
    let _main = MainThreadMarker::new().ok_or("expected the macOS main thread")?;
    // AppKit is loaded by GPUI. GPUI owns the delegate, so only add the callback
    // that its macOS platform does not implement. Do not replace the delegate.
    let delegate: Option<Retained<AnyObject>> = unsafe {
        let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
        msg_send![&*app, delegate]
    };
    let delegate = delegate.ok_or("AppKit has no application delegate")?;
    let class: *mut AnyClass = std::ptr::from_ref(delegate.class()).cast_mut();
    let selector = sel!(applicationShouldTerminate:);
    // SAFETY: the receiver is GPUI's Objective-C delegate class on the main
    // thread. The callback matches AppKit's NSUInteger(id, SEL, id) signature
    // on 64-bit macOS. class_addMethod does not replace an existing callback.
    let added = unsafe {
        let callback: Imp = mem::transmute::<
            unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> c_ulong,
            Imp,
        >(should_terminate);
        ffi::class_addMethod(class, selector, callback, c"Q@:@".as_ptr())
    };
    if !added.as_bool() {
        return Err("GPUI's AppKit delegate already has a quit handler");
    }
    GUARD.with_borrow_mut(|guard| {
        *guard = Some(QuitGuard {
            app: cx.to_async(),
            approved: false,
        });
    });
    Ok(())
}

pub(super) fn after_confirmation(cx: &App) {
    GUARD.with_borrow_mut(|guard| {
        if let Some(guard) = guard {
            guard.approved = true;
        }
    });
    cx.quit();
}

unsafe extern "C-unwind" fn should_terminate(
    _: *mut AnyObject,
    _: Sel,
    _: *mut AnyObject,
) -> c_ulong {
    let app = GUARD.with_borrow(|guard| {
        guard
            .as_ref()
            .filter(|guard| !guard.approved)
            .map(|guard| guard.app.clone())
    });
    match app {
        Some(app) => {
            // Return NSTerminateCancel now. The ordinary GPUI Quit action can
            // show a dialog and issue a new, approved quit without reentrancy.
            app.foreground_executor()
                .spawn({
                    let app = app.clone();
                    async move {
                        let _ = app.update(|cx| cx.dispatch_action(&Quit));
                    }
                })
                .detach();
            0
        }
        None => 1, // NSTerminateNow
    }
}
