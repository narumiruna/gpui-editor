//! Oracle: @codemirror/commands 6.11.2 (2d2511920a7201ba65e3a9d86e5bd941aef8dc25)
//! with @codemirror/state 6.7.6, @codemirror/view 6.44.0, and
//! @codemirror/language 6.13.1. See `docs/licenses/codemirror-commands.txt`.
//!
//! Run the named export on `{state, dispatch}` for state commands and on a
//! real `EditorView` for view commands. The reference view ran in headless
//! Chrome, with monospace text, no wrapping, and no atomic ranges. Enable
//! `EditorState.allowMultipleSelections`; for CRLF input set
//! `EditorState.lineSeparator.of("\r\n")`. The tab fixtures use `indentUnit.of("\t")`
//! and `EditorState.tabSize.of(8)`. Compare return values, the dispatched
//! transaction's `Transaction.userEvent` and `scrollIntoView`, and the final
//! `state.sliceDoc()`. Convert positions using the UTF-8 byte length of
//! `state.sliceDoc(0, pos)` in the respective input or output state.
//! This suite compares selection endpoints and the main index. Other suites
//! check cursor association, undirectional ranges, and retained columns.

use std::num::NonZeroUsize;

use gpui_editor_codemirror::commands::*;
use gpui_editor_codemirror::{EditorSelection, EditorState, History, SelectionRange, Transaction};

type Snapshot<'a> = (&'a str, &'a [(usize, usize)], usize);
type Command = fn(&EditorState) -> CommandResult;

fn state_from((source, ranges, main): Snapshot<'_>) -> EditorState {
    let mut state = EditorState::new(source);
    let ranges = ranges
        .iter()
        .map(|&(a, h)| SelectionRange::new(a, h))
        .collect();
    state
        .update(Transaction::default().with_selection(EditorSelection::new(ranges, main).unwrap()))
        .unwrap();
    state
}

fn assert_state(state: &EditorState, (source, ranges, main): Snapshot<'_>) {
    assert_eq!(state.text().as_str(), source);
    let actual = state
        .selection()
        .ranges()
        .iter()
        .map(|r| (r.anchor(), r.head()))
        .collect::<Vec<_>>();
    assert_eq!(actual, ranges);
    assert_eq!(state.selection().main_index(), main);
}

#[track_caller]
fn check(command: Command, before: Snapshot<'_>, after: Snapshot<'_>, meta: Option<(&str, bool)>) {
    let mut state = state_from(before);
    let transaction = command(&state).unwrap();
    assert_state(&state, before);
    assert_eq!(
        transaction.is_some(),
        meta.is_some(),
        "command handled state"
    );
    if let (Some(transaction), Some((event, scroll))) = (transaction, meta) {
        assert_eq!(transaction.user_event(), Some(event));
        assert_eq!(transaction.scroll_into_view(), scroll);
        state.update(transaction).unwrap();
    }
    assert_state(&state, after);
}

const TABS: Indentation = Indentation {
    unit: IndentUnit::Tab,
    tab_size: NonZeroUsize::new(8).unwrap(),
};

macro_rules! motion {
    ($command:ident, $text:literal, $before:expr, $after:expr) => {
        check(
            $command,
            ($text, &$before.0, $before.1),
            ($text, &$after.0, $after.1),
            Some(("select", true)),
        );
    };
}

#[test]
#[rustfmt::skip]
fn oracle_logical_grapheme_motion() {
    motion!(cursor_char_forward_logical, "éx\r\n¢👩‍💻y", ([(0, 0), (8, 8)], 1), ([(3, 3), (19, 19)], 1));
    motion!(cursor_char_forward_logical, "a\r\nb", ([(1, 1)], 0), ([(3, 3)], 0));
    motion!(cursor_char_forward_logical, "éz$", ([(3, 0)], 0), ([(3, 3)], 0));
    motion!(cursor_char_backward_logical, "a\r\né", ([(3, 3)], 0), ([(1, 1)], 0));
    motion!(cursor_char_backward_logical, "aé", ([(3, 3)], 0), ([(1, 1)], 0));
    motion!(cursor_char_backward_logical, "abc", ([(0, 2)], 0), ([(0, 0)], 0));
    motion!(select_char_forward_logical, "aéxz", ([(4, 1)], 0), ([(4, 3)], 0));
    motion!(select_char_backward_logical, "a🇺🇳z", ([(1, 10)], 0), ([(1, 9)], 0));
    check(select_char_backward_logical, ("é", &[(0, 0)], 0), ("é", &[(0, 0)], 0), None);
}

