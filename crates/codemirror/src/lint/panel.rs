use std::sync::LazyLock;

use crate::{EditorState, StateEffectType, Transaction, view::keymap::KeyBinding};

use super::{LintState, enable_lint, next_diagnostic};

pub(super) static TOGGLE_PANEL: LazyLock<StateEffectType<bool>> =
    LazyLock::new(StateEffectType::new);

/// Open the lint panel, installing lint state if needed. Return `None` if it
/// is already open. The host can focus the panel in both cases.
pub fn open_lint_panel(state: &EditorState) -> Option<Transaction> {
    if LintState::get(state).is_some_and(|lint| lint.panel_open()) {
        return None;
    }
    Some(enable_lint(
        state,
        Transaction::default().with_effects([TOGGLE_PANEL.of(true)]),
    ))
}

/// Close the lint panel, or return `None` if it is not open.
pub fn close_lint_panel(state: &EditorState) -> Option<Transaction> {
    LintState::get(state)?
        .panel_open()
        .then(|| Transaction::default().with_effects([TOGGLE_PANEL.of(false)]))
}

/// Commands used by the default lint keymap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LintCommand {
    OpenPanel,
    NextDiagnostic,
}

impl LintCommand {
    /// Build the state update. The host handles `OpenPanel` even if no
    /// transaction is necessary to focus an open panel.
    pub fn transaction(self, state: &EditorState) -> Option<Transaction> {
        match self {
            Self::OpenPanel => open_lint_panel(state),
            Self::NextDiagnostic => next_diagnostic(state),
        }
    }
}

/// The default editor bindings. The host dispatches the returned commands.
pub fn lint_keymap() -> [KeyBinding<LintCommand>; 2] {
    [
        KeyBinding {
            prevent_default: true,
            ..KeyBinding::new("Mod-Shift-m", LintCommand::OpenPanel)
        },
        KeyBinding::new("F8", LintCommand::NextDiagnostic),
    ]
}
