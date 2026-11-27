use std::{cell::RefCell, ops::Range, rc::Rc};

use gpui_editor_codemirror::{
    Change, ChangePermission, ChangeSet, EditError, EditorSelection, EditorState, Extension,
    RangeSet, Transaction, change_filter, inverted_effects, transaction_filter,
    view::{
        ANNOUNCE, EditorView, MarkDecoration, ViewPlugin, ViewUpdate, Viewport,
        decorations::{Decoration, DecorationSet},
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Recorded {
    start: String,
    text: String,
    changes: Vec<(Range<usize>, Range<usize>)>,
    selection_set: bool,
    viewport_changed: bool,
    focus_changed: bool,
    messages: Vec<String>,
}

#[derive(Default)]
struct Recording {
    updates: Vec<Recorded>,
    destroyed: usize,
}

struct Recorder(Rc<RefCell<Recording>>);

impl ViewPlugin for Recorder {
    fn update(&mut self, event: &ViewUpdate<'_>) {
        self.0.borrow_mut().updates.push(Recorded {
            start: event.start_state.text().as_str().into(),
            text: event.state.text().as_str().into(),
            changes: event.changes.changed_ranges().collect(),
            selection_set: event.selection_set,
            viewport_changed: event.viewport_changed,
            focus_changed: event.focus_changed,
            messages: event.announcements().map(str::to_owned).collect(),
        });
    }

    fn destroy(&mut self) {
        self.0.borrow_mut().destroyed += 1;
    }
}

fn tx(range: Range<usize>, insert: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, insert)).unwrap())
}

#[test]
fn viewport_and_focus_are_independent_from_editor_selection() {
    let recording = Rc::new(RefCell::new(Recording::default()));
    let mut view = EditorView::<()>::new(EditorState::new("a字bcd"));
    let viewport = Viewport::new(view.state().text(), 1..7, [1..4, 5..7]).unwrap();
    assert!(Viewport::new(view.state().text(), 1..7, [2..4, 5..7]).is_err());
    view.add_plugin(Recorder(recording.clone()));
    view.set_viewport(viewport.clone(), true).unwrap();
    view.set_viewport(viewport.clone(), true).unwrap();
    view.set_selection(EditorSelection::single(4, 4)).unwrap();
    view.set_viewport(viewport, false).unwrap();
    assert_eq!(view.revision(), 0);
    let recording = recording.borrow();
    assert_eq!(recording.updates.len(), 3);
    assert!(recording.updates[0].viewport_changed);
    assert!(recording.updates[0].focus_changed);
    assert!(!recording.updates[0].selection_set);
    assert!(recording.updates[1].selection_set);
    assert!(!recording.updates[1].viewport_changed);
    assert!(!recording.updates[1].focus_changed);
    assert!(recording.updates[2].focus_changed);
    assert!(
        recording
            .updates
            .iter()
            .all(|event| event.changes.is_empty())
    );
}

#[test]
fn accepted_edits_map_visible_ranges_and_emit_exact_undo_and_redo_changes() {
    let recording = Rc::new(RefCell::new(Recording::default()));
    let mut view = EditorView::new(EditorState::new("abcdef"));
    let viewport = Viewport::new(view.state().text(), 1..5, [1..2, 3..5]).unwrap();
    view.set_viewport(viewport, false).unwrap();
    view.add_plugin(Recorder(recording.clone()));
    view.update(tx(0..1, "中"), "before", |_| {
        (Some(EditorSelection::single(3, 3)), "after")
    })
    .unwrap();
    assert_eq!(view.viewport().interval(), 3..7);
    assert_eq!(view.viewport().visible_ranges(), [3..4, 5..7]);
    assert_eq!(view.undo(), Some("before"));
    assert_eq!(view.viewport().interval(), 1..5);
    assert_eq!(view.redo(), Some("after"));
    assert_eq!(view.revision(), 3);
    let recording = recording.borrow();
    assert_eq!(
        recording.updates[0],
        Recorded {
            start: "abcdef".into(),
            text: "中bcdef".into(),
            changes: vec![(0..1, 0..3)],
            selection_set: true,
            viewport_changed: true,
            focus_changed: false,
            messages: Vec::new(),
        }
    );
    assert_eq!(recording.updates[1].changes, [(0..3, 0..1)]);
    assert_eq!(recording.updates[2], recording.updates[0]);
}

