use super::Text;
use crate::{Change, ChangeSet, EditError, EditorState, Transaction};

#[test]
fn only_cross_chunk_borrows_cache_the_contiguous_snapshot() {
    let input = "ab\r\n".repeat(4096);
    let text = Text::new(&input);
    let clone = text.clone();
    let seam = text.0.tree.chunk_at(0).unwrap().end();
    assert_eq!(text.slice(0..2), Some("ab"));
    assert_eq!(text.line(1).unwrap().text, "ab");
    assert_eq!(text.line_at(seam).unwrap().range, seam..seam + 2);
    assert!(
        text.iter()
            .eq(["ab", "\r\n"].into_iter().cycle().take(8192))
    );
    assert!(
        text.iter_lines()
            .rev()
            .eq(std::iter::once("").chain(std::iter::repeat_n("ab", 4096)))
    );
    assert_eq!(
        text.slice_owned(seam - 3..seam + 3).unwrap().to_string(),
        input[seam - 3..seam + 3]
    );
    assert_eq!(
        text.slice_string(seam - 3..seam + 3).as_deref(),
        input.get(seam - 3..seam + 3)
    );
    assert_eq!(text, Text::new(&input));
    assert_eq!(text.to_string(), input);
    assert!(text.0.flat.get().is_none());

    assert_eq!(
        text.slice(seam - 3..seam + 3),
        input.get(seam - 3..seam + 3)
    );
    assert!(text.0.flat.get().is_some());
    assert!(std::ptr::eq(text.as_str(), clone.as_str()));
    let edited = text.replace(seam..seam, "X").unwrap();
    assert!(edited.0.flat.get().is_none());
    assert_eq!(clone.as_str(), input);
}

#[test]
fn a_line_crossing_several_chunks_has_one_contiguous_borrow() {
    let line = "abc💡".repeat(2000);
    let text = Text::new(format!("top\r\n{line}\nend"));
    assert!(text.0.flat.get().is_none());
    assert_eq!(text.iter().next(), Some("top"));
    assert_eq!(text.iter().next_back(), Some("end"));
    assert_eq!(
        text.cursor_range(text.len(), 0).unwrap().next(),
        Some("end")
    );
    assert_eq!(text.iter_line_range(3..4).unwrap().next(), Some("end"));
    assert_eq!(
        text.slice_with_line_separator(0..text.len(), "|").unwrap(),
        format!("top|{line}|end")
    );
    assert!(text.0.flat.get().is_none());
    let middle = text.line(2).unwrap();
    assert_eq!(middle.range, 5..line.len() + 5);
    assert_eq!(middle.text, line);
    assert!(text.0.flat.get().is_some());
    assert!(std::ptr::eq(middle.text, text.iter_lines().nth(1).unwrap()));
}

#[test]
fn line_cursor_copies_only_a_spanning_line_and_keeps_the_snapshot_unflattened() {
    use std::borrow::Cow;

    let prefix = "x".repeat(1023);
    let long = "abc💡".repeat(700);
    let source = format!("{prefix}\r\n{long}\nend\n");
    let text = Text::new(&source);
    // The first CR and LF are in different leaves.
    assert_eq!(text.0.tree.chunk_at(0).unwrap().end(), 1024);
    let mut cursor = text.line_cursor();
    assert!(matches!(cursor.next(), Some(Cow::Borrowed(row)) if row == prefix));
    assert!(matches!(cursor.next(), Some(Cow::Owned(row)) if row == long));
    assert!(matches!(cursor.next(), Some(Cow::Borrowed("end"))));
    assert!(matches!(cursor.next(), Some(Cow::Borrowed(""))));
    assert_eq!(cursor.next(), None);
    assert_eq!(cursor.next_with_skip(isize::MIN), Ok(None));

    let mut cursor = text.line_cursor();
    assert_eq!(
        cursor.next_with_skip(isize::MAX).unwrap().as_deref(),
        Some("")
    );
    assert_eq!(
        cursor.next_with_skip(isize::MIN).unwrap().as_deref(),
        Some(prefix.as_str())
    );
    let mut ranged = text.line_cursor_range(2..4).unwrap();
    assert!(matches!(ranged.next(), Some(Cow::Owned(row)) if row == long));
    assert!(matches!(ranged.next(), Some(Cow::Borrowed("end"))));
    assert_eq!(ranged.next_with_skip(isize::MIN), Ok(None));
    assert_eq!(text.to_string(), source);
    assert!(text.0.flat.get().is_none());
}

