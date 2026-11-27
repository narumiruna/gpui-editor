//! Ordered binding catalogs from `@codemirror/commands` 6.11.2.
//!
//! These catalogs describe command identities, not implementations. In particular,
//! visual character, group, syntax, line, and page motion must not be dispatched as
//! logical string motion. The host decides which commands it can run. See
//! `docs/licenses/codemirror-commands.txt` for the upstream MIT notice.

mod tab_focus;

use crate::view::keymap::KeyBinding;

pub use tab_focus::{TabFocusMode, temporarily_set_tab_focus_mode, toggle_tab_focus_mode};

macro_rules! command_ids {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        /// An upstream command referenced by at least one of these catalogs.
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum CommandId {
            $($variant),+
        }

        impl CommandId {
            /// The exact export name in `@codemirror/commands`.
            pub const fn upstream_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }
        }
    };
}

command_ids! {
    AddCursorAbove => "addCursorAbove",
    AddCursorBelow => "addCursorBelow",
    CopyLineDown => "copyLineDown",
    CopyLineUp => "copyLineUp",
    CursorCharLeft => "cursorCharLeft",
    CursorCharRight => "cursorCharRight",
    CursorDocEnd => "cursorDocEnd",
    CursorDocStart => "cursorDocStart",
    CursorGroupLeft => "cursorGroupLeft",
    CursorGroupRight => "cursorGroupRight",
    CursorLineBoundaryBackward => "cursorLineBoundaryBackward",
    CursorLineBoundaryForward => "cursorLineBoundaryForward",
    CursorLineBoundaryLeft => "cursorLineBoundaryLeft",
    CursorLineBoundaryRight => "cursorLineBoundaryRight",
    CursorLineDown => "cursorLineDown",
    CursorLineEnd => "cursorLineEnd",
    CursorLineStart => "cursorLineStart",
    CursorLineUp => "cursorLineUp",
    CursorMatchingBracket => "cursorMatchingBracket",
    CursorPageDown => "cursorPageDown",
    CursorPageUp => "cursorPageUp",
    CursorSyntaxLeft => "cursorSyntaxLeft",
    CursorSyntaxRight => "cursorSyntaxRight",
    DeleteCharBackward => "deleteCharBackward",
    DeleteCharForward => "deleteCharForward",
    DeleteGroupBackward => "deleteGroupBackward",
    DeleteGroupForward => "deleteGroupForward",
    DeleteLine => "deleteLine",
    DeleteLineBoundaryBackward => "deleteLineBoundaryBackward",
    DeleteLineBoundaryForward => "deleteLineBoundaryForward",
    DeleteToLineEnd => "deleteToLineEnd",
    IndentLess => "indentLess",
    IndentMore => "indentMore",
    IndentSelection => "indentSelection",
    InsertBlankLine => "insertBlankLine",
    InsertNewlineAndIndent => "insertNewlineAndIndent",
    MoveLineDown => "moveLineDown",
    MoveLineUp => "moveLineUp",
    Redo => "redo",
    RedoSelection => "redoSelection",
    SelectAll => "selectAll",
    SelectCharLeft => "selectCharLeft",
    SelectCharRight => "selectCharRight",
    SelectDocEnd => "selectDocEnd",
    SelectDocStart => "selectDocStart",
    SelectGroupLeft => "selectGroupLeft",
    SelectGroupRight => "selectGroupRight",
    SelectLine => "selectLine",
    SelectLineBoundaryBackward => "selectLineBoundaryBackward",
    SelectLineBoundaryForward => "selectLineBoundaryForward",
    SelectLineBoundaryLeft => "selectLineBoundaryLeft",
    SelectLineBoundaryRight => "selectLineBoundaryRight",
    SelectLineDown => "selectLineDown",
    SelectLineEnd => "selectLineEnd",
    SelectLineStart => "selectLineStart",
    SelectLineUp => "selectLineUp",
    SelectPageDown => "selectPageDown",
    SelectPageUp => "selectPageUp",
    SelectParentSyntax => "selectParentSyntax",
    SelectSyntaxLeft => "selectSyntaxLeft",
    SelectSyntaxRight => "selectSyntaxRight",
    SimplifySelection => "simplifySelection",
    SplitLine => "splitLine",
    ToggleBlockComment => "toggleBlockComment",
    ToggleComment => "toggleComment",
    ToggleTabFocusMode => "toggleTabFocusMode",
    TransposeChars => "transposeChars",
    Undo => "undo",
    UndoSelection => "undoSelection",
}

