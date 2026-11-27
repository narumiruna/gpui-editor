use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Precedence,
    SelectionRange, Transaction,
    view::{
        CURSOR_SCROLL_MARGIN, CursorScrollMargin, EditorView, EndScrollPadding, SCROLL_INTO_VIEW,
        SCROLL_SNAPSHOT, ScrollBlock, ScrollOffset, ScrollRequest, ScrollSnapshot,
        ScrollSnapshotGeometry, ScrollStrategy, ScrollTarget, scroll_past_end,
    },
};
use serde_json::Value;

fn number(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn strategy(value: &Value) -> ScrollStrategy {
    match value.as_str().unwrap() {
        "nearest" => ScrollStrategy::Nearest,
        "start" => ScrollStrategy::Start,
        "center" => ScrollStrategy::Center,
        "end" => ScrollStrategy::End,
        _ => panic!("unknown scroll strategy {value}"),
    }
}

fn target(value: &Value) -> ScrollTarget {
    ScrollTarget {
        range: SelectionRange::new(number(&value["range"][0]), number(&value["range"][1])),
        x: strategy(&value["x"]),
        y: strategy(&value["y"]),
        x_margin: value["xMargin"].as_f64().unwrap(),
        y_margin: value["yMargin"].as_f64().unwrap(),
    }
}

#[test]
fn scroll_past_end_tracks_host_measurements_and_reconfiguration() {
    let compartment = Compartment::new();
    let mut state =
        EditorState::with_extensions("\nlast", compartment.of(scroll_past_end())).unwrap();
    let mut padding = EndScrollPadding::default();
    assert_eq!(padding.update(&state, 0.0, 16.0, 4.0), Some(1000.0));
    assert_eq!(padding.update(&state, 300.0, 16.0, 4.0), Some(279.5));
    assert_eq!(padding.update(&state, 20.0, 16.0, 4.0), Some(279.5));
    assert_eq!(padding.update(&state, 20.5, 16.0, 4.0), Some(0.0));
    state
        .update(
            Transaction::default().with_effects([compartment.reconfigure(Extension::default())]),
        )
        .unwrap();
    assert_eq!(padding.update(&state, 300.0, 16.0, 4.0), None);
    state
        .update(Transaction::default().with_effects([compartment.reconfigure(scroll_past_end())]))
        .unwrap();
    assert_eq!(padding.update(&state, 0.0, 16.0, 4.0), Some(1000.0));
}

#[test]
fn maps_pinned_scroll_effect_ranges_and_options() {
    for row in include_str!("fixtures/view_scroll.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let changes = row["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| {
                let from = number(&value["from"]);
                let to = value["to"]
                    .as_u64()
                    .map_or(from, |to| to.try_into().unwrap());
                Change::new(from..to, value["insert"].as_str().unwrap_or(""))
            })
            .collect();
        let changes = ChangeSet::new(changes)
            .unwrap()
            .desc(number(&row["length"]))
            .unwrap();
        let effect = SCROLL_INTO_VIEW.of(target(&row["input"]));
        assert_eq!(
            effect.map(&changes).unwrap().get(&SCROLL_INTO_VIEW),
            Some(&target(&row["mapped"]))
        );
    }
}

#[test]
fn maps_unconsumed_scrolls_and_uses_the_last_explicit_target() {
    let mut view = EditorView::<()>::new(EditorState::new("a😀bc"));
    let center = ScrollTarget {
        y: ScrollStrategy::Center,
        ..ScrollTarget::at(5)
    };
    let request = Transaction::default()
        .with_scroll_into_view()
        .with_effects([
            SCROLL_INTO_VIEW.of(ScrollTarget::at(0)),
            SCROLL_INTO_VIEW.of(center),
        ]);
    view.update(request, (), |_| (None, ())).unwrap();
    let changes = ChangeSet::new(vec![Change::new(0..0, "字")]).unwrap();
    view.update(Transaction::new(changes), (), |_| (None, ()))
        .unwrap();
    assert_eq!(
        view.take_scroll_target(),
        Some(ScrollTarget {
            range: SelectionRange::new(8, 8),
            ..center
        })
    );
    assert_eq!(view.take_scroll_target(), None);

    let selection = Transaction::default()
        .with_scroll_into_view()
        .with_selection(EditorSelection::single(0, 4));
    view.update(selection, (), |_| (None, ())).unwrap();
    assert_eq!(view.take_scroll_target(), Some(ScrollTarget::at(4)));
}

