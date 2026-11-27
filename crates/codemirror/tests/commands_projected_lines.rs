use std::time::Duration;

use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, EditError, EditorSelection, EditorState, READ_ONLY, SelectionRange,
    Text, Transaction,
    commands::{CommandResult, copy_line_with_projection, move_line_with_projection},
    language_data::NativeLanguage,
    setup::{SetupKeyContext, basic_setup},
    view::keymap::{KeyEvent, KeyModifiers, KeyPlatform},
};

const COMMANDS: [(&str, bool, LineCommand); 4] = [
    ("move.line", false, move_line_with_projection),
    ("move.line", true, move_line_with_projection),
    ("input.copyline", false, copy_line_with_projection),
    ("input.copyline", true, copy_line_with_projection),
];

type LineCommand = fn(&EditorState, &Text, bool) -> CommandResult;
type Ranges<'a> = &'a [(usize, usize)];

fn normalized(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n")
}

// Use the same-width projection as the syntax documents. Preserve CRLF.
fn projection(raw: &str) -> Text {
    let mut text = String::new();
    for (at, ch) in raw.char_indices() {
        text.push(if ch == '\r' && !raw[at + 1..].starts_with('\n') {
            '\n'
        } else {
            ch
        });
    }
    Text::new(text)
}

fn normalized_pos(raw: &str, byte: usize) -> usize {
    assert!(byte == 0 || raw.as_bytes().get(byte - 1..byte + 1) != Some(b"\r\n"));
    normalized(&raw[..byte]).encode_utf16().count()
}

fn raw_pos(raw: &str, utf16: usize) -> usize {
    raw.char_indices()
        .map(|(at, _)| at)
        .chain([raw.len()])
        .filter(|&at| at == 0 || raw.as_bytes().get(at - 1..at + 1) != Some(b"\r\n"))
        .find(|&at| normalized_pos(raw, at) == utf16)
        .unwrap()
}

fn selection(ranges: Ranges<'_>, main: usize, pos: impl Fn(usize) -> usize) -> EditorSelection {
    EditorSelection::new(
        ranges
            .iter()
            .map(|&(a, h)| SelectionRange::new(pos(a), pos(h)))
            .collect(),
        main,
    )
    .unwrap()
}

fn state(raw: &str, selection: EditorSelection) -> EditorState {
    EditorState::create_from_text(
        Text::new(raw),
        Some(selection),
        ALLOW_MULTIPLE_SELECTIONS.of(true),
    )
    .unwrap()
}

