//! Cases checked by submitting the actual `gotoLine` dialog in scratch against
//! `@codemirror/search` 6.7.3, state 6.7.6, and view 6.44.0. On success it
//! emits one collapsed cursor and a centered scroll effect at the same UTF-16
//! position. On failure it only closes the dialog. Expected offsets below are
//! native UTF-8 bytes; a split surrogate pair snaps to the scalar start.

use gpui_editor_codemirror::{
    EditorSelection, EditorState, READ_ONLY, SelectionRange, Text, Transaction, search,
    view::{SCROLL_INTO_VIEW, ScrollStrategy, ScrollTarget},
};

const SOURCE: &str = "zero\nA🦀é\tX\n\nlast\n";
const HEAD: usize = 18;

#[test]
fn resolves_absolute_relative_and_optional_line_input() {
    let text = Text::new(SOURCE);
    for (input, expected) in [
        ("", 16),
        ("+", 16),
        ("-", 16),
        ("%", 16),
        ("+%", 16),
        ("-%", 16),
        (":0", 16),
        (":2", 18),
        ("+:2", 18),
        ("-:2%", 18),
        ("2", 5),
        ("0002:0003", 10),
        ("0:9", 4),
        ("999", 21),
        ("3:100", 15),
        ("5:9", 21),
        ("+0", 16),
        ("-0", 16),
        ("+1", 21),
        ("-1", 15),
        ("+999", 21),
        ("-999", 0),
    ] {
        assert_eq!(
            search::goto_line(&text, HEAD, input),
            Some(expected),
            "{input:?}"
        );
    }
}

#[test]
fn rounds_percentages_with_the_upstream_operation_order() {
    let text = Text::new(SOURCE);
    for (input, expected) in [
        ("0%", 0),
        ("29%", 0),
        ("30%", 5),
        ("49%", 5),
        ("50%", 15),
        ("90%", 21),
        ("200%", 21),
        ("+10%", 21),
        ("-10%", 16),
        ("+0%", 16),
        ("-0%", 16),
        ("+200%", 21),
        ("-200%", 0),
        ("30:2%", 6),
        ("-40:2%", 6),
    ] {
        assert_eq!(
            search::goto_line(&text, HEAD, input),
            Some(expected),
            "{input:?}"
        );
    }

    // In binary floating-point, 50 * (29 / 100) is just below 14.5.
    let fifty = Text::new("x\n".repeat(49) + "x");
    assert_eq!(search::goto_line(&fifty, 0, "29%"), Some(26));
    assert_eq!(search::goto_line(&fifty, 0, "31%"), Some(30));
    let ten = Text::new("x\n".repeat(9) + "x");
    assert_eq!(search::goto_line(&ten, 4, "+5%"), Some(6));
    assert_eq!(search::goto_line(&ten, 4, "-5%"), Some(4));
    assert_eq!(search::goto_line(&ten, 4, "+25:1%"), Some(11));
}

#[test]
fn handles_decimal_overflow_without_rejecting_valid_input() {
    let text = Text::new(SOURCE);
    let huge = "9".repeat(400);
    for (input, expected) in [
        (format!("{}2", "0".repeat(500)), 5),
        (huge.clone(), 21),
        (format!("-{huge}%"), 0),
        (format!("2:{huge}"), 14),
    ] {
        assert_eq!(search::goto_line(&text, HEAD, &input), Some(expected));
    }
}

#[test]
fn rejects_invalid_raw_input_and_heads() {
    let text = Text::new(SOURCE);
    for input in [
        " 2",
        "2 ",
        "\t2",
        "2\r",
        "2\n",
        "2\r\n",
        "2\u{2028}",
        "2\u{2029}",
        "2\r\n:2",
        "１２",
        "٢",
        "−2",
        "1.5",
        "2:1.5",
        "2:-1",
        "2:+1",
        "2:",
        "2%:1",
        ":",
        ":%",
        "++2",
        "5%%",
        "1e2",
        "0x2",
        "NaN",
        "Infinity",
    ] {
        assert_eq!(search::goto_line(&text, HEAD, input), None, "{input:?}");
    }
    assert_eq!(search::goto_line(&text, 7, "2"), None);
    assert_eq!(search::goto_line(&text, text.len() + 1, "2"), None);
}

#[test]
fn counts_utf16_columns_and_clamps_to_native_line_bounds() {
    let text = Text::new(SOURCE);
    // The upstream cursor is at UTF-16 offsets 5 through 11. Offset 7
    // splits 🦀, so its UTF-8 result is the same as for offset 6.
    for (input, expected) in [
        ("2:0", 5),
        ("2:1", 6),
        ("2:2", 6),
        ("2:3", 10),
        ("2:4", 12),
        ("2:5", 13),
        ("2:6", 14),
        ("2:7", 14),
    ] {
        assert_eq!(
            search::goto_line(&text, HEAD, input),
            Some(expected),
            "{input:?}"
        );
    }

    let graphemes = Text::new("éa\u{301}\t👩‍👩");
    assert_eq!(search::goto_line(&graphemes, 0, ":2"), Some(3));
    assert_eq!(search::goto_line(&graphemes, 0, ":5"), Some(6));
    assert_eq!(search::goto_line(&graphemes, 0, ":7"), Some(13));
    let crlf = Text::new("a\r\nb");
    assert_eq!(search::goto_line(&crlf, 2, ":9"), Some(1));
    assert_eq!(search::goto_line(&crlf, 0, "2:1"), Some(4));
    // Native Text keeps a lone CR in the line, unlike upstream's default
    // string-to-document conversion.
    let lone_cr = Text::new("a\rb\nc");
    assert_eq!(search::goto_line(&lone_cr, 0, "1:2"), Some(2));
    assert_eq!(search::goto_line(&Text::default(), 0, "999:999"), Some(0));
}

#[test]
fn dialog_origin_uses_the_saved_primary_head_and_result_is_a_valid_cursor() {
    let mut editor = EditorState::with_extensions(SOURCE, READ_ONLY.of(true)).unwrap();
    let selection = EditorSelection::new(
        vec![SelectionRange::new(1, 1), SelectionRange::new(21, HEAD)],
        1,
    )
    .unwrap();
    editor
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    let snapshot = editor.clone();
    editor
        .update(Transaction::default().with_selection(EditorSelection::single(1, 1)))
        .unwrap();

    let tx = search::goto_line_transaction(&snapshot, "-2:2").unwrap();
    assert!(tx.changes().is_empty());
    assert_eq!(tx.user_event(), None);
    assert!(!tx.scroll_into_view());
    assert_eq!(tx.effects().len(), 1);
    assert_eq!(
        tx.effects()[0].get(&SCROLL_INTO_VIEW),
        Some(&ScrollTarget {
            y: ScrollStrategy::Center,
            ..ScrollTarget::at(6)
        })
    );
    assert!(search::goto_line_transaction(&snapshot, "2:").is_none());
    editor.update(tx).unwrap();
    assert_eq!(editor.text(), snapshot.text());
    assert_eq!(editor.selection(), &EditorSelection::single(6, 6));
}
