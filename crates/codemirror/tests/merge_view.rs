//! Compared with the SHA-512-verified @codemirror/merge 6.13.0 bundle in
//! headless Chrome. Oracle positions are UTF-8 bytes, with chunk ends clipped
//! to the document. Controls are synchronous and not viewport-limited here.

use std::{ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Change, ChangePermission, ChangeSet, EditError, EditorSelection, EditorState, Extension,
    Transaction, change_filter,
    merge::{
        CollapseConfig, MergeView, MergeViewConfig, RevertControl, Side, get_chunks,
        go_to_next_chunk, go_to_previous_chunk,
    },
};

fn edit(range: Range<usize>, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, insert)).unwrap())
}

fn select(pos: usize) -> Transaction {
    Transaction::default().with_selection(EditorSelection::single(pos, pos))
}

fn view(a: &str, b: &str, config: MergeViewConfig) -> MergeView {
    MergeView::new(EditorState::new(a), EditorState::new(b), config).unwrap()
}

fn assert_chunks(
    view: &MergeView,
    expected: &[[usize; 4]],
) -> Arc<[gpui_editor_codemirror::merge::Chunk]> {
    let (a, a_side) = get_chunks(view.state(Side::A)).unwrap().unwrap();
    let (b, b_side) = get_chunks(view.state(Side::B)).unwrap().unwrap();
    assert_eq!((a_side, b_side), (Side::A, Side::B));
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(view.chunks(), a.as_ref());
    let actual = a
        .iter()
        .map(|chunk| {
            let a = chunk.range(Side::A);
            let b = chunk.range(Side::B);
            [a.start, a.end, b.start, b.end]
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    a
}

fn assert_hidden(view: &MergeView, a: &[Range<usize>], b: &[Range<usize>]) {
    assert_eq!(view.projection().collapsed_ranges(Side::A), a);
    assert_eq!(view.projection().collapsed_ranges(Side::B), b);
    assert_eq!(
        view.projection().rows(Side::A).len(),
        view.projection().rows(Side::B).len()
    );
}

#[test]
fn edits_on_both_sides_map_hidden_ranges_without_recollapsing_unhidden_lines() {
    let old = "start\nold 🍎\nkeep 1\nkeep 2\nkeep 3\nkeep 4\nkeep 5\nkeep 6\nend A\nfar 1\nfar 2\nfar 3\nfar 4";
    let fresh = old
        .replace("old 🍎", "new 🍋\nextra")
        .replace("end A", "end B");
    let collapse = CollapseConfig {
        margin: 1,
        min_size: 2,
    };
    let mut config = MergeViewConfig {
        first: Side::B,
        revert_to: Some(Side::A),
        collapse_unchanged: Some(collapse),
        ..MergeViewConfig::default()
    };
    let mut merge = view(old, &fresh, config);
    assert_chunks(&merge, &[[6, 15, 6, 21], [57, 63, 63, 69]]);
    assert_hidden(&merge, &[22..49, 69..86], &[28..55, 75..92]);
    assert_eq!(
        merge.revert_controls().collect::<Vec<_>>(),
        [
            RevertControl {
                chunk: 0,
                destination: Side::A,
                positions: [6, 6]
            },
            RevertControl {
                chunk: 1,
                destination: Side::A,
                positions: [57, 63]
            },
        ]
    );
    assert!(!merge.expand(Side::B, 22));
    assert!(merge.expand(Side::B, 28));
    assert!(!merge.expand(Side::A, 22));
    assert_hidden(&merge, &one(69..86), &one(75..92));

    merge
        .dispatch_batch(Side::A, [edit(0..0, "😀\n"), edit(5..10, "START")])
        .unwrap();
    assert_eq!(
        merge.state(Side::A).text().as_str(),
        format!("😀\nSTART{}", &old[5..])
    );
    assert_eq!(merge.state(Side::B).text().as_str(), fresh);
    let chunks = assert_chunks(&merge, &[[0, 20, 0, 21], [62, 68, 63, 69]]);
    assert_hidden(&merge, &one(74..91), &one(75..92));

    let next = go_to_next_chunk(merge.state(Side::A)).unwrap().unwrap();
    let previous = go_to_previous_chunk(merge.state(Side::B)).unwrap().unwrap();
    for (side, tr, head) in [(Side::A, next, 62), (Side::B, previous, 63)] {
        assert!(tr.is_user_event("select.byChunk"));
        assert!(tr.scroll_into_view());
        merge.dispatch(side, tr).unwrap();
        assert_eq!(merge.state(side).selection().main().head(), head);
    }
    // Neither explicit selections nor direct edits treat collapsed ranges as protected.
    merge.dispatch(Side::B, select(83)).unwrap();
    assert_eq!(merge.state(Side::B).selection().main().head(), 83);
    assert!(Arc::ptr_eq(
        &chunks,
        &assert_chunks(&merge, &[[0, 20, 0, 21], [62, 68, 63, 69]])
    ));
    assert_hidden(&merge, &one(74..91), &one(75..92));
    merge.dispatch(Side::B, edit(83..84, "🫐")).unwrap();
    assert_chunks(
        &merge,
        &[[0, 20, 0, 21], [62, 68, 63, 69], [80, 86, 81, 90]],
    );
    assert_hidden(&merge, &[], &[]);
    merge.dispatch(Side::B, edit(83..87, "r")).unwrap();
    assert_eq!(merge.state(Side::B).text().as_str(), fresh);
    assert_chunks(&merge, &[[0, 20, 0, 21], [62, 68, 63, 69]]);
    assert_hidden(&merge, &[], &[]);

    config.first = Side::A;
    config.revert_to = Some(Side::B);
    config.collapse_unchanged = None;
    merge.reconfigure(config);
    assert_eq!(merge.config().first, Side::A);
    config.collapse_unchanged = Some(collapse);
    merge.reconfigure(config);
    assert_hidden(&merge, &[27..54, 74..91], &[28..55, 75..92]);
    assert!(merge.revert(1).unwrap());
    assert_chunks(&merge, &[[0, 20, 0, 21]]);
    assert_hidden(&merge, &[27..54, 74..91], &[28..55, 75..92]);
    config.revert_to = Some(Side::A);
    merge.reconfigure(config);
    assert!(merge.revert(0).unwrap());
    assert_chunks(&merge, &[]);
    assert_hidden(&merge, &[28..55, 75..92], &[28..55, 75..92]);
    assert_eq!(merge.state(Side::A).text(), merge.state(Side::B).text());
    assert_eq!(merge.state(Side::A).selection().main().head(), 63);
    assert!(!merge.revert(0).unwrap());
}

#[test]
fn revert_copies_eof_and_empty_chunks_in_both_directions() {
    for (a, b, chunk) in [
        ("a\n", "a\nextra\n", [2, 2, 2, 8]),
        ("a\nb", "a\n\nb", [2, 2, 2, 3]),
        ("same\nold", "same\nnew\n", [5, 8, 5, 9]),
        ("", "new", [0, 0, 0, 3]),
        ("a", "a\n", [0, 1, 0, 2]),
        // Native extension: retain CRLF, without the oracle's LF normalization.
        ("a\r\nold\r\n", "a\r\nnew\r\n", [3, 8, 3, 8]),
    ] {
        for (destination, expected) in [(Side::A, b), (Side::B, a)] {
            let config = MergeViewConfig {
                revert_to: Some(destination),
                ..MergeViewConfig::default()
            };
            let mut merge = view(a, b, config);
            assert_chunks(&merge, &[chunk]);
            assert!(!merge.revert(1).unwrap());
            assert!(merge.revert(0).unwrap());
            assert_chunks(&merge, &[]);
            assert_eq!(merge.state(Side::A).text().as_str(), expected);
            assert_eq!(merge.state(Side::B).text().as_str(), expected);
        }
    }
    let mut merge = view("old", "new", MergeViewConfig::default());
    assert_eq!(merge.revert_controls().count(), 0);
    assert!(!merge.revert(0).unwrap());
}

#[test]
fn filtering_and_invalid_edits_keep_the_pair_synchronized() {
    let a = EditorState::new("a\nold\ntail");
    let b = EditorState::with_extensions(
        "a\nnew\ntail",
        change_filter(|_, transaction| {
            if transaction.is_user_event("revert") {
                ChangePermission::Reject
            } else {
                ChangePermission::Protect(Vec::from(one(2..5)))
            }
        }),
    )
    .unwrap();
    let config = MergeViewConfig {
        revert_to: Some(Side::B),
        ..MergeViewConfig::default()
    };
    let mut merge = MergeView::new(a, b, config).unwrap();
    let chunks = assert_chunks(&merge, &[[2, 6, 2, 6]]);
    assert!(merge.revert(0).unwrap());
    assert!(Arc::ptr_eq(
        &chunks,
        &assert_chunks(&merge, &[[2, 6, 2, 6]])
    ));
    merge
        .dispatch_batch(Side::A, [edit(0..0, "😀"), edit(0..4, "")])
        .unwrap();
    assert!(Arc::ptr_eq(
        &chunks,
        &assert_chunks(&merge, &[[2, 6, 2, 6]])
    ));
    assert_eq!(
        merge.dispatch_batch(Side::B, [edit(0..0, "😀"), edit(1..4, "invalid")]),
        Err(EditError::InvalidRange)
    );
    assert!(Arc::ptr_eq(
        &chunks,
        &assert_chunks(&merge, &[[2, 6, 2, 6]])
    ));
    assert_eq!(merge.state(Side::B).text().as_str(), "a\nnew\ntail");
    assert_eq!(merge.state(Side::B).selection().main().head(), 0);

    let changes = ChangeSet::new(vec![
        Change::new(0..0, "header\n"),
        Change::new(2..5, "old"),
    ])
    .unwrap();
    merge.dispatch(Side::B, Transaction::new(changes)).unwrap();
    assert_eq!(merge.state(Side::B).text().as_str(), "header\na\nnew\ntail");
    // The pin joins the adjacent changes in this case.
    let chunks = assert_chunks(&merge, &[[0, 6, 0, 13]]);
    // Installing a new root configuration does not lose the merge's chunks.
    merge
        .dispatch(
            Side::B,
            Transaction::default().with_effects([Extension::default().reconfigure()]),
        )
        .unwrap();
    assert!(Arc::ptr_eq(
        &chunks,
        &assert_chunks(&merge, &[[0, 6, 0, 13]])
    ));
}

#[test]
fn changed_collapse_config_rebuilds_the_ranges_without_rebuilding_chunks() {
    let doc = "one\ntwo\nthree\nfour";
    let mut config = MergeViewConfig {
        collapse_unchanged: Some(CollapseConfig {
            margin: 0,
            min_size: 4,
        }),
        ..MergeViewConfig::default()
    };
    let mut merge = view(doc, doc, config);
    assert_hidden(&merge, &one(0..18), &one(0..18));
    let old = assert_chunks(&merge, &[]);
    config.collapse_unchanged = Some(CollapseConfig {
        margin: 0,
        min_size: 5,
    });
    merge.reconfigure(config);
    assert_hidden(&merge, &[], &[]);
    assert!(Arc::ptr_eq(&old, &assert_chunks(&merge, &[])));
}

fn one(range: Range<usize>) -> [Range<usize>; 1] {
    [range]
}
