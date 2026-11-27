//! Fixed-height rows checked in headless Chrome against @codemirror/merge
//! 6.13.0, with the tarball's SHA-512 checked against the project pin.
//! Fixtures omit zero-height widgets and use native byte offsets.

use gpui_editor_codemirror::{
    Text,
    merge::{Chunk, CollapseConfig, MergeProjection, ProjectionRow, Side},
};
use std::ops::Range;

fn project(a: &str, b: &str, collapse: Option<CollapseConfig>) -> MergeProjection {
    let (a, b) = (Text::new(a), Text::new(b));
    MergeProjection::new(&a, &b, &Chunk::build(&a, &b), collapse)
}

fn line(number: usize, range: Range<usize>) -> ProjectionRow {
    ProjectionRow::Line { number, range }
}

fn collapsed(lines: Range<usize>, range: Range<usize>) -> ProjectionRow {
    ProjectionRow::Collapsed { lines, range }
}

fn spacer(at: usize) -> ProjectionRow {
    ProjectionRow::Spacer { at }
}

fn collapsed_rows(projection: &MergeProjection, side: Side) -> Vec<(usize, ProjectionRow)> {
    projection
        .rows(side)
        .iter()
        .enumerate()
        .filter(|(_, row)| matches!(row, ProjectionRow::Collapsed { .. }))
        .map(|(index, row)| (index, row.clone()))
        .collect()
}

#[test]
fn unchanged_document_collapses_at_the_exact_minimum_without_edge_context() {
    let text = "a\nb\nc\nd";
    let mut view = project(text, text, Some(CollapseConfig::default()));
    assert_eq!(view.rows(Side::A), &[collapsed(1..5, 0..7)]);
    assert_eq!(view.rows(Side::B), view.rows(Side::A));
    assert!(!view.expand(Side::A, 2));
    assert!(view.expand(Side::B, 0));
    assert!(!view.expand(Side::B, 0));
    assert_eq!(
        view.rows(Side::A),
        &[line(1, 0..1), line(2, 2..3), line(3, 4..5), line(4, 6..7)]
    );
    assert_eq!(view.rows(Side::B), view.rows(Side::A));

    let below_minimum = project("a\nb\nc", "a\nb\nc", Some(CollapseConfig::default()));
    assert_eq!(
        below_minimum.rows(Side::A),
        &[line(1, 0..1), line(2, 2..3), line(3, 4..5)]
    );
}

