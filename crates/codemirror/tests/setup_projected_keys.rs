use std::time::Duration;

use gpui_editor_codemirror::{
    EditorSelection, EditorState, Text, Transaction,
    commands::visual::{VerticalDistance, VisualCursor, VisualDirection, VisualGeometry},
    language_data::NativeLanguage,
    setup::{SetupGeometry, SetupKeyContext, basic_setup, minimal_setup},
    view::keymap::{KeyEvent, KeyModifiers, KeyPlatform},
};
use serde_json::{Value, json};

struct Rows(Vec<usize>);

impl VisualGeometry for Rows {
    fn move_vertical(
        &self,
        start: VisualCursor,
        direction: VisualDirection,
        _: VerticalDistance,
    ) -> VisualCursor {
        let index = self.0.iter().position(|&head| head == start.head).unwrap();
        let next = match direction {
            VisualDirection::Backward => index.checked_sub(1),
            VisualDirection::Forward => Some(index + 1),
        };
        next.and_then(|index| self.0.get(index))
            .map_or(start, |&head| VisualCursor::new(head))
    }

    fn line_boundary(&self, start: VisualCursor, _: VisualDirection) -> VisualCursor {
        start
    }
}

#[test]
fn setup_keys_use_logical_lines_without_current_syntax() {
    let stale = NativeLanguage::Json.parse(Text::new("[]")).unwrap();
    for row in include_str!("fixtures/setup_projected_keys.jsonl").lines() {
        let case: Value = serde_json::from_str(row).unwrap();
        let name = case["command"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let at = case["at"].as_u64().unwrap() as usize;
        let key = case["key"].as_str().unwrap();
        let mut modifiers = KeyModifiers::default();
        let mut parts = key.split('-').peekable();
        let key = loop {
            let part = parts.next().unwrap();
            if parts.peek().is_none() {
                break part;
            }
            match part {
                "Ctrl" => modifiers.control = true,
                "Alt" => modifiers.alt = true,
                "Shift" => modifiers.shift = true,
                _ => unreachable!("invalid fixture key {key}"),
            }
        };
        let rows = serde_json::from_value::<Option<Vec<usize>>>(case["rows"].clone())
            .unwrap()
            .map(Rows);
        for syntax in [None, Some(&stale)] {
            let mut state = EditorState::new(source);
            let original = EditorSelection::single(at, at);
            state
                .update(Transaction::default().with_selection(original.clone()))
                .unwrap();
            let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
            let result = editor
                .key_event_with(
                    KeyEvent::new(key, modifiers),
                    Duration::ZERO,
                    (),
                    SetupKeyContext {
                        syntax,
                        geometry: rows.as_ref().map(|geometry| SetupGeometry {
                            geometry,
                            selection: None,
                        }),
                        ..Default::default()
                    },
                )
                .unwrap();
            let view = editor.view();
            let Some(expected) = case["output"].as_str() else {
                assert_eq!(view.state().text().as_str(), source, "{name}");
                assert_eq!(view.state().selection(), &original, "{name}");
                assert_eq!(view.undo_depth(), 0);
                continue;
            };
            assert!(result.key.handled, "{name}");
            assert_eq!(view.state().text().as_str(), expected, "{name}");
            let selection = view.state().selection();
            let ranges: Vec<_> = selection
                .ranges()
                .iter()
                .map(|range| [range.anchor(), range.head()])
                .collect();
            assert_eq!(json!(ranges), case["select"], "{name}");
            assert_eq!(json!(selection.main_index()), case["main"], "{name}");
            assert_eq!(view.undo_depth(), usize::from(expected != source));
            if expected != source {
                assert!(editor.view_mut().undo().is_some());
                assert_eq!(editor.view().state().text().as_str(), source);
                assert_eq!(editor.view().state().selection(), &original);
            }
        }
    }
}

#[test]
fn minimal_setup_projects_hard_line_motion_without_normalizing_text() {
    let source = "甲\r😀x\r\né\n終";
    let mut state = EditorState::new(source);
    state
        .update(Transaction::default().with_selection(EditorSelection::single(8, 8)))
        .unwrap();
    let mut editor = minimal_setup::<()>(state, KeyPlatform::Linux);
    assert!(
        editor
            .key_event_with(
                KeyEvent::new("Home", KeyModifiers::default()),
                Duration::ZERO,
                (),
                SetupKeyContext::default()
            )
            .unwrap()
            .key
            .handled
    );
    assert_eq!(editor.view().state().selection().main().head(), 4);
    assert_eq!(editor.view().state().text().as_str(), source);
}
