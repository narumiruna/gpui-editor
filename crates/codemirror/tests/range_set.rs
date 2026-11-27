//! Fixtures checked against @codemirror/state 6.7.6 (651084c),
//! @codemirror/view 6.44.0, and @codemirror/lint 6.9.8 in Node.

#[path = "range_set/oracle.rs"]
mod oracle;

use gpui_editor_codemirror::{
    Change, ChangeDesc, ChangeSet, EditorState, MapMode, Range, RangeComparator, RangeSet,
    RangeSetBuilder, RangeSetError, RangeSetUpdate, RangeValue, SpanIterator, Text, Transaction,
    lint::{self, Diagnostic, Severity, diagnostic_count, for_each_diagnostic, set_diagnostics},
    view::{
        MarkDecoration,
        decorations::{Decoration, DecorationSet, LineDecoration, ReplaceSpec, WidgetSpec},
    },
};

#[derive(Clone, Debug)]
struct Value {
    name: &'static str,
    start: i32,
    end: i32,
    point: bool,
    mode: MapMode,
    identity_only: bool,
}

impl RangeValue for Value {
    fn eq(&self, other: &Self) -> bool {
        if self.identity_only {
            std::ptr::eq(self, other)
        } else {
            self.name == other.name
        }
    }

    fn start_side(&self) -> i32 {
        self.start
    }

    fn end_side(&self) -> i32 {
        self.end
    }

    fn is_point(&self) -> bool {
        self.point
    }

    fn map_mode(&self) -> MapMode {
        self.mode
    }
}

fn mark(from: usize, to: usize, name: &'static str) -> Range<Value> {
    Value {
        name,
        start: 1,
        end: -1,
        point: false,
        mode: MapMode::TrackDel,
        identity_only: false,
    }
    .range(from, to)
}

fn point(from: usize, to: usize, name: &'static str) -> Range<Value> {
    Value {
        name,
        start: 1,
        end: 1,
        point: true,
        mode: MapMode::TrackDel,
        identity_only: false,
    }
    .range(from, to)
}

fn sides(mut range: Range<Value>, start: i32, end: i32) -> Range<Value> {
    range.value.start = start;
    range.value.end = end;
    range
}

fn mode(mut range: Range<Value>, mode: MapMode) -> Range<Value> {
    range.value.mode = mode;
    range
}

fn set(ranges: Vec<Range<Value>>) -> RangeSet<Value> {
    RangeSet::of(ranges, false).unwrap()
}

fn entry(range: &Range<Value>) -> (usize, usize, &'static str) {
    (range.from, range.to, range.value.name)
}

#[test]
fn checks_start_and_side_order_and_accepts_overlap() {
    let late = point(4, 4, "late");
    let early = sides(point(4, 4, "early"), -1, -1);
    assert_eq!(
        RangeSet::of(vec![late.clone(), early.clone()], false).unwrap_err(),
        RangeSetError::UnsortedRanges
    );
    let sorted = RangeSet::of(vec![late.clone(), early.clone(), mark(1, 9, "wide")], true).unwrap();
    assert_eq!(
        sorted.iter(0).map(entry).collect::<Vec<_>>(),
        [(1, 9, "wide"), (4, 4, "early"), (4, 4, "late")]
    );

    let mut build = RangeSetBuilder::new();
    build.add(1, 10, mark(1, 10, "outer").value).unwrap();
    build.push(mark(3, 7, "inner")).unwrap();
    build.push(late).unwrap();
    assert_eq!(build.push(early), Err(RangeSetError::UnsortedRanges));
    assert_eq!(
        build.push(mark(7, 6, "reversed")),
        Err(RangeSetError::ReversedRange)
    );
    assert_eq!(
        build.push(mark(2, 6, "out of order")),
        Err(RangeSetError::UnsortedRanges)
    );
    let set = build.finish();
    assert_eq!((set.len(), set.end()), (3, 10));
}