#[test]
fn context_is_kept_at_each_chunk_and_expanding_one_gap_preserves_the_others() {
    let doc = |first: &[&str], second: &str| {
        (1..=8)
            .map(|i| format!("keep {i}"))
            .chain(first.iter().map(|line| (*line).to_owned()))
            .chain((9..=18).map(|i| format!("keep {i}")))
            .chain(std::iter::once(second.to_owned()))
            .chain((19..=26).map(|i| format!("keep {i}")))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let a = doc(&["old"], "gone");
    let b = doc(&["new", "extra"], "fresh");
    let mut view = project(&a, &b, Some(CollapseConfig::default()));
    assert_eq!(view.rows(Side::A).len(), 18);
    assert_eq!(view.rows(Side::B).len(), 18);
    assert_eq!(view.rows(Side::A)[5], spacer(60));
    assert_eq!(
        collapsed_rows(&view, Side::A),
        vec![
            (0, collapsed(1..6, 0..34)),
            (9, collapsed(13..17, 83..114)),
            (17, collapsed(24..29, 168..207)),
        ]
    );
    assert_eq!(
        collapsed_rows(&view, Side::B),
        vec![
            (0, collapsed(1..6, 0..34)),
            (9, collapsed(14..18, 89..120)),
            (17, collapsed(25..30, 175..214)),
        ]
    );
    assert!(!view.expand(Side::B, 83));
    assert!(!view.expand(Side::B, 90));
    assert!(view.expand(Side::B, 89));
    assert!(!view.expand(Side::A, 83));
    assert_eq!(view.rows(Side::A).len(), 21);
    assert_eq!(view.rows(Side::B).len(), 21);
    assert_eq!(view.rows(Side::A)[5], spacer(60));
    assert_eq!(view.rows(Side::A)[9], line(13, 83..90));
    assert_eq!(view.rows(Side::B)[9], line(14, 89..96));
    assert_eq!(
        collapsed_rows(&view, Side::A),
        vec![
            (0, collapsed(1..6, 0..34)),
            (20, collapsed(24..29, 168..207))
        ]
    );
    assert_eq!(
        collapsed_rows(&view, Side::B),
        vec![
            (0, collapsed(1..6, 0..34)),
            (20, collapsed(25..30, 175..214))
        ]
    );

    let config = CollapseConfig {
        margin: 3,
        min_size: 5,
    };
    let threshold = project(&a, &b, Some(config));
    assert_eq!(threshold.rows(Side::A), view.rows(Side::A));
    assert_eq!(threshold.rows(Side::B), view.rows(Side::B));
}

#[test]
fn custom_context_collapses_through_the_final_empty_line() {
    let mut view = project(
        "heading\nold\nread\nkeep\nstay\n",
        "heading\nnew\ninsert\nread\nkeep\nstay\n",
        Some(CollapseConfig {
            margin: 1,
            min_size: 2,
        }),
    );
    assert_eq!(
        view.rows(Side::A),
        &[
            line(1, 0..7),
            line(2, 8..11),
            spacer(12),
            line(3, 12..16),
            collapsed(4..7, 17..27),
        ]
    );
    assert_eq!(
        view.rows(Side::B),
        &[
            line(1, 0..7),
            line(2, 8..11),
            line(3, 12..18),
            line(4, 19..23),
            collapsed(5..8, 24..34),
        ]
    );
    assert!(view.expand(Side::A, 17));
    assert_eq!(view.rows(Side::A).last(), Some(&line(6, 27..27)));
    assert_eq!(view.rows(Side::B).last(), Some(&line(7, 34..34)));
}

#[test]
fn a_partial_line_change_and_insertion_align_at_the_next_shared_line() {
    let view = project(
        "one\npartial cat\nafter",
        "one\npartial dog\nextra\nafter",
        None,
    );
    assert_eq!(
        view.rows(Side::A),
        &[line(1, 0..3), line(2, 4..15), spacer(16), line(3, 16..21)]
    );
    assert_eq!(
        view.rows(Side::B),
        &[
            line(1, 0..3),
            line(2, 4..15),
            line(3, 16..21),
            line(4, 22..27)
        ]
    );
}

#[test]
fn separate_insertions_and_deletions_pad_both_sides_at_shared_anchors() {
    let view = project(
        "same\nremove one\nremove two\nshared\ncommon\nlast",
        "same\nshared\ncommon\nadd one\nadd two\nlast",
        None,
    );
    assert_eq!(view.rows(Side::A).len(), 8);
    assert_eq!(view.rows(Side::B).len(), 8);
    assert_eq!(&view.rows(Side::B)[1..3], &[spacer(5), spacer(5)]);
    assert_eq!(&view.rows(Side::A)[5..7], &[spacer(41), spacer(41)]);
    assert_eq!(view.rows(Side::A)[3], line(4, 27..33));
    assert_eq!(view.rows(Side::B)[3], line(2, 5..11));
    assert_eq!(view.rows(Side::A)[7], line(6, 41..45));
    assert_eq!(view.rows(Side::B)[7], line(6, 35..39));
}

#[test]
fn an_unchanged_empty_line_is_aligned_except_at_eof() {
    let start = project("\none\ntwo", "extra\n\none\ntwo", None);
    assert_eq!(start.rows(Side::A)[..2], [spacer(0), line(1, 0..0)]);
    assert_eq!(start.rows(Side::B)[..2], [line(1, 0..5), line(2, 6..6)]);

    let interior = project("a\n\nb", "a\nadded\n\nb", None);
    assert_eq!(
        interior.rows(Side::A),
        &[line(1, 0..1), spacer(2), line(2, 2..2), line(3, 3..4)]
    );
    assert_eq!(
        interior.rows(Side::B),
        &[line(1, 0..1), line(2, 2..7), line(3, 8..8), line(4, 9..10)]
    );

    let shorter = [line(1, 0..1), line(2, 2..2), spacer(2)];
    let longer = [line(1, 0..1), line(2, 2..7), line(3, 8..8)];
    let trailing = project("a\n", "a\nextra\n", None);
    assert_eq!(trailing.rows(Side::A), &shorter);
    assert_eq!(trailing.rows(Side::B), &longer);
    let reversed = project("a\nextra\n", "a\n", None);
    assert_eq!(reversed.rows(Side::A), &longer);
    assert_eq!(reversed.rows(Side::B), &shorter);
}

#[test]
fn crlf_and_unicode_offsets_use_native_bytes_and_zero_margin_keeps_touching_lines_visible() {
    let mut view = project(
        "前😀\r\nkeep\r\nold é\r\n🍎\r\nend",
        "前😀\r\nkeep\r\nnew é\r\nextra\r\n🍎\r\nend",
        Some(CollapseConfig {
            margin: 0,
            min_size: 1,
        }),
    );
    // The Chrome fixture uses LF in its Text. Native ranges contain CRLF.
    assert_eq!(
        view.rows(Side::A),
        &[
            collapsed(1..3, 0..13),
            line(3, 15..21),
            spacer(23),
            line(4, 23..27),
            line(5, 29..32),
        ]
    );
    assert_eq!(
        view.rows(Side::B),
        &[
            collapsed(1..3, 0..13),
            line(3, 15..21),
            line(4, 23..28),
            line(5, 30..34),
            line(6, 36..39),
        ]
    );
    assert!(!view.expand(Side::A, 1));
    assert!(!view.expand(Side::A, 8));
    assert!(view.expand(Side::A, 0));
    assert_eq!(view.rows(Side::A)[..2], [line(1, 0..7), line(2, 9..13)]);
    assert_eq!(view.rows(Side::B)[..2], [line(1, 0..7), line(2, 9..13)]);
}

#[test]
fn empty_buffers_and_partial_eof_lines_keep_their_coordinates() {
    let collapse = Some(CollapseConfig {
        margin: 0,
        min_size: 1,
    });
    let mut empty = project("", "", collapse);
    assert_eq!(empty.rows(Side::A), &[collapsed(1..2, 0..0)]);
    assert_eq!(empty.rows(Side::B), &[collapsed(1..2, 0..0)]);
    assert!(empty.expand(Side::A, 0));
    assert_eq!(empty.rows(Side::B), &[line(1, 0..0)]);

    let empty_vs_two = project("", "x\ny", collapse);
    assert_eq!(empty_vs_two.rows(Side::A), &[line(1, 0..0), spacer(0)]);
    assert_eq!(empty_vs_two.rows(Side::B), &[line(1, 0..1), line(2, 2..3)]);

    for (b, end) in [("a\nextra", 7), ("a\n", 2)] {
        let ending = project("a", b, collapse);
        assert_eq!(ending.rows(Side::A), &[line(1, 0..1), spacer(1)]);
        assert_eq!(ending.rows(Side::B), &[line(1, 0..1), line(2, 2..end)]);
    }

    let trailing = project("a\n", "a\nextra\n", collapse);
    assert_eq!(
        trailing.rows(Side::A),
        &[collapsed(1..2, 0..1), line(2, 2..2), spacer(2)]
    );
    assert_eq!(
        trailing.rows(Side::B),
        &[collapsed(1..2, 0..1), line(2, 2..7), line(3, 8..8)]
    );
}