#[test]
#[rustfmt::skip]
fn oracle_logical_group_motion() {
    motion!(cursor_group_forward, "one \r\n\télan...z", ([(3, 3)], 0), ([(12, 12)], 0));
    motion!(cursor_group_forward, "x...two", ([(1, 1)], 0), ([(4, 4)], 0));
    motion!(cursor_group_backward, "abc élan \t foo", ([(12, 12)], 0), ([(4, 4)], 0));
    motion!(select_group_forward, "xy  two", ([(1, 2)], 0), ([(1, 7)], 0));
    motion!(select_group_backward, "abc, !foo", ([(6, 6)], 0), ([(6, 5)], 0));
    motion!(cursor_group_forward_win, "one \t  two", ([(0, 0)], 0), ([(7, 7)], 0));
    motion!(select_group_forward_win, "x\t  two", ([(1, 1)], 0), ([(1, 4)], 0));
}

#[test]
#[rustfmt::skip]
fn oracle_hard_line_motion() {
    motion!(cursor_line_boundary_forward, "  word", ([(2, 2)], 0), ([(6, 6)], 0));
    motion!(cursor_line_boundary_backward, "  word", ([(4, 4)], 0), ([(2, 2)], 0));
    motion!(cursor_line_boundary_backward, "  word", ([(2, 2)], 0), ([(0, 0)], 0));
    motion!(select_line_boundary_forward, "  word", ([(2, 2)], 0), ([(2, 6)], 0));
    motion!(select_line_boundary_backward, "    word", ([(7, 7)], 0), ([(7, 4)], 0));
    motion!(cursor_line_start, "a\r\n  éx", ([(7, 7)], 0), ([(3, 3)], 0));
    motion!(cursor_line_end, "a\r\n  éx", ([(7, 7)], 0), ([(8, 8)], 0));
    motion!(select_line_start, "abc\r\n  text", ([(7, 7)], 0), ([(7, 5)], 0));
    motion!(select_line_end, "abc", ([(1, 1)], 0), ([(1, 3)], 0));
    motion!(cursor_line_down, "one\nt", ([(2, 2)], 0), ([(5, 5)], 0));
    motion!(cursor_line_up, "one\ntwo", ([(6, 6)], 0), ([(2, 2)], 0));
    motion!(select_line_down, "one\ntwo", ([(6, 6)], 0), ([(6, 7)], 0));
    motion!(select_line_up, "one\ntwo", ([(6, 6)], 0), ([(6, 2)], 0));
}

#[test]
#[rustfmt::skip]
fn oracle_document_selection() {
    motion!(cursor_doc_start, "abcd", ([(0, 0), (2, 2)], 1), ([(0, 0)], 0));
    motion!(cursor_doc_end, "abcd", ([(4, 4)], 0), ([(4, 4)], 0));
    motion!(select_doc_start, "abcdéf", ([(0, 0), (6, 3)], 1), ([(6, 0)], 0));
    motion!(select_doc_end, "abcdéf", ([(0, 0), (6, 3)], 1), ([(6, 7)], 0));
    check(select_all, ("abcé", &[(3, 3)], 0), ("abcé", &[(0, 5)], 0), Some(("select", false)));
    motion!(simplify_selection, "abcd", ([(0, 0), (3, 1)], 1), ([(3, 1)], 0));
    motion!(simplify_selection, "abcd", ([(3, 1)], 0), ([(1, 1)], 0));
    check(simplify_selection, ("ab", &[(1, 1)], 0), ("ab", &[(1, 1)], 0), None);
}

#[test]
#[rustfmt::skip]
fn oracle_whole_lines() {
    check(select_line, ("abc\r\nxy\r\nc\r\nlast", &[(1, 1), (9, 5), (16, 16)], 2), ("abc\r\nxy\r\nc\r\nlast", &[(0, 9), (12, 16)], 0), Some(("select", false)));
    check(select_line, ("", &[(0, 0)], 0), ("", &[(0, 0)], 0), Some(("select", false)));
    check(delete_line, ("abcd\nnext\nlast", &[(2, 2)], 0), ("next\nlast", &[(2, 2)], 0), Some(("delete.line", true)));
    check(delete_line, ("first\r\nline\r\nx\r\nlast", &[(9, 9), (20, 20)], 1), ("first\r\nx", &[(8, 8)], 0), Some(("delete.line", true)));
    check(delete_line, ("last", &[(4, 4)], 0), ("", &[(0, 0)], 0), Some(("delete.line", true)));
    check(delete_line, ("", &[(0, 0)], 0), ("", &[(0, 0)], 0), Some(("delete.line", true)));
}

