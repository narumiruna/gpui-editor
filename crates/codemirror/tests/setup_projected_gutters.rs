//! Pinned `codemirror` 6.0.2 browser snapshots, with offsets converted back
//! to bytes in the original source. The headless host supplies logical rows.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditError, EditorSelection, EditorState, Extension,
    LINE_SEPARATOR, Precedence, SelectionRange, Text, Transaction,
    language::fold::{FOLD_GUTTER_CLASS, fold_effect, fold_service, unfold_effect},
    language_data::NativeLanguage,
    setup::{SetupEditor, basic_setup},
    view::{
        MarkDecoration, MatchDecorator, ViewHighlight, Viewport,
        decorations::{Decoration, DecorationKind, DecorationSet},
        gutters::{
            GUTTER_LINE_CLASS, GutterConfig, GutterMarker, GutterMarkerSet, GutterSlot,
            LineNumberConfig, gutter, gutter_slots, line_numbers,
        },
        keymap::KeyPlatform,
        update_listener,
    },
};
use regex::Regex;
use serde_json::{Value, json};

const ORACLE: &str = include_str!("fixtures/setup_projected_gutters.jsonl");

fn byte(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn shared_marker(from: usize, class: &str) -> Extension {
    GUTTER_LINE_CLASS.of(Arc::new(
        GutterMarkerSet::of(vec![GutterMarker::class(class).at(from)], false).unwrap(),
    ))
}

fn snapshot(slots: &[GutterSlot]) -> Value {
    fn element(markers: &[GutterMarker]) -> Value {
        let class = std::iter::once("cm-gutterElement")
            .chain(
                markers
                    .iter()
                    .map(|marker| marker.element_class.as_str())
                    .filter(|class| !class.is_empty()),
            )
            .collect::<Vec<_>>()
            .join(" ");
        let text: String = markers
            .iter()
            .filter_map(|marker| marker.content.as_deref())
            .collect();
        json!([class, text])
    }
    Value::Array(
        slots
            .iter()
            .map(|slot| {
                json!({
                    "class": format!("cm-gutter {}", slot.class),
                    "spacer": slot.spacer.as_ref().map(|marker| element(std::slice::from_ref(marker))),
                    "rows": slot.rows.iter().map(|row| element(&row.markers)).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

fn active_lines(editor: &SetupEditor<()>) -> Vec<usize> {
    editor
        .decorations()
        .unwrap()
        .active_lines
        .iter(0)
        .map(|mark| {
            assert_eq!(mark.from, mark.to);
            assert!(matches!(mark.value.kind(), DecorationKind::Line(line) if line.class() == "cm-activeLine"));
            mark.from
        })
        .collect()
}

#[test]
fn matches_pinned_gutters_active_lines_spacers_and_clicks() {
    let expected: Value = serde_json::from_str(ORACLE.lines().next().unwrap()).unwrap();
    let raw = Text::new(expected["source"].as_str().unwrap());
    let logical = expected["lines"].as_array().unwrap();
    let bounds: Vec<_> = logical
        .iter()
        .map(|line| (byte(&line[0]), byte(&line[1])))
        .collect();
    let selection = EditorSelection::new(
        expected["selection"]
            .as_array()
            .unwrap()
            .iter()
            .map(|range| SelectionRange::new(byte(&range[0]), byte(&range[1])))
            .collect(),
        byte(&expected["main"]),
    )
    .unwrap();
    let updates = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = EditorState::create_from_text(
        raw.clone(),
        Some(selection.clone()),
        Extension::group([
            gutter(GutterConfig {
                class: FOLD_GUTTER_CLASS.into(),
                render_empty_elements: true,
                line_marker: Some(Arc::new({
                    let raw = raw.clone();
                    let bounds = bounds.clone();
                    move |state, line, _| {
                        assert_eq!(state.text(), &raw);
                        let (from, to) = bounds[line.number - 1];
                        assert_eq!(line.range, from..to);
                        assert_eq!(line.text, &raw.as_str()[from..to]);
                        (from == 0).then(|| GutterMarker::text("custom"))
                    }
                })),
                ..Default::default()
            }),
            line_numbers(LineNumberConfig {
                format_number: Some(Arc::new({
                    let raw = raw.clone();
                    move |number, state| {
                        assert_eq!(state.text(), &raw);
                        format!("n{number}")
                    }
                })),
            }),
            shared_marker(bounds[5].0, "before-active"),
            shared_marker(bounds[5].0, "after-active").with_precedence(Precedence::Low),
            fold_service({
                let raw = raw.clone();
                let calls = calls.clone();
                let (start, end) = bounds[1];
                let close = bounds[3].0;
                move |state, from, to| {
                    assert_eq!(state.text(), &raw);
                    calls.lock().unwrap().push((from, to));
                    ((from, to) == (start, end)).then_some(end..close)
                }
            }),
            update_listener({
                let updates = updates.clone();
                move |update| {
                    let effects = update
                        .effects
                        .iter()
                        .map(|effect| {
                            if let Some(range) = effect.get(fold_effect()) {
                                json!(["fold", range.start, range.end])
                            } else if let Some(range) = effect.get(unfold_effect()) {
                                json!(["unfold", range.start, range.end])
                            } else {
                                json!(["other"])
                            }
                        })
                        .collect::<Vec<_>>();
                    updates.lock().unwrap().push(json!({
                        "selectionSet": update.selection_set,
                        "docChanged": update.doc_changed(),
                        "effects": effects,
                    }));
                    Ok(())
                }
            }),
        ]),
    )
    .unwrap();
    let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
    let slots = editor.gutters(1..=logical.len(), None).unwrap();
    assert_eq!(snapshot(&slots), expected["gutters"]);
    assert_eq!(*calls.lock().unwrap(), bounds);
    for (index, row) in slots[1].rows.iter().enumerate() {
        assert_eq!((row.line_number, row.from), (index + 1, bounds[index].0));
    }
    assert_eq!(
        slots[2]
            .rows
            .iter()
            .map(|row| row.line_number)
            .collect::<Vec<_>>(),
        [2, 5, 6, 11]
    );
    let viewport = Viewport::around(&raw, std::iter::once(0..bounds[0].1)).unwrap();
    editor.view_mut().set_viewport(viewport, false).unwrap();
    assert_eq!(json!(active_lines(&editor)), expected["active"]);

    calls.lock().unwrap().clear();
    let end = logical.len();
    let empty = editor.gutters(end + 1..=end, None).unwrap();
    assert!(empty.iter().all(|slot| slot.rows.is_empty()));
    assert_eq!(empty[1].spacer, slots[1].spacer);
    assert_eq!(empty[2].spacer, slots[2].spacer);
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(editor.gutters(0..=1, None), Err(EditError::InvalidRange));
    assert_eq!(
        editor.gutters(end..=end + 1, None),
        Err(EditError::InvalidRange)
    );
    assert_eq!(
        editor.toggle_line_fold(end + 1, None, Duration::ZERO, ()),
        Err(EditError::InvalidRange)
    );

    updates.lock().unwrap().clear();
    assert!(
        !editor
            .toggle_line_fold(3, None, Duration::ZERO, ())
            .unwrap()
    );
    assert!(updates.lock().unwrap().is_empty());
    assert_eq!(std::mem::take(&mut *calls.lock().unwrap()), [bounds[2]]);
    for click in expected["click"].as_array().unwrap() {
        let handled = editor
            .toggle_line_fold(2, None, Duration::ZERO, ())
            .unwrap();
        let scroll = editor.view_mut().take_scroll_request().is_some();
        let mut dispatched = std::mem::take(&mut *updates.lock().unwrap());
        assert_eq!(dispatched.len(), 1);
        dispatched[0]["scroll"] = json!(scroll);
        assert_eq!(
            json!({"handled": handled, "updates": dispatched, "undo": editor.view().undo_depth(),
                "redo": editor.view().redo_depth()}),
            *click
        );
        assert_eq!(editor.view().state().text(), &raw);
        assert_eq!(editor.view().state().selection(), &selection);
    }
    // Only the open click asks the service. The close click uses the stored range.
    assert_eq!(*calls.lock().unwrap(), [bounds[1]]);
}

#[test]
fn fold_syntax_fallback_uses_one_service_pass_for_mixed_lf_and_crlf_sources() {
    for (first, second) in [("\r", "\r\n"), ("\n", "\n"), ("\r\n", "\r\n")] {
        let source = format!("[{first}  {{{second}    \"猫\": 1{first}  }}{second}]");
        let raw = Text::new(&source);
        let start = source.find("  {").unwrap();
        let end = start + "  {".len();
        let body = end + second.len();
        let body_end = body + "    \"猫\": 1".len();
        let region = end..source.find('}').unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let state = EditorState::create_from_text(
            raw.clone(),
            None,
            fold_service({
                let calls = calls.clone();
                let raw = raw.clone();
                move |state, from, to| {
                    assert_eq!(state.text(), &raw);
                    calls.lock().unwrap().push((from, to));
                    None
                }
            }),
        )
        .unwrap();
        let mut editor = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
        let syntax = NativeLanguage::Json.parse(raw).unwrap();
        let stale = NativeLanguage::Json
            .parse(Text::new(source.replace('猫', "犬")))
            .unwrap();
        let slots = editor.gutters(2..=2, Some(&stale)).unwrap();
        assert!(slots[1].rows.is_empty());
        assert_eq!(std::mem::take(&mut *calls.lock().unwrap()), [(start, end)]);
        let slots = editor.gutters(2..=2, Some(&syntax)).unwrap();
        assert_eq!(slots[0].rows[0].from, start);
        assert_eq!(slots[1].rows[0].markers, [GutterMarker::text("⌄")]);
        assert_eq!(std::mem::take(&mut *calls.lock().unwrap()), [(start, end)]);
        let slots = editor.gutters(3..=3, Some(&syntax)).unwrap();
        assert!(slots[1].rows.is_empty());
        assert_eq!(
            std::mem::take(&mut *calls.lock().unwrap()),
            [(body, body_end)]
        );
        assert!(
            editor
                .toggle_line_fold(2, Some(&syntax), Duration::ZERO, ())
                .unwrap()
        );
        assert_eq!(std::mem::take(&mut *calls.lock().unwrap()), [(start, end)]);
        let folds = editor.configured_fold_placeholders().unwrap();
        let fold = folds.iter(0).next().unwrap();
        assert_eq!(fold.from..fold.to, region);
        let slots = editor.gutters(2..=2, None).unwrap();
        assert_eq!(slots[1].rows[0].markers, [GutterMarker::text("›")]);
        assert!(
            editor
                .toggle_line_fold(2, Some(&stale), Duration::ZERO, ())
                .unwrap()
        );
        assert!(calls.lock().unwrap().is_empty());
    }
}

#[test]
fn projected_rows_refresh_after_edits_and_do_not_survive_preset_removal() {
    let source = "\n".repeat(8);
    let mut editor = basic_setup::<()>(EditorState::new(&source), KeyPlatform::Linux).unwrap();
    assert_eq!(
        editor.gutters(9..=9, None).unwrap()[0].spacer,
        Some(GutterMarker::text("9"))
    );
    let eof = source.len();
    let changes = ChangeSet::single(Change::new(eof..eof, "\r")).unwrap();
    editor
        .view_mut()
        .update(
            Transaction::new(changes).with_selection(EditorSelection::single(eof + 1, eof + 1)),
            (),
            |_| (None, ()),
        )
        .unwrap();
    let state = editor.view().state();
    assert_eq!(state.text().as_str(), format!("{source}\r"));
    assert_eq!(state.text().line_count(), 9);
    let slots = gutter_slots(state, 10..=10).unwrap();
    assert_eq!(slots[0].spacer, Some(GutterMarker::text("99")));
    assert_eq!(slots[0].rows[0].from, eof + 1);
    assert_eq!(
        slots[0].rows[0].markers[1].element_class,
        "cm-activeLineGutter"
    );
    assert_eq!(active_lines(&editor), [eof + 1]);

    let extension = gutter(GutterConfig {
        class: FOLD_GUTTER_CLASS.into(),
        render_empty_elements: true,
        ..Default::default()
    });
    editor
        .view_mut()
        .update(
            Transaction::default().with_effects([extension.reconfigure()]),
            (),
            |_| (None, ()),
        )
        .unwrap();
    assert_eq!(editor.gutters(10..=10, None), Err(EditError::InvalidRange));
    let slots = editor.gutters(9..=9, None).unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].rows[0].from, eof);
    assert!(slots[0].rows[0].markers.is_empty());
    assert!(active_lines(&editor).is_empty());
    assert!(
        !editor
            .toggle_line_fold(9, None, Duration::ZERO, ())
            .unwrap()
    );
}

#[test]
fn installed_matchers_use_projected_lines_with_raw_ranges() {
    fn ranges(set: &DecorationSet) -> Value {
        json!(
            set.iter(0)
                .map(|mark| [mark.from, mark.to])
                .collect::<Vec<_>>()
        )
    }

    let regex = Regex::new("^[ab]").unwrap();
    for line in ORACLE.lines().skip(1) {
        let expected: Value = serde_json::from_str(line).unwrap();
        let raw = Text::new(expected["text"].as_str().unwrap());
        let separator = expected["separator"].as_str();
        let slot = Compartment::new();
        let matcher = MatchDecorator::<()>::with_decoration(regex.clone(), {
            let raw = raw.clone();
            move |context| {
                assert!(matches!(
                    context.text.as_str().strip_prefix(raw.as_str()),
                    Some("" | "!")
                ));
                assert_eq!(context.state.unwrap().text(), context.text);
                assert_eq!(
                    context.captures.get(0).unwrap().as_str(),
                    &raw.as_str()[context.range]
                );
                Some(Decoration::mark(MarkDecoration::new("anchor")))
            }
        });
        let drawn = Arc::new(Mutex::new(DecorationSet::empty()));
        let mut editor = basic_setup::<()>(
            EditorState::create_from_text(
                raw.clone(),
                None,
                Extension::group([
                    ViewHighlight::Whitespace.extension(),
                    ViewHighlight::TrailingWhitespace.extension(),
                    slot.of(
                        separator.map_or_else(Extension::default, |s| LINE_SEPARATOR.of(s.into()))
                    ),
                    update_listener({
                        let matcher = matcher.clone();
                        let drawn = drawn.clone();
                        move |update| {
                            let mut drawn = drawn.lock().unwrap();
                            *drawn = matcher.update_deco(update, &drawn).unwrap();
                            Ok(())
                        }
                    }),
                ]),
            )
            .unwrap(),
            KeyPlatform::Linux,
        )
        .unwrap();
        let state = editor.view().state();
        let viewport = editor.view().viewport();
        for (extension, name) in [
            (ViewHighlight::Whitespace, "whitespace"),
            (ViewHighlight::TrailingWhitespace, "trailing"),
        ] {
            let marks = extension.project(state, viewport).unwrap();
            assert_eq!(
                json!(
                    marks
                        .iter(0)
                        .map(|mark| {
                            let DecorationKind::Mark(class) = mark.value.kind() else {
                                panic!("expected a mark")
                            };
                            json!([mark.from, mark.to, class.class()])
                        })
                        .collect::<Vec<_>>()
                ),
                expected[name]
            );
        }
        let special = editor.decorations().unwrap().special_chars;
        assert_eq!(
            json!(
                special
                    .iter(0)
                    .map(|mark| {
                        [
                            mark.from,
                            mark.to,
                            mark.value.widget_value().unwrap().character as usize,
                        ]
                    })
                    .collect::<Vec<_>>()
            ),
            expected["special"]
        );
        *drawn.lock().unwrap() = matcher.create_state_deco(state, viewport).unwrap();
        assert_eq!(ranges(&drawn.lock().unwrap()), expected["anchors"]);

        assert_eq!(editor.view().state().text(), &raw);
        if let Some(separator) = separator {
            editor
                .view_mut()
                .update(
                    Transaction::default().with_effects([slot.reconfigure(Extension::default())]),
                    (),
                    |_| (None, ()),
                )
                .unwrap();
            assert_eq!(ranges(&drawn.lock().unwrap()), json!([[0, 1], [3, 4]]));
            assert_eq!(editor.view().state().text(), &raw);
            let append = ChangeSet::single(Change::new(raw.len()..raw.len(), "!")).unwrap();
            editor
                .view_mut()
                .update(
                    Transaction::new(append)
                        .with_effects([slot.reconfigure(LINE_SEPARATOR.of(separator.into()))]),
                    (),
                    |_| (None, ()),
                )
                .unwrap();
            assert_eq!(ranges(&drawn.lock().unwrap()), expected["anchors"]);
            assert_eq!(
                editor.view().state().text().as_str(),
                format!("{}!", raw.as_str())
            );
        }
    }
}