fn check_oracle(
    raw: &str,
    ranges: Ranges<'_>,
    main: usize,
    expected: [Option<(&str, &str, Ranges<'_>)>; 4],
) {
    let lines = projection(raw);
    for ((event, down, run), expected) in COMMANDS.into_iter().zip(expected) {
        let mut state = state(raw, selection(ranges, main, |pos| raw_pos(raw, pos)));
        let edit = run(&state, &lines, down).unwrap();
        let Some((expected_raw, expected_normalized, expected_ranges)) = expected else {
            assert!(edit.is_none(), "{event}, down: {down}");
            continue;
        };
        let edit = edit.expect("the line command is handled");
        assert_eq!(edit.user_event(), Some(event));
        assert!(edit.scroll_into_view());
        for (range, _) in edit.changes().changes() {
            for at in [range.start, range.end] {
                assert!(raw.is_char_boundary(at));
                assert!(at == 0 || raw.as_bytes().get(at - 1..at + 1) != Some(b"\r\n"));
            }
        }
        state.update(edit).unwrap();
        let actual = state.text().as_str();
        assert_eq!(actual, expected_raw, "{event}, down: {down}");
        assert_eq!(normalized(actual), expected_normalized);
        assert_eq!(
            state.selection(),
            &selection(expected_ranges, main, |pos| raw_pos(actual, pos))
        );
    }
}

// Captured in Node v26.10.0 using the SHA-512-verified npm archives in
// docs/codemirror-upstream.json: commands 6.11.2 and state 6.7.6. The commands
// receive {state, dispatch: tr => { state = tr.state }}. The default state
// normalizes line endings; only the multiple selection cases enable its facet.
// Entries are (native bytes, pinned doc.toString(), pinned UTF-16 ranges).
#[test]
#[rustfmt::skip]
fn projected_line_edits_match_the_pinned_normalized_documents_and_selections() {
    check_oracle("甲\r😀x\r\né\n終", &[(4, 4)], 0, [
        Some(("😀x\r甲\r\né\n終", "😀x\n甲\né\n終", &[(2, 2)])),
        Some(("甲\ré\r\n😀x\n終", "甲\né\n😀x\n終", &[(6, 6)])),
        Some(("甲\r😀x\r\n😀x\r\né\n終", "甲\n😀x\n😀x\né\n終", &[(4, 4)])),
        Some(("甲\r😀x\r\n😀x\r\né\n終", "甲\n😀x\n😀x\né\n終", &[(8, 8)])),
    ]);
    // Backward selection ending at the start of an unselected line.
    check_oracle("甲\r😀x\r\né\n終", &[(6, 0)], 0, [
        None,
        Some(("é\r\n甲\r😀x\n終", "é\n甲\n😀x\n終", &[(8, 2)])),
        Some(("甲\r😀x\r\n甲\r😀x\r\né\n終", "甲\n😀x\n甲\n😀x\né\n終", &[(12, 0)])),
        Some(("甲\r😀x\r\n甲\r😀x\r\né\n終", "甲\n😀x\n甲\n😀x\né\n終", &[(12, 6)])),
    ]);
    // Separate blocks use different local breaks and retain the second primary.
    check_oracle("a\r😀b\r\nc\né\r界\r\nx", &[(4, 4), (9, 9)], 1, [
        Some(("😀b\ra\r\né\nc\r界\r\nx", "😀b\na\né\nc\n界\nx", &[(2, 2), (7, 7)])),
        Some(("a\rc\r\n😀b\n界\ré\r\nx", "a\nc\n😀b\n界\né\nx", &[(6, 6), (11, 11)])),
        Some(("a\r😀b\r\n😀b\r\nc\né\ré\r界\r\nx", "a\n😀b\n😀b\nc\né\né\n界\nx", &[(4, 4), (13, 13)])),
        Some(("a\r😀b\r\n😀b\r\nc\né\ré\r界\r\nx", "a\n😀b\n😀b\nc\né\né\n界\nx", &[(8, 8), (15, 15)])),
    ]);
    // Adjacent selections act on one block, including its internal raw CRLF.
    check_oracle("a\r😀b\r\nc\né\r界\r\nx", &[(4, 4), (7, 7)], 1, [
        Some(("😀b\r\nc\ra\né\r界\r\nx", "😀b\nc\na\né\n界\nx", &[(2, 2), (5, 5)])),
        Some(("a\ré\n😀b\r\nc\r界\r\nx", "a\né\n😀b\nc\n界\nx", &[(6, 6), (9, 9)])),
        Some(("a\r😀b\r\nc\n😀b\r\nc\né\r界\r\nx", "a\n😀b\nc\n😀b\nc\né\n界\nx", &[(4, 4), (7, 7)])),
        Some(("a\r😀b\r\nc\n😀b\r\nc\né\r界\r\nx", "a\n😀b\nc\n😀b\nc\né\n界\nx", &[(10, 10), (13, 13)])),
    ]);
    check_oracle("a\r\nb\r終", &[(5, 5)], 0, [
        Some(("a\r\n終\rb", "a\n終\nb", &[(3, 3)])),
        None,
        Some(("a\r\nb\r終\r終", "a\nb\n終\n終", &[(5, 5)])),
        Some(("a\r\nb\r終\r終", "a\nb\n終\n終", &[(7, 7)])),
    ]);
    // A range through the full separator is clamped to EOF on move down.
    check_oracle("a\r😀b\r\nc", &[(2, 6)], 0, [
        Some(("😀b\ra\r\nc", "😀b\na\nc", &[(0, 4)])),
        Some(("a\rc\r\n😀b", "a\nc\n😀b", &[(4, 7)])),
        Some(("a\r😀b\r\n😀b\r\nc", "a\n😀b\n😀b\nc", &[(2, 10)])),
        Some(("a\r😀b\r\n😀b\r\nc", "a\n😀b\n😀b\nc", &[(6, 10)])),
    ]);
    check_oracle("a\r\r\n😀b", &[(0, 0)], 0, [
        None,
        Some(("\ra\r\n😀b", "\na\n😀b", &[(1, 1)])),
        Some(("a\ra\r\r\n😀b", "a\na\n\n😀b", &[(0, 0)])),
        Some(("a\ra\r\r\n😀b", "a\na\n\n😀b", &[(2, 2)])),
    ]);
}

#[test]
fn basic_setup_move_and_copy_use_the_same_projected_line_bounds() {
    let raw = "甲\r😀x\r\né\n終";
    let original = selection(&[(4, 4)], 0, |pos| raw_pos(raw, pos));
    for (key, shift, expected, pos) in [
        ("ArrowUp", false, "😀x\r甲\r\né\n終", 2),
        ("ArrowDown", false, "甲\ré\r\n😀x\n終", 6),
        ("ArrowUp", true, "甲\r😀x\r\n😀x\r\né\n終", 4),
        ("ArrowDown", true, "甲\r😀x\r\n😀x\r\né\n終", 8),
    ] {
        let state = state(raw, original.clone());
        let syntax = NativeLanguage::Go.parse(state.text().clone()).unwrap();
        let mut editor = basic_setup::<()>(state, KeyPlatform::Mac).unwrap();
        let modifiers = KeyModifiers {
            alt: true,
            shift,
            ..Default::default()
        };
        let input = SetupKeyContext {
            syntax: Some(&syntax),
            ..Default::default()
        };
        assert!(
            editor
                .key_event_with(KeyEvent::new(key, modifiers), Duration::ZERO, (), input)
                .unwrap()
                .key
                .handled
        );
        assert_eq!(editor.view().state().text().as_str(), expected);
        assert_eq!(
            editor.view().state().selection().main().head(),
            raw_pos(expected, pos)
        );
        assert!(editor.view_mut().undo().is_some());
        assert_eq!(editor.view().state().text().as_str(), raw);
        assert_eq!(editor.view().state().selection(), &original);
    }
}

#[test]
fn projected_line_edits_do_not_join_separate_breaks() {
    let raw = "a\rX\n\nz";
    let move_state = state(raw, EditorSelection::single(2, 2));
    assert!(
        move_line_with_projection(&move_state, &projection(raw), true)
            .unwrap()
            .is_none()
    );

    let raw = "\nX\rY";
    let copy_state = state(raw, EditorSelection::single(0, 3));
    for down in [false, true] {
        assert!(
            copy_line_with_projection(&copy_state, &projection(raw), down)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn projected_moves_keep_blocked_ranges_and_handle_an_endpoint_inside_crlf() {
    // The native API permits a selection inside CRLF; pinned state has no such position.
    let raw = "a\r😀b\r\nc\r終";
    let mut state = state(raw, selection(&[(8, 2), (14, 14)], 1, |pos| pos));
    let edit = move_line_with_projection(&state, &projection(raw), true)
        .unwrap()
        .unwrap();
    state.update(edit).unwrap();
    assert_eq!(state.text().as_str(), "a\rc\r\n😀b\r終");
    assert_eq!(
        state.selection(),
        &selection(&[(11, 5), (14, 14)], 1, |pos| pos)
    );
}

#[test]
fn projected_line_edits_reject_invalid_projections_and_read_only_state() {
    let raw = "甲\r😀x\r\né\n終";
    let mut state = EditorState::new(raw);
    for (_, down, run) in COMMANDS {
        for invalid in [raw, "甲\n😀x\né\n終", "乙\n😀x\r\né\n終"] {
            assert_eq!(
                run(&state, &Text::new(invalid), down).unwrap_err(),
                EditError::InvalidRange
            );
        }
    }
    state
        .update(Transaction::default().with_effects([READ_ONLY.of(true).append()]))
        .unwrap();
    for (_, down, run) in COMMANDS {
        assert!(run(&state, &projection(raw), down).unwrap().is_none());
    }
}
