use crate::{
    EditorState,
    commands::{
        self as core, CommandResult, CommentMode, CommentStyle, Indentation, bindings::CommandId,
        visual,
    },
    language::{IndentOptions, get_indentation},
    language_data::{NativeSyntax, SupportedDocument, attached_language_snapshot},
    view::keymap::KeyPlatform,
};

use super::SetupGeometry;

pub(super) fn transaction(
    command: CommandId,
    state: &EditorState,
    platform: KeyPlatform,
    syntax: Option<&NativeSyntax>,
    visual_syntax: Option<&dyn visual::VisualSyntax>,
    geometry: Option<&SetupGeometry<'_>>,
    visual_selection: &mut Option<visual::VisualSelection>,
) -> CommandResult {
    use CommandId::*;

    if let Some(layout) = geometry {
        // Keep the old logical fallbacks only when no layout was supplied.
        let initial;
        let selection = match layout.selection {
            Some(selection) => selection,
            None => {
                initial = visual::VisualSelection::from_selection(state.selection());
                &initial
            }
        };
        let geometry = layout.geometry;
        let update = match command {
            CursorCharLeft => visual::cursor_char_left(state, geometry, selection),
            CursorCharRight => visual::cursor_char_right(state, geometry, selection),
            SelectCharLeft => visual::select_char_left(state, geometry, selection),
            SelectCharRight => visual::select_char_right(state, geometry, selection),
            CursorGroupLeft => visual::cursor_group_left(state, geometry, selection),
            CursorGroupRight => visual::cursor_group_right(state, geometry, selection),
            SelectGroupLeft => visual::select_group_left(state, geometry, selection),
            SelectGroupRight => visual::select_group_right(state, geometry, selection),
            CursorLineBoundaryLeft => visual::cursor_line_boundary_left(state, geometry, selection),
            CursorLineBoundaryRight => {
                visual::cursor_line_boundary_right(state, geometry, selection)
            }
            SelectLineBoundaryLeft => visual::select_line_boundary_left(state, geometry, selection),
            SelectLineBoundaryRight => {
                visual::select_line_boundary_right(state, geometry, selection)
            }
            CursorLineBoundaryBackward => {
                visual::cursor_line_boundary_backward(state, geometry, selection)
            }
            CursorLineBoundaryForward => {
                visual::cursor_line_boundary_forward(state, geometry, selection)
            }
            SelectLineBoundaryBackward => {
                visual::select_line_boundary_backward(state, geometry, selection)
            }
            SelectLineBoundaryForward => {
                visual::select_line_boundary_forward(state, geometry, selection)
            }
            CursorLineStart => visual::cursor_line_start(state, geometry, selection),
            CursorLineEnd => visual::cursor_line_end(state, geometry, selection),
            SelectLineStart => visual::select_line_start(state, geometry, selection),
            SelectLineEnd => visual::select_line_end(state, geometry, selection),
            CursorLineUp => visual::cursor_line_up(state, geometry, selection),
            CursorLineDown => visual::cursor_line_down(state, geometry, selection),
            SelectLineUp => visual::select_line_up(state, geometry, selection),
            SelectLineDown => visual::select_line_down(state, geometry, selection),
            CursorPageUp => visual::cursor_page_up(state, geometry, selection),
            CursorPageDown => visual::cursor_page_down(state, geometry, selection),
            SelectPageUp => visual::select_page_up(state, geometry, selection),
            SelectPageDown => visual::select_page_down(state, geometry, selection),
            AddCursorAbove => visual::add_cursor_above(state, geometry, selection),
            AddCursorBelow => visual::add_cursor_below(state, geometry, selection),
            CursorSyntaxLeft => {
                visual::cursor_syntax_left(state, geometry, selection, visual_syntax)
            }
            CursorSyntaxRight => {
                visual::cursor_syntax_right(state, geometry, selection, visual_syntax)
            }
            SelectSyntaxLeft => {
                visual::select_syntax_left(state, geometry, selection, visual_syntax)
            }
            SelectSyntaxRight => {
                visual::select_syntax_right(state, geometry, selection, visual_syntax)
            }
            DeleteLineBoundaryBackward => {
                return visual::delete_line_boundary_backward(state, geometry, selection);
            }
            DeleteLineBoundaryForward => {
                return visual::delete_line_boundary_forward(state, geometry, selection);
            }
            _ => return logical(command, state, platform, syntax, visual_syntax),
        }?;
        return Ok(update.map(|update| {
            *visual_selection = Some(update.selection);
            update.transaction
        }));
    }
    logical(command, state, platform, syntax, visual_syntax)
}

