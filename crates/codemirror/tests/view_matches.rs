//! Oracle: @codemirror/view 6.44.0 and @codemirror/state 6.7.6 tarballs
//! verified against docs/codemirror-upstream.json. The fixture uses the real
//! MatchDecorator with supplied geometry and the real special character and
//! placeholder plugins in headless Chrome 142.0.7444.175. Offsets are converted
//! from UTF-16 to UTF-8, preserving the source's CRLF byte lengths.

use std::{cell::RefCell, ops::Range as ByteRange, rc::Rc};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditError, EditorSelection, EditorState, Extension, Precedence,
    RangeValue, Text, Transaction,
    view::{
        EditorView, MarkDecoration, MatchDecorator, ViewPlugin, ViewUpdate, Viewport,
        decorations::{DecorationKind, DecorationSet},
        highlight_special_chars, placeholder, placeholder_extension, placeholder_projection,
    },
};
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};

fn position(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn offsets(value: &Value) -> ByteRange<usize> {
    position(&value[0])..position(&value[1])
}

fn viewport(text: &Text, spec: &Value) -> Viewport {
    Viewport::new(
        text,
        offsets(&spec["interval"]),
        spec["visible"].as_array().unwrap().iter().map(offsets),
    )
    .unwrap()
}

fn marks(set: &DecorationSet) -> Value {
    Value::Array(
        set.iter(0)
            .map(|range| {
                let DecorationKind::Mark(mark) = range.value.kind() else {
                    panic!("expected an inline mark");
                };
                json!([range.from, range.to, mark.class()])
            })
            .collect(),
    )
}

fn check_matches(case: &Value) {
    let pattern = RegexBuilder::new(case["pattern"].as_str().unwrap())
        .case_insensitive(case["flags"] == "i")
        .build()
        .unwrap();
    let mut matcher = MatchDecorator::new(pattern, MarkDecoration::new("test-match"));
    if let Some(boundary) = case["boundary"].as_str() {
        matcher = matcher.with_boundary(Regex::new(boundary).unwrap());
    }
    if !case["max_length"].is_null() {
        matcher = matcher.with_max_length(position(&case["max_length"]));
    }
    let start_state = EditorState::new(case["text"].as_str().unwrap());
    let start_viewport = viewport(start_state.text(), case);
    let previous = matcher
        .create_deco(start_state.text(), &start_viewport)
        .unwrap();
    let name = &case["name"];
    assert_eq!(marks(&previous), case["initial"], "{name}: creation");
    let next = &case["next"];
    if let Some(edits) = next["changes"].as_array() {
        let changes = ChangeSet::new(
            edits
                .iter()
                .map(|edit| Change::new(offsets(edit), edit[2].as_str().unwrap()))
                .collect(),
        )
        .unwrap();
        let desc = changes.desc(start_state.text().len()).unwrap();
        let mut state = start_state.clone();
        state.update(Transaction::new(changes)).unwrap();
        let viewport = viewport(state.text(), next);
        let update = ViewUpdate {
            start_state: &start_state,
            state: &state,
            changes: &desc,
            effects: &[],
            start_viewport: &start_viewport,
            viewport: &viewport,
            selection_set: false,
            viewport_changed: start_viewport != viewport,
            focus_changed: false,
            focused: false,
        };
        assert_eq!(
            marks(&matcher.update_deco(&update, &previous).unwrap()),
            next["update"],
            "{name}: update",
        );
        assert_eq!(
            marks(&matcher.create_deco(state.text(), &viewport).unwrap()),
            next["rebuild"],
            "{name}: rebuild",
        );
    }
}

#[test]
fn matches_the_verified_view_oracle() {
    for row in include_str!("fixtures/view_matches.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let text = Text::new(case["text"].as_str().unwrap());
        let name = &case["name"];
        match case["kind"].as_str().unwrap() {
            "match" => check_matches(&case),
            "special" => {
                let set = highlight_special_chars(&text, &viewport(&text, &case)).unwrap();
                let ranges: Vec<_> = set
                    .iter(0)
                    .map(|range| {
                        let DecorationKind::Replace {
                            widget: Some(item),
                            spec,
                        } = range.value.kind()
                        else {
                            panic!("{name}: expected a replacement widget");
                        };
                        assert_eq!(*spec, Default::default());
                        json!([
                            range.from,
                            range.to,
                            u32::from(item.character),
                            item.placeholder(),
                            item.description(),
                        ])
                    })
                    .collect();
                assert_eq!(json!(ranges), case["ranges"], "{name}");
            }
            "placeholder" => {
                let set = placeholder(&text, case["label"].as_str().unwrap());
                assert_eq!(set.iter(0).count(), usize::from(!case["widget"].is_null()));
                let widget = set.iter(0).next().map_or(Value::Null, |range| {
                    json!([
                        range.from,
                        range.to,
                        range.value.start_side(),
                        range.value.widget_value().unwrap(),
                    ])
                });
                assert_eq!(widget, case["widget"], "{name}");
                assert_eq!(text.as_str(), case["text"].as_str().unwrap());
            }
            kind => panic!("unknown fixture kind: {kind}"),
        }
    }
}

#[test]
fn configured_placeholders_keep_accessibility_labels_and_update_widgets() {
    let compartment = Compartment::new();
    let shared = placeholder_extension("second");
    let labels = Extension::group([
        placeholder_extension("last").with_precedence(Precedence::Low),
        shared.clone(),
        shared,
        placeholder_extension("").with_precedence(Precedence::High),
    ]);
    let mut state = EditorState::with_extensions("", compartment.of(labels)).unwrap();
    for row in include_str!("fixtures/view_placeholder_config.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let label = case["label"].as_str().unwrap();
        match label {
            "multiple" => {}
            "typed" => {
                state
                    .update(Transaction::new(
                        ChangeSet::single(Change::new(0..0, "typed")).unwrap(),
                    ))
                    .unwrap();
            }
            "replaced" => {
                state
                    .update(
                        Transaction::new(ChangeSet::single(Change::new(0..5, "")).unwrap())
                            .with_effects([compartment.reconfigure(placeholder_extension("new"))]),
                    )
                    .unwrap();
            }
            "removed" => {
                state
                    .update(
                        Transaction::default()
                            .with_effects([compartment.reconfigure(Extension::default())]),
                    )
                    .unwrap();
            }
            _ => panic!("Unrecognized case {label}"),
        }
        assert_eq!(
            state.text().as_str(),
            case["doc"].as_str().unwrap(),
            "{label}"
        );
        let projection = placeholder_projection(&state).unwrap();
        assert_eq!(
            json!(projection.aria_placeholder.as_deref()),
            case["aria"],
            "{label}"
        );
        let widget_sets: Vec<_> = projection
            .widgets
            .iter()
            .flat_map(|set| {
                set.iter(0)
                    .map(|range| json!([range.from, range.to, range.value.widget_value().unwrap()]))
            })
            .collect();
        assert_eq!(json!(widget_sets), case["widgetSets"], "{label}");
    }
}

struct MatchPlugin(MatchDecorator, Rc<RefCell<DecorationSet>>);

impl ViewPlugin for MatchPlugin {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        let mut decorations = self.1.borrow_mut();
        *decorations = self.0.update_deco(update, &decorations).unwrap();
    }
}