macro_rules! binding {
    ($($field:ident: $value:expr),+ $(,)?) => {
        KeyBinding {
            $($field: binding!(@value $field $value),)+
            ..KeyBinding::default()
        }
    };
    (@value key $value:expr) => { Some($value.into()) };
    (@value mac $value:expr) => { Some($value.into()) };
    (@value linux $value:expr) => { Some($value.into()) };
    (@value run $value:expr) => { Some($value) };
    (@value shift $value:expr) => { Some($value) };
    (@value prevent_default $value:expr) => { $value };
}

/// The platform-independent Emacs-style catalog. The standard catalog includes
/// a macOS-only projection of these entries.
pub fn emacs_style_keymap() -> Vec<KeyBinding<CommandId>> {
    use CommandId::*;
    vec![
        binding!(key: "Ctrl-b", run: CursorCharLeft, shift: SelectCharLeft, prevent_default: true),
        binding!(key: "Ctrl-f", run: CursorCharRight, shift: SelectCharRight),
        binding!(key: "Ctrl-p", run: CursorLineUp, shift: SelectLineUp),
        binding!(key: "Ctrl-n", run: CursorLineDown, shift: SelectLineDown),
        binding!(key: "Ctrl-a", run: CursorLineStart, shift: SelectLineStart),
        binding!(key: "Ctrl-e", run: CursorLineEnd, shift: SelectLineEnd),
        binding!(key: "Ctrl-d", run: DeleteCharForward),
        binding!(key: "Ctrl-h", run: DeleteCharBackward),
        binding!(key: "Ctrl-k", run: DeleteToLineEnd),
        binding!(key: "Ctrl-Alt-h", run: DeleteGroupBackward),
        binding!(key: "Ctrl-o", run: SplitLine),
        binding!(key: "Ctrl-t", run: TransposeChars),
        binding!(key: "Ctrl-v", run: CursorPageDown),
    ]
}

/// The standard catalog with platform overrides and the macOS-only Emacs entries.
pub fn standard_keymap() -> Vec<KeyBinding<CommandId>> {
    use CommandId::*;
    let mut bindings = vec![
        binding!(key: "ArrowLeft", run: CursorCharLeft, shift: SelectCharLeft, prevent_default: true),
        binding!(key: "Mod-ArrowLeft", mac: "Alt-ArrowLeft", run: CursorGroupLeft, shift: SelectGroupLeft, prevent_default: true),
        binding!(mac: "Cmd-ArrowLeft", run: CursorLineBoundaryLeft, shift: SelectLineBoundaryLeft, prevent_default: true),
        binding!(key: "ArrowRight", run: CursorCharRight, shift: SelectCharRight, prevent_default: true),
        binding!(key: "Mod-ArrowRight", mac: "Alt-ArrowRight", run: CursorGroupRight, shift: SelectGroupRight, prevent_default: true),
        binding!(mac: "Cmd-ArrowRight", run: CursorLineBoundaryRight, shift: SelectLineBoundaryRight, prevent_default: true),
        binding!(key: "ArrowUp", run: CursorLineUp, shift: SelectLineUp, prevent_default: true),
        binding!(mac: "Cmd-ArrowUp", run: CursorDocStart, shift: SelectDocStart),
        binding!(mac: "Ctrl-ArrowUp", run: CursorPageUp, shift: SelectPageUp),
        binding!(key: "ArrowDown", run: CursorLineDown, shift: SelectLineDown, prevent_default: true),
        binding!(mac: "Cmd-ArrowDown", run: CursorDocEnd, shift: SelectDocEnd),
        binding!(mac: "Ctrl-ArrowDown", run: CursorPageDown, shift: SelectPageDown),
        binding!(key: "PageUp", run: CursorPageUp, shift: SelectPageUp),
        binding!(key: "PageDown", run: CursorPageDown, shift: SelectPageDown),
        binding!(key: "Home", run: CursorLineBoundaryBackward, shift: SelectLineBoundaryBackward, prevent_default: true),
        binding!(key: "Mod-Home", run: CursorDocStart, shift: SelectDocStart),
        binding!(key: "End", run: CursorLineBoundaryForward, shift: SelectLineBoundaryForward, prevent_default: true),
        binding!(key: "Mod-End", run: CursorDocEnd, shift: SelectDocEnd),
        binding!(key: "Enter", run: InsertNewlineAndIndent, shift: InsertNewlineAndIndent),
        binding!(key: "Mod-a", run: SelectAll),
        binding!(key: "Backspace", run: DeleteCharBackward, shift: DeleteCharBackward, prevent_default: true),
        binding!(key: "Delete", run: DeleteCharForward, prevent_default: true),
        binding!(key: "Mod-Backspace", mac: "Alt-Backspace", run: DeleteGroupBackward, prevent_default: true),
        binding!(key: "Mod-Delete", mac: "Alt-Delete", run: DeleteGroupForward, prevent_default: true),
        binding!(mac: "Mod-Backspace", run: DeleteLineBoundaryBackward, prevent_default: true),
        binding!(mac: "Mod-Delete", run: DeleteLineBoundaryForward, prevent_default: true),
    ];
    bindings.extend(emacs_style_keymap().into_iter().map(|binding| KeyBinding {
        mac: binding.key,
        run: binding.run,
        shift: binding.shift,
        ..KeyBinding::default()
    }));
    bindings
}