fn logical(
    command: CommandId,
    state: &EditorState,
    platform: KeyPlatform,
    syntax: Option<&NativeSyntax>,
    visual_syntax: Option<&dyn visual::VisualSyntax>,
) -> CommandResult {
    use CommandId::*;
    let windows = platform == KeyPlatform::Windows;
    let indent = Indentation::for_state(state);
    let indentation_at = |pos, _, options: IndentOptions<'_>| {
        get_indentation(state, pos, options, syntax).ok().flatten()
    };
    let edit = match command {
        MoveLineUp => core::move_line_up,
        CopyLineUp => core::copy_line_up,
        MoveLineDown => core::move_line_down,
        CopyLineDown => core::copy_line_down,
        SimplifySelection => core::simplify_selection,
        SelectLine => core::select_line,
        IndentLess => core::indent_less,
        IndentMore => core::indent_more,
        DeleteLine => core::delete_line,
        CursorMatchingBracket => core::cursor_matching_bracket,
        CursorCharLeft => core::cursor_char_backward_logical,
        SelectCharLeft => core::select_char_backward_logical,
        CursorGroupLeft => core::cursor_group_backward,
        SelectGroupLeft => core::select_group_backward,
        CursorLineBoundaryLeft | CursorLineBoundaryBackward => core::cursor_line_boundary_backward,
        SelectLineBoundaryLeft | SelectLineBoundaryBackward => core::select_line_boundary_backward,
        CursorCharRight => core::cursor_char_forward_logical,
        SelectCharRight => core::select_char_forward_logical,
        CursorGroupRight if windows => core::cursor_group_forward_win,
        SelectGroupRight if windows => core::select_group_forward_win,
        CursorGroupRight => core::cursor_group_forward,
        SelectGroupRight => core::select_group_forward,
        CursorLineBoundaryRight | CursorLineBoundaryForward => core::cursor_line_boundary_forward,
        SelectLineBoundaryRight | SelectLineBoundaryForward => core::select_line_boundary_forward,
        CursorLineUp => core::cursor_line_up,
        SelectLineUp => core::select_line_up,
        CursorDocStart => core::cursor_doc_start,
        SelectDocStart => core::select_doc_start,
        CursorLineDown => core::cursor_line_down,
        SelectLineDown => core::select_line_down,
        CursorDocEnd => core::cursor_doc_end,
        SelectDocEnd => core::select_doc_end,
        SelectAll => core::select_all,
        DeleteCharBackward => core::delete_char_backward,
        DeleteCharForward => core::delete_char_forward,
        DeleteGroupBackward => core::delete_group_backward,
        DeleteGroupForward if windows => core::delete_group_forward_win,
        DeleteGroupForward => core::delete_group_forward,
        DeleteLineBoundaryBackward => core::delete_to_line_start,
        DeleteLineBoundaryForward | DeleteToLineEnd => core::delete_to_line_end,
        CursorLineStart => core::cursor_line_start,
        SelectLineStart => core::select_line_start,
        CursorLineEnd => core::cursor_line_end,
        SelectLineEnd => core::select_line_end,
        SplitLine => core::split_line,
        TransposeChars => core::transpose_chars,
        InsertNewlineAndIndent => {
            let tracked = if syntax.is_none() {
                attached_language_snapshot(state)
            } else {
                None
            };
            let bracket_syntax =
                syntax.or_else(|| tracked.as_deref().map(SupportedDocument::syntax));
            return core::insert_newline_and_indent_with_ending(
                state,
                indent,
                indentation_at,
                |pos| bracket_syntax?.bracket_gap_for_indent(pos),
                &state.line_break(),
            );
        }
        InsertBlankLine => {
            return core::insert_blank_line(state, indent, indentation_at, &state.line_break());
        }
        IndentSelection => return core::indent_selection(state, indent, indentation_at),
        SelectParentSyntax => {
            return visual_syntax.map_or(Ok(None), |syntax| {
                core::select_parent_syntax(state, syntax.tree())
            });
        }
        ToggleComment | ToggleBlockComment => {
            return core::comment_with_language_data(
                state,
                CommentMode::Toggle,
                if command == ToggleComment {
                    CommentStyle::PreferLine
                } else {
                    CommentStyle::Block
                },
            );
        }
        _ => return Ok(None),
    };
    edit(state)
}
