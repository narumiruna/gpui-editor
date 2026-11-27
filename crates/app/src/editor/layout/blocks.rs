use gpui::{
    App, Bounds, Font, FontWeight, Pixels, Point, Rgba, ShapedLine, TextRun, Window, fill, point,
    px, quad, size,
};

use std::ops::Range;

use gpui_editor_codemirror::{
    EditorState, Text,
    language::{
        Color, HighlightLayer, HighlightProperties, HighlightStyle, HighlightTag, StreamSettings,
        highlighting_for_context,
    },
    theme_one_dark::{highlight_configuration, highlight_style},
};

use crate::{
    document::{Alignment, BlockPreview},
    editor::source_fence::fence_parser_in,
    theme::Theme,
};

use super::{apply_highlight, code_highlights};

const PADDING: f32 = 12.;
const MARGIN: f32 = 4.;
const CODE_LINE_HEIGHT: f32 = 22.;
const CELL_HEIGHT: f32 = 30.;

pub(in crate::editor) struct BlockLayout {
    pub(super) width: Pixels,
    pub(super) height: Pixels,
    line_height: Pixels,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    grid: Option<Grid>,
}

struct Grid {
    columns: Vec<Pixels>,
    rows: usize,
}

impl BlockLayout {
    pub(super) fn new(
        preview: &BlockPreview,
        state: &EditorState,
        theme: Theme,
        window: &Window,
    ) -> Self {
        match preview {
            BlockPreview::Code { body, language } => {
                Self::code(body, language.as_deref(), state, theme, window)
            }
            BlockPreview::Table { alignments, rows } => {
                Self::table(alignments, rows, theme, window)
            }
        }
    }

    fn code(
        body: &str,
        language: Option<&str>,
        state: &EditorState,
        theme: Theme,
        window: &Window,
    ) -> Self {
        let style = window.text_style();
        let mut font = style.font();
        font.family = "Menlo".into();
        let document = fence_parser_in(state, language.unwrap_or("")).and_then(|parser| {
            parser
                .parse(Text::new(body), StreamSettings::default())
                .ok()
        });
        let system_dark = Theme::is_system_dark(window);
        // Keep the old preview palette only on its normal background. A view-only
        // extension can switch a light preview to dark without installing syntax.
        let use_legacy = system_dark == theme.view.is_some_and(|style| style.dark);
        let (layers, appearance) = highlight_configuration(state);
        let mut layers_with_legacy = use_legacy.then(|| {
            let mut layers = layers.to_vec();
            // The app palette participates only after every installed fallback.
            layers.push(HighlightLayer::Fallback(HighlightStyle::define(&[])));
            layers
        });
        let highlights = document
            .as_ref()
            .and_then(|doc| {
                // Keep the preview's existing spans, without expanded Markdown mounts.
                let spans = doc.syntax().highlights(0..body.len())?;
                Some(code_highlights(doc, spans, |tags, scope| {
                    let layers = if let Some(layers) = &mut layers_with_legacy {
                        let properties = legacy_preview_highlight(tags, system_dark);
                        *layers.last_mut().unwrap() = HighlightLayer::Fallback(
                            HighlightStyle::define(&[]).with_all(properties),
                        );
                        layers.as_slice()
                    } else {
                        layers.as_slice()
                    };
                    highlighting_for_context(layers, tags, scope, appearance)
                }))
            })
            .unwrap_or_default();
        let mut width = px(120.);
        let mut offset = 0;
        let lines: Vec<_> = body
            .split_inclusive('\n')
            .enumerate()
            .map(|(row, raw)| {
                let text = raw
                    .strip_suffix('\n')
                    .map_or(raw, |line| line.strip_suffix('\r').unwrap_or(line));
                let range = offset..offset + text.len();
                offset += raw.len();
                let (text, runs) = highlighted_runs(body, range, &highlights, &font, theme.text);
                #[cfg(test)]
                crate::editor::one_dark_tests::record_fonts(true, &text, &runs);
                let line = window
                    .text_system()
                    .shape_line(text.into(), px(14.), &runs, None);
                width = width.max(line.width);
                (
                    point(px(PADDING), px(2. * MARGIN + row as f32 * CODE_LINE_HEIGHT)),
                    line,
                )
            })
            .collect();
        Self {
            width: width + px(2. * PADDING),
            height: px(lines.len().max(1) as f32 * CODE_LINE_HEIGHT + 4. * MARGIN),
            line_height: px(CODE_LINE_HEIGHT),
            lines,
            grid: None,
        }
    }

