//! Native commands based on `@codemirror/commands` 6.11.2.
//!
//! Commands build a transaction but do not apply it. The caller checks whether
//! edits are allowed, then applies the transaction through the state or history.
//! Positions are UTF-8 bytes. Motions use Unicode grapheme boundaries in
//! logical order. A line break is LF or CRLF. When a command inserts a line
//! break, it uses the break after the source line, then the break before it,
//! then LF. Commands do not use visual layout or atomic view ranges. Syntax-aware
//! commands take the required syntax or language information from the caller.
//!
//! See `docs/licenses/codemirror-commands.txt` for the upstream MIT notice.

pub mod bindings;
mod comment;
mod delete;
mod indent;
mod lines;
pub(crate) mod logical;
mod motion;
mod subword;
mod syntax_indent;
pub mod visual;

use std::ops::Range;

use unicode_segmentation::GraphemeCursor;

use crate::{
    Change, ChangeSet, EditError, EditorSelection, EditorState, Line, RangeEdit, SelectionRange,
    Text, Transaction,
};

pub use comment::{
    CommentMode, CommentStyle, CommentTokens, comment, comment_with_language_data,
    comment_with_language_data_in_lines,
};
pub use delete::{
    announce_selection_deleted, delete_char_backward, delete_char_backward_strict,
    delete_char_backward_with, delete_char_forward, delete_group_backward, delete_group_forward,
    delete_group_forward_win, delete_to_line_end, delete_to_line_start, delete_trailing_whitespace,
};
pub use indent::{
    IndentUnit, Indentation, indent_less, indent_less_with, indent_more, indent_more_with,
    insert_tab, insert_tab_with,
};
pub use lines::{
    copy_line_down, copy_line_up, copy_line_with_ending, copy_line_with_projection, delete_line,
    insert_newline, insert_newline_keep_indent, move_line_down, move_line_up,
    move_line_with_projection, select_line, split_line, transpose_chars,
};
pub use logical::{cursor_matching_bracket, select_matching_bracket, select_parent_syntax};
pub use motion::{
    cursor_char_backward_logical, cursor_char_forward_logical, cursor_doc_end, cursor_doc_start,
    cursor_group_backward, cursor_group_forward, cursor_group_forward_win,
    cursor_line_boundary_backward, cursor_line_boundary_forward, cursor_line_down, cursor_line_end,
    cursor_line_start, cursor_line_up, select_all, select_char_backward_logical,
    select_char_forward_logical, select_doc_end, select_doc_start, select_group_backward,
    select_group_forward, select_group_forward_win, select_line_boundary_backward,
    select_line_boundary_forward, select_line_down, select_line_end, select_line_start,
    select_line_up, simplify_selection,
};
pub use subword::{
    cursor_subword_backward, cursor_subword_forward, select_subword_backward,
    select_subword_forward,
};
pub use syntax_indent::{
    indent_selection, indent_selection_with_projection, insert_blank_line,
    insert_blank_line_with_projection, insert_newline_and_indent,
    insert_newline_and_indent_with_ending, insert_newline_and_indent_with_projection,
};

/// A transaction to apply, `None` when the command is not handled, or an edit error.
/// A handled command can return a transaction without text changes.
pub type CommandResult = Result<Option<Transaction>, EditError>;

#[derive(Clone, Copy)]
enum Direction {
    Backward,
    Forward,
}

use Direction::{Backward, Forward};

impl Direction {
    fn edge(self, range: &Range<usize>) -> usize {
        match self {
            Backward => range.start,
            Forward => range.end,
        }
    }

    fn step(self, source: &str, cursor: &mut GraphemeCursor) -> usize {
        let next = match self {
            Backward => cursor.prev_boundary(source, 0),
            Forward => cursor.next_boundary(source, 0),
        };
        next.expect("the cursor has the entire source")
            .unwrap_or_else(|| cursor.cur_cursor())
    }

    fn grapheme(self, source: &str, position: usize) -> usize {
        self.step(
            source,
            &mut GraphemeCursor::new(position, source.len(), true),
        )
    }
}

fn caret(pos: usize) -> SelectionRange {
    SelectionRange::new(pos, pos)
}

fn line_at(text: &Text, pos: usize) -> Line<'_> {
    text.line_at(pos).expect("selection positions are valid")
}

fn selection_by(
    state: &EditorState,
    by: impl FnMut(SelectionRange) -> SelectionRange,
) -> Result<EditorSelection, EditError> {
    let ranges = state.selection().ranges().iter().copied().map(by).collect();
    EditorSelection::new(ranges, state.selection().main_index())
}

fn select(selection: EditorSelection, scroll: bool) -> Transaction {
    let transaction = Transaction::default()
        .with_selection(selection)
        .with_user_event("select");
    if scroll {
        transaction.with_scroll_into_view()
    } else {
        transaction
    }
}

fn range_edit(changes: ChangeSet, selection: SelectionRange) -> RangeEdit {
    RangeEdit {
        changes,
        selection,
        effects: Vec::new(),
    }
}

fn replace_range(
    range: Range<usize>,
    text: impl Into<String>,
    pos: usize,
) -> Result<RangeEdit, EditError> {
    Ok(range_edit(
        ChangeSet::single(Change::new(range, text))?,
        caret(pos),
    ))
}

fn unchanged(selection: SelectionRange) -> RangeEdit {
    range_edit(ChangeSet::default(), selection)
}

fn break_after(source: &str, end: usize) -> &str {
    let tail = &source[end..];
    if tail.starts_with("\r\n") {
        "\r\n"
    } else if tail.starts_with('\n') {
        "\n"
    } else {
        ""
    }
}

fn break_before(source: &str, start: usize) -> &str {
    let prefix = &source[..start];
    if prefix.ends_with("\r\n") {
        "\r\n"
    } else if prefix.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

fn inside_crlf(source: &str, pos: usize) -> bool {
    pos > 0 && source.as_bytes().get(pos - 1..pos + 1) == Some(b"\r\n")
}

fn include_crlf(source: &str, range: Range<usize>) -> Range<usize> {
    range.start - usize::from(inside_crlf(source, range.start))
        ..range.end + usize::from(inside_crlf(source, range.end))
}

fn space(ch: char) -> bool {
    // ECMAScript whitespace excludes NEL and includes BOM.
    (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}'
}

fn leading_space(text: &str) -> &str {
    &text[..text.len() - text.trim_start_matches(space).len()]
}
