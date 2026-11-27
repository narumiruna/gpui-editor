use gpui::TestAppContext;
use gpui_editor_codemirror::{
    ALLOW_MULTIPLE_SELECTIONS, Compartment, Extension, READ_ONLY, StateEffect, Transaction,
    language::fold::fold_effect, theme_one_dark::one_dark_theme,
    view::gutters::highlight_active_line_gutter,
};
use serde::Deserialize;

use super::*;
use crate::document::BlockPreview;

const ONE_DARK_CONTENT: u32 = 0x6699_ff0b;
const ONE_DARK_GUTTER: u32 = 0x2c31_3aff;
const BASE_CONTENT: u32 = 0xccee_ff44;
const BASE_GUTTER: u32 = 0xe2f2_ffff;

#[derive(Deserialize)]
struct Document {
    body: String,
    fold: [usize; 2],
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    main_index: usize,
    focused: bool,
    selections: Vec<[usize; 2]>,
    blocks: Vec<[usize; 2]>,
    content: Vec<(usize, u32)>,
    gutter: Vec<(usize, u32)>,
}

fn enter_fence(editor: &mut Editor, cx: &mut Context<Editor>) {
    let fence = editor
        .markdown()
        .sections()
        .into_iter()
        .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
        .unwrap()
        .source;
    editor.activate(
        Section::new(SectionKind::DraftCode, fence.clone(), fence),
        false,
        cx,
    );
}

fn select(
    editor: &mut Editor,
    ranges: impl IntoIterator<Item = [usize; 2]>,
    main: usize,
    cx: &mut Context<Editor>,
) {
    let ranges = ranges
        .into_iter()
        .map(|[anchor, head]| SelectionRange::new(anchor, head))
        .collect();
    editor
        .view
        .set_selection(EditorSelection::new(ranges, main).unwrap())
        .unwrap();
    editor.reveal_caret = false;
    cx.notify();
}

fn reconfigure(editor: &mut Editor, effects: Vec<StateEffect>, cx: &mut Context<Editor>) {
    let active = editor.active.clone();
    editor
        .view
        .update(
            Transaction::default().with_effects(effects),
            active.clone(),
            |_| (None, active),
        )
        .unwrap();
    cx.notify();
}

fn assert_quads(
    layout: &Layout,
    body: usize,
    content: &[(usize, u32)],
    gutter: &[(usize, u32)],
    legacy: bool,
    case: &str,
) {
    let frame = layout.source_frame.as_ref();
    let columns = frame.and_then(|frame| frame.gutter.as_ref());
    let rows = |start| {
        let first = layout
            .rows
            .iter()
            .position(|row| row.start == body + start && row.y >= frame.unwrap().bounds.top())
            .unwrap();
        let end = layout.rows[first..]
            .iter()
            .position(|row| !row.soft_end)
            .unwrap()
            + first;
        &layout.rows[first..=end]
    };
    let expected_content = content
        .iter()
        .flat_map(|&(start, color)| {
            let frame = frame.unwrap();
            let left = match columns.filter(|_| !legacy) {
                Some(columns) => SOURCE_PADDING + columns.width,
                None => frame.bounds.left() + px(1.),
            };
            rows(start).iter().map(move |row| {
                (
                    Bounds::new(
                        layout.origin + point(left, row.y),
                        size(frame.bounds.right() - px(1.) - left, row.height),
                    ),
                    rgba(color).into(),
                )
            })
        })
        .collect::<Vec<_>>();
    let expected_gutter = gutter
        .iter()
        .flat_map(|&(start, color)| {
            let columns = columns.unwrap();
            rows(start).iter().map(move |row| {
                (
                    Bounds::new(
                        layout.origin + point(SOURCE_PADDING, row.y),
                        size(columns.width, row.height),
                    ),
                    rgba(color).into(),
                )
            })
        })
        .collect::<Vec<_>>();
    let painted = |quads: &[gpui::PaintQuad]| {
        quads
            .iter()
            .map(|quad| (quad.bounds, quad.background))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        painted(&layout.painted_active_lines.content),
        expected_content,
        "{case}: content"
    );
    assert_eq!(
        painted(&layout.painted_active_lines.gutter),
        expected_gutter,
        "{case}: gutter"
    );
}

