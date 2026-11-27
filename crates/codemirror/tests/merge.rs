use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, Text,
    merge::{Chunk, DiffChange, DiffConfig, Side, diff, presentable_diff},
};

fn change(a0: usize, a1: usize, b0: usize, b1: usize) -> DiffChange {
    DiffChange {
        a: a0..a1,
        b: b0..b1,
    }
}

#[test]
fn diff_and_display_match_pinned_examples() {
    // @codemirror/merge 6.13.0, with UTF-16 offsets converted to UTF-8.
    let fixtures = [
        (
            "hello world",
            "hello there",
            vec![change(6, 8, 6, 9), change(9, 11, 10, 11)],
            vec![change(6, 11, 6, 11)],
        ),
        (
            "abcd",
            "acbd",
            vec![change(1, 2, 1, 1), change(3, 3, 2, 3)],
            vec![change(1, 3, 1, 3)],
        ),
        (
            "hello",
            "heyyyyllo",
            vec![change(2, 2, 2, 6)],
            vec![change(0, 5, 0, 9)],
        ),
        (
            "abc αβα def",
            "abc αζα def",
            vec![change(6, 8, 6, 8)],
            vec![change(4, 10, 4, 10)],
        ),
        (
            "😀 écho",
            "🌱 écha",
            vec![change(0, 4, 0, 4), change(9, 10, 9, 10)],
            vec![change(0, 10, 0, 10)],
        ),
    ];
    for (a, b, raw, display) in fixtures {
        let result = diff(a, b);
        assert!(result.precise);
        assert_eq!(result.changes, raw, "{a:?} vs {b:?}");
        assert_eq!(presentable_diff(a, b).changes, display, "{a:?} vs {b:?}");
    }
    assert!(
        diff("no change\r\n😀", "no change\r\n😀")
            .changes
            .is_empty()
    );
}

#[test]
fn presentable_unicode_thresholds_match_the_pinned_diff() {
    use serde_json::{Value, json};

    for row in include_str!("fixtures/merge_presentable_utf16.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let a = row["a"].as_str().unwrap();
        let b = row["b"].as_str().unwrap();
        for (result, expected) in [
            (diff(a, b), &row["raw"]),
            (presentable_diff(a, b), &row["display"]),
        ] {
            assert!(result.precise, "{a:?} vs {b:?}");
            let actual: Vec<_> = result
                .changes
                .into_iter()
                .map(|change| json!([change.a.start, change.a.end, change.b.start, change.b.end]))
                .collect();
            assert_eq!(&json!(actual), expected, "{a:?} vs {b:?}");
        }
    }
}

#[test]
fn chunk_ranges_and_relative_changes_match_pinned_examples() {
    // Chunk ranges stop at real EOF rather than the package's sentinel EOF + 1.
    let a = Text::new("cut A\nkeep\ncut B");
    let b = Text::new("put A\nkeep\nput B");
    let chunks = Chunk::build(&a, &b);
    assert_eq!(chunks.len(), 2);
    assert_chunk(&chunks[0], change(0, 6, 0, 6), &[change(0, 3, 0, 3)]);
    assert_chunk(&chunks[1], change(11, 16, 11, 16), &[change(0, 3, 0, 3)]);
    assert_eq!(
        chunks[1].absolute_changes().collect::<Vec<_>>(),
        [change(11, 14, 11, 14)]
    );
    assert_eq!(
        chunks[1].apply(&a, &b, Side::A).unwrap().as_str(),
        "cut A\nkeep\nput B"
    );
    assert_eq!(
        chunks[0].apply(&a, &b, Side::B).unwrap().as_str(),
        "cut A\nkeep\nput B"
    );
    assert_chunks(&a, &b, &chunks);

    let a = Text::new("one cat; one fox");
    let b = Text::new("one dog; one yak");
    let chunks = Chunk::build(&a, &b);
    assert_eq!(chunks.len(), 1);
    assert_chunk(
        &chunks[0],
        change(0, 16, 0, 16),
        &[change(4, 7, 4, 7), change(13, 16, 13, 16)],
    );

    let a = Text::new("a\nb");
    let b = Text::new("a\nnew\nb");
    let chunks = Chunk::build(&a, &b);
    assert_eq!(chunks.len(), 1);
    assert_chunk(&chunks[0], change(2, 2, 2, 6), &[change(0, 0, 0, 4)]);
    assert_chunks(&a, &b, &chunks);
}

