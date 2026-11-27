use gpui_editor_codemirror::{
    EditError, EditorSelection, EditorState, History, READ_ONLY, SelectionRange, Transaction,
    language::html::{HtmlOptions, auto_close_tags, auto_close_tags_in},
};

// The HTML decisions and carets below follow the 6.4.12 input handler.
// `|` marks each caret in the source; offsets count native UTF-8 bytes.
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

fn state(source: &str, main: usize) -> EditorState {
    let (text, selection) = marked(source, main);
    with_selection(EditorState::new(text), selection)
}

fn with_selection(mut state: EditorState, selection: EditorSelection) -> EditorState {
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    state
}

fn assert_state(state: &EditorState, expected: &str, main: usize) {
    let (text, selection) = marked(expected, main);
    assert_eq!(state.text().as_str(), text);
    assert_eq!(state.selection(), &selection);
}

fn assert_typing(
    input: &str,
    typed: &str,
    options: HtmlOptions,
    expected: Option<&str>,
    main: usize,
) {
    let mut state = state(input, main);
    let transaction = auto_close_tags(&state, typed, options).unwrap();
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
fn finishes_opening_tags_and_reuses_an_adjacent_angle_bracket() {
    for (input, expected) in [
        ("<div|", "<div>|</div>"),
        ("<div|>content", "<div>|</div>content"),
        (
            "<custom-component id='ok'|",
            "<custom-component id='ok'>|</custom-component>",
        ),
        ("<ul><li|<li>two</ul>", "<ul><li>|</li><li>two</ul>"),
        ("<div><div|</div>tail", "<div><div>|</div></div>tail"),
        ("<DIV><DIV|</DIV>", "<DIV><DIV>|</DIV></DIV>"),
        ("<div|</span>", "<div>|</div></span>"),
        ("<div></i|", "<div></i>|</div>"),
    ] {
        assert_typing(input, ">", HtmlOptions::default(), Some(expected), 0);
    }
}

#[test]
fn finishes_an_incomplete_close_tag_and_moves_past_its_name() {
    for (input, expected) in [
        ("<div>x<|", "<div>x</div>|"),
        ("<div>x<|>after", "<div>x</div>|after"),
        ("<div><p>hi<|", "<div><p>hi</p>|"),
        ("<div><p>hi</p><|", "<div><p>hi</p></div>|"),
        ("<div><img><|", "<div><img></div>|"),
        ("<div><|</span>", "<div></div>|</span>"),
        ("<div></i><|", "<div></i></div>|"),
    ] {
        assert_typing(input, "/", HtmlOptions::default(), Some(expected), 0);
    }
}

#[test]
fn does_not_duplicate_existing_closes_or_act_outside_tag_tokens() {
    for (input, typed) in [
        ("<div|</div>", ">"),
        ("<div><div|</div></div>", ">"),
        ("<DIV><div|</div>", ">"),
        ("<div><|</div>", "/"),
        ("<div><|p></p>", "/"),
        ("<x>x</x|", ">"),
        ("<|", "/"),
        ("text |", ">"),
        ("<!-- <div| -->", ">"),
        ("<div class=\"<span|\">", ">"),
        ("<textarea><x|", ">"),
        ("<textarea><|", "/"),
        ("<svg><path/|", ">"),
        ("<br|", ">"),
        ("<command|", ">"),
        ("<menuitem|", ">"),
        ("<div|", ">>"),
    ] {
        assert_typing(input, typed, HtmlOptions::default(), None, 0);
    }
}

#[test]
fn honors_the_parser_dialects_and_the_pinned_void_name_comparison() {
    let options = HtmlOptions::default();
    assert_typing("<BR|", ">", options, Some("<BR>|</BR>"), 0);
    assert_typing("<custom/|", ">", options, Some("<custom/>|</custom>"), 0);
    assert_typing(
        "<custom/|",
        ">",
        HtmlOptions {
            self_closing_tags: true,
            ..options
        },
        None,
        0,
    );
    let no_match = HtmlOptions {
        match_closing_tags: false,
        ..options
    };
    assert_typing("<div|</span>", ">", no_match, None, 0);
    assert_typing("<div><|</span>", "/", no_match, None, 0);
    assert_typing("<div></i><|", "/", no_match, Some("<div></i></div>|"), 0);
    assert_typing(
        "<main><div></main><|",
        "/",
        no_match,
        Some("<main><div></main></div>|"),
        0,
    );
}

#[test]
fn checks_whether_html_is_active_at_the_main_caret() {
    let options = HtmlOptions::default();
    for (input, typed, main) in [
        ("<script|", ">", 0),
        ("<style |", ">", 0),
        ("<script/|", ">", 0),
        ("<div|\n<script|", ">", 1),
        ("<script>const v = '<|';</script>", "/", 0),
        ("<div style=\"<|\">", "/", 0),
        ("<div|\n<div style=color:red|", ">", 1),
        ("<div|\n<script>t|</script>", ">", 1),
        ("<div|\n<div style=\"color:|red", ">", 1),
    ] {
        assert_typing(input, typed, options, None, main);
    }
    for (input, expected) in [
        ("<script|>", "<script>|</script>"),
        ("<script| ", "<script>|</script> "),
        ("<script type=\"test\"|", "<script type=\"test\">|</script>"),
        ("<style lang=\"scss\"|", "<style lang=\"scss\">|</style>"),
        ("<textarea|", "<textarea>|</textarea>"),
        ("<div|\n<script|", "<div>|</div>\n<script>|</script>"),
        ("<div style=|color:red", "<div style=>|</div>color:red"),
    ] {
        assert_typing(input, ">", options, Some(expected), 0);
    }
    assert_typing(
        "<div|\n<div style=\"color:red|",
        ">",
        options,
        Some("<div>|</div>\n<div style=\"color:red>|"),
        1,
    );
}

#[test]
fn maps_carets_through_all_changes_without_losing_the_main_range() {
    let options = HtmlOptions::default();
    assert_typing(
        "<div|\n<hr|\n<P|",
        ">",
        options,
        Some("<div>|</div>\n<hr>|\n<P>|</P>"),
        1,
    );
    assert_typing(
        "<div><|\n<em><|",
        "/",
        options,
        Some("<div></div>|\n<em></em>|"),
        1,
    );
    assert_typing(
        "😀\r\n<Élément|\r\n<em|",
        ">",
        options,
        Some("😀\r\n<Élément>|</Élément>\r\n<em>|</em>"),
        0,
    );
    assert_typing(
        "😀\r\n<Élément>α<|",
        "/",
        options,
        Some("😀\r\n<Élément>α</Élément>|"),
        0,
    );
}

#[test]
fn replaces_secondary_ranges_but_does_not_handle_a_selected_main_range() {
    let selection = |main| {
        EditorSelection::new(
            vec![
                SelectionRange::new(4, 4),
                SelectionRange::new(9, 8),
                SelectionRange::new(12, 12),
            ],
            main,
        )
        .unwrap()
    };
    let mut editor = with_selection(EditorState::new("<div\n<hr?\n<P"), selection(2));
    let transaction = auto_close_tags(&editor, ">", HtmlOptions::default())
        .unwrap()
        .unwrap();
    editor.update(transaction).unwrap();
    assert_state(&editor, "<div>|</div>\n<hr>|\n<P>|</P>", 2);

    let editor = with_selection(EditorState::new("<div\n<hr?\n<P"), selection(1));
    assert!(
        auto_close_tags(&editor, ">", HtmlOptions::default())
            .unwrap()
            .is_none()
    );
}

#[test]
fn keeps_scoped_input_inside_html_and_uses_document_coordinates() {
    let prefix = "😀\r\n```html\r\n";
    let start = prefix.len();
    let end = start + "<a".len();
    let mut editor = state(&format!("{prefix}<a|>\r\n```"), 0);
    assert!(matches!(
        auto_close_tags_in(&editor, 1..end, ">", HtmlOptions::default()),
        Err(EditError::InvalidRange)
    ));
    let transaction = auto_close_tags_in(&editor, start..end, ">", HtmlOptions::default())
        .unwrap()
        .unwrap();
    editor.update(transaction).unwrap();
    assert_state(&editor, &format!("{prefix}<a>|</a>>\r\n```"), 0);

    let editor = state(&format!("{prefix}<a|>\r\n```|"), 0);
    assert!(
        auto_close_tags_in(&editor, start..end, ">", HtmlOptions::default())
            .unwrap()
            .is_none()
    );
}

#[test]
fn respects_read_only_and_records_the_native_edit_as_one_undo() {
    let read_only = EditorState::with_extensions("<div", READ_ONLY.of(true)).unwrap();
    let read_only = with_selection(read_only, EditorSelection::single(4, 4));
    assert!(
        auto_close_tags(&read_only, ">", HtmlOptions::default())
            .unwrap()
            .is_none()
    );

    let mut history = History::new(state("<div|", 0));
    let transaction = auto_close_tags(history.state(), ">", HtmlOptions::default())
        .unwrap()
        .unwrap();
    history.update(transaction, (), |_| (None, ())).unwrap();
    assert_state(history.state(), "<div>|</div>", 0);
    assert_eq!(history.undo(), Some(()));
    assert_state(history.state(), "<div|", 0);
}