#[gpui::test]
fn active_line_paints_pinned_blocks_and_independent_gutters_in_canvas(cx: &mut TestAppContext) {
    let mut fixture =
        include_str!("../../../../codemirror/tests/fixtures/active_line.jsonl").lines();
    let doc: Document = serde_json::from_str(fixture.next().unwrap()).unwrap();
    let prefix = "Prose 🐱\r\n\r\n```text\r\n";
    let source = format!("{prefix}{}\r\n```\r\n\r\nAfter", doc.body);
    let offset = prefix.len();
    let theme = Compartment::new();
    let line = Compartment::new();
    let gutter = Compartment::new();
    let active_line = ViewHighlight::ActiveLine.extension();
    let active_gutter = highlight_active_line_gutter();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.clone(),
            Extension::group([
                ALLOW_MULTIPLE_SELECTIONS.of(true),
                READ_ONLY.of(true),
                theme.of(one_dark_theme()),
                line.of(Extension::default()),
                gutter.of(Extension::default()),
            ]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(240.), px(870.)));
    cx.update(|_, cx| editor.update(cx, enter_fence));
    for case in fixture.map(|row| serde_json::from_str::<Case>(row).unwrap()) {
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                let none = Extension::default;
                let effects = match case.name.as_str() {
                    "line-only-light" => vec![
                        theme.reconfigure(none()),
                        line.reconfigure(active_line.clone()),
                    ],
                    "gutter-only-light" => vec![
                        line.reconfigure(none()),
                        gutter.reconfigure(active_gutter.clone()),
                    ],
                    "both-one-dark-readonly-uneditable" => vec![
                        theme.reconfigure(one_dark_theme()),
                        line.reconfigure(active_line.clone()),
                    ],
                    "remove-gutter-opt-in" => vec![gutter.reconfigure(none())],
                    "remove-content-opt-in" => vec![
                        gutter.reconfigure(active_gutter.clone()),
                        line.reconfigure(none()),
                    ],
                    "remove-both-opt-ins" => vec![gutter.reconfigure(none())],
                    "no-theme-no-opt-ins" => vec![theme.reconfigure(none())],
                    _ => Vec::new(),
                };
                if !effects.is_empty() {
                    reconfigure(editor, effects, cx);
                }
                select(
                    editor,
                    case.selections
                        .iter()
                        .map(|pair| pair.map(|pos| offset + pos)),
                    case.main_index,
                    cx,
                );
                if matches!(case.name.as_str(), "fold-start" | "fold-inside-hidden") {
                    // Native folding opens a fold when the main head enters it.
                    // Apply the fold afterward to measure a retained hidden head.
                    reconfigure(
                        editor,
                        vec![fold_effect().of(offset + doc.fold[0]..offset + doc.fold[1])],
                        cx,
                    );
                }
                if case.focused {
                    window.focus(&editor.focus);
                } else {
                    window.blur();
                }
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            let state = editor.view.state();
            assert!(state.read_only());
            assert_eq!(editor.focus.is_focused(window), case.focused);
            assert_eq!(state.selection().main_index(), case.main_index);
            let actual = state
                .selection()
                .ranges()
                .iter()
                .map(|range| [range.anchor() - offset, range.head() - offset])
                .collect::<Vec<_>>();
            assert_eq!(actual, case.selections, "{}", case.name);
            let blocks = actual
                .iter()
                .map(|&[_, head]| {
                    let block = layout.line_block(offset + head).unwrap();
                    [block.start - offset, block.end - offset]
                })
                .collect::<Vec<_>>();
            assert_eq!(blocks, case.blocks, "{}", case.name);
            if case.name == "fold-start" {
                assert!(
                    layout
                        .rows
                        .iter()
                        .filter(|row| (offset..offset + case.blocks[0][1]).contains(&row.start))
                        .count()
                        > 2
                );
            }
            match case.name.as_str() {
                // The upstream theme alone never opts in. GPUI retains its
                // focused legacy paint when neither explicit extension is installed.
                "theme-only-focused" => assert_quads(
                    layout,
                    offset,
                    &[(0, ONE_DARK_CONTENT), (84, ONE_DARK_CONTENT)],
                    &[(0, ONE_DARK_GUTTER), (84, ONE_DARK_GUTTER)],
                    true,
                    &case.name,
                ),
                "remove-both-opt-ins" => assert_quads(
                    layout,
                    offset,
                    &[(146, ONE_DARK_CONTENT)],
                    &[(146, ONE_DARK_GUTTER)],
                    true,
                    &case.name,
                ),
                _ => assert_quads(
                    layout,
                    offset,
                    &case.content,
                    &case.gutter,
                    false,
                    &case.name,
                ),
            }
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.revision(), 0);
            assert!(!editor.view.can_undo());
        });
    }
}

