use crate::{
    autocomplete::{AutocompleteCommand, close_brackets_keymap, completion_keymap},
    commands::bindings::{CommandId, default_keymap, history_keymap},
    language::{
        fold::{FoldCommand, fold_keymap},
        markdown::MarkdownMarkupCommand,
    },
    lint::{LintCommand, lint_keymap},
    search::{SearchAction, search_keymap},
    view::keymap::KeyBinding,
};

/// Command identities in the setup keymaps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupCommand {
    Editor(CommandId),
    Search(SearchAction),
    Completion(AutocompleteCommand),
    Fold,
    Unfold,
    FoldAll,
    UnfoldAll,
    Lint(LintCommand),
    Markdown(MarkdownMarkupCommand),
}

/// Effective binding order for `basicSetup`. The first completion group is
/// installed at highest precedence by `autocompletion()` itself. The second
/// is part of the literal setup keymap. Dispatch skips duplicate commands.
pub fn basic_setup_keymap() -> Vec<KeyBinding<SetupCommand>> {
    highest_setup_bindings()
        .chain(basic_setup_bindings())
        .collect()
}

pub(super) fn highest_setup_bindings() -> impl Iterator<Item = KeyBinding<SetupCommand>> {
    completion_keymap()
        .into_iter()
        .map(|key| map(key, SetupCommand::Completion))
}

pub(super) fn basic_setup_bindings() -> impl Iterator<Item = KeyBinding<SetupCommand>> {
    use SetupCommand::*;
    close_brackets_keymap()
        .into_iter()
        .map(|key| map(key, Completion))
        .chain(default_keymap().into_iter().map(|key| map(key, Editor)))
        .chain(search_keymap().into_iter().map(|key| map(key, Search)))
        .chain(history_keymap().into_iter().map(|key| map(key, Editor)))
        .chain(fold_keymap().into_iter().map(|key| {
            map(key, |command| match command {
                FoldCommand::FoldCode => Fold,
                FoldCommand::UnfoldCode => Unfold,
                FoldCommand::FoldAll => FoldAll,
                FoldCommand::UnfoldAll => UnfoldAll,
            })
        }))
        .chain(
            completion_keymap()
                .into_iter()
                .map(|key| map(key, Completion)),
        )
        .chain(lint_keymap().into_iter().map(|key| map(key, Lint)))
}

/// Effective binding order for `minimalSetup`.
pub fn minimal_setup_keymap() -> Vec<KeyBinding<SetupCommand>> {
    default_keymap()
        .into_iter()
        .chain(history_keymap())
        .map(|key| map(key, SetupCommand::Editor))
        .collect()
}

fn map<C>(key: KeyBinding<C>, convert: impl Fn(C) -> SetupCommand) -> KeyBinding<SetupCommand> {
    KeyBinding {
        key: key.key,
        mac: key.mac,
        windows: key.windows,
        linux: key.linux,
        scope: key.scope,
        run: key.run.map(&convert),
        shift: key.shift.map(&convert),
        any: key.any.map(convert),
        prevent_default: key.prevent_default,
        stop_propagation: key.stop_propagation,
    }
}