#[test]
fn filters_only_old_ranges_that_touch_either_boundary() {
    let old = set(vec![
        mark(0, 2, "left"),
        mark(1, 4, "cross"),
        point(2, 2, "edge"),
        mark(3, 7, "stray"),
        mark(6, 7, "right"),
    ]);
    let mut seen = Vec::new();
    let new = old
        .update(RangeSetUpdate {
            add: vec![point(5, 5, "late"), sides(mark(2, 3, "fresh"), -1, -1)],
            sort: true,
            filter: Some(&mut |range| {
                seen.push(range.value.name);
                matches!(range.from, 0 | 3)
            }),
            filter_range: Some(2..=3),
        })
        .unwrap();
    seen.sort_unstable();
    assert_eq!(seen, ["cross", "edge", "left", "stray"]);
    assert_eq!(
        new.iter(0).map(entry).collect::<Vec<_>>(),
        [
            (0, 2, "left"),
            (2, 3, "fresh"),
            (3, 7, "stray"),
            (5, 5, "late"),
            (6, 7, "right")
        ]
    );
    assert_eq!(new.iter(3).next().map(entry), Some((2, 3, "fresh")));
    assert_eq!(old.len(), 5);
    assert!(
        old.update(RangeSetUpdate {
            filter: Some(&mut |_| false),
            ..Default::default()
        })
        .unwrap()
        .is_empty()
    );
}

#[test]
fn queries_include_touching_ranges_and_points_at_zero() {
    let set = set(vec![
        point(0, 0, "zero"),
        mark(0, 2, "left"),
        mark(1, 4, "wide"),
        mark(2, 5, "right"),
        point(5, 5, "end"),
    ]);
    let mut seen = Vec::new();
    set.between(0, 0, |range| {
        seen.push(range.value.name);
        true
    });
    seen.sort_unstable();
    assert_eq!(seen, ["left", "zero"]);
    seen.clear();
    set.between(2, 2, |range| {
        seen.push(range.value.name);
        true
    });
    seen.sort_unstable();
    assert_eq!(seen, ["left", "right", "wide"]);
    seen.clear();
    set.between(2, 5, |range| {
        seen.push(range.value.name);
        false
    });
    assert_eq!(seen.len(), 1);
}

#[test]
fn merges_iterators_with_their_original_set_index() {
    let empty = RangeSet::empty();
    let first = set(vec![mark(0, 2, "left"), mark(2, 7, "first")]);
    let second = set(vec![
        sides(point(2, 2, "point"), -1, -1),
        mark(2, 4, "second"),
    ]);
    let sets = [&first, &empty, &second];
    assert_eq!(
        RangeSet::iter_sets(&sets, 2)
            .map(|(index, range)| (index, range.value.name))
            .collect::<Vec<_>>(),
        [(0, "left"), (2, "point"), (0, "first"), (2, "second")]
    );
    let joined = RangeSet::join(&sets);
    assert_eq!(
        joined.iter(3).map(entry).collect::<Vec<_>>(),
        [(2, 4, "second"), (2, 7, "first")]
    );
    assert_eq!((joined.len(), joined.end()), (4, 7));
}

#[test]
fn maps_edges_and_collapses_or_drops_ranges() {
    let old = RangeSet::of(
        vec![
            sides(mark(2, 6, "inclusive"), -1, 1),
            sides(point(2, 2, "split"), -1, 1),
            sides(point(2, 2, "left"), -1, -1),
            sides(point(2, 2, "default"), 0, 0),
            mark(2, 6, "exclusive"),
            sides(point(2, 2, "crossed"), 1, -1),
            sides(mark(8, 10, "collapse"), -1, 1),
            mark(8, 10, "drop"),
        ],
        true,
    )
    .unwrap();
    let changes = ChangeSet::new(vec![
        Change::new(2..2, "AB"),
        Change::new(6..6, "XYZ"),
        Change::new(8..10, ""),
    ])
    .unwrap();
    assert_eq!(
        old.map(&changes).iter(0).map(entry).collect::<Vec<_>>(),
        [
            (2, 2, "left"),
            (2, 4, "split"),
            (2, 11, "inclusive"),
            (4, 4, "default"),
            (4, 8, "exclusive"),
            (13, 13, "collapse")
        ]
    );
}