#[test]
fn shifts_marks_after_an_edit_before_the_viewport_and_keeps_them_on_selection_updates() {
    let mut view = EditorView::new(EditorState::new("prefix\nfoo"));
    let viewport = Viewport::around(view.state().text(), std::iter::once(7..10)).unwrap();
    view.set_viewport(viewport, false).unwrap();
    let matcher = MatchDecorator::new(Regex::new("foo").unwrap(), MarkDecoration::new("match"));
    let initial = matcher
        .create_deco(view.state().text(), view.viewport())
        .unwrap();
    let mark = initial.iter(0).next().unwrap().value.clone();
    let live = Rc::new(RefCell::new(initial));
    view.add_plugin(MatchPlugin(matcher, live.clone()));

    // Upstream can retain stale offsets here because it does not map the set
    // when the changed extents are outside the viewport.
    let change = ChangeSet::single(Change::new(0..0, "😀")).unwrap();
    view.update(Transaction::new(change), (), |_| (None, ()))
        .unwrap();
    view.set_selection(EditorSelection::single(11, 11)).unwrap();
    assert_eq!(view.viewport().interval(), 11..14);
    assert_eq!(marks(&live.borrow()), json!([[11, 14, "match"]]));
    assert!(mark.same_instance(&live.borrow().iter(0).next().unwrap().value));
    assert_eq!(view.undo(), Some(()));
    assert_eq!(marks(&live.borrow()), json!([[7, 10, "match"]]));
}

#[test]
fn skips_empty_matches_and_partial_crlf_endings() {
    let mut view = EditorView::new(EditorState::new("x1\r\ny2"));
    let viewport = Viewport::around(view.state().text(), std::iter::once(0..3)).unwrap();
    view.set_viewport(viewport, false).unwrap();
    let matcher = MatchDecorator::new(
        Regex::new(r"\r|\n|[0-9]*").unwrap(),
        MarkDecoration::new("match"),
    );
    let initial = matcher
        .create_deco(view.state().text(), view.viewport())
        .unwrap();
    assert_eq!(marks(&initial), json!([[1, 2, "match"]]));
    let live = Rc::new(RefCell::new(initial));
    view.add_plugin(MatchPlugin(matcher, live.clone()));
    let change = ChangeSet::single(Change::new(1..2, "12")).unwrap();
    view.update(Transaction::new(change), (), |_| (None, ()))
        .unwrap();
    assert_eq!(marks(&live.borrow()), json!([[1, 3, "match"]]));
    assert!(
        highlight_special_chars(view.state().text(), view.viewport())
            .unwrap()
            .as_range_set()
            .is_empty()
    );
}

#[test]
fn rejects_a_viewport_with_non_character_boundaries_in_this_text() {
    let viewport = Viewport::complete(&Text::new("ab"));
    let text = Text::new("😀");
    let matcher = MatchDecorator::new(Regex::new(".").unwrap(), MarkDecoration::new("match"));
    assert_eq!(
        matcher.create_deco(&text, &viewport).unwrap_err(),
        EditError::InvalidRange,
    );
    assert_eq!(
        highlight_special_chars(&text, &viewport).unwrap_err(),
        EditError::InvalidRange,
    );
}
