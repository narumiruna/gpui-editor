use std::ops::Range;

use gpui_editor_codemirror::{Change, ChangeSet, EditError, Text};

#[test]
fn cloned_text_shares_bytes_until_an_edit() {
    let source = "é💡\r\nx\ry\n";
    let original = Text::new(source);
    let mut edited = original.clone();
    assert!(std::ptr::eq(original.as_str(), edited.as_str()));

    ChangeSet::single(Change::new(2..6, "β"))
        .unwrap()
        .apply_to(&mut edited)
        .unwrap();

    assert_eq!(original.as_str(), source);
    assert_eq!(edited.as_str(), "éβ\r\nx\ry\n");
}

#[test]
fn replaces_slices_and_appends_at_utf8_offsets() {
    // The same fixture in @codemirror/state 6.7.6 has length 7 in UTF-16.
    let original = Text::from_lines(&["a💡", "", "β", ""]);
    assert_eq!(original.len(), 10);
    assert!(!original.is_empty());
    assert_eq!(original.line_count(), 4);

    let insert = Text::from_lines(&["é", "Z"]);
    assert_eq!(
        original.replace(1..7, &insert).unwrap().as_str(),
        "aé\nZβ\n"
    );
    let owned = original.slice_owned(1..9).unwrap();
    assert_eq!(original.slice(1..9), Some(owned.as_str()));
    assert_eq!(owned.as_str(), "💡\n\nβ");
    assert_eq!(
        original.iter_range(1..9).unwrap().collect::<Vec<_>>(),
        ["💡", "\n", "\n", "β"]
    );
    assert_eq!(
        original.iter_range(1..9).unwrap().rev().collect::<Vec<_>>(),
        ["β", "\n", "\n", "💡"]
    );
    assert_eq!(
        original.append(Text::new("🌱\n")).as_str(),
        "a💡\n\nβ\n🌱\n"
    );
    assert_eq!(original.to_string(), "a💡\n\nβ\n");
    assert!(original.replace(0..original.len(), "").unwrap().is_empty());
}

#[test]
fn tree_splices_match_pinned_code_mirror_text() {
    for row in include_str!("fixtures/text_splice.jsonl").lines() {
        let case: serde_json::Value = serde_json::from_str(row).unwrap();
        let lines = |key: &str| {
            case[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|line| line.as_str().unwrap())
                .collect::<Vec<_>>()
        };
        let range = |key: &str| {
            let [start, end] = serde_json::from_value::<[usize; 2]>(case[key].clone()).unwrap();
            start..end
        };
        let source = Text::new(lines("source").join("\n"));
        let insert = if case["insert"].is_array() {
            Text::new(lines("insert").join("\n"))
        } else if case["slice"].is_array() {
            source.slice_owned(range("slice")).unwrap()
        } else {
            source.clone()
        };
        let changed = source.replace_text(range("range"), &insert).unwrap();
        assert_eq!(changed.to_lines(), lines("changed"), "{}", case["name"]);
        let appended = source.append_text(&insert).unwrap();
        assert_eq!(appended.to_lines(), lines("appended"), "{}", case["name"]);
        let rebuilt = Text::from_lines(&lines("changed"));
        assert_eq!(
            serde_json::json!([changed == rebuilt, changed == source, appended == changed]),
            case["equal"],
            "{}",
            case["name"]
        );
        assert_eq!(source.to_lines(), lines("source"));
    }
}

#[test]
fn checks_byte_ranges_without_clipping() {
    let text = Text::new("é💡\r\n");
    for range in [1..2, 2..4, 9..9, Range { start: 6, end: 2 }] {
        assert!(text.slice(range.clone()).is_none());
        assert!(text.slice_owned(range.clone()).is_none());
        assert!(text.iter_range(range.clone()).is_none());
        assert_eq!(text.replace(range, "X"), Err(EditError::InvalidRange));
    }
    let end = text.len();
    assert!(text.slice_owned(end..end).unwrap().is_empty());
    assert!(text.iter_range(end..end).unwrap().next().is_none());
    assert_eq!(text.replace(end..end, "X").unwrap().as_str(), "é💡\r\nX");
    assert_eq!(text.as_str(), "é💡\r\n");
}

#[test]
fn retains_empty_lines_and_rebuilds_lf_documents() {
    for lines in [&[""][..], &["x", "", "λ", "", ""]] {
        let text = Text::from_lines(lines);
        assert_eq!(text.line_count(), lines.len());
        assert_eq!(text.to_lines(), lines);
        assert_eq!(text, Text::from_lines(&text.to_lines()));
    }
    let text = Text::from_lines(&["a💡", "", "β", ""]);
    assert_eq!(
        text.iter_lines().skip(1).take(2).collect::<Vec<_>>(),
        ["", "β"]
    );
    assert_eq!(
        text.iter_lines().rev().collect::<Vec<_>>(),
        ["", "β", "", "a💡"]
    );

    let empty = Text::default();
    assert_eq!(Text::from_lines(&[]), empty);
    assert_eq!(empty.len(), 0);
    assert!(empty.is_empty());
    assert_eq!(empty.line_count(), 1);
    assert_eq!(empty.to_lines(), [""]);
    assert_eq!(empty.iter().next(), None);
}