#[test]
fn plugin_maps_decorations_through_description_on_edit_undo_and_redo() {
    struct MapDecorations {
        set: DecorationSet,
        mark: Decoration,
        seen: Rc<RefCell<Vec<Range<usize>>>>,
    }

    impl ViewPlugin for MapDecorations {
        fn update(&mut self, event: &ViewUpdate<'_>) {
            self.set = self.set.map_desc(event.changes).unwrap();
            let ranges: Vec<_> = self.set.iter(0).collect();
            assert_eq!(ranges.len(), 1);
            let range = ranges[0];
            assert!(range.value.same_instance(&self.mark));
            let bytes = range.from..range.to;
            assert_eq!(&event.state.text().as_str()[bytes.clone()], "bc");
            self.seen.borrow_mut().push(bytes);
        }
    }

    let mut view = EditorView::new(EditorState::new("a字bc"));
    let mark = Decoration::mark(MarkDecoration::new("mapped"));
    let ranges = vec![mark.try_range(4..6).unwrap()];
    let set = DecorationSet::new(view.state().text(), ranges, false).unwrap();
    let too_short = gpui_editor_codemirror::ChangeDesc::new(5, vec![]).unwrap();
    assert!(matches!(
        set.map_desc(&too_short),
        Err(EditError::InvalidRange)
    ));
    let seen = Rc::new(RefCell::new(Vec::new()));
    view.add_plugin(MapDecorations {
        set,
        mark,
        seen: seen.clone(),
    });
    view.update(tx(0..1, "🧭"), (), |_| (None, ())).unwrap();
    assert_eq!(
        view.update(tx(5..6, "!"), (), |_| (None, ())),
        Err(EditError::InvalidRange)
    );
    assert_eq!(view.undo(), Some(()));
    assert_eq!(view.redo(), Some(()));
    assert_eq!(*seen.borrow(), [7..9, 4..6, 7..9]);
}

#[test]
fn selection_history_reports_transition_without_changing_text_or_viewport() {
    let recording = Rc::new(RefCell::new(Recording::default()));
    let mut view = EditorView::new(EditorState::new("a字b"));
    view.add_plugin(Recorder(recording.clone()));
    view.update(tx(4..5, "猫"), "original", |_| (None, "edited"))
        .unwrap();
    let start = view.state().selection().main();
    let selection = EditorSelection::single(4, 1);
    view.update(
        Transaction::default()
            .with_selection(selection.clone())
            .with_user_event("select.pointer"),
        "edited",
        |_| (None, "edited"),
    )
    .unwrap();
    let viewport = view.viewport().clone();
    assert!(view.can_undo_selection());
    assert_eq!(view.undo_selection("current"), Some("current"));
    assert_eq!(view.state().selection().main(), start);
    assert!(view.can_redo_selection());
    assert_eq!(view.redo_selection("current"), Some("current"));
    assert_eq!(view.state().selection(), &selection);
    assert_eq!(view.revision(), 1);
    assert_eq!(view.viewport(), &viewport);
    assert!(
        recording.borrow().updates[1..]
            .iter()
            .all(|update| update.selection_set
                && update.changes.is_empty()
                && !update.viewport_changed)
    );

    view.undo_selection("current").unwrap();
    assert!(!view.can_undo_selection());
    assert_eq!(view.undo_selection("current"), Some("original"));
    assert_eq!(view.state().text().as_str(), "a字b");
    assert_eq!(view.revision(), 2);
    assert_eq!(
        recording.borrow().updates.last().unwrap().changes,
        [(4..7, 4..5)]
    );
}

