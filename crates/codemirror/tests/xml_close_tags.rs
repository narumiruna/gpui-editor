//! Typing results from the `@codemirror/lang-xml` 6.1.0 input handler in
//! Chromium with `@codemirror/view` 6.44.0, `@codemirror/state` 6.7.6,
//! `@codemirror/language` 6.13.1, and `@lezer/xml` 1.0.6.
//! `|` marks carets; the native editor uses UTF-8 byte offsets.

use gpui_editor_codemirror::{
    EditError, EditorSelection, EditorState, History, READ_ONLY, SelectionRange, Transaction,
    language::xml::{auto_close_tags, auto_close_tags_in},
};

fn marked(source: &str, main: usize) -> (String, EditorSelection) {
    let mut text = String::new();
    let mut ranges = Vec::new();
    for (i, part) in source.split('|').enumerate() {
        if i != 0 {
            ranges.push(SelectionRange::new(text.len(), text.len()));
        }
        text.push_str(part);
    }
    (text, EditorSelection::new(ranges, main).unwrap())
}

fn select(mut state: EditorState, selection: EditorSelection) -> EditorState {
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    state
}

fn state(source: &str, main: usize) -> EditorState {
    let (text, selection) = marked(source, main);
    select(EditorState::new(text), selection)
}

fn assert_state(state: &EditorState, expected: &str, main: usize) {
    let (text, selection) = marked(expected, main);
    assert_eq!(state.text().as_str(), text);
    assert_eq!(state.selection(), &selection);
}

fn assert_typing(input: &str, typed: &str, main: usize, expected: Option<&str>) {
    let mut state = state(input, main);
    let transaction = auto_close_tags(&state, typed).unwrap();
    if let Some(expected) = expected {
        let transaction = transaction.unwrap_or_else(|| panic!("not handled: {input:?}"));
        assert_eq!(transaction.user_event(), Some("input.complete"));
        assert!(transaction.scroll_into_view());
        state.update(transaction).unwrap();
        assert_state(&state, expected, main);
    } else {
        assert!(transaction.is_none(), "unexpected close: {input:?}");
    }
}

#[test]
fn closes_open_tags_without_html_void_or_raw_text_rules() {
    for (input, expected) in [
        ("<a|", "<a>|</a>"),
        ("<a|>text", "<a>|</a>text"),
        ("<a|>>", "<a>|</a>>"),
        ("<a|/>", "<a>|</a>/>"),
        ("<br|", "<br>|</br>"),
        ("<script|", "<script>|</script>"),
        ("<a id=\"ok\"|", "<a id=\"ok\">|</a>"),
        ("<a x=|", "<a x=>|</a>"),
        ("<a x=y|", "<a x=y>|</a>"),
        (
            "😀\r\n<ns:名字-𐀀1 attr=\"é\"|",
            "😀\r\n<ns:名字-𐀀1 attr=\"é\">|</ns:名字-𐀀1>",
        ),
    ] {
        assert_typing(input, ">", 0, Some(expected));
    }
}

#[test]
fn uses_the_parser_close_tag_decision_without_html_ancestor_matching() {
    for input in [
        "<a|</a>",
        "<a><a|</a>",
        "<a><a|</a></a>",
        "<a><b|</b>",
        "<a1|</a2>",
        "<a1><a2|</a3>",
        "<a></a|",
        "<a></b|</a>",
    ] {
        assert_typing(input, ">", 0, None);
    }
    for (input, expected) in [
        ("<a|</A>", "<a>|</a></A>"),
        ("<A><a|</A>", "<A><a>|</a></A>"),
        ("<a><b|</a>", "<a><b>|</b></a>"),
        ("<a></b|", "<a></b>|</a>"),
        ("<a></b|>", "<a></b>|</a>"),
        ("</a|", "</a>|</a>"),
        ("<a><b></c|", "<a><b></c>|</b>"),
    ] {
        assert_typing(input, ">", 0, Some(expected));
    }
}

#[test]
fn completes_slash_and_moves_the_caret_after_the_close() {
    for (input, expected) in [
        ("<a><|", "<a></a>|"),
        ("<a><|>tail", "<a></a>|tail"),
        ("<a><|>>tail", "<a></a>|>tail"),
        ("<a><|</b>", "<a></a>|</b>"),
        ("<a><b><|", "<a><b></b>|"),
        ("<a><b></b><|", "<a><b></b></a>|"),
        ("<a><b/><|", "<a><b/></a>|"),
        ("<a></b><|", "<a></b></a>|"),
        ("<root><a><|</root>", "<root><a></a>|</root>"),
        ("<a><|1>", "<a></a>|1>"),
        ("😀\r\n<ns:名字-𐀀1>α<|", "😀\r\n<ns:名字-𐀀1>α</ns:名字-𐀀1>|"),
    ] {
        assert_typing(input, "/", 0, Some(expected));
    }
    for input in [
        "<a><|</a>",
        "<root><a><|</a></root>",
        "<a><|b></b>",
        "<a><| b>",
        "<a><|.>",
        "<a><|-x>",
        "<a><|😀>",
        "<a></a><|",
        "<a>< |",
        "<a |>",
        "<|>",
        "<|",
        "|",
    ] {
        assert_typing(input, "/", 0, None);
    }
}