#[test]
#[rustfmt::skip]
fn oracle_group_and_line_deletion() {
    check(delete_group_forward, ("one two three", &[(3, 3)], 0), ("one three", &[(3, 3)], 0), Some(("delete.forward", true)));
    check(delete_group_forward, ("one  \ttwo", &[(3, 3)], 0), ("onetwo", &[(3, 3)], 0), Some(("delete.forward", true)));
    check(delete_group_forward, ("one...two", &[(3, 3)], 0), ("onetwo", &[(3, 3)], 0), Some(("delete.forward", true)));
    check(delete_group_forward, ("one \r\n two", &[(3, 3)], 0), ("one\r\n two", &[(3, 3)], 0), Some(("delete.forward", true)));
    check(delete_group_forward, ("one\r\n two", &[(3, 3)], 0), ("one two", &[(3, 3)], 0), Some(("delete.forward", true)));
    check(delete_group_backward, ("one two three", &[(8, 8)], 0), ("one three", &[(4, 4)], 0), Some(("delete.backward", true)));
    check(delete_group_backward, ("one \t two", &[(6, 6)], 0), ("onetwo", &[(3, 3)], 0), Some(("delete.backward", true)));
    check(delete_group_backward, ("one \r\ntwo", &[(6, 6)], 0), ("one two", &[(4, 4)], 0), Some(("delete.backward", true)));
    check(delete_group_backward, ("one", &[(0, 0)], 0), ("one", &[(0, 0)], 0), None);
    check(delete_group_forward, ("élan", &[(0, 0), (2, 2)], 0), ("", &[(0, 0)], 0), Some(("delete.forward", true)));
    check(delete_group_forward, ("abcd", &[(0, 0), (1, 2), (3, 3)], 1), ("", &[(0, 0)], 0), Some(("delete.forward", true)));
    check(delete_group_forward, ("abc ďef", &[(4, 6)], 0), ("abc ef", &[(4, 4)], 0), Some(("delete.selection", true)));
    check(delete_group_forward_win, ("one   two", &[(0, 0)], 0), ("two", &[(0, 0)], 0), Some(("delete.forward", true)));
    check(delete_to_line_end, ("one\r\ntwo", &[(3, 3)], 0), ("onetwo", &[(3, 3)], 0), Some(("delete.forward", true)));
    check(delete_to_line_end, ("one\nnext", &[(2, 2)], 0), ("on\nnext", &[(2, 2)], 0), Some(("delete.forward", true)));
    check(delete_to_line_start, ("one\r\ntwo", &[(5, 5)], 0), ("onetwo", &[(3, 3)], 0), Some(("delete.backward", true)));
    check(delete_to_line_start, ("one\r\n two", &[(8, 8)], 0), ("one\r\no", &[(5, 5)], 0), Some(("delete.backward", true)));
}

#[test]
#[rustfmt::skip]
fn oracle_character_deletion_and_trailing_trim() {
    check(delete_char_forward, ("aéx", &[(1, 1)], 0), ("ax", &[(1, 1)], 0), Some(("delete.forward", true)));
    check(delete_char_forward, ("one\r\ntwo", &[(3, 3)], 0), ("onetwo", &[(3, 3)], 0), Some(("delete.forward", true)));
    check(delete_char_backward_strict, ("\t   word", &[(4, 4)], 0), ("\t  word", &[(3, 3)], 0), Some(("delete.backward", true)));
    check(delete_char_backward_strict, ("éx", &[(3, 3)], 0), ("ex", &[(1, 1)], 0), Some(("delete.backward", true)));
    check(delete_char_backward_strict, ("a✈️x", &[(7, 7)], 0), ("ax", &[(1, 1)], 0), Some(("delete.backward", true)));
    check(delete_char_backward_strict, ("a👩‍💻x", &[(12, 12)], 0), ("ax", &[(1, 1)], 0), Some(("delete.backward", true)));
    check(delete_char_backward_strict, ("a🇺🇳x", &[(9, 9)], 0), ("ax", &[(1, 1)], 0), Some(("delete.backward", true)));
    check(delete_char_backward, ("\t   word", &[(4, 4)], 0), ("\t  word", &[(3, 3)], 0), Some(("delete.backward", true)));
    check(|s| delete_char_backward_with(s, TABS), ("\t  word", &[(3, 3)], 0), ("\tword", &[(1, 1)], 0), Some(("delete.backward", true)));
    check(delete_char_backward_strict, ("one\r\ntwo", &[(5, 5)], 0), ("onetwo", &[(3, 3)], 0), Some(("delete.backward", true)));
    check(delete_trailing_whitespace, ("é \r\n\r\nfoo\t\r\n \t", &[(11, 11)], 0), ("é\r\n\r\nfoo\r\n", &[(9, 9)], 0), Some(("delete", false)));
    check(delete_trailing_whitespace, ("none", &[(2, 2)], 0), ("none", &[(2, 2)], 0), None);
}