#[test]
fn uses_each_empty_ranges_deletion_policy() {
    let old = set(vec![
        point(2, 2, "unchanged"),
        point(3, 3, "del-start"),
        mode(point(3, 3, "after-start"), MapMode::TrackAfter),
        mode(point(3, 3, "before-start"), MapMode::TrackBefore),
        point(5, 5, "interior"),
        mode(point(5, 5, "simple"), MapMode::Simple),
        point(7, 7, "del-end"),
        mode(point(7, 7, "before-end"), MapMode::TrackBefore),
        mode(point(7, 7, "after-end"), MapMode::TrackAfter),
        point(8, 8, "trailing"),
    ]);
    let change = ChangeSet::single(Change::new(3..7, "abc")).unwrap();
    assert_eq!(
        old.map(&change).iter(0).map(entry).collect::<Vec<_>>(),
        [
            (2, 2, "unchanged"),
            (3, 3, "del-start"),
            (3, 3, "before-start"),
            (6, 6, "simple"),
            (6, 6, "del-end"),
            (6, 6, "after-end"),
            (7, 7, "trailing")
        ]
    );
}

#[test]
fn maps_utf8_byte_offsets_and_restores_side_order() {
    // "a猫b" becomes "a🦊b". Points that meet at the new right edge are
    // sorted by start side instead of their old position.
    let old = set(vec![
        sides(point(1, 1, "left"), -1, -1),
        point(1, 1, "after"),
        sides(point(4, 4, "before"), -1, -1),
    ]);
    let changes = ChangeSet::new(vec![Change::new(1..1, "🦊"), Change::new(1..4, "")]).unwrap();
    assert_eq!(
        old.map(&changes).iter(0).map(entry).collect::<Vec<_>>(),
        [(1, 1, "left"), (5, 5, "before"), (5, 5, "after")]
    );
}

#[test]
fn drops_a_strictly_covered_chunk_but_maps_points_on_its_boundaries() {
    let old = set(vec![mode(point(4, 4, "simple"), MapMode::Simple)]);
    let cover = ChangeSet::single(Change::new(0..8, "word")).unwrap();
    assert!(old.map(&cover).is_empty());
    for range in [0..4, 4..8] {
        let touch = ChangeSet::single(Change::new(range, "word")).unwrap();
        assert_eq!(
            old.map(&touch).iter(0).map(entry).collect::<Vec<_>>(),
            [(4, 4, "simple")]
        );
    }
    let adjacent = ChangeSet::new(vec![Change::new(2..4, "X"), Change::new(4..6, "YZ")]).unwrap();
    assert_eq!(
        old.map(&adjacent).iter(0).map(entry).collect::<Vec<_>>(),
        [(3, 3, "simple")]
    );
}

#[test]
fn drops_only_the_covered_chunk_after_the_chunk_size_limit() {
    let mut builder = RangeSetBuilder::new();
    for i in 0..=250 {
        builder
            .push(mode(point(i, i, "simple"), MapMode::Simple))
            .unwrap();
    }
    let changes = ChangeSet::single(Change::new(249..252, "XYZ")).unwrap();
    let mapped = builder.finish().map(&changes);
    assert_eq!(mapped.len(), 250);
    assert!(
        mapped
            .iter(0)
            .enumerate()
            .all(|(i, range)| entry(range) == (i, i, "simple"))
    );
}

