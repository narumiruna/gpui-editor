use gpui_editor_codemirror::{Text, language::lezer_grammar::LezerDocument};

// The matching Node EditorState checks are in the app's lezer_oracle.mjs.
#[test]
fn plain_scan_counts_utf16_units_and_normalized_line_breaks() {
    for ending in ["\r\n", "\r", "\n"] {
        for (count, forward_matches) in [(4999, true), (5000, false)] {
            let source = format!("// [{}{}// ]", "😀".repeat(count), ending);
            let end = source.len();
            let doc = LezerDocument::parse(Text::new(source));
            let forward = doc.bracket_near(3);
            if forward_matches {
                let hit = forward.unwrap();
                assert!(hit.matched);
                assert_eq!(hit.start, 3..4);
                assert_eq!(hit.end, Some(end - 1..end));
            } else {
                assert!(forward.is_none());
            }
            let backward = doc.bracket_near(end).unwrap();
            assert!(backward.matched);
            assert_eq!(backward.end, Some(3..4));
            assert!(doc.bracket_near(5).is_none(), "not a UTF-8 boundary");
        }
    }
}

#[test]
fn marked_pairs_are_not_limited_by_plain_scan_distance() {
    let source = format!("Root {{\r\n{}\r\n}}", " ".repeat(15000));
    let end = source.len();
    let doc = LezerDocument::parse(Text::new(source));
    let hit = doc.bracket_near(5).unwrap();
    assert!(hit.matched);
    assert_eq!(hit.start, 5..6);
    assert_eq!(hit.end, Some(end - 1..end));
}