    fn table(
        alignments: &[Alignment],
        rows: &[Vec<String>],
        theme: Theme,
        window: &Window,
    ) -> Self {
        let font = window.text_style().font();
        let mut header = font.clone();
        header.weight = FontWeight::BOLD;
        let mut widths = vec![px(60.); alignments.len()];
        let shaped: Vec<_> = rows
            .iter()
            .enumerate()
            .flat_map(|(row, cells)| {
                cells
                    .iter()
                    .enumerate()
                    .filter(|(_, cell)| !cell.is_empty())
                    .map(move |(col, cell)| (row, col, cell))
            })
            .map(|(row, col, cell)| {
                let line = shape(
                    cell.replace(['\n', '\r', '\t'], " "),
                    if row == 0 { &header } else { &font },
                    px(15.),
                    theme.text,
                    window,
                );
                widths[col] = widths[col].max(line.width + px(2. * PADDING));
                (row, col, line)
            })
            .collect();
        let mut columns = vec![px(0.)];
        for width in &widths {
            columns.push(columns.last().copied().unwrap() + *width);
        }
        let lines = shaped
            .into_iter()
            .map(|(row, col, line)| {
                let room = widths[col] - px(2. * PADDING) - line.width;
                let align = match alignments[col] {
                    Alignment::Center => room / 2.,
                    Alignment::Right => room,
                    _ => px(0.),
                };
                (
                    point(
                        columns[col] + px(PADDING) + align,
                        px(MARGIN + row as f32 * CELL_HEIGHT),
                    ),
                    line,
                )
            })
            .collect();
        Self {
            width: *columns.last().unwrap(),
            height: px(rows.len() as f32 * CELL_HEIGHT + 2. * MARGIN),
            line_height: px(CELL_HEIGHT),
            lines,
            grid: Some(Grid {
                columns,
                rows: rows.len(),
            }),
        }
    }

    pub(in crate::editor) fn paint_frame(
        &self,
        origin: Point<Pixels>,
        theme: Theme,
        window: &mut Window,
    ) {
        let top = origin + point(px(0.), px(MARGIN));
        let height = self.height - px(2. * MARGIN);
        let bounds = Bounds::new(top, size(self.width, height));
        let clip = window.content_mask().bounds;
        if !clip.intersects(&bounds) {
            return;
        }
        let border = theme.border;
        window.paint_quad(quad(
            bounds,
            px(if self.grid.is_some() { 0. } else { 6. }),
            theme.preview,
            gpui::Edges::all(px(1.)),
            border,
            Default::default(),
        ));
        if let Some(grid) = &self.grid {
            window.paint_quad(fill(
                Bounds::new(
                    top + point(px(1.), px(1.)),
                    size(self.width - px(2.), px(CELL_HEIGHT - 1.)),
                ),
                theme.preview_header,
            ));
            for &x in &grid.columns[1..grid.columns.len() - 1] {
                window.paint_quad(fill(
                    Bounds::new(top + point(x, px(0.)), size(px(1.), height)),
                    border,
                ));
            }
            for row in 1..grid.rows {
                let rule = Bounds::new(
                    top + point(px(0.), px(row as f32 * CELL_HEIGHT)),
                    size(self.width, px(1.)),
                );
                if clip.intersects(&rule) {
                    window.paint_quad(fill(rule, border));
                }
            }
        }
    }

    #[cfg(test)]
    pub(in crate::editor) fn code_lines(&self) -> impl Iterator<Item = &ShapedLine> {
        self.lines.iter().map(|(_, line)| line)
    }

    pub(in crate::editor) fn paint_text(
        &self,
        origin: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let clip = window.content_mask().bounds;
        for (offset, line) in &self.lines {
            let top = origin + *offset;
            if clip.intersects(&Bounds::new(top, size(line.width, self.line_height))) {
                let _ = line.paint(top, self.line_height, window, cx);
            }
        }
    }
}

