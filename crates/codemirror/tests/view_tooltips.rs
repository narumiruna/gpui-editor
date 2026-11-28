//! Snapshots from the SHA-512-verified npm packages @codemirror/view 6.44.0
//! and @codemirror/state 6.7.6. An external JavaScript harness exposes the
//! original configuration and plugin definitions. It drives pointer and
//! timer events and supplies measured rectangles to the original layout.
//! The harness converts reported UTF-16 document positions to UTF-8 bytes.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditorSelection, EditorState, Extension, Precedence,
    StateEffect, Transaction,
    view::{
        EditorView, ViewPlugin, ViewUpdate,
        tooltips::{
            HoverOptions, HoverPointer, HoverRequest, HoverSession, HoverSide, HoverTarget,
            HoverTooltip, REPOSITION_TOOLTIPS, SHOW_TOOLTIP, Tooltip, TooltipConfig,
            TooltipGeometry, TooltipId, TooltipLayout, TooltipMeasurement, TooltipParentId,
            TooltipPoint, TooltipPosition, TooltipRect, active_tooltips, close_hover_tooltip,
            close_hover_tooltips, get_tooltip, has_hover_tooltips, hover_tooltip,
            reposition_tooltips, tooltip_config, tooltip_space, tooltips,
        },
    },
};
use serde_json::{Value, json};

fn compare(group: &str, step: &str, actual: Value) {
    let expected = include_str!("fixtures/view_tooltips.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|case| case["group"] == group && case["step"] == step)
        .unwrap_or_else(|| panic!("missing fixture: {group}: {step}"));
    assert_eq!(actual, expected["result"], "{group}: {step}");
}

fn describe(tip: &Tooltip, names: &[(&str, &Tooltip)]) -> Value {
    let name = names
        .iter()
        .find_map(|(name, known)| (known.id() == tip.id()).then_some(*name))
        .unwrap_or("host");
    json!([name, tip.pos, tip.end, tip.above, tip.arrow])
}

fn position_name(position: TooltipPosition) -> &'static str {
    match position {
        TooltipPosition::Fixed => "fixed",
        TooltipPosition::Absolute => "absolute",
    }
}

