use gpui_editor_codemirror::{
    Assoc, Change, ChangeDesc, ChangeSet, EditError, MapMode, RangeTouch, Text,
};

fn edit(source: &str, changes: &ChangeSet) -> String {
    let mut text = Text::new(source);
    changes.apply_to(&mut text).unwrap();
    text.as_str().to_owned()
}

#[test]
fn reports_geometry_and_deleted_context() {
    let set = ChangeSet::new(vec![
        Change::new(1..3, ""),
        Change::new(3..5, ""),
        Change::new(6..6, "XY"),
    ])
    .unwrap();
    let desc = set.desc(7).unwrap();
    assert_eq!(desc.length(), 7);
    assert_eq!(desc.new_length(), 5);
    assert_eq!(
        desc.gaps().collect::<Vec<_>>(),
        [(0..1, 0..1), (5..6, 1..2), (6..7, 4..5)]
    );
    assert_eq!(
        desc.changed_ranges().collect::<Vec<_>>(),
        [(1..5, 1..1), (6..6, 2..4)]
    );
    assert_eq!(desc.touches_range(2..4), RangeTouch::Cover);
    assert_eq!(desc.touches_range(5..6), RangeTouch::Touch);
    assert_eq!(desc.touches_range(7..7), RangeTouch::None);

    for (pos, mode, expected) in [
        (1, MapMode::TrackBefore, Some(1)),
        (1, MapMode::TrackAfter, None),
        (3, MapMode::TrackDel, None),
        (5, MapMode::TrackBefore, None),
        (5, MapMode::TrackAfter, Some(1)),
        (6, MapMode::TrackDel, Some(4)),
    ] {
        assert_eq!(desc.map_pos(pos, Assoc::After, mode), expected);
        assert_eq!(set.map_pos_mode(pos, Assoc::After, mode), expected);
    }
    assert_eq!(desc.map_pos(8, Assoc::Before, MapMode::Simple), None);
    assert_eq!(
        desc.inverted().changed_ranges().collect::<Vec<_>>(),
        [(1..1, 1..5), (2..4, 6..6)]
    );
}

#[test]
fn composes_changes_that_partly_replace_inserted_unicode() {
    let source = "a中z";
    let first = ChangeSet::single(Change::new(1..4, "新🦊字")).unwrap();
    let second = ChangeSet::new(vec![Change::new(4..8, "語"), Change::new(11..12, "!")]).unwrap();
    let composed = first.compose(&second, source.len()).unwrap();
    assert_eq!(edit(source, &composed), "a新語字!");
    assert_eq!(edit(&edit(source, &first), &second), "a新語字!");
    assert_eq!(
        composed.desc(source.len()).unwrap(),
        first
            .desc(source.len())
            .unwrap()
            .compose(&second.desc(12).unwrap())
            .unwrap()
    );
    let inverse = composed.inverted(&Text::new(source)).unwrap();
    assert_eq!(edit("a新語字!", &inverse), source);
    let invalid = ChangeSet::single(Change::new(6..7, "x")).unwrap();
    assert_eq!(
        first.compose(&invalid, source.len()),
        Err(EditError::InvalidRange)
    );
}

#[test]
fn concurrent_replacements_and_insertions_converge() {
    let source = "abcdef";
    let a = ChangeSet::new(vec![Change::new(1..1, "AA"), Change::new(3..5, "K")]).unwrap();
    let b = ChangeSet::new(vec![
        Change::new(1..1, "BB"),
        Change::new(2..4, ""),
        Change::new(5..5, "V"),
    ])
    .unwrap();
    let a_desc = a.desc(source.len()).unwrap();
    let b_desc = b.desc(source.len()).unwrap();
    let mapped_b = b.map(&a_desc, false).unwrap();
    let mapped_a = a.map(&b_desc, true).unwrap();
    let final_a = edit(&edit(source, &a), &mapped_b);
    let final_b = edit(&edit(source, &b), &mapped_a);
    assert_eq!(final_a, "aAABBbKVf");
    assert_eq!(final_a, final_b);
    assert_eq!(
        mapped_b.desc(a_desc.new_length()).unwrap(),
        b_desc.map(&a_desc, false).unwrap()
    );
    assert_eq!(
        mapped_a.desc(b_desc.new_length()).unwrap(),
        a_desc.map(&b_desc, true).unwrap()
    );
}

#[test]
fn composition_and_rebasing_hold_across_short_edits() {
    fn changes(length: usize) -> Vec<ChangeSet> {
        let mut result = vec![ChangeSet::default()];
        for start in 0..=length {
            for end in start..=length {
                for insert in ["", "X", "XY"] {
                    result.push(ChangeSet::single(Change::new(start..end, insert)).unwrap());
                }
            }
        }
        result
    }
    let source = "abc";
    let first = changes(source.len());
    for a in &first {
        let a_desc = a.desc(source.len()).unwrap();
        let result_a = edit(source, a);
        for after in changes(result_a.len()) {
            let composed = a.compose(&after, source.len()).unwrap();
            assert_eq!(
                edit(source, &composed),
                edit(&result_a, &after),
                "{a:?} then {after:?}"
            );
        }
        for b in &first {
            let b_desc = b.desc(source.len()).unwrap();
            let mapped_b = b.map(&a_desc, false).unwrap();
            let mapped_a = a.map(&b_desc, true).unwrap();
            assert_eq!(
                edit(&result_a, &mapped_b),
                edit(&edit(source, b), &mapped_a),
                "{a:?} and {b:?}"
            );
        }
    }
}

#[test]
fn rejects_invalid_descriptions() {
    assert_eq!(
        ChangeDesc::new(2, vec![(0..3, 1)]),
        Err(EditError::InvalidRange)
    );
    assert_eq!(
        ChangeDesc::new(4, vec![(0..3, 1), (1..2, 0)]),
        Err(EditError::OverlappingChanges)
    );
    assert_eq!(
        ChangeDesc::new(usize::MAX, vec![(0..0, 1)]),
        Err(EditError::InvalidRange)
    );
    let a = ChangeDesc::new(3, vec![(1..1, 1)]).unwrap();
    let b = ChangeDesc::new(2, Vec::new()).unwrap();
    assert_eq!(a.compose(&b), Err(EditError::MismatchedLength));
    assert_eq!(a.map(&b, true), Err(EditError::MismatchedLength));
}