fn highlighted_runs(
    source: &str,
    range: Range<usize>,
    highlights: &[(Range<usize>, HighlightProperties)],
    font: &Font,
    foreground: Rgba,
) -> (String, Vec<TextRun>) {
    let mut text = String::new();
    let mut runs = Vec::new();
    let mut push = |content: &str, style| {
        if content.is_empty() {
            return;
        }
        let before = text.len();
        text.push_str(&content.replace('\t', "    "));
        let mut run = TextRun {
            len: text.len() - before,
            font: font.clone(),
            color: foreground.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        apply_highlight(&mut run, style);
        runs.push(run);
    };
    let first = highlights.partition_point(|(span, _)| span.end <= range.start);
    let mut start = range.start;
    for (span, style) in &highlights[first..] {
        if span.start >= range.end {
            break;
        }
        let from = start.max(span.start);
        let to = range.end.min(span.end);
        if from < to {
            push(&source[start..from], HighlightProperties::default());
            push(&source[from..to], *style);
            start = to;
        }
    }
    push(&source[start..range.end], HighlightProperties::default());
    (text, runs)
}

// This is the app's existing preview palette, not a layer in the native state.
fn legacy_preview_highlight(tags: &[HighlightTag], dark: bool) -> HighlightProperties {
    if dark {
        return highlight_style(tags);
    }
    let foreground = match tags.first().map(HighlightTag::name) {
        Some("string" | "inserted") => 0x14683f,
        Some("propertyName" | "attributeName") => 0x92400e,
        Some("tagName" | "typeName") => 0x0f6674,
        Some("comment" | "lineComment" | "blockComment") => 0x55616a,
        Some("number") => 0x673ab7,
        Some("bool" | "null" | "keyword" | "definitionKeyword" | "modifier") => 0x1040a2,
        Some("deleted") => 0x9e2525,
        _ => return HighlightProperties::default(),
    };
    HighlightProperties {
        foreground: Some(Color::from_rgba32(foreground << 8 | 0xff)),
        ..Default::default()
    }
}

fn shape(text: String, font: &Font, size: Pixels, foreground: Rgba, window: &Window) -> ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: font.clone(),
        color: foreground.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text.into(), size, &[run], None)
}