fn number(value: f64) -> Value {
    if value.fract() == 0.0 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

fn rect(left: f64, top: f64, right: f64, bottom: f64) -> TooltipRect {
    TooltipRect {
        left,
        top,
        right,
        bottom,
    }
}

#[test]
fn configuration_precedence() {
    let state = EditorState::new("");
    let defaults = tooltip_config(&state);
    assert_eq!(defaults, TooltipConfig::default());
    compare(
        "config",
        "default",
        json!([
            position_name(defaults.position.unwrap_or_default()),
            defaults.parent.map(|id| id.0),
            null
        ]),
    );

    let state = EditorState::with_extensions(
        "",
        Extension::group([
            tooltips(TooltipConfig {
                position: Some(TooltipPosition::Absolute),
                parent: Some(TooltipParentId(9)),
                tooltip_space: None,
            }),
            tooltips(TooltipConfig {
                position: Some(TooltipPosition::Fixed),
                parent: None,
                tooltip_space: Some(rect(5.0, 6.0, 95.0, 96.0)),
            })
            .with_precedence(Precedence::High),
            tooltips(TooltipConfig {
                position: None,
                parent: Some(TooltipParentId(10)),
                tooltip_space: Some(TooltipRect::default()),
            }),
        ]),
    )
    .unwrap();
    let config = tooltip_config(&state);
    let bounds = config.tooltip_space.unwrap();
    compare(
        "config",
        "precedence",
        json!([
            position_name(config.position.unwrap()), config.parent.unwrap().0,
            {"left": number(bounds.left), "top": number(bounds.top), "right": number(bounds.right), "bottom": number(bounds.bottom)}
        ]),
    );

    let positioned = TooltipLayout::default().position(&state, geometry(), |_| unreachable!());
    assert_eq!(positioned.space, bounds);
    assert_eq!(positioned.parent, Some(TooltipParentId(9)));
}

struct HoverHarness {
    state: EditorState,
    session: HoverSession,
    epoch: Instant,
    now: Instant,
}

impl HoverHarness {
    fn new(state: EditorState) -> Self {
        let now = Instant::now();
        Self {
            state,
            session: HoverSession::default(),
            epoch: now,
            now,
        }
    }

    fn apply(&mut self, tr: Transaction) {
        self.state.update(tr).unwrap();
        self.session.view_updated(&self.state, self.now);
    }

    fn dispatch(&mut self, effects: Vec<StateEffect>) {
        if !effects.is_empty() {
            self.apply(Transaction::default().with_effects(effects));
        }
    }

    fn change(&mut self, from: usize, to: usize, insert: &str) {
        self.apply(Transaction::new(
            ChangeSet::single(Change::new(from..to, insert)).unwrap(),
        ));
    }

    fn activate(&mut self, source: Option<&HoverTooltip>) -> Vec<HoverRequest> {
        self.session
            .activate_hover(&self.state, target(5, HoverSide::After), source, None)
    }

    fn complete(&mut self, req: HoverRequest, tips: Vec<Tooltip>) -> bool {
        let effect = self.session.complete(&self.state, req, tips);
        let accepted = effect.is_some();
        self.dispatch(effect.into_iter().collect());
        accepted
    }

    fn clock(&mut self, ms: u64) {
        self.now = self.epoch + Duration::from_millis(ms);
    }

    fn move_at(&mut self, ms: u64, pointer: HoverPointer) {
        self.clock(ms);
        let effects = self.session.pointer_move(&self.state, self.now, pointer);
        self.dispatch(effects);
    }

    fn leave(&mut self, into_tooltip: bool) {
        let effects = self.session.pointer_leave(&self.state, into_tooltip);
        self.dispatch(effects);
    }
}

fn target(pos: usize, side: HoverSide) -> HoverTarget {
    HoverTarget { pos, side }
}

#[test]
fn active_sources_and_mapping() {
    let before = Tooltip::new(0);
    let after = Tooltip::new(0);
    let mut first = Tooltip::new(5);
    first.end = Some(7);
    first.above = true;
    first.strict_side = true;
    first.clip = false;
    let point = Tooltip::new(5);
    let point_id = point.id();
    let mut second = Tooltip::new(1);
    second.end = Some(5);
    second.arrow = true;
    let a = hover_tooltip(HoverOptions {
        hide_on: Some(Arc::new(move |tr, tip| {
            tr.transaction().is_user_event("drop") && tip.id() == point_id
        })),
        ..HoverOptions::default()
    });
    let b = hover_tooltip(HoverOptions {
        hide_on_change: true,
        ..HoverOptions::default()
    });
    let state = EditorState::with_extensions(
        "a😀bcd",
        Extension::group([
            SHOW_TOOLTIP.of(Some(before.clone())),
            a.extension(),
            SHOW_TOOLTIP.of(None),
            SHOW_TOOLTIP.of(Some(after.clone())),
            b.extension(),
        ]),
    )
    .unwrap();
    let mut run = HoverHarness::new(state);
    let names = [
        ("before", &before),
        ("after", &after),
        ("first", &first),
        ("point", &point),
        ("second", &second),
    ];
    let snap = |run: &HoverHarness, step| {
        let state = &run.state;
        compare(
            "state",
            step,
            json!({
                "active": active_tooltips(state).iter().map(|active| describe(&active.tooltip, &names)).collect::<Vec<_>>(),
                "sources": ([&a, &b].map(|source| source.active(state).unwrap().tooltips().iter().map(|t| describe(t, &names)).collect::<Vec<_>>())),
                "hasHover": has_hover_tooltips(state)
            }),
        );
    };
    snap(&run, "initial");
    let activate_both = |run: &mut HoverHarness| {
        let requests = run.activate(None);
        assert_eq!(
            requests.iter().map(|r| r.source).collect::<Vec<_>>(),
            [a.id(), b.id()]
        );
        assert!(run.complete(requests[0], vec![first.clone(), point.clone()]));
        assert!(run.complete(requests[1], vec![second.clone()]));
    };
    activate_both(&mut run);
    snap(&run, "locked groups");
    let host = get_tooltip(&run.state, TooltipId::HOVER_HOST).unwrap();
    assert!(!host.tooltip.strict_side && host.tooltip.clip);
    assert_eq!(
        host.sections.unwrap().as_slice(),
        [&first, &point, &second].map(Clone::clone)
    );
    assert!(a.active(&run.state).unwrap().is_locked());
    run.change(5, 5, "X");
    snap(&run, "map at start");
    assert!(a.active(&run.state).unwrap().is_locked());
    run.change(8, 8, "Y");
    snap(&run, "map at end");
    run.apply(Transaction::default().with_user_event("drop"));
    snap(&run, "hide predicate");
    let requests = run.activate(Some(&b));
    assert_eq!(requests.len(), 1);
    assert!(run.complete(requests[0], vec![second.clone()]));
    run.apply(Transaction::default().with_selection(EditorSelection::single(0, 0)));
    snap(&run, "explicit selection");
    run.change(1, 7, "");
    snap(&run, "deleted anchor");
    run.change(0, 0, "a😀bc");
    activate_both(&mut run);
    run.dispatch(vec![close_hover_tooltip(&a)]);
    snap(&run, "targeted close");
    run.dispatch(vec![close_hover_tooltips()]);
    snap(&run, "close all");
}

#[test]
fn one_activation_processes_synchronous_and_async_sources_in_order() {
    let [a, b, c] = std::array::from_fn(|_| hover_tooltip(HoverOptions::default()));
    let sources = [&a, &b, &c];
    let state = EditorState::with_extensions(
        "abcd",
        Extension::group(sources.map(HoverTooltip::extension)),
    )
    .unwrap();
    let mut run = HoverHarness::new(state);
    let req = run
        .session
        .activate_hover(&run.state, target(1, HoverSide::After), None, None);
    let sync = Tooltip::new(1);
    let deferred = Tooltip::new(1);
    assert!(run.session.defer(req[0]));
    assert!(run.complete(req[1], vec![sync.clone()]));
    assert!(run.session.defer(req[2]));
    assert!(!run.complete(req[0], vec![Tooltip::new(1)]));
    assert!(run.complete(req[2], vec![deferred.clone()]));
    let names = [("sync", &sync), ("async", &deferred)];
    let state = &run.state;
    let active = active_tooltips(state)
        .iter()
        .map(|active| describe(&active.tooltip, &names))
        .collect::<Vec<_>>();
    let sources = sources.map(|source| {
        source
            .active(state)
            .unwrap()
            .tooltips()
            .iter()
            .map(|tip| describe(tip, &names))
            .collect::<Vec<_>>()
    });
    compare(
        "state",
        "mixed sources",
        json!({"active":active, "sources":sources, "hasHover":has_hover_tooltips(state)}),
    );
}

#[test]
fn hover_dwell_cancellation_and_lock() {
    let source = hover_tooltip(HoverOptions {
        hover_time: Duration::ZERO,
        ..HoverOptions::default()
    });
    let state = EditorState::with_extensions("abcdef", source.extension()).unwrap();
    let mut run = HoverHarness::new(state);
    let mut range = Tooltip::new(2);
    range.end = Some(4);
    let point = Tooltip::new(2);
    let names = [("range", &range), ("point", &point)];
    let hit = |pos| HoverPointer {
        position: Some(pos),
        range_position: Some(pos),
        over_tooltip: false,
    };
    let before = target(2, HoverSide::Before);
    let widget = target(2, HoverSide::After);
    let mut calls = Vec::new();
    let mut poll = |run: &mut HoverHarness, ms, hit, due| {
        run.clock(ms);
        let requests = run.session.poll(&run.state, run.now, Some(hit));
        assert_eq!(requests.len(), usize::from(due));
        if let Some(request) = requests.first().copied() {
            assert_eq!(request.source, source.id());
            assert!(run.session.defer(request));
            calls.push(request.target);
        }
        (requests.first().copied(), calls.clone())
    };
    let snap = |run: &HoverHarness, step, calls: &[HoverTarget]| {
        let latest = source.active(&run.state).unwrap();
        let active: Vec<_> = latest
            .tooltips()
            .iter()
            .map(|tip| describe(tip, &names))
            .collect();
        let calls: Vec<_> = calls
            .iter()
            .map(|target| {
                json!([
                    target.pos,
                    if target.side == HoverSide::Before {
                        -1
                    } else {
                        1
                    }
                ])
            })
            .collect();
        let next = run
            .session
            .next_deadline()
            .map(|at| at.duration_since(run.epoch).as_millis());
        compare(
            "hover",
            step,
            json!({"calls":calls,"active":active,"next":next}),
        );
    };

    run.move_at(0, hit(2));
    run.move_at(200, hit(2));
    let (_, calls) = poll(&mut run, 300, before, false);
    snap(&run, "dwell reset", &calls);
    let (first, calls) = poll(&mut run, 500, before, true);
    snap(&run, "first request", &calls);
    run.clock(505);
    run.apply(Transaction::default());
    assert!(!run.complete(first.unwrap(), vec![range.clone()]));
    snap(&run, "stale after update", &calls);
    let (_, calls) = poll(&mut run, 524, before, false);
    snap(&run, "before retry", &calls);
    let (retry, calls) = poll(&mut run, 525, before, true);
    assert!(run.complete(retry.unwrap(), vec![range.clone()]));
    snap(&run, "retry result", &calls);
    run.move_at(530, hit(4));
    snap(&run, "inclusive range end", &calls);
    run.move_at(
        540,
        HoverPointer {
            over_tooltip: true,
            ..HoverPointer::default()
        },
    );
    snap(&run, "over tooltip", &calls);
    run.clock(550);
    run.leave(true);
    snap(&run, "entered tooltip", &calls);
    let effects = run.session.tooltip_leave(&run.state, false);
    run.dispatch(effects);
    snap(&run, "left tooltip", &calls);
    run.move_at(560, hit(2));
    let (stale, calls) = poll(&mut run, 860, before, true);
    run.move_at(860, hit(3));
    assert!(!run.complete(stale.unwrap(), vec![point.clone()]));
    snap(&run, "stale after move", &calls);
    run.move_at(870, hit(2));
    poll(&mut run, 880, widget, true);
    poll(&mut run, 1160, widget, false);
    let (pending, mut calls) = poll(&mut run, 1170, widget, true);
    run.leave(false);
    assert!(run.complete(pending.unwrap(), vec![point.clone()]));
    snap(&run, "pending after leave", &calls);
    let locked = run.session.activate_hover(
        &run.state,
        before,
        Some(&source),
        Some(Arc::new(|tr| tr.transaction().is_user_event("unlock"))),
    );
    calls.push(locked[0].target);
    assert!(run.complete(locked[0], vec![point.clone()]));
    run.leave(false);
    snap(&run, "locked after leave", &calls);
    run.apply(Transaction::default().with_user_event("unlock"));
    snap(&run, "until", &calls);
}

#[test]
fn measured_tooltip_space_uses_current_state_and_configuration_precedence() {
    let slot = Compartment::new();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let dynamic = tooltip_space({
        let calls = calls.clone();
        move |state, geometry| {
            let at = state.selection().main().head();
            calls.lock().unwrap().push((at, geometry.viewport.right));
            rect(8. + at as f64, 0., geometry.viewport.right - 10., 180.)
        }
    });
    let fixed = rect(4., 1., 90., 120.);
    let mut state = EditorState::with_extensions(
        "abc",
        Extension::group([
            slot.of(dynamic.clone()),
            tooltips(TooltipConfig {
                tooltip_space: Some(fixed),
                ..Default::default()
            }),
            SHOW_TOOLTIP.of(Some(Tooltip::new(0))),
        ]),
    )
    .unwrap();
    let mut layout = TooltipLayout::default();
    for line in include_str!("fixtures/tooltip_space.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let at = usize::try_from(row["at"].as_u64().unwrap()).unwrap();
        let width = row["width"].as_f64().unwrap();
        let enabled = row["enabled"] == true;
        let ext = if enabled {
            dynamic.clone()
        } else {
            Extension::default()
        };
        state
            .update(
                Transaction::default()
                    .with_selection(EditorSelection::single(at, at))
                    .with_effects([slot.reconfigure(ext), reposition_tooltips()]),
            )
            .unwrap();
        assert_eq!(
            tooltip_config(&state).tooltip_space,
            (!enabled).then_some(fixed)
        );
        assert!(calls.lock().unwrap().is_empty());
        let mut geometry = geometry();
        geometry.viewport.right = width;
        let measured = layout.position(&state, geometry, |_| {
            TooltipMeasurement::new(Some(rect(35., 16., 35., 26.)), 40., 20.)
        });
        let actual: Vec<_> = std::mem::take(&mut *calls.lock().unwrap());
        assert_eq!(
            actual,
            if enabled {
                vec![(at, width)]
            } else {
                Vec::new()
            }
        );
        let value = |edge| row["box"][edge].as_f64().unwrap();
        assert_eq!(
            measured.space,
            rect(value("left"), value("top"), value("right"), value("bottom"))
        );
        assert!(measured.placements[0].1.is_some());
        assert_eq!(state.text().as_str(), "abc");
    }
}

#[test]
fn tooltip_space_measures_only_for_active_tooltips_in_view() {
    let slot = Compartment::new();
    let tip = Tooltip::new(0);
    let space_reads = Arc::new(Mutex::new(0_u64));
    let space = rect(0., 0., 240., 180.);
    let measured_space = tooltip_space({
        let space_reads = space_reads.clone();
        move |_, _| {
            *space_reads.lock().unwrap() += 1;
            space
        }
    });
    let mut state = EditorState::with_extensions(
        "abc",
        Extension::group([measured_space, slot.of(SHOW_TOOLTIP.of(Some(tip.clone())))]),
    )
    .unwrap();
    let mut layout = TooltipLayout::default();
    let mut was_active = true;
    let mut actual = Vec::new();
    let mut expected = Vec::new();
    for line in include_str!("fixtures/tooltip_measure_gate.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let active = row["active"].as_bool().unwrap();
        if active != was_active {
            let input = SHOW_TOOLTIP.of(active.then(|| tip.clone()));
            state
                .update(Transaction::default().with_effects([slot.reconfigure(input)]))
                .unwrap();
            was_active = active;
        }
        let geometry = TooltipGeometry {
            visible: rect(32., 32., 192., 128.),
            in_view: row["inView"].as_bool().unwrap(),
            ..geometry()
        };
        let anchor = if row["clipAnchor"] == true {
            rect(60., 4., 60., 20.)
        } else {
            rect(60., 60., 60., 76.)
        };
        let mut anchor_reads = 0;
        let placed = layout.position(&state, geometry, |_| {
            anchor_reads += 1;
            TooltipMeasurement::new(Some(anchor), 40., 20.)
        });
        assert_eq!(
            placed.placements.len(),
            usize::from(active),
            "{}",
            row["step"]
        );
        let visible = placed.placements.iter().any(|(_, tip)| tip.is_some());
        actual.push(json!([
            row["step"],
            anchor_reads,
            std::mem::take(&mut *space_reads.lock().unwrap()),
            visible
        ]));
        expected.push(json!([
            row["step"],
            row["anchorReads"],
            row["spaceReads"],
            row["visible"]
        ]));
    }
    assert_eq!(actual, expected);
}

fn geometry() -> TooltipGeometry {
    TooltipGeometry {
        viewport: rect(5.0, 0.0, 105.0, 100.0),
        visible: rect(10.0, 10.0, 100.0, 90.0),
        parent_origin: TooltipPoint { x: 10.0, y: 15.0 },
        scale: TooltipPoint { x: 1.0, y: 1.0 },
        fixed_left: 5.0,
        line_height: 8.0,
        right_to_left: false,
        in_view: true,
        force_absolute: false,
    }
}

struct RepositionCounter(Arc<Mutex<u32>>);
impl ViewPlugin for RepositionCounter {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        if update
            .effects
            .iter()
            .any(|effect| effect.get(&REPOSITION_TOOLTIPS).is_some())
            && !active_tooltips(update.state).is_empty()
        {
            *self.0.lock().unwrap() += 1;
        }
    }
}