/// The default catalog, with its additional commands ahead of the standard ones.
/// History and Tab indentation are separate opt-ins.
pub fn default_keymap() -> Vec<KeyBinding<CommandId>> {
    use CommandId::*;
    let mut bindings = vec![
        binding!(key: "Alt-ArrowLeft", mac: "Ctrl-ArrowLeft", run: CursorSyntaxLeft, shift: SelectSyntaxLeft),
        binding!(key: "Alt-ArrowRight", mac: "Ctrl-ArrowRight", run: CursorSyntaxRight, shift: SelectSyntaxRight),
        binding!(key: "Alt-ArrowUp", run: MoveLineUp),
        binding!(key: "Shift-Alt-ArrowUp", run: CopyLineUp),
        binding!(key: "Alt-ArrowDown", run: MoveLineDown),
        binding!(key: "Shift-Alt-ArrowDown", run: CopyLineDown),
        binding!(key: "Mod-Alt-ArrowUp", run: AddCursorAbove),
        binding!(key: "Mod-Alt-ArrowDown", run: AddCursorBelow),
        binding!(key: "Escape", run: SimplifySelection),
        binding!(key: "Mod-Enter", run: InsertBlankLine),
        binding!(key: "Alt-l", mac: "Ctrl-l", run: SelectLine),
        binding!(key: "Mod-i", run: SelectParentSyntax, prevent_default: true),
        binding!(key: "Mod-[", run: IndentLess),
        binding!(key: "Mod-]", run: IndentMore),
        binding!(key: "Mod-Alt-\\", run: IndentSelection),
        binding!(key: "Shift-Mod-k", run: DeleteLine),
        binding!(key: "Shift-Mod-\\", run: CursorMatchingBracket),
        binding!(key: "Mod-/", run: ToggleComment),
        binding!(key: "Alt-A", mac: "Ctrl-A", run: ToggleBlockComment),
        binding!(key: "Ctrl-m", mac: "Shift-Alt-m", run: ToggleTabFocusMode),
    ];
    bindings.extend(standard_keymap());
    bindings
}

/// The history catalog, including the selection-only undo and redo commands.
pub fn history_keymap() -> Vec<KeyBinding<CommandId>> {
    use CommandId::*;
    vec![
        binding!(key: "Mod-z", run: Undo, prevent_default: true),
        binding!(key: "Mod-y", mac: "Mod-Shift-z", run: Redo, prevent_default: true),
        binding!(linux: "Ctrl-Shift-z", run: Redo, prevent_default: true),
        binding!(key: "Mod-u", run: UndoSelection, prevent_default: true),
        binding!(key: "Alt-u", mac: "Mod-Shift-u", run: RedoSelection, prevent_default: true),
    ]
}

/// The opt-in Tab and Shift-Tab indentation binding. A host that installs this
/// should permit keyboard focus to leave the editor, for example with [`TabFocusMode`].
pub fn indent_with_tab() -> KeyBinding<CommandId> {
    binding!(key: "Tab", run: CommandId::IndentMore, shift: CommandId::IndentLess)
}
