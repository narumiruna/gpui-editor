//! Observations from the verified @codemirror/state 6.7.6 tarball in external
//! scratch. The fixture gives native UTF-8 offsets and the original JS skips.

use std::ops::Range;

use gpui_editor_codemirror::{EditError, Text};
use serde_json::{Value, json};

fn rows(kind: &str) -> impl Iterator<Item = Value> + '_ {
    include_str!("fixtures/text_state.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn offset(value: &Value) -> Option<usize> {
    value.as_u64().map(|value| usize::try_from(value).unwrap())
}

#[test]
fn cursor_steps_match_the_pinned_iterator_on_corresponding_offsets() {
    for row in rows("cursor") {
        let id = row["id"].as_str().unwrap();
        let text = Text::new(row["source"].as_str().unwrap());
        let mut cursor = if row["raw"] == true {
            text.cursor()
        } else {
            text.cursor_range(offset(&row["from"]).unwrap(), offset(&row["to"]).unwrap())
                .unwrap()
        };
        assert_eq!(
            json!([cursor.value(), cursor.line_break(), cursor.done()]),
            row["initial"],
            "{id}"
        );
        for (index, step) in row["steps"].as_array().unwrap().iter().enumerate() {
            let skip = isize::try_from(step[0].as_i64().unwrap()).unwrap();
            let chunk = if skip == 0 {
                cursor.next()
            } else {
                cursor.next_with_skip(skip).unwrap()
            };
            assert_eq!(chunk, (step[3] == false).then(|| step[1].as_str().unwrap()));
            assert_eq!(
                json!([cursor.value(), cursor.line_break(), cursor.done()]),
                json!([step[1], step[2], step[3]]),
                "{id} step {index}"
            );
        }
    }
}

#[test]
fn line_ranges_match_pinned_nonempty_ranges_with_checked_native_bounds() {
    for row in rows("lines") {
        let text = Text::new(row["source"].as_str().unwrap());
        let id = row["id"].as_str().unwrap();
        let from = offset(&row["from"]);
        let to = offset(&row["to"]);
        let result = &row["result"];
        match id {
            "equal-endpoints" | "past-last-start" => {
                assert_eq!(
                    result,
                    &if id == "equal-endpoints" {
                        json!([""])
                    } else {
                        json!("RangeError")
                    }
                );
                assert!(
                    text.iter_line_range(from.unwrap()..to.unwrap())
                        .unwrap()
                        .next()
                        .is_none()
                );
            }
            "reversed-endpoints" | "zero-end" | "past-last-end" => {
                assert!(text.iter_line_range(from.unwrap()..to.unwrap()).is_none());
                assert_eq!(
                    result,
                    &if id == "past-last-end" {
                        json!("RangeError")
                    } else {
                        json!([""])
                    }
                );
            }
            "omitted-start" | "all" => {
                // Upstream ignores `to` if `from` is missing. Native ranges have both bounds.
                assert_eq!(json!(text.iter_lines().collect::<Vec<_>>()), *result);
                if let Some(to) = to {
                    assert_eq!(text.iter_line_range(1..to).unwrap().len(), 1);
                }
            }
            _ => assert_eq!(
                json!(
                    text.iter_line_range(from.unwrap()..to.unwrap_or(text.line_count() + 1))
                        .unwrap()
                        .collect::<Vec<_>>()
                ),
                *result,
                "{id}"
            ),
        }
    }
}

#[test]
fn explicit_output_separator_matches_pinned_slice_string() {
    for row in rows("slice") {
        let text = Text::new(row["source"].as_str().unwrap());
        let range = offset(&row["from"]).unwrap()..offset(&row["to"]).unwrap();
        let separator = row["separator"].as_str().unwrap();
        assert_eq!(
            text.slice_with_line_separator(range, separator).as_deref(),
            row["result"].as_str(),
            "{}",
            row["id"]
        );
    }
}

