use gpui::{
    App, Bounds, FontWeight, Pixels, Point, ShapedLine, TextRun, Window, fill, point, px, quad,
    size,
};

use crate::{
    document::{Alignment, BlockPreview},
    theme::Theme,
};

use super::CODE_FONT_FAMILY;

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
    pub(super) fn new(preview: &BlockPreview, window: &Window) -> Self {
        match preview {
            BlockPreview::Code { body } => Self::code(body, window),
            BlockPreview::Table { alignments, rows } => Self::table(alignments, rows, window),
        }
    }

    fn code(body: &str, window: &Window) -> Self {
        let style = window.text_style();
        let mut font = style.font();
        font.family = CODE_FONT_FAMILY.into();
        let mut width = px(120.);
        let lines: Vec<_> = body
            .lines()
            .enumerate()
            .map(|(row, line)| {
                let line = shape(line.replace('\t', "    "), &font, px(14.), window);
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

    fn table(alignments: &[Alignment], rows: &[Vec<String>], window: &Window) -> Self {
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

fn shape(text: String, font: &gpui::Font, size: Pixels, window: &Window) -> ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: font.clone(),
        color: Theme::for_window(window).text.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text.into(), size, &[run], None)
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::editor::Editor;

    #[gpui::test]
    fn lays_out_code_indentation_and_table_alignment(cx: &mut TestAppContext) {
        let (_editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.update(|window, _| {
            let code = BlockLayout::code("first\n\tsecond\n\nlast\n", window);
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
