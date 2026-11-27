use gpui_editor_codemirror::{
    EditError, EditorSelection, EditorState, History, READ_ONLY, SelectionRange, Transaction,
    language::javascript::{JavascriptOptions, auto_close_tags, auto_close_tags_in},
};

const JSX: JavascriptOptions = JavascriptOptions {
    typescript: false,
    jsx: true,
};
const TSX: JavascriptOptions = JavascriptOptions {
    typescript: true,
    jsx: true,
};

// The typing cases come from a scratch DOM oracle for @codemirror/lang-javascript
// 6.2.5 with @lezer/javascript 1.5.7. Here `|` marks a UTF-8 byte position.
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

fn assert_input(
    input: &str,
    key: &str,
    options: JavascriptOptions,
    expected: Option<&str>,
    main: usize,
) {
    let (text, selection) = marked(input, main);
    let mut state = with_selection(EditorState::new(text), selection);
    let transaction = auto_close_tags(&state, key, options).unwrap();
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
fn inserts_exact_jsx_names_and_keeps_the_caret_before_the_close() {
    for (input, expected) in [
        ("<div|", "<div>|</div>"),
        ("<br|", "<br>|</br>"),
        ("<x-box|", "<x-box>|</x-box>"),
        ("const v=<svg:path|", "const v=<svg:path>|</svg:path>"),
        ("<UI.Button|", "<UI.Button>|</UI.Button>"),
        ("<X a=\"yes\"|", "<X a=\"yes\">|</X>"),
        ("<A><B| </B></A>", "<A><B>|</B> </B></A>"),
        ("<a><a| </a>", "<a><a>|</a> </a>"),
        ("<Outer el={<Inner|} />", "<Outer el={<Inner>|</Inner>} />"),
        (
            "const s=\"😀\"; const el = <Élément| </Élément>",
            "const s=\"😀\"; const el = <Élément>|</Élément> </Élément>",
        ),
    ] {
        assert_input(input, ">", JSX, Some(expected), 0);
    }
}

#[test]
fn closes_fragments_even_with_an_adjacent_angle_or_close() {
    for (input, expected) in [("<|", "<>|</>"), ("<|>", "<>|</>>"), ("<|</>", "<>|</></>")] {
        assert_input(input, ">", JSX, Some(expected), 0);
    }
    assert_input("<>x<|", "/", JSX, Some("<>x</>|"), 0);
}

#[test]
fn slash_completes_the_enclosing_name_without_consuming_following_text() {
    for (input, expected) in [
        ("<div>x<|", "<div>x</div>|"),
        ("<X><| name", "<X></X>| name"),
        ("<X><|name>", "<X></X>|name>"),
        ("<X><|</X>", "<X></X>|</X>"),
        ("<X><Y>t<|</Y></X>", "<X><Y>t</Y>|</Y></X>"),
        ("<X>😀<|>after", "<X>😀</X>|>after"),
        ("<UI.Button><|", "<UI.Button></UI.Button>|"),
        (
            "const s=\"😀\"; const el = <Élément>α<|",
            "const s=\"😀\"; const el = <Élément>α</Élément>|",
        ),
    ] {
        assert_input(input, "/", JSX, Some(expected), 0);
    }
}

#[test]
fn skips_adjacent_closes_and_input_outside_the_expected_syntax() {
    for (input, key) in [
        ("<X|</X>", ">"),
        ("<X|</Other>", ">"),
        ("<X|>x", ">"),
        ("<X|/>", ">"),
        ("<X/|", ">"),
        ("<X /|>", ">"),
        ("<X>y</X|", ">"),
        ("<X>text|", ">"),
        ("<X>text|", "/"),
        ("<|", "/"),
        ("<X id=\"foo|bar\" />", ">"),
        ("<X id={a|b}></X>", ">"),
        ("const v = <UI.Button a={1}|", ">"),
        ("const s = \"<X|\";", ">"),
        ("/* <X| */", ">"),
        ("a | b", ">"),
        ("<X|", ">>"),
    ] {
        assert_input(input, key, JSX, None, 0);
    }
}

#[test]
fn enables_jsx_and_tsx_but_not_javascript_or_typescript_alone() {
    assert_input("<div|", ">", TSX, Some("<div>|</div>"), 0);
    assert_input("<X>😀<|>after", "/", TSX, Some("<X>😀</X>|>after"), 0);
    assert_input("const el = <Comp<T>|", ">", TSX, None, 0);
    assert_input("const x = <T,|>(x: T) => x", ">", TSX, None, 0);
    for typescript in [false, true] {
        let options = JavascriptOptions {
            typescript,
            jsx: false,
        };
        assert_input("<X|", ">", options, None, 0);
        assert_input("<X><|", "/", options, None, 0);
    }
}

#[test]
fn maps_all_carets_and_keeps_the_primary_range() {
    assert_input(
        "const a = <X| </X>; const b = <Y|</Y>; n | 1",
        ">",
        JSX,
        Some("const a = <X>|</X> </X>; const b = <Y>|</Y>; n >| 1"),
        1,
    );
    assert_input(
        "const a = <X| </X>; const b = <Y| </Y>; n | 1",
        ">",
        JSX,
        Some("const a = <X>|</X> </X>; const b = <Y>|</Y> </Y>; n >| 1"),
        2,
    );
    assert_input(
        "const a = <X>x<|; const b = <Y>y<|",
        "/",
        JSX,
        Some("const a = <X>x</X>|; const b = <Y>y</Y>|"),
        1,
    );
}

#[test]
fn replaces_secondary_ranges_but_not_a_nonempty_main_range() {
    let text = "const a = <X? </X>; const b = <Y</Y>";
    let ranges = vec![SelectionRange::new(13, 12), SelectionRange::new(32, 32)];
    let mut state = with_selection(
        EditorState::new(text),
        EditorSelection::new(ranges.clone(), 1).unwrap(),
    );
    let transaction = auto_close_tags(&state, ">", JSX).unwrap().unwrap();
    state.update(transaction).unwrap();
    assert_state(&state, "const a = <X>|</X> </X>; const b = <Y>|</Y>", 1);

    let state = with_selection(
        EditorState::new(text),
        EditorSelection::new(ranges, 0).unwrap(),
    );
    assert!(auto_close_tags(&state, ">", JSX).unwrap().is_none());
}

#[test]
fn limits_parsing_to_the_host_range_and_returns_full_document_offsets() {
    let prefix = "😀\r\n```tsx\r\n";
    let start = prefix.len();
    let end = start + "const el = <X".len();
    let (text, selection) = marked(&format!("{prefix}const el = <X|>\r\n```"), 0);
    let mut state = with_selection(EditorState::new(text), selection);
    assert!(matches!(
        auto_close_tags_in(&state, 1..end, ">", TSX),
        Err(EditError::InvalidRange)
    ));
    let transaction = auto_close_tags_in(&state, start..end, ">", TSX)
        .unwrap()
        .unwrap();
    state.update(transaction).unwrap();
    assert_state(&state, &format!("{prefix}const el = <X>|</X>>\r\n```"), 0);

    let (text, selection) = marked(&format!("{prefix}const el = <X|>\r\n```|"), 0);
    let state = with_selection(EditorState::new(text), selection);
    assert!(
        auto_close_tags_in(&state, start..end, ">", TSX)
            .unwrap()
            .is_none()
    );
}

#[test]
fn closes_the_tag_when_recovery_cuts_a_deep_suffix() {
    // The oracle closes X. Before stack cutting, native recovery hit its
    // depth limit in the suffix and lost the opening tag.
    let suffix = "(".repeat(3000) + "1";
    let input = format!("<X| </X>; {suffix}");
    let expected = format!("<X>|</X> </X>; {suffix}");
    assert_input(&input, ">", JSX, Some(&expected), 0);
}

#[test]
fn respects_read_only_and_records_one_native_undo() {
    let selection = EditorSelection::single(4, 4);
    let read_only = EditorState::with_extensions("<div", READ_ONLY.of(true)).unwrap();
    let read_only = with_selection(read_only, selection.clone());
    assert!(auto_close_tags(&read_only, ">", JSX).unwrap().is_none());

    let mut history = History::new(with_selection(EditorState::new("<div"), selection));
    let transaction = auto_close_tags(history.state(), ">", JSX).unwrap().unwrap();
    history.update(transaction, (), |_| (None, ())).unwrap();
    assert_state(history.state(), "<div>|</div>", 0);
    assert_eq!(history.undo(), Some(()));
    assert_state(history.state(), "<div|", 0);
}