#[test]
fn applying_chunks_keeps_both_documents_valid_at_crlf_and_eof() {
    for (a, b) in [
        ("前\r\nkeep\r\n🍎", "後\r\nkeep\r\n🍋\r\n"),
        ("a\r\nb", "a\nb"),
        ("x\nfoo\n", "x\nbar"),
        ("a\nb", "a\nb\n"),
        ("a\nb\n", "a\nb\n\n"),
        ("a\nb\n", "a\nb\né"),
        ("", "\r\n"),
        ("", "\n\n"),
        ("", ""),
        ("\na", "\n\na"),
        ("\n ", "\n\n\n\n\n\nabbbba \n "),
    ] {
        let (a, b) = (Text::new(a), Text::new(b));
        assert_chunks(&a, &b, &Chunk::build(&a, &b));
        assert_chunks(&b, &a, &Chunk::build(&b, &a));
    }

    let a = Text::new("a\r\nb");
    let b = Text::new("a\nb");
    let chunks = Chunk::build(&a, &b);
    assert_chunk(&chunks[0], change(0, 3, 0, 2), &[change(1, 2, 1, 1)]);
    assert_eq!(
        chunks[0].apply(&a, &Text::new("🦀"), Side::A),
        Err(EditError::InvalidRange)
    );
    assert_eq!(
        chunks[0].apply(&Text::new("🦀"), &b, Side::A),
        Err(EditError::InvalidRange)
    );
}

#[test]
fn limited_search_outputs_a_valid_coarse_diff() {
    let a = format!("start{}end", "a".repeat(4_000));
    let b = format!("start{}end", "b".repeat(4_000));
    let config = DiffConfig { scan_limit: 4 };
    let result = config.diff(&a, &b);
    assert!(!result.precise);
    assert_eq!(result.changes, [change(5, 4_005, 5, 4_005)]);
    let (a, b) = (Text::new(a), Text::new(b));
    let chunks = Chunk::build_with_config(&a, &b, config);
    assert!(!chunks[0].precise());
    assert_chunks(&a, &b, &chunks);
}

#[test]
fn incremental_diff_matches_pinned_a_and_b_updates() {
    let pad = "keep-0123456789\n".repeat(90);
    let mut a = Text::new(format!("old head\n{pad}cat here\n{pad}old tail"));
    let mut b = Text::new(format!("new head\n{pad}dog here\n{pad}new tail"));
    let initial = Chunk::build(&a, &b);
    assert_eq!(initial.len(), 3);
    let at = a.as_str().find("cat here").unwrap();
    let changes = ChangeSet::new(vec![
        Change::new(at..at + 3, "rare bird"),
        Change::new(at + 4..at + 4, "now "),
    ])
    .unwrap();
    let desc = changes.desc(a.len()).unwrap();
    changes.apply_to(&mut a).unwrap();
    let updated_a = Chunk::update(&initial, &a, &b, Side::A, &desc, DiffConfig::default());
    assert_eq!(updated_a.len(), 3);
    assert_eq!(updated_a[0], initial[0]);
    assert_chunk(
        &updated_a[1],
        change(1449, 1468, 1449, 1458),
        &[change(0, 13, 0, 3)],
    );
    assert_chunk(
        &updated_a[2],
        change(2908, 2916, 2898, 2906),
        &[change(0, 3, 0, 3)],
    );
    assert_chunks(&a, &b, &updated_a);

    let at = b.as_str().find("dog here").unwrap();
    let changes = ChangeSet::single(Change::new(at..at + 8, "rare bird now here")).unwrap();
    let desc = changes.desc(b.len()).unwrap();
    changes.apply_to(&mut b).unwrap();
    let updated_b = Chunk::update(&updated_a, &a, &b, Side::B, &desc, DiffConfig::default());
    assert_eq!(updated_b.len(), 2);
    assert_eq!(updated_b[0], initial[0]);
    assert_chunk(
        &updated_b[1],
        change(2908, 2916, 2908, 2916),
        &[change(0, 3, 0, 3)],
    );
    assert_chunks(&a, &b, &updated_b);
}

#[test]
fn incremental_diff_accepts_utf8_crlf_and_eof_edits() {
    let pad = "😀αβ keep\r\n".repeat(200);
    let mut a = Text::new(format!("hello\r\n{pad}é🍊\r\n{pad}EOF\r\n"));
    let mut b = Text::new(format!("greets\r\n{pad}í🍊\r\n{pad}EOF"));
    let chunks = Chunk::build(&a, &b);
    let at = a.as_str().find("é🍊").unwrap();
    let changes = ChangeSet::single(Change::new(at..at + 'é'.len_utf8(), "í")).unwrap();
    let desc = changes.desc(a.len()).unwrap();
    changes.apply_to(&mut a).unwrap();
    let chunks = Chunk::update(&chunks, &a, &b, Side::A, &desc, DiffConfig::default());
    assert_eq!(chunks, Chunk::build(&a, &b));
    assert_chunks(&a, &b, &chunks);

    let end = b.len();
    let changes = ChangeSet::single(Change::new(end..end, "\r\n")).unwrap();
    let desc = changes.desc(end).unwrap();
    changes.apply_to(&mut b).unwrap();
    let updated = Chunk::update(&chunks, &a, &b, Side::B, &desc, DiffConfig::default());
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0], chunks[0]);
    assert_chunks(&a, &b, &updated);
}

