use crate::view::keymap::KeyBinding;

/// Command IDs for the default completion, bracket, and snippet bindings.
/// The host calls the completion session or builds a bracket or snippet transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutocompleteCommand {
    StartCompletion,
    CloseCompletion,
    NextCompletion,
    PreviousCompletion,
    NextCompletionPage,
    PreviousCompletionPage,
    AcceptCompletion,
    DeleteBracketPair,
    NextSnippetField,
    PreviousSnippetField,
    ClearSnippet,
}

/// The upstream completion keymap, in binding order. The two Alt bindings
/// apply only to macOS. Page commands use the host's visible option count.
pub fn completion_keymap() -> [KeyBinding<AutocompleteCommand>; 9] {
    use AutocompleteCommand::*;

    let mac = |key: &str| KeyBinding {
        mac: Some(key.into()),
        run: Some(StartCompletion),
        ..KeyBinding::default()
    };
    [
        KeyBinding::new("Ctrl-Space", StartCompletion),
        mac("Alt-`"),
        mac("Alt-i"),
        KeyBinding::new("Escape", CloseCompletion),
        KeyBinding::new("ArrowDown", NextCompletion),
        KeyBinding::new("ArrowUp", PreviousCompletion),
        KeyBinding::new("PageDown", NextCompletionPage),
        KeyBinding::new("PageUp", PreviousCompletionPage),
        KeyBinding::new("Enter", AcceptCompletion),
    ]
}

/// The upstream close-brackets keymap.
pub fn close_brackets_keymap() -> [KeyBinding<AutocompleteCommand>; 1] {
    [KeyBinding::new(
        "Backspace",
        AutocompleteCommand::DeleteBracketPair,
    )]
}

/// The upstream default snippet keymap. A host can use a different catalog.
pub fn snippet_keymap() -> [KeyBinding<AutocompleteCommand>; 2] {
    use AutocompleteCommand::*;

    [
        KeyBinding {
            shift: Some(PreviousSnippetField),
            ..KeyBinding::new("Tab", NextSnippetField)
        },
        KeyBinding::new("Escape", ClearSnippet),
    ]
}