#[test]
fn ignores_non_tag_tokens_and_unrelated_input() {
    for (input, typed) in [
        ("<a/|", ">"),
        ("<a / |", ">"),
        ("<|", ">"),
        ("text|", ">"),
        ("<a></|", ">"),
        ("<a x=\"unfinished|", ">"),
        ("<a x=\"<b|\">", ">"),
        ("<!-- <a| -->", ">"),
        ("<![CDATA[<a|]]>", ">"),
        ("<?pi <a| ?>", ">"),
        ("<!DOCTYPE root|", ">"),
        ("<a><!-- <| -->", "/"),
        ("<a><![CDATA[<|]]>", "/"),
        ("<a><?pi <| ?>", "/"),
        ("<a attr=\"<|\">", "/"),
        ("<a|", ">>"),
        ("<a|", "<"),
    ] {
        assert_typing(input, typed, 0, None);
    }
}

#[test]
fn uses_one_post_input_tree_for_all_ranges_even_when_the_main_caret_does_not_close() {
    for (input, typed, main, expected) in [
        ("<a|\n<b|\nplain|", ">", 2, "<a>|</a>\n<b>|</b>\nplain>|"),
        ("<a|\n<b x=\"|", ">", 1, "<a>|</a>\n<b x=\">|"),
        ("<a><|\n<b><|\n<|", "/", 1, "<a></a>|\n<b></b>|\n</b>|"),
        ("<a><|\n<b><|\nplain|", "/", 1, "<a></a>|\n<b></|\nplain/|"),
    ] {
        assert_typing(input, typed, main, Some(expected));
    }
}

#[test]
fn replaces_secondary_selections_but_not_a_nonempty_main_selection() {
    let selection = |main| {
        EditorSelection::new(
            vec![SelectionRange::new(2, 2), SelectionRange::new(6, 5)],
            main,
        )
        .unwrap()
    };
    let mut state = select(EditorState::new("<a\n<b?\nplain"), selection(0));
    let transaction = auto_close_tags(&state, ">").unwrap().unwrap();
    state.update(transaction).unwrap();
    assert_state(&state, "<a>|</a>\n<b>|</b>\nplain", 0);

    let state = select(EditorState::new("<a\n<b?\nplain"), selection(1));
    assert!(auto_close_tags(&state, ">").unwrap().is_none());
}

#[test]
fn scoped_closes_use_document_byte_coordinates_and_leave_outside_text_alone() {
    let prefix = "😀\r\n```xml\r\n";
    let start = prefix.len();
    for (input, typed, expected) in [
        ("<名|>", ">", "<名>|</名>>"),
        ("<名><|>", "/", "<名></名>|>"),
    ] {
        let mut editor = state(&format!("{prefix}{input}\r\n```"), 0);
        let end = editor.selection().main().head();
        let transaction = auto_close_tags_in(&editor, start..end, typed)
            .unwrap()
            .unwrap();
        editor.update(transaction).unwrap();
        assert_state(&editor, &format!("{prefix}{expected}\r\n```"), 0);
    }
}

#[test]
fn scoped_closes_require_a_valid_scope_and_keep_all_ranges_inside() {
    let editor = state("😀<a|>|", 0);
    assert!(matches!(
        auto_close_tags_in(&editor, 1..editor.text().len(), ">"),
        Err(EditError::InvalidRange)
    ));
    assert!(auto_close_tags_in(&editor, 4..6, ">").unwrap().is_none());

    let editor = state("😀<a><|", 0);
    assert!(auto_close_tags_in(&editor, 7..8, "/").unwrap().is_none());
}

#[test]
fn respects_read_only_and_records_the_native_edit_as_one_undo() {
    let read_only = EditorState::with_extensions("<a", READ_ONLY.of(true)).unwrap();
    let read_only = select(read_only, EditorSelection::single(2, 2));
    assert!(auto_close_tags(&read_only, ">").unwrap().is_none());

    let mut history = History::new(state("<a|>", 0));
    let transaction = auto_close_tags(history.state(), ">").unwrap().unwrap();
    history.update(transaction, (), |_| (None, ())).unwrap();
    assert_state(history.state(), "<a>|</a>", 0);
    assert_eq!(history.undo(), Some(()));
    assert_state(history.state(), "<a|>", 0);
    assert_eq!(history.redo(), Some(()));
    assert_state(history.state(), "<a>|</a>", 0);
}
