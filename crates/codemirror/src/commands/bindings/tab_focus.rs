use std::time::Duration;

use crate::view::keymap::KeyEvent;

const TEMPORARY_WINDOW: Duration = Duration::from_secs(2);

/// Per-editor Tab focus state from `@codemirror/view` 6.44.0. The host owns the
/// clock and focus traversal. Use the same monotonic clock for all calls.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TabFocusMode {
    #[default]
    Disabled,
    Persistent,
    TemporaryUntil(Duration),
}

impl TabFocusMode {
    /// Call before key dispatch. If this returns true, skip the editor's key
    /// handlers and let the host process Tab. Other non-modifier keys cancel
    /// temporary mode, except Escape.
    pub fn bypass_key_bindings(&mut self, event: KeyEvent<'_>, now: Duration) -> bool {
        if event.name == "Tab"
            && match *self {
                Self::Disabled => false,
                Self::Persistent => true,
                Self::TemporaryUntil(deadline) => now <= deadline,
            }
        {
            return true;
        }
        if matches!(self, Self::TemporaryUntil(_))
            && !event.modifier_only
            && !matches!(event.name, "Escape" | "CapsLock" | "AltGraph" | "OS")
        {
            *self = Self::Disabled;
        }
        false
    }

    /// Call after dispatch if no handler prevented the key's default action.
    /// An unhandled Escape starts or renews temporary mode.
    pub fn handle_unhandled_key(&mut self, event: KeyEvent<'_>, now: Duration) {
        if event.name == "Escape" {
            temporarily_set_tab_focus_mode(self, now);
        }
    }
}

/// Toggle persistent Tab focus mode. Report the command as handled.
/// This turns off a stored temporary mode even if its deadline has passed.
pub fn toggle_tab_focus_mode(mode: &mut TabFocusMode) -> bool {
    *mode = match mode {
        TabFocusMode::Disabled => TabFocusMode::Persistent,
        _ => TabFocusMode::Disabled,
    };
    true
}

/// Let Tab leave the editor for two seconds unless another non-modifier key
/// cancels it first. Leave persistent mode on. Report the command as handled.
pub fn temporarily_set_tab_focus_mode(mode: &mut TabFocusMode, now: Duration) -> bool {
    if *mode != TabFocusMode::Persistent {
        *mode = TabFocusMode::TemporaryUntil(now.saturating_add(TEMPORARY_WINDOW));
    }
    true
}