#[test]
fn iterates_mixed_line_endings_without_changing_bytes() {
    let source = "α🦀\r\n\r\nβ\rγ\n\n終\r";
    let text = Text::new(source);
    let chunks = ["α🦀", "\r\n", "\r\n", "β\rγ", "\n", "\n", "終\r"];
    assert!(text.iter().eq(chunks));
    assert!(text.iter().rev().eq(chunks.into_iter().rev()));
    assert_eq!(
        text.iter().collect::<String>().as_bytes(),
        source.as_bytes()
    );
    assert_eq!(text.to_lines(), ["α🦀", "", "β\rγ", "", "終\r"]);
    assert_eq!(text.line_count(), 5);
    assert_ne!(text, Text::from_lines(&text.to_lines()));

    // Each end is inside a CRLF. The two included bytes are from separate breaks.
    let mut between = text.iter_range(7..9).unwrap();
    assert_eq!(between.next_back(), Some("\r"));
    assert_eq!(between.next(), Some("\n"));
    assert_eq!(between.next_back(), None);
    assert!(text.iter_range(2..7).unwrap().eq(["🦀", "\r"]));
    assert!(text.iter_range(2..7).unwrap().rev().eq(["\r", "🦀"]));
}

#[test]
fn reconstructs_each_checked_byte_range_from_chunks() {
    let source = "é\r\nα\n🦀\r";
    let text = Text::new(source);
    for start in 0..=source.len() {
        for end in start..=source.len() {
            let range = start..end;
            let Some(expected) = source.get(range.clone()) else {
                assert!(text.iter_range(range).is_none());
                continue;
            };
            let chunks: Vec<_> = text.iter_range(range.clone()).unwrap().collect();
            assert_eq!(chunks.concat().as_bytes(), expected.as_bytes(), "{range:?}");
            assert!(
                text.iter_range(range)
                    .unwrap()
                    .rev()
                    .eq(chunks.into_iter().rev())
            );
        }
    }
}

#[test]
fn preserves_bytes_when_edits_form_or_split_crlf() {
    let text = Text::new("é\r").append("\nx\n");
    assert!(text.iter().eq(["é", "\r\n", "x", "\n"]));
    assert_eq!(text.replace(2..3, "").unwrap().as_str(), "é\nx\n");
    assert_eq!(text.replace(3..4, "").unwrap().as_str(), "é\rx\n");
    assert_eq!(text.slice_owned(2..3).unwrap().to_lines(), ["\r"]);
    assert_eq!(Text::new("x\r\r\n").to_lines(), ["x\r", ""]);
}

#[test]
fn line_and_range_queries_cross_chunk_seams() {
    let prefix = "x".repeat(1023);
    let source = format!("{prefix}\r\n\r\né💡\n\n\r");
    let text = Text::new(&source);
    assert_eq!(text.line_count(), 5);
    for at in [0, 1023, 1024] {
        let line = text.line_at(at).unwrap();
        assert_eq!(line.number, 1);
        assert_eq!(line.range, 0..1023);
        assert_eq!(line.text, prefix);
    }
    for at in [1025, 1026] {
        let line = text.line_at(at).unwrap();
        assert_eq!(line.number, 2);
        assert_eq!(line.range, 1025..1025);
        assert!(line.text.is_empty());
    }
    assert_eq!(text.line_at(1027).unwrap(), text.line(3).unwrap());
    assert_eq!(text.line_at(1028), None);
    assert_eq!(text.line_at(1032), None);
    assert_eq!(text.line_at(text.len()).unwrap(), text.line(5).unwrap());
    assert_eq!(text.line_at(text.len() + 1), None);
    assert_eq!(text.line(0), None);
    assert_eq!(text.line(6), None);

    let mut lines = text.iter_lines();
    assert_eq!(lines.next_back(), Some("\r"));
    assert_eq!(lines.next(), Some(prefix.as_str()));
    assert_eq!(lines.next_back(), Some(""));
    assert_eq!(lines.next(), Some(""));
    assert_eq!(lines.next_back(), Some("é💡"));
    assert_eq!(lines.next(), None);
    assert_eq!(lines.next_back(), None);

    let spans = [
        0..1023,
        1023..1025,
        1025..1027,
        1027..1033,
        1033..1034,
        1034..1035,
        1035..1036,
    ];
    for start in 1020..=source.len() {
        for end in start..=source.len() {
            let actual = text.iter_range(start..end);
            if source.get(start..end).is_none() {
                assert!(actual.is_none());
                continue;
            }
            let mut actual = actual.unwrap();
            let mut expected: std::collections::VecDeque<_> = spans
                .iter()
                .filter_map(|span| {
                    let from = span.start.max(start);
                    let to = span.end.min(end);
                    (from < to).then(|| &source[from..to])
                })
                .collect();
            while !expected.is_empty() {
                assert_eq!(actual.next_back(), expected.pop_back(), "{start}..{end}");
                assert_eq!(actual.next(), expected.pop_front(), "{start}..{end}");
            }
            assert_eq!(actual.next(), None);
            assert_eq!(actual.next_back(), None);
        }
    }
}

#[test]
fn edit_and_slice_across_many_chunks_match_contiguous_bytes() {
    // Adapted from @codemirror/state 6.7.6. See docs/licenses/codemirror-state.txt.
    let line = "1234567890".repeat(10);
    let original = Text::from_lines(&[line.as_str(); 200]);
    let source = original.to_string();
    let insert = "abc".repeat(200);
    let mut edited = original.clone();
    let at = source.len() / 2;
    for _ in 0..10 {
        edited = edited.replace(at..at, &insert).unwrap();
    }
    let mut expected = source.clone();
    expected.insert_str(at, &insert.repeat(10));
    assert_eq!(edited, Text::new(&expected));
    assert_eq!(edited.to_string(), expected);
    assert_eq!(original.to_string(), source);
    assert_eq!(
        edited.slice_owned(at - 30..at + 7000).unwrap().as_str(),
        &expected[at - 30..at + 7000]
    );
    assert_ne!(edited, edited.replace(at..at + 1, "z").unwrap());
    assert_eq!(edited.replace(0..at, "").unwrap().as_str(), &expected[at..]);
    assert_eq!(
        edited.replace(at..edited.len(), "").unwrap().as_str(),
        &expected[..at]
    );
}
