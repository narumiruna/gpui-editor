use std::sync::Arc;

use gpui_editor_codemirror::{
    Change, ChangeSet, EditError, EditorState, PHRASES, Range, Text, Transaction,
    view::{
        MarkDecoration, MatchDecorator, ViewUpdate, Viewport,
        decorations::{Decoration, DecorationKind, DecorationSet, ReplaceSpec, WidgetSpec},
    },
};
use regex::Regex;
use serde_json::{Value, json};

fn snapshot(set: &DecorationSet<String>) -> Value {
    json!(
        set.iter(0)
            .map(|range| {
                let (kind, label) = match range.value.kind() {
                    DecorationKind::Mark(mark) => ("mark", mark.class()),
                    DecorationKind::Widget { widget, .. } => ("point", widget.as_str()),
                    DecorationKind::Replace {
                        widget: Some(widget),
                        ..
                    } => ("replace", widget.as_str()),
                    other => panic!("Unexpected decoration {other:?}"),
                };
                json!([range.from, range.to, kind, label])
            })
            .collect::<Vec<_>>()
    )
}

fn offset(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

#[test]
fn callbacks_match_the_pinned_browser_on_create_and_update() {
    let regexp = Regex::new("(@)(?<name>[a-z]+)").unwrap();
    let boundary = Regex::new(r"\s").unwrap();
    for row in include_str!("fixtures/view_match_callbacks.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let name = case["case"].as_str().unwrap();
        let matcher = match name {
            "decorate" => MatchDecorator::with_decorate(regexp.clone(), |context| {
                let prefix = context.group_range(1).unwrap();
                let suffix = context.group_range(2).unwrap();
                let label = context.captures.name("name").unwrap().as_str();
                assert_eq!(context.text.slice(suffix.clone()).unwrap(), label);
                assert_eq!(suffix.end, context.range.end);
                vec![
                    Decoration::replace(Some("at".to_owned()), ReplaceSpec::default())
                        .try_range(prefix)
                        .unwrap(),
                    Decoration::mark(MarkDecoration::new(format!("name-{label}")))
                        .try_range(suffix.clone())
                        .unwrap(),
                    Decoration::widget(
                        format!("#{label}"),
                        WidgetSpec {
                            side: 1,
                            ..Default::default()
                        },
                    )
                    .try_range(suffix.end..suffix.end)
                    .unwrap(),
                ]
            })
            .with_boundary(boundary.clone()),
            "optional" => MatchDecorator::with_decoration(regexp.clone(), |context| {
                let label = context.captures.name("name").unwrap().as_str();
                (label != "cat").then(|| {
                    let prefix = context.state.unwrap().phrase("Match", &[]);
                    Decoration::replace(Some(format!("{prefix}:{label}")), ReplaceSpec::default())
                })
            }),
            _ => panic!("Unrecognized case {name}"),
        };
        let phrases = PHRASES.of(Arc::new([("Match".into(), "Hit".into())].into()));
        let mut state =
            EditorState::with_extensions(case["doc"].as_str().unwrap(), phrases).unwrap();
        let mut drawn = matcher
            .create_state_deco(&state, &Viewport::complete(state.text()))
            .unwrap();
        assert_eq!(snapshot(&drawn), case["seed"]);
        let untouched = drawn
            .iter(0)
            .find(|range| {
                matches!(
                    range.value.widget_value().map(String::as_str),
                    Some("Hit:dog" | "#dog")
                )
            })
            .unwrap()
            .value
            .clone();
        for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            let previous = state.clone();
            let edit = &step["edit"];
            let changes = ChangeSet::single(Change::new(
                offset(&edit[0])..offset(&edit[1]),
                edit[2].as_str().unwrap(),
            ))
            .unwrap();
            let desc = changes.desc(previous.text().len()).unwrap();
            state.update(Transaction::new(changes)).unwrap();
            let viewport = Viewport::complete(state.text());
            let report = ViewUpdate {
                start_state: &previous,
                state: &state,
                changes: &desc,
                effects: &[],
                input_steps: &[],
                start_viewport: &Viewport::complete(previous.text()),
                viewport: &viewport,
                viewport_changed: false,
                focus_changed: false,
                selection_set: false,
                focused: false,
            };
            drawn = matcher.update_deco(&report, &drawn).unwrap();
            if index == 0 {
                assert!(
                    drawn
                        .iter(0)
                        .any(|range| range.value.same_instance(&untouched))
                );
            }
            assert_eq!(state.text().as_str(), step["doc"].as_str().unwrap());
            assert_eq!(snapshot(&drawn), step["updated"], "{name}, {edit}");
            assert_eq!(
                snapshot(&matcher.create_state_deco(&state, &viewport).unwrap()),
                step["rebuilt"],
                "{name}, {edit}"
            );
        }
    }
}

#[test]
fn capture_positions_are_document_bytes_even_in_a_visible_fragment() {
    let text = Text::new("猫: tag-🦀, tag-x");
    let start = text.as_str().find("tag-").unwrap();
    let end = start + "tag-🦀".len();
    let view = Viewport::new(&text, 0..text.len(), std::iter::once(start..end)).unwrap();
    let matcher =
        MatchDecorator::<()>::with_decorate(Regex::new("tag-(.)").unwrap(), move |context| {
            assert_eq!(context.viewport.interval(), 0..context.text.len());
            assert!(context.state.is_none());
            assert_eq!(context.captures.get(1).unwrap().range(), 4..8);
            assert_eq!(context.range, start..end);
            vec![
                Decoration::mark(MarkDecoration::new("capture"))
                    .try_range(context.group_range(1).unwrap())
                    .unwrap(),
            ]
        })
        .with_max_length(0);
    let result = matcher.create_deco(&text, &view).unwrap();
    let [range] = result.iter(0).collect::<Vec<_>>().try_into().unwrap();
    assert_eq!(range.from..range.to, end - '🦀'.len_utf8()..end);
}

#[test]
fn callback_output_must_fit_the_match_and_use_valid_decoration_ranges() {
    let text = Text::new("猫: x");
    let view = Viewport::complete(&text);
    let regexp = Regex::new("x").unwrap();
    let outside = MatchDecorator::<()>::with_decorate(regexp.clone(), |_| {
        vec![Range::new(
            0,
            3,
            Decoration::mark(MarkDecoration::new("outside")),
        )]
    });
    assert_eq!(
        outside.create_deco(&text, &view).unwrap_err(),
        EditError::InvalidRange
    );
    let point = MatchDecorator::with_decoration(regexp, |_| {
        Some(Decoration::widget((), WidgetSpec::default()))
    });
    assert_eq!(
        point.create_deco(&text, &view).unwrap_err(),
        EditError::InvalidRange
    );
}
