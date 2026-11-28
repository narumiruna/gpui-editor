use gpui::{Hsla, ShapedLine, TestAppContext, WindowAppearance, px, rgb, size};
use gpui_editor_codemirror::{
    Compartment,
    theme_one_dark::{one_dark, one_dark_theme},
};
use serde::Deserialize;

use crate::editor::{Editor, layout::ItemPaint, one_dark_tests::configure};

#[derive(Deserialize)]
struct Case {
    language: String,
    mode: String,
    source: String,
    samples: Vec<Sample>,
}

#[derive(Deserialize)]
struct Sample {
    row: usize,
    token: String,
    color: String,
}

#[gpui::test]
fn html_preview_keeps_mounted_css_and_javascript_colors(cx: &mut TestAppContext) {
    check_preview("html", cx);
}

#[gpui::test]
fn markdown_preview_colors_html_and_its_mounted_css_and_javascript(cx: &mut TestAppContext) {
    check_preview("markdown", cx);
}

fn check_preview(language: &str, cx: &mut TestAppContext) {
    let mut cases = include_str!("mounted_preview_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Case>(line).unwrap())
        .filter(|case| case.language == language);
    let view_only = cases.next().unwrap();
    let combined = cases.next().unwrap();
    assert!(cases.next().is_none());
    assert_eq!(view_only.mode, "view-only");
    assert_eq!(combined.mode, "combined");
    assert_eq!(view_only.source, combined.source);

    let body = &view_only.source;
    let source = format!("```{language}\r\n{}```\r\n", body.replace('\n', "\r\n"));
    let displayed: Vec<_> = body
        .lines()
        .map(|line| line.replace('\t', "    "))
        .collect();
    let config = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        assert_eq!(window.appearance(), WindowAppearance::Light);
        Editor::with_native_extensions(source.clone(), config.of(one_dark_theme()), window, cx)
            .unwrap()
    });
    cx.simulate_resize(size(px(520.), px(360.)));
    let mut geometry = None;
    for case in [view_only, combined] {
        if case.mode == "combined" {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    configure(editor, config.reconfigure(one_dark()), cx)
                });
            });
        }
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            let layout = editor.layout.as_ref().unwrap();
            let mut previews = layout.rows.iter().flat_map(|row| {
                row.items.iter().filter_map(move |item| match &item.paint {
                    ItemPaint::Block(block) => Some((row, item, block)),
                    _ => None,
                })
            });
            let (row, item, block) = previews.next().unwrap();
            assert!(previews.next().is_none());
            let lines: Vec<_> = block.code_lines().collect();
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.text.as_ref())
                    .collect::<Vec<_>>(),
                displayed,
            );
            let current = (
                item.source.clone(),
                row.y,
                row.height,
                item.left,
                item.width,
                block
                    .lines
                    .iter()
                    .map(|(point, line)| (*point, line.width))
                    .collect::<Vec<_>>(),
            );
            assert_eq!(&current, geometry.get_or_insert_with(|| current.clone()));
            for sample in case.samples {
                assert_token_color(language, &case.mode, lines[sample.row], &sample);
            }
        });
    }
}

fn assert_token_color(language: &str, mode: &str, line: &ShapedLine, sample: &Sample) {
    let start = line.text.find(&sample.token).unwrap();
    let end = start + sample.token.len();
    let expected = Hsla::from(rgb(u32::from_str_radix(&sample.color, 16).unwrap()));
    let expected = format!("color: {expected:?},");
    // Debug exposes the decoration runs used by ShapedLine::paint.
    let shaped = format!("{line:?}");
    let (_, runs) = shaped.split_once("decoration_runs:").unwrap();
    let mut offset = 0;
    for run in runs.split("DecorationRun { len: ").skip(1) {
        let (len, style) = run.split_once(", ").unwrap();
        let next = offset + len.parse::<usize>().unwrap();
        if offset < end && start < next {
            assert!(
                style.starts_with(&expected),
                "{language} {mode}, line {} ({:?}): expected {expected} got {style}",
                sample.row + 1,
                sample.token,
            );
        }
        offset = next;
    }
    assert_eq!(offset, line.text.len());
}