#[test]
#[rustfmt::skip]
fn oracle_newlines_and_transpose() {
    check(split_line, ("ébrok\r\nword", &[(2, 4), (12, 12)], 1), ("é\r\nok\r\nword\r\n", &[(2, 2), (12, 12)], 1), Some(("input", true)));
    check(insert_newline, ("abcd\r\nxx", &[(3, 1), (8, 8)], 0), ("a\r\nd\r\nxx\r\n", &[(3, 3), (10, 10)], 0), Some(("input", true)));
    check(insert_newline_keep_indent, ("  onetwo\r\n  threefour", &[(5, 17)], 0), ("  one\r\n  four", &[(9, 9)], 0), Some(("input", true)));
    check(transpose_chars, ("é👩‍💻z", &[(3, 3)], 0), ("👩‍💻éz", &[(14, 14)], 0), Some(("move.character", true)));
    check(transpose_chars, ("one\r\ntwo", &[(3, 3)], 0), ("on\r\netwo", &[(5, 5)], 0), Some(("move.character", true)));
    check(transpose_chars, ("one\r\ntwo", &[(5, 5)], 0), ("onet\r\nwo", &[(6, 6)], 0), Some(("move.character", true)));
    check(transpose_chars, ("éxyz", &[(0, 0), (2, 4), (5, 5)], 1), ("éxyz", &[(0, 0), (2, 4), (5, 5)], 1), None);
    check(transpose_chars, ("abc", &[(1, 1), (2, 2)], 1), ("bacb", &[(2, 2), (4, 4)], 1), Some(("move.character", true)));
    check(transpose_chars, ("aa", &[(1, 1)], 0), ("aa", &[(2, 2)], 0), Some(("move.character", true)));
}

#[test]
#[rustfmt::skip]
fn oracle_move_and_copy_line_blocks() {
    check(move_line_up, ("one\r\ntwo\r\nthree\r\nfour", &[(17, 5)], 0), ("two\r\nthree\r\none\r\nfour", &[(12, 0)], 0), Some(("move.line", true)));
    check(move_line_down, ("one\ntwo\nthree\nfour", &[(4, 14)], 0), ("one\nfour\ntwo\nthree", &[(9, 18)], 0), Some(("move.line", true)));
    check(move_line_down, ("one\ntwo\nthree", &[(5, 5), (6, 6), (7, 7)], 2), ("one\nthree\ntwo", &[(11, 11), (12, 12), (13, 13)], 2), Some(("move.line", true)));
    check(move_line_up, ("one\ntwo", &[(3, 3)], 0), ("one\ntwo", &[(3, 3)], 0), None);
    check(move_line_down, ("one\ntwo", &[(7, 7)], 0), ("one\ntwo", &[(7, 7)], 0), None);
    check(copy_line_up, ("one\r\ntwo\r\nlast", &[(5, 5)], 0), ("one\r\ntwo\r\ntwo\r\nlast", &[(5, 5)], 0), Some(("input.copyline", true)));
    check(copy_line_down, ("one\r\ntwo\r\nlast", &[(5, 5)], 0), ("one\r\ntwo\r\ntwo\r\nlast", &[(10, 10)], 0), Some(("input.copyline", true)));
    check(copy_line_down, ("one\ntwo\nlast", &[(8, 0)], 0), ("one\ntwo\none\ntwo\nlast", &[(16, 8)], 0), Some(("input.copyline", true)));
    check(copy_line_up, ("", &[(0, 0)], 0), ("\n", &[(0, 0)], 0), Some(("input.copyline", true)));
}