#[test]
fn automatic_scroll_uses_the_start_states_cursor_margin() {
    // Values from the pinned 6.44.0 view dispatch in headless Chromium.
    let source = "😀abc\nlast";
    let select = Transaction::default()
        .with_selection(EditorSelection::single(4, 4))
        .with_scroll_into_view();
    let query = |view: &mut EditorView<()>, transaction| {
        view.update(transaction, (), |_| (None, ())).unwrap();
        let target = view.take_scroll_target().unwrap();
        [target.x_margin, target.y_margin]
    };
    let uniform = |margin| CURSOR_SCROLL_MARGIN.of(CursorScrollMargin::uniform(margin));
    let distinct = |x, y| CURSOR_SCROLL_MARGIN.of(CursorScrollMargin { x, y });
    let mut view = EditorView::<()>::new(EditorState::new(source));
    assert_eq!(query(&mut view, select.clone()), [5.0, 5.0]);
    view.set_state(
        EditorState::with_extensions(
            source,
            Extension::group([uniform(7.0), distinct(9.0, 14.0)]),
        )
        .unwrap(),
    );
    assert_eq!(query(&mut view, select.clone()), [9.0, 14.0]);
    view.set_state(
        EditorState::with_extensions(
            source,
            Extension::group([
                uniform(21.0).with_precedence(Precedence::Low),
                uniform(36.0),
                uniform(44.0).with_precedence(Precedence::High),
            ]),
        )
        .unwrap(),
    );
    assert_eq!(query(&mut view, select.clone()), [21.0, 21.0]);

    let compartment = Compartment::new();
    view.set_state(
        EditorState::with_extensions(source, compartment.of(distinct(3.0, 8.0))).unwrap(),
    );
    let reconfigure = select
        .clone()
        .with_effects([compartment.reconfigure(distinct(11.0, 13.0))]);
    assert_eq!(query(&mut view, reconfigure), [3.0, 8.0]);
    assert_eq!(query(&mut view, select.clone()), [11.0, 13.0]);
    let explicit = select.with_effects([SCROLL_INTO_VIEW.of(ScrollTarget {
        x_margin: 2.0,
        y_margin: 6.0,
        ..ScrollTarget::at(0)
    })]);
    assert_eq!(query(&mut view, explicit), [2.0, 6.0]);

    let edit = ChangeSet::single(Change::new(4..5, "B")).unwrap();
    view.update(Transaction::new(edit), (), |_| (None, ()))
        .unwrap();
    view.undo().unwrap();
    let undo = view.take_scroll_target().unwrap();
    assert_eq!([undo.x_margin, undo.y_margin], [11.0, 13.0]);
}

#[test]
fn restores_a_mapped_snapshot_unless_a_later_scroll_request_overrides_it() {
    let state = EditorState::new("a😀\r\nlast");
    let first = ScrollBlock { from: 0, top: 0.0 };
    let snapshot = ScrollSnapshot::capture(
        state.text(),
        ScrollSnapshotGeometry {
            offset: ScrollOffset { x: 12.0, y: 21.0 },
            padding_top: 0.0,
            viewport_from: 0,
            first_viewport_block: first,
        },
        |_| Some(ScrollBlock { from: 7, top: 20.0 }),
    )
    .unwrap();
    let mut view = EditorView::<()>::new_with_scroll(state, ScrollRequest::Restore(snapshot));
    // An ordinary range consumer must not consume a snapshot.
    assert_eq!(view.take_scroll_target(), None);
    let changes = ChangeSet::single(Change::new(0..0, "字\n")).unwrap();
    view.update(Transaction::new(changes), (), |_| (None, ()))
        .unwrap();
    let mapped = view.take_scroll_snapshot().unwrap();
    assert_eq!(mapped.anchor(), 11);
    assert_eq!(
        mapped.plan_restore(view.state().text(), |from| {
            assert_eq!(from, 11);
            Some(40.0)
        }),
        Some(ScrollOffset { x: 12.0, y: 41.0 })
    );
    assert_eq!(view.take_scroll_request(), None);

    let target = ScrollTarget::at(0);
    let reveal_last = Transaction::default()
        .with_effects([SCROLL_SNAPSHOT.of(mapped), SCROLL_INTO_VIEW.of(target)]);
    view.update(reveal_last, (), |_| (None, ())).unwrap();
    assert_eq!(view.take_scroll_snapshot(), None);
    assert_eq!(view.take_scroll_target(), Some(target));
    let restore_last = Transaction::default()
        .with_scroll_into_view()
        .with_effects([SCROLL_INTO_VIEW.of(target), SCROLL_SNAPSHOT.of(mapped)]);
    view.update(restore_last.clone(), (), |_| (None, ()))
        .unwrap();
    assert_eq!(
        view.take_scroll_request(),
        Some(ScrollRequest::Restore(mapped))
    );
    view.update(restore_last, (), |_| (None, ())).unwrap();
    view.set_state(EditorState::new("new"));
    assert_eq!(view.take_scroll_request(), None);
}

#[test]
fn discards_out_of_boundary_requests_and_does_not_queue_rejected_updates() {
    let mut view = EditorView::<()>::new(EditorState::new("😀"));
    let invalid = Transaction::default().with_effects([SCROLL_INTO_VIEW.of(ScrollTarget::at(1))]);
    view.update(invalid, (), |_| (None, ())).unwrap();
    assert_eq!(view.take_scroll_target(), None);
    let truncated =
        Transaction::default().with_effects([SCROLL_INTO_VIEW.of(ScrollTarget::at(10))]);
    view.update(truncated, (), |_| (None, ())).unwrap();
    assert_eq!(view.take_scroll_target(), Some(ScrollTarget::at(4)));
    let changes = ChangeSet::new(vec![Change::new(1..3, "")]).unwrap();
    let invalid =
        Transaction::new(changes).with_effects([SCROLL_INTO_VIEW.of(ScrollTarget::at(0))]);
    assert!(view.update(invalid, (), |_| (None, ())).is_err());
    assert_eq!(view.take_scroll_target(), None);
}
