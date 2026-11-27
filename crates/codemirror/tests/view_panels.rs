//! Parity snapshots from Chrome 142 and SHA-512-verified `@codemirror/view`
//! 6.44.0 and `@codemirror/state` 6.7.6. The scratch oracle records stock
//! panel hooks, placement, lookup, focus, and dialog outcomes. Native tests
//! compare lifecycle and settled results, not DOM or microtask timing.

use std::{cell::RefCell, rc::Rc};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditError, EditorState, Extension, Precedence, StateEffect,
    Transaction,
    view::{
        EditorView,
        panels::{
            DialogConfig, DialogContent, DialogElementId, DialogFocus, DialogId, DialogInput,
            DialogOutcome, DialogRequest, DialogSubmission, Panel, PanelConfig, PanelContainerId,
            PanelEvent, PanelId, PanelLifecycle, PanelPlacement, PanelSlot, SHOW_PANEL,
            active_panels, get_dialog, get_panel, panel_config, panels, show_dialog,
        },
    },
};
use serde_json::{Value, json};

const TOP: PanelPlacement = PanelPlacement::Top;
const BOTTOM: PanelPlacement = PanelPlacement::Bottom;
type Events = Rc<RefCell<Vec<PanelEvent>>>;

fn fixture(case: &str) -> Value {
    include_str!("fixtures/view_panels.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|row| row["case"] == case)
        .unwrap()
}

fn watch(view: &mut EditorView<()>) -> Events {
    let events = Events::default();
    let queued = events.clone();
    view.add_plugin(PanelLifecycle::new(view.state(), move |event| {
        queued.borrow_mut().push(event);
    }));
    events
}

fn dispatch(view: &mut EditorView<()>, transaction: Transaction) {
    view.update(transaction, (), |_| (None, ())).unwrap();
}

fn effects(effects: impl IntoIterator<Item = StateEffect>) -> Transaction {
    Transaction::default().with_effects(effects)
}

fn edit(range: std::ops::Range<usize>, text: &str) -> Transaction {
    Transaction::new(ChangeSet::single(Change::new(range, text)).unwrap())
}

fn shown(panel: Panel) -> Extension {
    SHOW_PANEL.of(Some(panel))
}

fn groups(state: &EditorState, name: impl Fn(&Panel) -> String) -> Value {
    json!([TOP, BOTTOM].map(|side| {
        active_panels(state)
            .iter()
            .filter(|panel| panel.placement() == side)
            .map(&name)
            .collect::<Vec<_>>()
    }))
}

#[test]
fn matches_stock_panel_order_and_lifecycle() {
    let trace = fixture("panel-lifecycle");
    let steps = trace["steps"].as_array().unwrap();
    let ids = [(); 4].map(|()| PanelId::new());
    let [a, b, c, d] = ids;
    let name =
        |panel: &Panel| ["a", "b", "c", "d"][ids.iter().position(|id| *id == panel.id()).unwrap()];
    let custom = |id, placement| Panel::Custom { id, placement };
    let config = Compartment::new();
    let state = EditorState::with_extensions(
        "abc",
        config.of(Extension::group([
            shown(custom(b, BOTTOM)),
            SHOW_PANEL.of(None),
            shown(custom(c, TOP)),
            shown(custom(a, TOP)),
        ])),
    )
    .unwrap();
    let mut view = EditorView::new(state);
    let events = watch(&mut view);
    let original_a = get_panel(view.state(), a).unwrap();
    let record = |step: &Value, state: Option<&EditorState>| {
        let hooks: Vec<_> = events
            .borrow_mut()
            .drain(..)
            .map(|event| {
                let (verb, slot) = match event {
                    PanelEvent::Mounted(slot) => ("mount", slot),
                    PanelEvent::Updated(slot) => ("update", slot),
                    PanelEvent::Destroyed(slot) => ("destroy", slot),
                    _ => panic!("unexpected dialog event"),
                };
                format!("{verb}:{}", name(&slot.panel))
            })
            .collect();
        assert_eq!(json!(hooks), step["events"], "{}", step["name"]);
        if let Some(state) = state {
            assert_eq!(
                groups(state, |panel| name(panel).to_owned()),
                step["groups"]
            );
            let panel = get_panel(state, a);
            assert_eq!(json!(panel.is_some()), step["hasA"]);
            assert_eq!(json!(panel.as_ref() == Some(&original_a)), step["sameA"]);
        }
    };
    record(&steps[0], Some(view.state()));
    dispatch(&mut view, edit(1..1, "q"));
    record(&steps[1], Some(view.state()));
    dispatch(
        &mut view,
        effects([config.reconfigure(Extension::group([
            shown(custom(d, BOTTOM)),
            shown(custom(c, TOP)),
            shown(custom(a, TOP)).with_precedence(Precedence::Highest),
        ]))]),
    );
    record(&steps[2], Some(view.state()));
    assert!(get_panel(view.state(), b).is_none());
    drop(view);
    record(&steps[3], None);
}

fn dialog_name(panel: &Panel) -> &str {
    match panel {
        Panel::Dialog(dialog) => &dialog.config().classes[0],
        _ => panic!("expected a dialog"),
    }
}

fn dialog_outcomes(events: &Events, names: &[(DialogId, &str)]) -> Value {
    let trace: Vec<_> = events
        .borrow_mut()
        .drain(..)
        .filter_map(|event| match event {
            PanelEvent::DialogFinished(id, outcome) => {
                let name = names.iter().find(|(known, _)| *known == id).unwrap().1;
                Some(match outcome {
                    DialogOutcome::Dismissed => json!([name, null]),
                    DialogOutcome::Submitted(form) => json!([name, form.fields]),
                })
            }
            _ => None,
        })
        .collect();
    json!(trace)
}

#[test]
fn matches_stock_settled_dialog_lifecycle() {
    let trace = fixture("dialogs");
    let steps = trace["steps"].as_array().unwrap();
    let mut view = EditorView::new(EditorState::new("abc"));
    let events = watch(&mut view);
    let open = |view: &mut EditorView<()>, name: &str, placement, field| {
        let DialogRequest { id, transaction } = show_dialog(
            view.state(),
            DialogConfig {
                placement,
                classes: vec![name.into(), "shared".into()],
                focus: DialogFocus::First,
                content: DialogContent::Prompt {
                    label: name.into(),
                    input: field,
                    submit_label: String::new(),
                },
            },
        );
        dispatch(view, transaction);
        id
    };
    let record = |name, state: &EditorState, names: &[(DialogId, &str)]| {
        let step = steps.iter().find(|step| step["name"] == name).unwrap();
        assert_eq!(
            groups(state, |panel| dialog_name(panel).to_owned()),
            step["groups"]
        );
        let dialog = get_dialog(state, "shared");
        assert_eq!(
            json!(dialog.as_ref().map(|d| &d.config().classes[0])),
            step["shared"]
        );
        assert_eq!(
            json!(
                get_dialog(state, "cm-dialog") == dialog && get_dialog(state, "cm-panel") == dialog
            ),
            step["builtIn"]
        );
        assert_eq!(dialog_outcomes(&events, names), step["results"]);
        if let Some(dialog) = dialog
            .filter(|dialog| Some(dialog.config().classes[0].as_str()) == step["name"].as_str())
        {
            assert_eq!(dialog.config().focus, DialogFocus::First);
            let DialogContent::Prompt {
                input,
                submit_label,
                ..
            } = &dialog.config().content
            else {
                panic!("expected a prompt")
            };
            assert_eq!(submit_label, "OK");
            assert_eq!(
                json!(input.as_ref().map_or("button", |input| input.name.as_str())),
                step["focus"]
            );
        }
    };
    let one = open(
        &mut view,
        "one",
        TOP,
        Some(DialogInput {
            value: "start".into(),
            ..Default::default()
        }),
    );
    record("one", view.state(), &[]);
    let two = open(
        &mut view,
        "two",
        BOTTOM,
        Some(DialogInput {
            name: "name".into(),
            value: "answer".into(),
            ..Default::default()
        }),
    );
    record("two", view.state(), &[]);
    assert!(get_panel(view.state(), two.panel_id()).is_some());
    let details = DialogSubmission {
        form: None,
        fields: vec![("name".into(), "answer".into())],
    };
    dispatch(&mut view, effects([two.submit(details)]));
    record("submitted", view.state(), &[(two, "two")]);
    dispatch(&mut view, effects([one.close()]));
    record("closed", view.state(), &[(one, "one")]);
    let three = open(&mut view, "three", BOTTOM, None);
    record("three", view.state(), &[]);
    dispatch(&mut view, effects([three.close()]));
    record("escape", view.state(), &[(three, "three")]);
    let four = open(&mut view, "four", BOTTOM, None);
    dispatch(
        &mut view,
        effects([
            four.submit(DialogSubmission::default()),
            Extension::default().reconfigure(),
        ]),
    );
    record("removed", view.state(), &[(four, "four")]);
    let five = open(&mut view, "five", BOTTOM, None);
    drop(view);
    assert_eq!(
        dialog_outcomes(&events, &[(five, "five")]),
        trace["destroyResults"]
    );
}

#[test]
fn first_panel_id_and_container_win_and_relocation_keeps_the_panel() {
    let id = PanelId::new();
    let low = PanelContainerId(1);
    let high = PanelContainerId(2);
    let bottom = PanelContainerId(3);
    let change = Compartment::new();
    let high_config = PanelConfig {
        top_container: Some(high),
        bottom_container: None,
    };
    let low_config = PanelConfig {
        top_container: Some(low),
        bottom_container: Some(bottom),
    };
    let extensions = Extension::group([
        panels(low_config),
        change.of(panels(high_config).with_precedence(Precedence::High)),
        shown(Panel::Custom { id, placement: TOP }),
        shown(Panel::Custom {
            id,
            placement: BOTTOM,
        }),
    ]);
    let mut view = EditorView::new(EditorState::with_extensions("", extensions).unwrap());
    let events = watch(&mut view);
    assert_eq!(
        panel_config(view.state()),
        PanelConfig {
            top_container: Some(high),
            ..low_config
        }
    );
    let mut slot = PanelSlot {
        panel: Panel::Custom { id, placement: TOP },
        container: Some(high),
        index: 0,
    };
    assert_eq!(events.take(), [PanelEvent::Mounted(slot.clone())]);
    dispatch(
        &mut view,
        effects([change.reconfigure(Extension::default())]),
    );
    assert_eq!(panel_config(view.state()), low_config);
    slot.container = Some(low);
    assert_eq!(events.take(), [PanelEvent::Updated(slot.clone())]);
    drop(view);
    assert_eq!(events.take(), [PanelEvent::Destroyed(slot)]);
}

#[test]
fn mapped_submission_is_atomic_and_late_effects_do_not_finish_another_dialog() {
    let mut view = EditorView::new(EditorState::new("a😀b"));
    let events = watch(&mut view);
    let config = DialogConfig {
        classes: vec!["prompt".into()],
        content: DialogContent::Custom(DialogElementId(10)),
        focus: DialogFocus::Element(DialogElementId(11)),
        ..Default::default()
    };
    let first = show_dialog(view.state(), config.clone());
    let second = show_dialog(view.state(), config.clone());
    let both = first
        .transaction
        .with_effects(second.transaction.effects().iter().cloned());
    dispatch(&mut view, both);
    assert_eq!(get_dialog(view.state(), "prompt").unwrap().id(), second.id);
    let list = active_panels(view.state());
    assert_eq!(
        list.iter().map(Panel::id).collect::<Vec<_>>(),
        [second.id.panel_id(), first.id.panel_id()]
    );
    let mut mounts = events.take().into_iter();
    for index in 0..2 {
        let Some(PanelEvent::Mounted(slot)) = mounts.next() else {
            panic!("expected a mount")
        };
        assert_eq!(slot.index, index);
    }
    assert!(mounts.next().is_none());
    let form = DialogSubmission {
        form: Some(DialogElementId(12)),
        fields: vec![("name".into(), "猫".into()), ("name".into(), "犬".into())],
    };
    let submit = second.id.submit(form.clone());
    assert_eq!(
        view.update(edit(2..3, "!").with_effects([submit.clone()]), (), |_| (
            None,
            ()
        )),
        Err(EditError::InvalidRange)
    );
    assert!(events.borrow().is_empty());
    assert_eq!(get_dialog(view.state(), "prompt").unwrap().id(), second.id);
    let transaction = edit(5..5, "!");
    let mapping = transaction
        .changes()
        .desc(view.state().text().len())
        .unwrap();
    dispatch(
        &mut view,
        transaction.with_effects(StateEffect::map_all(&[submit, second.id.close()], &mapping)),
    );
    assert_eq!(view.state().text().as_str(), "a😀!b");
    let updates = events.take();
    assert_eq!(updates.len(), 3);
    assert!(
        matches!(&updates[0], PanelEvent::Updated(slot) if slot.index == 0 && slot.panel.id() == first.id.panel_id())
    );
    assert!(
        matches!(&updates[1], PanelEvent::Destroyed(slot) if slot.panel.id() == second.id.panel_id())
    );
    assert_eq!(
        updates[2],
        PanelEvent::DialogFinished(second.id, DialogOutcome::Submitted(form))
    );
    assert_eq!(
        get_dialog(view.state(), "prompt").unwrap().config(),
        &config
    );
    dispatch(&mut view, effects([second.id.close()]));
    assert!(
        events
            .take()
            .iter()
            .all(|event| matches!(event, PanelEvent::Updated(_)))
    );
    assert_eq!(view.undo(), Some(()));
    assert_eq!(get_dialog(view.state(), "prompt").unwrap().id(), first.id);
    assert_eq!(view.state().text().as_str(), "a😀b");
}