#[test]
fn native_input_and_edits_keep_bytes_independent_of_separator_output() {
    for row in rows("state") {
        let id = row["id"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let text = Text::new(source);
        let separator = row["lineBreak"].as_str().unwrap();
        let output = text.slice_with_line_separator(0..text.len(), separator);
        assert_eq!(text.as_str(), source);
        assert_eq!(text.append(source).as_str(), source.repeat(2));
        assert_eq!(
            row["string"].as_str().unwrap(),
            row["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|line| line.as_str().unwrap())
                .collect::<Vec<_>>()
                .join("\n")
        );
        if id == "uniform-crlf" {
            assert_eq!(json!(text.to_lines()), row["lines"]);
            assert_eq!(output.as_deref(), row["sliceDoc"].as_str());
            assert_eq!(json!(text.append(source).to_lines()), row["insert"]);
        } else {
            assert_eq!(text.to_lines(), ["a", "b|c\rd", ""]);
            assert_ne!(json!(text.to_lines()), row["lines"], "{id}");
            assert_ne!(json!(text.append(source).to_lines()), row["insert"], "{id}");
            assert_ne!(output.as_deref(), row["sliceDoc"].as_str(), "{id}");
        }
    }
}

#[test]
fn native_cursor_checks_utf8_and_keeps_position_at_its_bounds() {
    let row = rows("limits").next().unwrap();
    let source = row["source"].as_str().unwrap();
    let text = Text::new(source);
    assert_eq!(row["surrogate"], json!([0xdca1]));
    assert_eq!(row["clippedSlice"], source);
    assert_eq!(
        row["oversizedRange"],
        json!(text.iter().collect::<Vec<_>>())
    );
    assert!(text.cursor_range(2, 5).is_none());
    assert!(text.cursor_range(5, 2).is_none());
    assert!(text.cursor_range(2, 2).is_none());
    assert!(text.cursor_range(0, text.len() + 1).is_none());
    assert!(
        text.slice_with_line_separator(0..text.len() + 1, "|")
            .is_none()
    );
    assert!(text.slice_with_line_separator(2..5, "|").is_none());
    assert!(
        text.slice_with_line_separator(Range { start: 5, end: 1 }, "|")
            .is_none()
    );

    let mut partial = text.cursor_range(1, 5).unwrap();
    assert_eq!(partial.next(), Some("💡"));
    assert_eq!(partial.next_with_skip(-1), Err(EditError::InvalidRange));
    assert_eq!(partial.value(), "💡");
    assert!(!partial.done());
    assert!(!partial.line_break());
    assert_eq!(partial.next(), None);
    assert_eq!(row["partialRewind"][2], json!(["", false, true]));
    assert_eq!(partial.next_with_skip(-4), Ok(Some("💡")));
    assert!(!partial.done());

    let mut reverse = text.cursor_range(9, 0).unwrap();
    assert_eq!(row["reverseInitialSkip"], json!(["\n", true, false]));
    assert_eq!(reverse.next_with_skip(isize::MAX), Ok(None));
    assert!(reverse.done());
    assert_eq!(reverse.next_with_skip(isize::MIN), Ok(Some("\n")));
    assert!(reverse.line_break());
}

#[test]
fn native_from_lines_indexes_the_joined_bytes() {
    let row = rows("limits").next().unwrap();
    assert_eq!(row["ofEmpty"], "RangeError");
    let empty = Text::from_lines(&[]);
    assert!(empty.is_empty());
    assert!(empty.iter_line_range(1..2).unwrap().eq([""]));
    assert!(empty.iter_line_range(2..2).unwrap().next().is_none());
    assert!(empty.iter_line_range(0..1).is_none());

    assert_eq!(
        row["ofEmbeddedBreak"]["step"],
        json!(["a\nb", false, false])
    );
    assert_eq!(Text::from_lines(&["a\nb"]).to_lines(), ["a", "b"]);
}

#[test]
fn cursor_labels_partial_crlf_without_changing_its_bytes() {
    // CR and LF cross a physical leaf boundary. The first line after them is empty.
    let prefix = "x".repeat(1023);
    let source = format!("{prefix}\r\n\r\nα\rβ\n");
    let text = Text::new(&source);
    let mut cursor = text.cursor_range(1024, 1026).unwrap();
    for expected in ["\n", "\r"] {
        assert_eq!(cursor.next(), Some(expected));
        assert!(cursor.line_break());
    }
    assert_eq!(cursor.next(), None);
    assert!(!cursor.line_break());
    let mut reverse = text.cursor_range(1026, 1024).unwrap();
    for expected in ["\r", "\n"] {
        assert_eq!(reverse.next(), Some(expected));
        assert!(reverse.line_break());
    }
    assert_eq!(reverse.next(), None);
    assert_eq!(reverse.next_with_skip(-1), Ok(Some("\n")));
    assert!(reverse.line_break());

    let mut forward = text.cursor_range(1023, 1027).unwrap();
    assert_eq!(forward.next_with_skip(1), Ok(Some("\n")));
    assert!(forward.line_break());
    assert_eq!(forward.next_with_skip(-2), Ok(Some("\r\n")));
    assert!(forward.line_break());

    assert_eq!(
        text.slice_with_line_separator(1024..1026, "|").as_deref(),
        Some("\n\r")
    );
    assert_eq!(
        text.slice_with_line_separator(1023..1026, "|").as_deref(),
        Some("|\r")
    );
    assert_eq!(
        text.slice_with_line_separator(1024..1027, "|").as_deref(),
        Some("\n|")
    );
    assert_eq!(
        text.slice_with_line_separator(0..text.len(), "|").unwrap(),
        format!("{prefix}||α\rβ|")
    );
    assert_eq!(text.as_str(), source);

    let mut first_lines = text.iter_line_range(1..3).unwrap().rev();
    assert_eq!(first_lines.next(), Some(""));
    assert_eq!(first_lines.next(), Some(prefix.as_str()));
    assert_eq!(first_lines.next(), None);

    let mut lines = text.iter_line_range(2..5).unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines.next_back(), Some(""));
    assert_eq!(lines.next(), Some(""));
    assert_eq!(lines.next_back(), Some("α\rβ"));
    assert_eq!(lines.len(), 0);
    assert_eq!(lines.next(), None);
    assert_eq!(lines.next_back(), None);
}