#[gpui::test]
fn active_line_respects_workspace_freeze_and_measured_fence_body(cx: &mut TestAppContext) {
    let prefix = "Before\r\n\r\n```text\r\n";
    let source = format!("{prefix}alpha\r\nbeta\r\n```\r\n\r\nAfter");
    let origin = prefix.len();
    let midpoint = origin + "alpha\r".len();
    let separator = origin + "alpha\r\nbeta\r".len();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.clone(),
            Extension::group([
                ALLOW_MULTIPLE_SELECTIONS.of(true),
                ViewHighlight::ActiveLine.extension(),
                highlight_active_line_gutter(),
            ]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(440.), px(530.)));
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            window.focus(&editor.focus);
            select(editor, [[2; 2]], 0, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let layout = editor.layout.as_ref().unwrap();
            assert!(layout.source_frame.is_none());
            assert_quads(
                layout,
                origin,
                &[],
                &[],
                false,
                "prose and read-only preview",
            );
            enter_fence(editor, cx);
        });
    });
    let header = prefix.find("text").unwrap();
    let closing_fence = source.rfind("```").unwrap();
    select_outside_and_inside(cx, &editor, [2, header, midpoint, separator, closing_fence]);
    for enabled in [true, false, true] {
        cx.update(|_, cx| editor.update(cx, |editor, cx| editor.set_input_enabled(enabled, cx)));
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let layout = editor.layout.as_ref().unwrap();
            assert_eq!(layout.line_block(origin).unwrap(), origin..midpoint - 1);
            let editable = layout
                .source_frame
                .as_ref()
                .unwrap()
                .editable
                .as_ref()
                .unwrap();
            assert_eq!(editable.end, separator - 1);
            assert!(ViewHighlight::ActiveLine.enabled(editor.view.state()));
            assert!(highlight_active_line_gutter_enabled(editor.view.state()));
            let content: &[_] = if enabled { &[(0, BASE_CONTENT)] } else { &[] };
            let gutter: &[_] = if enabled { &[(0, BASE_GUTTER)] } else { &[] };
            assert_quads(
                layout,
                origin,
                content,
                gutter,
                false,
                "input freeze and CRLF midpoint",
            );
            assert_eq!(editor.source(), source);
        });
    }
    select_outside_and_inside(cx, &editor, [2, header, separator, closing_fence]);
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_quads(
            editor.layout.as_ref().unwrap(),
            origin,
            &[],
            &[],
            false,
            "protected heads only",
        );
    });
}

fn select_outside_and_inside<const N: usize>(
    cx: &mut gpui::VisualTestContext,
    editor: &gpui::Entity<Editor>,
    heads: [usize; N],
) {
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            select(editor, heads.map(|head| [head; 2]), 1, cx)
        })
    });
}

#[gpui::test]
fn active_line_does_not_add_gutter_to_unlabeled_fence(cx: &mut TestAppContext) {
    let source = "```\r\nUnlabeled source\r\n```";
    let body = source.find("Unlabeled").unwrap();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            source.into(),
            Extension::group([
                ViewHighlight::ActiveLine.extension(),
                highlight_active_line_gutter(),
            ]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            enter_fence(editor, cx);
            window.blur();
            select(editor, [[body + 3; 2]], 0, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let layout = editor.layout.as_ref().unwrap();
        let frame = layout.source_frame.as_ref().unwrap();
        assert!(frame.gutter.is_none());
        assert_quads(
            layout,
            body,
            &[(0, BASE_CONTENT)],
            &[],
            false,
            "no gutter projection",
        );
        assert_eq!(editor.source(), source);
    });
}
