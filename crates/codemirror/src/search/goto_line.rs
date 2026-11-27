use crate::{
    EditorSelection, EditorState, Text, Transaction,
    view::{SCROLL_INTO_VIEW, ScrollStrategy, ScrollTarget},
};

/// Resolve `@codemirror/search` 6.7.3 `gotoLine` input to a UTF-8 byte position.
///
/// Pass the document and main selection head from when the dialog opened.
/// The input has an optional sign, line number, `:` followed by a column, and
/// `%`, in that order. Numbers use ASCII digits. Do not include whitespace.
/// An omitted line number uses the origin line, even with a sign or `%`.
/// Lines are one-based. A signed line number is relative to the origin line.
/// Percentages use the document's line count and round to the nearest line.
///
/// An omitted column is zero. Columns count UTF-16 code units, not graphemes
/// or display cells. A column inside a surrogate pair maps to the start of
/// that scalar.
/// Lines and columns clip to the bounds of [`Text`], which preserves CRLF
/// bytes and treats a lone CR as content.
///
/// Return `None` for invalid input or an invalid head. The host can use the
/// result to set a single cursor and center it. No dialog or editor state is
/// changed here.
pub fn goto_line(text: &Text, head: usize, input: &str) -> Option<usize> {
    let (sign, input) = match input.as_bytes().first().copied() {
        sign @ Some(b'+' | b'-') => (sign, &input[1..]),
        _ => (None, input),
    };
    let percent = input.strip_suffix('%');
    let input = percent.unwrap_or(input);
    let (line, column) = match input.split_once(':') {
        Some((line, column)) => (line, decimal(column)? as usize),
        None => (input, 0),
    };

    let origin = text.line_at(head)?.number;
    let lines = text.line_count();
    let number = if line.is_empty() {
        origin
    } else {
        let magnitude = decimal(line)?;
        let signed = if sign == Some(b'-') {
            -magnitude
        } else {
            magnitude
        };
        let target = if percent.is_some() {
            // Keep the reference's floating-point operation order.
            let mut ratio = signed / 100.0;
            if sign.is_some() {
                ratio += origin as f64 / lines as f64;
            }
            // A negative tie may round differently from Math.round, but it
            // clips to line 1 in both cases.
            (lines as f64 * ratio).round()
        } else if sign.is_some() {
            signed + origin as f64
        } else {
            magnitude
        };
        (target as usize).clamp(1, lines)
    };

    let line = text.line(number)?;
    let mut position = line.range.start;
    let mut remaining = column;
    for scalar in line.text.chars() {
        let Some(next) = remaining.checked_sub(scalar.len_utf16()) else {
            break;
        };
        remaining = next;
        position += scalar.len_utf8();
    }
    Some(position)
}

/// Resolve a Go to line submission against the editor snapshot from when
/// the dialog opened. Set a single cursor and request vertical centering.
/// The host closes the dialog. If the document changed while it was open,
/// the host must map the result or choose not to apply it.
pub fn goto_line_transaction(opening: &EditorState, input: &str) -> Option<Transaction> {
    let head = opening.selection().main().head();
    let position = goto_line(opening.text(), head, input)?;
    let target = ScrollTarget {
        y: ScrollStrategy::Center,
        ..ScrollTarget::at(position)
    };
    Some(
        Transaction::default()
            .with_selection(EditorSelection::single(position, position))
            .with_effects([SCROLL_INTO_VIEW.of(target)]),
    )
}

fn decimal(input: &str) -> Option<f64> {
    if !input.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // The reference parses with Number, including overflow to infinity.
    input.parse().ok()
}