#[test]
fn incremental_updates_reconstruct_documents_after_mixed_edits() {
    let mut a = Text::new(
        (0..300)
            .map(|i| format!("record {i}: η🍊 and tail\r\n"))
            .collect::<String>(),
    );
    let mut b = a.clone();
    let mut chunks = Vec::new();
    let mut state = 158u32;
    let config = DiffConfig::default();
    for i in 0..80 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let (side, edited) = if i % 2 == 0 {
            (Side::A, &mut a)
        } else {
            (Side::B, &mut b)
        };
        let text = edited.as_str();
        let mut from = state as usize % text.len();
        while !text.is_char_boundary(from) {
            from -= 1;
        }
        let width = if i % 19 == 0 { 1_020 } else { i % 17 };
        let mut to = (from + width).min(text.len());
        while !text.is_char_boundary(to) {
            to += 1;
        }
        let insert = ["", "🌱\r\n", "\n", "α text", "0"][i % 5];
        let mut edits = vec![Change::new(from..to, insert)];
        if i % 7 == 0 {
            if from > 2_100 {
                edits.push(Change::new(0..0, "α header\r\n"));
            } else if text.len() - to > 2_100 {
                edits.push(Change::new(text.len()..text.len(), "\r\ntail 🍊"));
            }
        }
        let edit = ChangeSet::new(edits).unwrap();
        let desc = edit.desc(edited.len()).unwrap();
        edit.apply_to(edited).unwrap();
        chunks = Chunk::update(&chunks, &a, &b, side, &desc, config);
        assert_chunks(&a, &b, &chunks);
    }
}

#[test]
fn diff_and_chunks_reconstruct_mixed_short_inputs() {
    let mut state = 17u32;
    let mut next = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 16) as usize
    };
    let chars = ['a', 'b', 'α', '🍊', ' ', '\r', '\n', '\n'];
    for _ in 0..8_000 {
        let a: String = (0..next() % 28)
            .map(|_| chars[next() % chars.len()])
            .collect();
        let b: String = (0..next() % 28)
            .map(|_| chars[next() % chars.len()])
            .collect();
        assert_gaps(&a, &b, diff(&a, &b).changes);
        assert_gaps(&a, &b, presentable_diff(&a, &b).changes);
        let (a, b) = (Text::new(a), Text::new(b));
        assert_chunks(&a, &b, &Chunk::build(&a, &b));
    }
}

fn assert_chunk(chunk: &Chunk, bounds: DiffChange, changes: &[DiffChange]) {
    assert_eq!(chunk.range(Side::A), bounds.a);
    assert_eq!(chunk.range(Side::B), bounds.b);
    assert_eq!(chunk.changes(), changes);
}

fn assert_chunks(a: &Text, b: &Text, chunks: &[Chunk]) {
    assert_gaps(
        a.as_str(),
        b.as_str(),
        chunks.iter().map(|ch| DiffChange {
            a: ch.range(Side::A),
            b: ch.range(Side::B),
        }),
    );
    let (mut into_a, mut into_b) = (a.clone(), b.clone());
    for chunk in chunks.iter().rev() {
        let (ra, rb) = (chunk.range(Side::A), chunk.range(Side::B));
        for (range, doc) in [(&ra, a), (&rb, b)] {
            let bytes = doc.as_str().as_bytes();
            assert!(range.start == 0 || bytes[range.start - 1] == b'\n');
            assert!(range.end == bytes.len() || range.end == 0 || bytes[range.end - 1] == b'\n');
        }
        assert_gaps(
            a.slice(ra).unwrap(),
            b.slice(rb).unwrap(),
            chunk.changes().iter().cloned(),
        );
        into_a = chunk.apply(&into_a, b, Side::A).unwrap();
        into_b = chunk.apply(a, &into_b, Side::B).unwrap();
    }
    assert_eq!(&into_a, b);
    assert_eq!(&into_b, a);
}

fn assert_gaps(a: &str, b: &str, changes: impl IntoIterator<Item = DiffChange>) {
    let (mut pa, mut pb) = (0, 0);
    for ch in changes {
        assert!(
            ch.a.start >= pa && ch.b.start >= pb,
            "{a:?} vs {b:?}: {ch:?}"
        );
        assert!(
            a.get(ch.a.clone()).is_some() && b.get(ch.b.clone()).is_some(),
            "{a:?} vs {b:?}: {ch:?}"
        );
        assert_eq!(
            a[pa..ch.a.start],
            b[pb..ch.b.start],
            "{a:?} vs {b:?}: {ch:?}"
        );
        (pa, pb) = (ch.a.end, ch.b.end);
    }
    assert_eq!(a[pa..], b[pb..], "{a:?} vs {b:?}");
}