#[cfg(test)]
mod preview_highlight_tests;

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, rgb, rgba};

    use super::*;
    use crate::editor::{Editor, source_fence::fence_language};

    #[gpui::test]
    fn json_preview_applies_one_dark_tags_without_changing_text_or_tab_expansion(
        cx: &mut TestAppContext,
    ) {
        let (_, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.update(|window, _| {
            let body = "{\r\n\t\"字\": \"🐱\"\r\n}";
            let language = fence_language("json").unwrap();
            let json = language.support().parse(Text::new(body)).unwrap();
            let line = json.text().line(2).unwrap();
            let spans = json.syntax().highlights(0..body.len()).unwrap();
            let highlights = code_highlights(&json, spans, |tags, _| highlight_style(tags));
            let foreground = rgb(0xabb2bf);
            let (displayed, runs) = highlighted_runs(
                body,
                line.range,
                &highlights,
                &window.text_style().font(),
                foreground,
            );
            assert_eq!(displayed, "    \"字\": \"🐱\"");
            let mut offset = 0;
            let colors: Vec<_> = runs
                .iter()
                .map(|run| {
                    let range = offset..offset + run.len;
                    offset += run.len;
                    (&displayed[range], run.color)
                })
                .collect();
            let palette = gpui_editor_codemirror::theme_one_dark::COLOR;
            assert!(colors.contains(&("\"字\"", rgba(palette.coral.rgba32()).into())));
            assert!(colors.contains(&("\"🐱\"", rgba(palette.sage.rgba32()).into())));
            assert_eq!(json.text().as_str(), body);
            assert_eq!(offset, displayed.len());
        });
    }

    #[gpui::test]
    fn code_previews_apply_native_language_colors(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.update(|window, _| {
            let palette = gpui_editor_codemirror::theme_one_dark::COLOR;
            let font = window.text_style().font();
            for (name, body, token, color) in [
                ("yml", "---\r\nname: \"字\"\r\n", "\"字\"", palette.sage),
                (
                    "xml",
                    "<?xml version=\"1.0\"?>\r\n<節 id=\"x\">🐱</節>\r\n",
                    "節",
                    palette.chalky,
                ),
                (
                    "wat",
                    "(module\r\n (func (result i32) (i32.const 7)))\r\n",
                    "i32",
                    palette.chalky,
                ),
                (
                    "lezer",
                    "@top Main {\r\n  \"字\" }\r\n",
                    "\"字\"",
                    palette.sage,
                ),
                ("diff", "-前\r\n+後\r\n", "+後", palette.sage),
                (
                    "css",
                    ".card {\r\n  color: crimson; }\r\n",
                    "color",
                    palette.coral,
                ),
                (
                    "go",
                    "package main\r\nvar s = \"字\"\r\n",
                    "\"字\"",
                    palette.sage,
                ),
                (
                    "html",
                    "<style>\r\na { color: red; }\r\n",
                    "color",
                    palette.coral,
                ),
                (
                    "js",
                    "const name =\r\n  \"字\";\r\n",
                    "\"字\"",
                    palette.sage,
                ),
                ("sql", "SELECT\r\n  '字';\r\n", "'字'", palette.sage),
                (
                    "angular",
                    "<p>\r\n  {{ '字' }}</p>\r\n",
                    "'字'",
                    palette.sage,
                ),
                (
                    "vue",
                    "<template>\r\n  {{ '字' }}</template>\r\n",
                    "'字'",
                    palette.sage,
                ),
                (
                    "python",
                    "def f():\r\n    return \"字\"\r\n",
                    "\"字\"",
                    palette.sage,
                ),
                (
                    "rust",
                    "fn f() {\r\n  let name = \"字\"; }\r\n",
                    "\"字\"",
                    palette.sage,
                ),
            ] {
                let doc = fence_language(name)
                    .unwrap()
                    .support()
                    .parse(Text::new(body))
                    .unwrap();
                let spans = doc.syntax().highlights(0..body.len()).unwrap();
                let highlights = code_highlights(&doc, spans, |tags, _| highlight_style(tags));
                let line = doc.text().line(2).unwrap();
                let (displayed, runs) =
                    highlighted_runs(body, line.range.clone(), &highlights, &font, rgb(0xabb2bf));
                assert_eq!(displayed, body[line.range]);
                let mut offset = 0;
                assert!(runs.iter().any(|run| {
                    let range = offset..offset + run.len;
                    offset += run.len;
                    &displayed[range] == token && run.color == rgba(color.rgba32()).into()
                }));
                let state = EditorState::new("");
                let code = BlockLayout::code(
                    body,
                    Some(name),
                    &state,
                    Theme::for_editor(window, &state),
                    window,
                );
                assert_eq!(code.lines.len(), 2);
                assert_eq!(code.lines[1].1.text.as_ref(), displayed);
            }
        });
    }

    #[gpui::test]
    fn lays_out_code_indentation_and_table_alignment(cx: &mut TestAppContext) {
        let (_editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.update(|window, _| {
            let state = EditorState::new("");
            let theme = Theme::for_editor(window, &state);
            let code = BlockLayout::code("first\n\tsecond\n\nlast\n", None, &state, theme, window);
            assert_eq!(code.lines.len(), 4);
            assert_eq!(code.lines[1].1.text.as_ref(), "    second");
            assert!(code.lines[2].1.text.is_empty());
            assert_eq!(code.lines[3].0.y - code.lines[2].0.y, px(CODE_LINE_HEIGHT));
            let table = BlockLayout::table(
                &[Alignment::Left, Alignment::Center, Alignment::Right],
                &[
                    vec!["A".into(), "B".into(), "C".into()],
                    vec!["x".into(), "x".into(), "x".into()],
                ],
                theme,
                window,
            );
            let grid = table.grid.unwrap();
            let body: Vec<_> = table.lines[3..]
                .iter()
                .zip(&grid.columns)
                .map(|((origin, line), &left)| (origin.x - left, line.width))
                .collect();
            assert_eq!(grid.rows, 2);
            assert_eq!(body[0].0, px(PADDING));
            assert_eq!(
                body[1].0 + body[1].1 / 2.,
                (grid.columns[2] - grid.columns[1]) / 2.
            );
            assert_eq!(
                body[2].0 + body[2].1 + px(PADDING),
                grid.columns[3] - grid.columns[2]
            );
        });
    }
}