#[test]
fn local_edits_and_owned_slices_share_branches() {
    let text = Text::new("abλ\n".repeat(60_000));
    let at = text.len() / 2;
    let edited = text.replace(at..at + 4, "\r\nθ").unwrap();
    let slice = text.slice_owned(text.len() / 4..at).unwrap();
    assert!(text.0.tree.shared_branches(&edited.0.tree) > 100);
    assert!(text.0.tree.shared_branches(&slice.0.tree) > 40);
    assert_eq!(text.slice(at..at + 5), Some("abλ\n"));
    assert_eq!(edited.slice(at..at + 5), Some("\r\nθ\n"));
    assert!(text.0.flat.get().is_none());
    assert!(edited.0.flat.get().is_none());
    assert!(slice.0.flat.get().is_none());
}

#[test]
fn splices_share_both_inputs_and_keep_raw_breaks_at_each_join() {
    let prefix = "aβ\n".repeat(10_000);
    let suffix = "尾\r\n".repeat(10_000);
    let source = Text::new(format!("{prefix}\rX\n{suffix}"));
    let insert = Text::new(format!("\n{}\r", "💡x\n".repeat(30_000)));
    let at = prefix.len() + 1;
    let output = source.replace_text(at..at + 1, &insert).unwrap();
    let appended = source.append_text(&insert).unwrap();
    let doubled = source.append_text(&source).unwrap();
    for (text, expected) in [
        (&output, format!("{prefix}\r{insert}\n{suffix}")),
        (&appended, format!("{source}{insert}")),
        (&doubled, format!("{source}{source}")),
    ] {
        text.0.tree.check_structure();
        assert_eq!(text.to_string(), expected);
        assert!(source.0.tree.shared_branches(&text.0.tree) > 60);
        assert!(text.0.flat.get().is_none());
    }
    for text in [&output, &appended] {
        assert!(insert.0.tree.shared_branches(&text.0.tree) > 120);
    }
    assert_eq!(output.line(10_001).unwrap().text, "");
    assert_eq!(output.line(40_002).unwrap().text, "");
    assert_eq!(output.line(40_003).unwrap().text, "尾");
    assert!(source.0.flat.get().is_none());
    assert!(insert.0.flat.get().is_none());
}

#[test]
fn tree_splices_check_ranges_before_sharing_snapshots() {
    let source = Text::new("a🦀\r\n");
    let insert = Text::new("λ");
    for range in [2..2, 0..2, 7..8, std::ops::Range { start: 5, end: 1 }] {
        assert_eq!(
            source.replace_text(range, &insert),
            Err(EditError::InvalidRange)
        );
    }
    let empty = Text::default();
    assert!(std::sync::Arc::ptr_eq(
        &source.append_text(&empty).unwrap().0,
        &source.0
    ));
    assert!(std::sync::Arc::ptr_eq(
        &empty.append_text(&source).unwrap().0,
        &source.0
    ));
    assert!(std::sync::Arc::ptr_eq(
        &source.replace_text(0..source.len(), &insert).unwrap().0,
        &insert.0,
    ));
    let split = source.replace_text(6..6, &insert).unwrap();
    assert_eq!(split.to_string(), "a🦀\rλ\n");
    assert_eq!(split.to_lines(), ["a🦀\rλ", ""]);
}

