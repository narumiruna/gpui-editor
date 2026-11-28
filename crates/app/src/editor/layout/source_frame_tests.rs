use gpui::{Background, Corners, EntityInputHandler, Rgba, TestAppContext, WindowAppearance};
use gpui_editor_codemirror::{
    Compartment, Extension,
    language::Color,
    theme_one_dark::{EditorSurfaceColors, one_dark_theme, override_editor_surface},
};
use serde::Deserialize;

use super::{Bounds, Edges, Editor, ItemPaint, SOURCE_HEADER, SourceFrame, point, px, rgba, size};
use crate::editor::one_dark_tests::{activate_code, configure, painted_runs};

#[derive(Deserialize)]
struct BrowserFrame<'a> {
    name: &'a str,
    source: String,
    root: &'a str,
    scroller: &'a str,
    content: &'a str,
    line: &'a str,
}

fn color(hex: &str) -> Rgba {
    rgba(u32::from_str_radix(hex, 16).unwrap())
}

fn background_override(background: u32) -> Extension {
    override_editor_surface(EditorSurfaceColors {
        background: Some(Color::from_rgba32(background)),
        ..Default::default()
    })
}

#[gpui::test]
fn editable_source_frame_inherits_the_installed_background(cx: &mut TestAppContext) {
    const PREVIEW: &str = "{\"title\": \"preview\"}";
    let oracle: Vec<BrowserFrame> = include_str!("source_frame_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let code = &oracle[0].source;
    let mark = code.find("draft").unwrap();
    let unmarked = code.replace("draft", "X");
    let config = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            format!("```json\n{unmarked}\n```\n\n```json\n{PREVIEW}\n```\n"),
            config.of(Extension::default()),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(640.), px(420.)));
    let (start, marked, source, selection, history) = cx.update(|window, cx| {
        assert_eq!(window.appearance(), WindowAppearance::Light);
        editor.update(cx, |editor, cx| {
            let start = activate_code(editor, &unmarked, cx);
            editor.set_selection(start + mark, start + mark + 1);
            editor.replace_and_mark_text_in_range(None, "draft", None, window, cx);
            (
                start,
                editor.marked.clone().unwrap(),
                editor.source().to_owned(),
                editor.view.state().selection().clone(),
                editor.view.undo_depth(),
            )
        })
    });
    assert_eq!(&source[start..start + code.len()], code);
    assert_eq!(marked, start + mark..start + mark + "draft".len());

    let host = background_override(0x1020_30ff);
    let cases = [
        ("bare", Extension::default(), false),
        ("oneDark", one_dark_theme(), true),
        (
            "hostFirst",
            Extension::group([host.clone(), one_dark_theme()]),
            true,
        ),
        (
            "themeFirst",
            Extension::group([one_dark_theme(), host.clone()]),
            true,
        ),
        ("hostOnly", host, false),
        ("translucentHost", background_override(0x1020_3080), false),
        (
            "foregroundOnly",
            override_editor_surface(EditorSurfaceColors {
                foreground: Some(Color::from_rgba32(0x3040_50ff)),
                ..Default::default()
            }),
            false,
        ),
        ("removed", Extension::default(), false),
    ];
    assert_eq!(oracle.len(), cases.len());
    let mut geometry = None;
    for (browser, (name, extension, dark)) in oracle.iter().zip(cases) {
        assert_eq!(browser.name, name);
        assert_eq!(&browser.source, code);
        for region in [browser.scroller, browser.content, browser.line] {
            assert_eq!(region, "00000000");
        }
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, config.reconfigure(extension), cx)
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert_eq!(editor.marked.as_ref(), Some(&marked));
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.undo_depth(), history);

            let layout = editor.layout.as_ref().unwrap();
            let frame = layout.source_frame.as_ref().unwrap();
            let bounds = Bounds::new(layout.origin + frame.bounds.origin, frame.bounds.size);
            let root = color(browser.root);
            let native = root.a != 0.;
            let chrome = rgba(if dark { 0x2328_32ff } else { 0xf6f8_fcff });
            assert_eq!(
                layout.theme.surface,
                if native { root } else { rgba(0xffff_ffff) }
            );
            assert_eq!(layout.theme.preview, chrome);
            assert_eq!(frame.label.text.as_ref(), "<code>");

            let (row, line) = layout
                .rows
                .iter()
                .find_map(|row| {
                    row.items.iter().find_map(|item| match &item.paint {
                        ItemPaint::Text(line) if item.source.start == start => Some((row, line)),
                        _ => None,
                    })
                })
                .unwrap();
            assert_eq!(line.text.as_ref(), code);
            assert!(
                painted_runs(line, layout.theme.text).contains(&(mark..mark + "draft".len(), true))
            );
            let caret = layout.bounds_at(marked.start, false).unwrap();
            assert_eq!(layout.index_at(caret.center()), marked.start);
            let (preview, preview_row) = layout
                .rows
                .iter()
                .find_map(|row| {
                    row.items.iter().find_map(|item| match &item.paint {
                        ItemPaint::Block(block) => Some((block, (row.y, row.height, item.width))),
                        _ => None,
                    })
                })
                .unwrap();
            assert_eq!(preview.code_lines().next().unwrap().text.as_ref(), PREVIEW);
            let geometry_now = (bounds, caret, preview_row);
            assert_eq!(&geometry_now, geometry.get_or_insert(geometry_now));

            // Sample the solid backgrounds that the painter submits at glyph-free points.
            // Do not sample the border, glyphs, or pixels from a GPU scene.
            let quads: Vec<_> = SourceFrame::background_quads(bounds, layout.theme).collect();
            let x = bounds.right() - px(12.);
            assert!(x > layout.origin.x + row.width);
            let background_at = |at| -> Background {
                let mut covering = quads.iter().filter(|(mask, quad)| {
                    quad.bounds.contains(&at)
                        && mask.as_ref().is_none_or(|mask| mask.bounds.contains(&at))
                });
                let (_, quad) = covering.next().unwrap();
                assert!(covering.next().is_none());
                quad.background
            };
            let header = background_at(point(x, bounds.top() + SOURCE_HEADER / 2.));
            let body = background_at(point(x, layout.origin.y + row.y + row.height / 2.));
            assert_eq!(header, chrome.into(), "{name} header");
            if native {
                assert!(body.is_transparent(), "{name} body must expose the root");
                assert_eq!(quads.len(), 2);
                let seam = bounds.top() + SOURCE_HEADER;
                let header = quads[0].0.as_ref().unwrap().bounds;
                let body = quads[1].0.as_ref().unwrap().bounds;
                assert_eq!(
                    header,
                    Bounds::from_corners(bounds.origin, point(bounds.right(), seam))
                );
                assert_eq!(
                    body,
                    Bounds::from_corners(point(bounds.left(), seam), bounds.bottom_right())
                );
            } else {
                assert_eq!(body, chrome.into(), "{name} keeps the stock fill");
                assert_eq!(quads.len(), 1);
                assert!(quads[0].0.is_none());
            }
            for (_, quad) in quads {
                assert_eq!(quad.bounds, bounds);
                assert_eq!(quad.corner_radii, Corners::all(px(6.)));
                assert_eq!(quad.border_widths, Edges::all(px(1.)));
                assert_eq!(quad.border_color, layout.theme.border.into());
            }
        });
    }
}