#[test]
fn view_reports_filtered_changes_and_does_not_notify_on_invalid_edits() {
    let recording = Rc::new(RefCell::new(Recording::default()));
    let config = change_filter(|_, _| ChangePermission::Protect(std::iter::once(1..4).collect()));
    let mut view = EditorView::new(EditorState::with_extensions("a字b", config).unwrap());
    view.add_plugin(Recorder(recording.clone()));
    let second = view.add_plugin(Recorder(recording.clone()));
    assert!(view.remove_plugin(second));
    assert!(!view.remove_plugin(second));
    assert_eq!(recording.borrow().destroyed, 1);
    assert_eq!(
        view.update(tx(2..3, "!"), (), |_| (None, ())),
        Err(EditError::InvalidRange)
    );
    assert_eq!(view.revision(), 0);
    assert!(recording.borrow().updates.is_empty());
    view.update(tx(0..5, "!"), (), |_| (None, ())).unwrap();
    assert_eq!(view.state().text().as_str(), "!字");
    assert_eq!(
        recording.borrow().updates[0].changes,
        [(0..1, 0..1), (4..5, 4..4)]
    );
    drop(view);
    assert_eq!(recording.borrow().destroyed, 2);
}

#[test]
fn plugins_receive_only_accepted_announcements_and_replay_effects() {
    let recording = Rc::new(RefCell::new(Recording::default()));
    let config = Extension::group([
        transaction_filter(|_, proposed| {
            Ok(Transaction::new(proposed.changes().clone()).with_effects([
                ANNOUNCE.of("accepted".to_owned()),
                ANNOUNCE.of("second".to_owned()),
            ]))
        }),
        inverted_effects(|_, accepted, _| {
            if accepted
                .effects()
                .iter()
                .any(|effect| effect.get(&ANNOUNCE).is_some_and(|text| text == "undo"))
            {
                vec![ANNOUNCE.of("redo".to_owned())]
            } else {
                vec![ANNOUNCE.of("undo".to_owned())]
            }
        }),
    ]);
    let mut view = EditorView::new(EditorState::with_extensions("a字b", config).unwrap());
    view.add_plugin(Recorder(recording.clone()));
    let proposed = || tx(0..1, "Q").with_effects([ANNOUNCE.of("discarded".to_owned())]);
    assert_eq!(
        view.update(proposed(), (), |_| (
            Some(EditorSelection::single(2, 2)),
            ()
        )),
        Err(EditError::InvalidSelection)
    );
    assert!(recording.borrow().updates.is_empty());
    assert_eq!(view.state().text().as_str(), "a字b");
    view.update(proposed(), (), |_| (None, ())).unwrap();
    view.undo().unwrap();
    view.redo().unwrap();
    view.set_viewport(Viewport::complete(view.state().text()), true)
        .unwrap();
    let updates = &recording.borrow().updates;
    assert_eq!(updates.len(), 4);
    assert_eq!(updates[0].messages, ["accepted", "second"]);
    assert_eq!(updates[1].messages, ["undo"]);
    assert_eq!(updates[2].messages, ["redo"]);
    assert!(updates[3].messages.is_empty());
}

#[test]
fn normal_and_inclusive_marks_map_insertions_at_both_edges() {
    let changes = ChangeSet::new(vec![Change::new(1..1, "A"), Change::new(3..3, "B")]).unwrap();
    for (start, end, expected) in [
        (false, false, (2, 4)),
        (true, false, (1, 4)),
        (false, true, (2, 5)),
        (true, true, (1, 5)),
    ] {
        let mark = MarkDecoration::new("search-match").with_inclusive_edges(start, end);
        assert_eq!(mark.clone().range(2..2), Err(EditError::InvalidRange));
        let set = RangeSet::of(vec![mark.range(1..3).unwrap()], true)
            .unwrap()
            .map(&changes);
        let marked = set.iter(0).next().unwrap();
        assert_eq!((marked.from, marked.to), expected);
        assert_eq!(marked.value.class(), "search-match");
    }
}