#[test]
fn equality_compares_bytes_across_shared_and_differently_packed_trees() {
    let mut shared = Text::new("aβ\n".repeat(256));
    for _ in 0..16 {
        shared = shared.append_text(&shared).unwrap();
    }
    let before = Text::new("X\r");
    let after = Text::new("tail 🦀\n");
    let left = before
        .append_text(&shared)
        .unwrap()
        .append_text(&after)
        .unwrap();
    let right = before.append_text(&shared.append(after.as_str())).unwrap();
    assert_ne!(left.len(), shared.len());
    assert!(!std::sync::Arc::ptr_eq(&left.0, &right.0));
    assert_eq!(left, right);
    let changed = right
        .replace(right.len() - 5..right.len() - 1, "🐱")
        .unwrap();
    assert_ne!(left, changed);
    assert_eq!(left.len(), changed.len());
    assert_eq!(left.line_count(), changed.line_count());
    assert!(left.0.flat.get().is_none());
    assert!(right.0.flat.get().is_none());

    let source = "one é🦀\r\n".repeat(500);
    let original = Text::new(&source);
    let at = source.floor_char_boundary(773);
    let joined = Text::new(&source[..at])
        .append_text(&Text::new(&source[at..]))
        .unwrap();
    assert_eq!(joined, original);
    assert_ne!(joined.replace(4..6, "λ").unwrap(), original);
}

#[test]
fn shared_text_splices_reject_length_overflow_without_copying() {
    let mut text = Text::new("x".repeat(1024));
    while text.len() <= isize::MAX as usize / 2 {
        text = text.append_text(&text).unwrap();
    }
    let len = text.len();
    assert_eq!(text.append_text(&text), Err(EditError::LengthOverflow));
    assert_eq!(
        text.replace_text(0..0, &text),
        Err(EditError::LengthOverflow)
    );
    let mut max = text.replace_text(0..1, &text).unwrap();
    assert_eq!(max.len(), isize::MAX as usize);
    assert_eq!(max.replace(0..0, "x"), Err(EditError::LengthOverflow));
    let changes = ChangeSet::single(Change::new(0..0, "x")).unwrap();
    let previous = max.clone();
    assert_eq!(changes.apply_to(&mut max), Err(EditError::LengthOverflow));
    assert!(std::sync::Arc::ptr_eq(&previous.0, &max.0));
    assert_eq!(text.len(), len);
    assert_eq!(text.slice(len - 1..len), Some("x"));
    assert!(text.0.flat.get().is_none());
}

#[test]
fn transactions_share_old_storage_without_flattening() {
    let source = "λx\r\n".repeat(50_000);
    let mut state = EditorState::new(source);
    let original = state.text().clone();
    let changes = ChangeSet::single(Change::new(5..7, "ab")).unwrap();
    let inverse = changes.inverted(&original).unwrap();
    state.update(Transaction::new(changes)).unwrap();
    let edited = state.text().clone();
    assert_eq!(edited.slice(0..10), Some("λx\r\nabx\r\n"));
    assert!(original.0.tree.shared_branches(&edited.0.tree) > 100);
    state.update(Transaction::new(inverse)).unwrap();
    assert_eq!(*state.text(), original);
    for text in [&original, &edited, state.text()] {
        assert!(text.0.flat.get().is_none());
    }
}

#[test]
fn repeated_edits_keep_leaves_packed_and_the_tree_balanced() {
    let mut oracle = "a💡\r\nλ".repeat(5_000);
    let mut text = Text::new(&oracle);
    let mut random = 13_u64;
    for i in 0..1500 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let at = oracle.floor_char_boundary((random >> 32) as usize % (oracle.len() + 1));
        let end = oracle.floor_char_boundary((at + i % 11).min(oracle.len()));
        let insert = ["", "\n", "\r", "β\r\n", "🦀éabcdef"][i % 5];
        text = text.replace(at..end, insert).unwrap();
        oracle.replace_range(at..end, insert);
        if i % 50 == 0 {
            text.0.tree.check_structure();
            assert_eq!(text.to_string(), oracle);
        }
    }
    for insert in ["β", "x", "\n", "🦀"].into_iter().cycle().take(3000) {
        text = text.append(insert);
        oracle.push_str(insert);
    }
    text.0.tree.check_structure();
    assert_eq!(text.to_string(), oracle);
    assert!(text.0.flat.get().is_none());
    let head = oracle.floor_char_boundary(2);
    let tail = oracle.len() - "🦀".len();
    let reduced = text.replace(head..tail, "").unwrap();
    reduced.0.tree.check_structure();
    let expected = [&oracle[..head], &oracle[tail..]].concat();
    assert_eq!(reduced.0.tree.contiguous(), Some(expected.as_str()));
}