#[test]
#[rustfmt::skip]
fn oracle_indentation_and_tabs() {
    check(indent_more, ("one\r\ntwo\r\nthree\r\nfour", &[(2, 2), (3, 3), (5, 12), (21, 12)], 2), ("  one\r\n  two\r\n  three\r\n  four", &[(4, 4), (5, 5), (9, 18), (29, 18)], 2), Some(("input.indent", false)));
    check(indent_more, ("one\ntwo\nthree", &[(2, 8)], 0), ("  one\n  two\nthree", &[(4, 12)], 0), Some(("input.indent", false)));
    check(|s| indent_more_with(s, TABS), ("one\ntwo", &[(4, 4)], 0), ("one\n\ttwo", &[(5, 5)], 0), Some(("input.indent", false)));
    check(indent_less, ("   \tone\r\n  \ttwo", &[(4, 4), (12, 12)], 1), ("  one\r\n  two", &[(2, 2), (9, 9)], 1), Some(("delete.dedent", false)));
    check(indent_less, ("\té", &[(1, 1)], 0), ("  é", &[(2, 2)], 0), Some(("delete.dedent", false)));
    check(|s| indent_less_with(s, TABS), ("\t\t é", &[(3, 3)], 0), ("\t é", &[(2, 2)], 0), Some(("delete.dedent", false)));
    check(indent_less, ("word", &[(4, 4)], 0), ("word", &[(4, 4)], 0), Some(("delete.dedent", false)));
    check(insert_tab, ("aé\r\n党z", &[(1, 1), (8, 8)], 1), ("a\té\r\n党\tz", &[(2, 2), (10, 10)], 1), Some(("input", true)));
    check(insert_tab, ("abc\nxyz", &[(0, 2), (4, 4)], 1), ("  abc\n  xyz", &[(2, 4), (8, 8)], 1), Some(("input.indent", false)));
}

// Native fixtures for details the pinned JS state or view does not represent.

#[test]
fn native_line_edits_keep_mixed_endings() {
    check(
        insert_newline_keep_indent,
        ("a\r\n  b\n c\r\nlast", &[(1, 1), (6, 6), (15, 15)], 2),
        (
            "a\r\n\r\n  b\n  \n c\r\nlast\r\n",
            &[(3, 3), (11, 11), (22, 22)],
            2,
        ),
        Some(("input", true)),
    );
    check(
        move_line_down,
        ("a\nb\r\nc", &[(0, 2)], 0),
        ("b\na\r\nc", &[(2, 5)], 0),
        Some(("move.line", true)),
    );
    check(
        move_line_up,
        ("a\r\nb\nc\r\nd", &[(0, 0), (5, 5)], 0),
        ("a\r\nc\nb\r\nd", &[(0, 0), (3, 3)], 0),
        Some(("move.line", true)),
    );
    // The JS command stringifies the copied block with LF separators.
    check(
        copy_line_down,
        ("one\r\ntwo", &[(0, 8)], 0),
        ("one\r\ntwo\r\none\r\ntwo", &[(10, 18)], 0),
        Some(("input.copyline", true)),
    );
}

#[test]
fn native_commands_do_not_split_crlf() {
    let source = "é\r\nx";
    motion!(
        cursor_char_forward_logical,
        "é\r\nx",
        ([(3, 3)], 0),
        ([(4, 4)], 0)
    );
    motion!(
        cursor_char_backward_logical,
        "é\r\nx",
        ([(3, 3)], 0),
        ([(2, 2)], 0)
    );
    motion!(
        cursor_char_forward_logical,
        "a\rb",
        ([(1, 1)], 0),
        ([(2, 2)], 0)
    );
    motion!(cursor_line_start, "a\rb", ([(2, 2)], 0), ([(0, 0)], 0));
    for (command, event) in [
        (delete_char_forward as Command, "delete.forward"),
        (delete_char_backward_strict, "delete.backward"),
        (delete_group_forward, "delete.forward"),
        (delete_group_backward, "delete.backward"),
    ] {
        check(
            command,
            (source, &[(3, 3)], 0),
            ("éx", &[(2, 2)], 0),
            Some((event, true)),
        );
    }
}

#[test]
fn native_vertical_motion_uses_the_current_grapheme_column() {
    motion!(
        cursor_line_down,
        "é🙂q\r\nα\r\n🇯🇵bc",
        ([(7, 7)], 0),
        ([(12, 12)], 0)
    );
    motion!(
        cursor_line_down,
        "é🙂q\r\nα\r\n🇯🇵bc",
        ([(12, 12)], 0),
        ([(22, 22)], 0)
    );
}

#[test]
fn native_command_transaction_can_be_undone_and_redone() {
    let before = ("éa\r\n中z", &[(2, 2), (8, 8)][..], 1);
    let after = ("aé\r\nz中", &[(3, 3), (9, 9)][..], 1);
    let mut history = History::new(state_from(before));
    let transaction = transpose_chars(history.state()).unwrap().unwrap();
    history.update(transaction, (), |_| (None, ())).unwrap();
    assert_state(history.state(), after);
    assert_eq!(history.undo(), Some(()));
    assert_state(history.state(), before);
    assert_eq!(history.redo(), Some(()));
    assert_state(history.state(), after);
}