#[test]
fn placement_and_reposition_request() {
    let mut tips = [1, 2, 3, 4, 5, 6, 7].map(Tooltip::new);
    tips[0].arrow = true;
    tips[1].arrow = true;
    tips[1].strict_side = true;
    tips[2].clip = false;
    let state = EditorState::with_extensions(
        "abcdefgh",
        Extension::group(tips.iter().cloned().map(|tip| SHOW_TOOLTIP.of(Some(tip)))),
    )
    .unwrap();
    let mut view = EditorView::<()>::new(state);
    let count = Arc::new(Mutex::new(1));
    view.add_plugin(RepositionCounter(count.clone()));
    compare(
        "layout",
        "reposition",
        json!([
            *count.lock().unwrap(),
            get_tooltip(view.state(), tips[0].id()).is_some(),
            get_tooltip(view.state(), Tooltip::new(tips[0].pos).id()).is_some()
        ]),
    );
    view.update(
        Transaction::default().with_effects([reposition_tooltips()]),
        (),
        |_| (None, ()),
    )
    .unwrap();
    compare("layout", "requested", json!(*count.lock().unwrap()));

    let mut positions = [
        Some(rect(30.0, 85.0, 31.0, 89.0)),
        Some(rect(90.0, 60.0, 91.0, 70.0)),
        Some(rect(0.0, 0.0, 1.0, 2.0)),
        Some(rect(19.0, 55.0, 20.0, 61.0)),
        Some(rect(1.0, 4.0, 2.0, 8.0)),
        None,
        Some(rect(19.0, 55.0, 20.0, 61.0)),
    ];
    let sizes = [
        (30.0, 20.0),
        (25.0, 50.0),
        (120.0, 10.0),
        (30.0, 20.0),
        (25.0, 15.0),
        (20.0, 5.0),
        (30.0, 20.0),
    ];
    let mut layout = TooltipLayout::default();
    let mut measure = |step, geometry, positions: [_; 7]| {
        let projected = layout.position(view.state(), geometry, |active| {
            let index = active.tooltip.pos - 1;
            let (width, height) = sizes[index];
            let mut item = TooltipMeasurement::new(positions[index], width, height);
            if index == 0 {
                item.offset = TooltipPoint { x: 3.0, y: 2.0 };
            }
            item.resize = index != 2;
            item.overlap = index == 3;
            item
        });
        assert_eq!(
            projected
                .placements
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            tips.clone().map(|tip| tip.id())
        );
        compare(
            "layout",
            step,
            json!({
                "position": position_name(projected.position),
                "results": projected.placements.into_iter().map(|(_, p)| p.map(|p| json!([
                    number(p.origin.x), number(p.origin.y), p.above, p.height_limit.map(number), p.arrow_left.map(number)
                ]))).collect::<Vec<_>>()
            }),
        );
    };
    measure("fixed", geometry(), positions);
    positions[0] = Some(rect(30.0, 50.0, 31.0, 55.0));
    positions[1] = Some(rect(90.0, 86.0, 91.0, 90.0));
    let mut absolute = TooltipGeometry {
        force_absolute: true,
        right_to_left: true,
        scale: TooltipPoint { x: 2.0, y: 2.0 },
        ..geometry()
    };
    measure("absolute", absolute, positions);
    absolute.force_absolute = false;
    measure("sticky absolute", absolute, positions);

    let offscreen = layout.position(
        view.state(),
        TooltipGeometry {
            in_view: false,
            ..geometry()
        },
        |_| unreachable!(),
    );
    assert!(
        offscreen
            .placements
            .iter()
            .all(|(_, placement)| placement.is_none())
    );
}

#[test]
fn removal_rejects_late_results_and_effects_follow_dispatch_order() {
    let compartment = Compartment::new();
    let source = hover_tooltip(HoverOptions::default());
    let state = EditorState::with_extensions("a😀bc", compartment.of(source.extension())).unwrap();
    let mut run = HoverHarness::new(state);
    let stale = run.activate(Some(&source))[0];
    assert!(run.session.defer(stale));
    run.dispatch(vec![compartment.reconfigure(Extension::default())]);
    assert!(source.active(&run.state).is_none());
    assert!(!run.complete(stale, vec![Tooltip::new(5)]));
    run.dispatch(vec![compartment.reconfigure(source.extension())]);
    assert!(!has_hover_tooltips(&run.state));
    let request = run.activate(Some(&source))[0];
    let result = run
        .session
        .complete(&run.state, request, vec![Tooltip::new(5)])
        .unwrap();
    run.dispatch(vec![close_hover_tooltip(&source), result.clone()]);
    assert!(has_hover_tooltips(&run.state));
    run.dispatch(vec![result, close_hover_tooltip(&source)]);
    assert!(!has_hover_tooltips(&run.state));
}