#[test]
fn updates_reuse_untouched_chunks_and_joins_keep_chunks_separate() {
    let old = set(vec![
        mode(point(4, 4, "first"), MapMode::Simple),
        mode(point(5, 5, "second"), MapMode::Simple),
    ]);
    let prefix = sides(mark(0, 1, "prefix"), -1, 1);
    let changes = ChangeSet::single(Change::new(2..7, "X")).unwrap();
    let reused = old
        .update(RangeSetUpdate {
            add: vec![prefix.clone()],
            ..Default::default()
        })
        .unwrap();
    let joined = RangeSet::join(&[&set(vec![prefix.clone()]), &old]);
    for set in [reused, joined] {
        assert_eq!(
            set.map(&changes).iter(0).map(entry).collect::<Vec<_>>(),
            [(0, 1, "prefix")]
        );
    }

    let rebuilt = old
        .update(RangeSetUpdate {
            add: vec![prefix],
            filter: Some(&mut |_| true),
            filter_range: Some(5..=5),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        rebuilt.map(&changes).iter(0).map(entry).collect::<Vec<_>>(),
        [(0, 1, "prefix"), (3, 3, "first"), (3, 3, "second")]
    );
}

#[test]
fn second_map_distinguishes_reordered_points_from_points_that_spill() {
    let ranges = [
        mode(point(2, 2, "left"), MapMode::Simple),
        mode(sides(point(4, 4, "right"), -1, -1), MapMode::Simple),
        mode(point(6, 6, "tail"), MapMode::Simple),
    ];
    let collapse = ChangeSet::single(Change::new(2..4, "")).unwrap();
    let cover = ChangeSet::single(Change::new(1..3, "X")).unwrap();
    let spilled = set(ranges.to_vec()).map(&collapse);
    assert_eq!(
        spilled.iter(0).map(entry).collect::<Vec<_>>(),
        [(2, 2, "right"), (2, 2, "left"), (4, 4, "tail")]
    );
    assert_eq!(
        spilled.map(&cover).iter(0).map(entry).collect::<Vec<_>>(),
        [(2, 2, "left"), (3, 3, "tail")]
    );
    let reordered = set([mode(point(0, 0, "start"), MapMode::Simple)]
        .into_iter()
        .chain(ranges)
        .collect())
    .map(&collapse);
    assert_eq!(
        reordered.map(&cover).iter(0).map(entry).collect::<Vec<_>>(),
        [
            (0, 0, "start"),
            (1, 1, "right"),
            (2, 2, "left"),
            (3, 3, "tail")
        ]
    );
}

#[test]
fn maps_decorations_in_layered_chunks() {
    let text = Text::new("a\nbcdefghij");
    let line = Decoration::line(LineDecoration::new("line"));
    let mark = Decoration::mark(MarkDecoration::new("mark").with_inclusive_edges(true, true));
    let block = Decoration::replace(
        None,
        ReplaceSpec {
            block: true,
            ..Default::default()
        },
    );
    let widget = Decoration::widget(
        "widget",
        WidgetSpec {
            side: 1,
            ..Default::default()
        },
    );
    let set = DecorationSet::new(
        &text,
        vec![
            line.try_range(2..2).unwrap(),
            mark.try_range(2..7).unwrap(),
            block.try_range(3..5).unwrap(),
            widget.try_range(7..7).unwrap(),
        ],
        false,
    )
    .unwrap();
    // The line and mark meet the change boundary. The overlapping block
    // belongs to a lower chunk whose entire extent is inside the change.
    let changes = ChangeSet::single(Change::new(2..6, "--")).unwrap();
    let mapped = set.map(&changes);
    let expected = [(2, 2, &line), (2, 5, &mark), (5, 5, &widget)];
    assert_eq!(mapped.as_range_set().len(), expected.len());
    for (range, (from, to, value)) in mapped.iter(0).zip(expected) {
        assert_eq!((range.from, range.to), (from, to));
        assert!(range.value.same_instance(value));
    }
}

#[test]
fn untouched_empty_replacements_keep_their_shape() {
    let value = Decoration::<()>::replace(
        None,
        ReplaceSpec {
            inclusive_start: Some(true),
            ..Default::default()
        },
    );
    let set = DecorationSet::new(
        &Text::new("abcdef"),
        vec![value.try_range(4..4).unwrap()],
        false,
    )
    .unwrap();
    let unrelated = ChangeSet::single(Change::new(1..1, "Y")).unwrap();
    let mapped = set.map(&unrelated);
    let range = mapped.iter(0).next().unwrap();
    assert_eq!((range.from, range.to), (5, 5));
    assert!(range.value.same_instance(&value));
    let touching = ChangeSet::single(Change::new(4..4, "Y")).unwrap();
    assert!(set.map(&touching).as_range_set().is_empty());
}

#[test]
fn lint_markers_keep_boundary_segments_when_a_point_layer_is_covered() {
    let mut state = EditorState::with_extensions("abcdefghijkl", lint::extension()).unwrap();
    state
        .update(set_diagnostics(
            [
                (2..9, "outer"),
                (4..4, "point"),
                (5..6, "nested"),
                (10..11, "tail"),
            ]
            .map(|(range, name)| Diagnostic::new(range, Severity::Warning, name))
            .to_vec(),
        ))
        .unwrap();
    assert_eq!(diagnostic_count(&state), 6);
    let changes = ChangeSet::single(Change::new(3..5, "--")).unwrap();
    state.update(Transaction::new(changes)).unwrap();
    assert_eq!(diagnostic_count(&state), 4);
    let mut diagnostics = Vec::new();
    for_each_diagnostic(&state, |entry| {
        diagnostics.push((
            entry.range.start,
            entry.range.end,
            entry.diagnostic.message.clone(),
        ))
    });
    assert_eq!(
        diagnostics,
        [
            (5, 6, "nested".into()),
            (2, 9, "outer".into()),
            (10, 11, "tail".into())
        ]
    );
    let changes = ChangeSet::single(Change::new(1..12, "--")).unwrap();
    state.update(Transaction::new(changes)).unwrap();
    assert_eq!(diagnostic_count(&state), 0);
}

#[derive(Debug, Eq, PartialEq)]
enum Event {
    Span(usize, usize, Vec<&'static str>, usize),
    Point(usize, usize, &'static str, Vec<&'static str>, usize, usize),
}

#[derive(Default)]
struct Recorder(Vec<Event>);

fn names(active: &[&Value]) -> Vec<&'static str> {
    active.iter().map(|value| value.name).collect()
}

impl SpanIterator<Value> for Recorder {
    fn span(&mut self, from: usize, to: usize, active: &[&Value], open: usize) {
        self.0.push(Event::Span(from, to, names(active), open));
    }

    fn point(
        &mut self,
        from: usize,
        to: usize,
        value: &Value,
        active: &[&Value],
        open: usize,
        index: usize,
    ) {
        self.0.push(Event::Point(
            from,
            to,
            value.name,
            names(active),
            open,
            index,
        ));
    }
}

#[test]
fn walks_spans_with_overlapping_points_and_ranked_active_ranges() {
    use Event::{Point, Span};

    let low = set(vec![
        mark(0, 13, "outer"),
        mark(2, 7, "brief"),
        sides(point(3, 8, "atom"), 0, 1),
        mark(5, 11, "tail"),
        sides(point(10, 10, "left"), -1, -1),
        point(10, 10, "right"),
        point(10, 10, "next"),
    ]);
    let high = set(vec![
        mark(1, 14, "high"),
        point(4, 7, "nested"),
        point(6, 10, "overlap"),
    ]);
    let sets = [&low, &high];
    let mut visitor = Recorder::default();
    assert_eq!(RangeSet::spans(&sets, 2, 12, &mut visitor, None), 2);
    assert_eq!(
        visitor.0,
        [
            Span(2, 3, vec!["brief", "outer", "high"], 2),
            Point(3, 8, "atom", vec!["outer", "high"], 2, 0),
            Point(8, 10, "overlap", vec!["high"], 1, 1),
            Point(10, 10, "right", vec!["tail", "outer", "high"], 1, 0),
            Point(10, 10, "next", vec!["tail", "outer", "high"], 3, 0),
            Span(10, 11, vec!["tail", "outer", "high"], 3),
            Span(11, 12, vec!["outer", "high"], 2),
        ]
    );
    visitor.0.clear();
    assert_eq!(RangeSet::spans(&sets, 5, 6, &mut visitor, None), 3);
    assert_eq!(
        visitor.0,
        [Point(5, 6, "atom", vec!["outer", "high"], 3, 0)]
    );
    visitor.0.clear();
    assert_eq!(RangeSet::spans(&sets, 2, 12, &mut visitor, Some(4)), 0);
    assert_eq!(
        visitor.0,
        [
            Span(2, 3, vec![], 0),
            Point(3, 8, "atom", vec![], 0, 0),
            Point(8, 10, "overlap", vec![], 0, 1),
            Span(10, 12, vec![], 0)
        ]
    );
    visitor.0.clear();
    assert_eq!(RangeSet::spans(&sets, 0, 3, &mut visitor, None), 3);
    assert_eq!(
        visitor.0.last(),
        Some(&Point(3, 3, "atom", vec!["outer", "high"], 2, 0))
    );
}

#[derive(Debug, Eq, PartialEq)]
enum Difference {
    Range(usize, usize, Vec<&'static str>, Vec<&'static str>),
    Point(usize, usize, Option<&'static str>, Option<&'static str>),
    Bound(usize),
}

#[derive(Default)]
struct Comparator(Vec<Difference>);

impl RangeComparator<Value> for Comparator {
    fn compare_range(&mut self, from: usize, to: usize, old: &[&Value], new: &[&Value]) {
        self.0
            .push(Difference::Range(from, to, names(old), names(new)));
    }

    fn compare_point(&mut self, from: usize, to: usize, old: Option<&Value>, new: Option<&Value>) {
        self.0.push(Difference::Point(
            from,
            to,
            old.map(|value| value.name),
            new.map(|value| value.name),
        ));
    }

    fn bound_change(&mut self, pos: usize) {
        self.0.push(Difference::Bound(pos));
    }
}

fn compare(
    old: &[&RangeSet<Value>],
    new: &[&RangeSet<Value>],
    changes: &ChangeDesc,
    min_point_size: Option<usize>,
) -> Vec<Difference> {
    let mut comparator = Comparator::default();
    RangeSet::compare(old, new, changes, &mut comparator, min_point_size);
    comparator.0
}

#[test]
fn compares_unchanged_text_in_new_document_coordinates() {
    use Difference::{Bound, Range};

    let old = set(vec![mark(0, 20, "a")]);
    let new = set(vec![mark(25, 28, "b")]);
    let changes = ChangeDesc::new(20, vec![(5..15, 20)]).unwrap();
    assert_eq!(
        compare(&[&old], &[&new], &changes, None),
        [
            Range(0, 5, vec!["a"], vec![]),
            Range(25, 28, vec!["a"], vec!["b"]),
            Bound(28),
            Range(28, 30, vec!["a"], vec![]),
        ]
    );

    // "a猫b" becomes "a🦊b". The latter gap starts at byte 5, not UTF-16 offset 3.
    let old = set(vec![mark(0, 5, "a")]);
    let new = set(vec![mark(5, 6, "b")]);
    let changes = ChangeSet::single(Change::new(1..4, "🦊"))
        .unwrap()
        .desc("a猫b".len())
        .unwrap();
    assert_eq!(
        compare(&[&old], &[&new], &changes, None),
        [
            Range(0, 1, vec!["a"], vec![]),
            Range(5, 6, vec!["a"], vec!["b"])
        ],
    );
}

#[test]
fn compares_atomic_points_when_their_cover_changes() {
    use Difference::Point;

    let old = set(vec![mark(0, 4, "cover"), sides(point(1, 3, "atom"), -1, 1)]);
    let changes = ChangeSet::single(Change::new(2..4, "")).unwrap();
    let new = old.map(&changes);
    assert_eq!(
        compare(&[&old], &[&new], &changes.desc(4).unwrap(), None),
        [
            Point(1, 2, Some("atom"), Some("atom")),
            Point(2, 2, Some("atom"), None)
        ],
    );
}

#[test]
fn reports_changed_boundaries_even_when_the_active_values_match() {
    use Difference::{Bound, Point};

    let split = set(vec![mark(0, 3, "a"), mark(3, 6, "a")]);
    let split_with_point = set(vec![mark(0, 3, "a"), point(3, 3, "at"), mark(3, 6, "a")]);
    let wide = set(vec![mark(0, 6, "a")]);
    let changes = ChangeDesc::new(6, vec![]).unwrap();
    assert_eq!(compare(&[&split], &[&wide], &changes, None), [Bound(3)]);
    assert_eq!(
        compare(&[&split_with_point], &[&wide], &changes, None),
        [Point(3, 3, Some("at"), None)],
    );

    // Upstream checks both open counts at the new position, also after a shift.
    let old = set(vec![mark(0, 2, "a"), mark(0, 4, "b")]);
    let new = set(vec![mark(5, 7, "a"), mark(5, 9, "b")]);
    let changes = ChangeDesc::new(4, vec![(0..0, 5)]).unwrap();
    assert_eq!(compare(&[&old], &[&new], &changes, None), [Bound(7)]);
}

#[test]
fn compares_only_points_that_meet_the_requested_size() {
    let new = set(vec![
        point(1, 1, "zero"),
        point(3, 5, "short"),
        point(7, 10, "long"),
        mark(10, 12, "mark"),
    ]);
    let changes = ChangeDesc::new(12, vec![]).unwrap();
    assert_eq!(
        compare(&[], &[&new], &changes, Some(3)),
        [Difference::Point(7, 10, None, Some("long"))],
    );
    assert!(compare(&[], &[&new], &changes, Some(4)).is_empty());
}

#[test]
fn compares_points_in_an_unchanged_empty_document() {
    let new = set(vec![point(0, 0, "zero")]);
    let unchanged = ChangeDesc::new(0, vec![]).unwrap();
    assert_eq!(
        compare(&[], &[&new], &unchanged, None),
        [Difference::Point(0, 0, None, Some("zero"))],
    );
    let insertion = ChangeDesc::new(0, vec![(0..0, 2)]).unwrap();
    assert!(compare(&[], &[&new], &insertion, None).is_empty());
}

#[test]
fn equality_compares_point_values_after_excluding_empty_and_shared_sets() {
    let marks_a = set(vec![mark(0, 8, "a")]);
    let marks_b = set(vec![mark(3, 6, "b")]);
    let a = set(vec![point(2, 4, "p")]);
    let b = set(vec![sides(point(2, 4, "p"), -1, -1)]);
    let other = set(vec![point(2, 4, "other")]);
    let empty = RangeSet::empty();
    assert!(RangeSet::eq(&[&marks_a], &[&marks_b], 0, None));
    assert!(!RangeSet::eq(&[&marks_a], &[], 0, None));
    assert!(RangeSet::eq(
        &[&marks_a, &a, &empty],
        &[&b, &marks_a],
        0,
        None
    ));
    assert!(!RangeSet::eq(&[&a], &[&other], 0, None));
    assert!(!RangeSet::eq(&[&a], &[&b, &a], 0, None));

    let left = set(vec![point(0, 4, "outer"), point(1, 2, "inner-a")]);
    let right = set(vec![point(0, 4, "outer"), point(1, 2, "inner-b")]);
    assert!(RangeSet::eq(&[&left], &[&right], 0, None));
}

#[test]
fn equality_checks_the_next_boundary_and_only_requires_old_points_to_match() {
    let tail = set(vec![point(4, 4, "tail")]);
    let shifted = set(vec![point(5, 5, "tail")]);
    let wide = set(vec![point(0, 4, "wide")]);
    let other = set(vec![point(4, 4, "other")]);
    assert!(RangeSet::eq(&[&tail], &[&wide], 0, Some(3)));
    assert!(!RangeSet::eq(&[&wide], &[&tail], 0, Some(3)));
    assert!(!RangeSet::eq(&[&tail], &[&wide], 0, Some(4)));
    assert!(!RangeSet::eq(&[&tail], &[&shifted], 0, Some(1)));
    assert!(RangeSet::eq(&[&tail], &[&other], 0, Some(3)));
    assert!(!RangeSet::eq(&[&tail], &[&other], 0, Some(4)));
    assert!(RangeSet::eq(&[&tail], &[&other], 5, None));
}

#[test]
fn distinguishes_the_maximum_native_offset_from_an_open_end() {
    let point = set(vec![point(usize::MAX, usize::MAX, "max")]);
    let changes = ChangeDesc::new(usize::MAX, vec![]).unwrap();
    assert_eq!(
        compare(&[], &[&point], &changes, None),
        [Difference::Point(usize::MAX, usize::MAX, None, Some("max"))],
    );
    assert!(!RangeSet::eq(
        &[&point],
        &[&set(vec![mark(0, 1, "a")])],
        0,
        None
    ));
}
