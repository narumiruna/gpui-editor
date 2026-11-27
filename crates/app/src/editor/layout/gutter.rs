use std::{collections::BTreeMap, ops::Range, sync::Arc};

use gpui::{
    App, Bounds, Context, Pixels, Point, Rgba, ShapedLine, TextRun, Window, point, px, rgba, size,
};
use gpui_editor_codemirror::{
    EditorSelection, EditorState, Text,
    language::fold::{FoldGutterConfig, FoldGutterMarker, FoldGutterState},
    view::{
        gutters::{gutter_slots, line_numbers},
        highlight_active_line_for_blocks,
    },
};

use super::{Layout, Row, SOURCE_PADDING, one_line};
use crate::{editor::Editor, theme::Theme};

#[cfg(test)]
mod tests;

const LABEL_SIZE: Pixels = px(12.);
const LABEL_PAD: Pixels = px(6.);

pub(super) struct SourceGutter {
    pub(super) body: Range<usize>,
    pub(super) width: Pixels,
    state: EditorState,
    numbers_width: Pixels,
    folds: Option<FoldSlot>,
}

struct FoldSlot {
    config: Arc<FoldGutterConfig>,
    markers: BTreeMap<usize, FoldGutterMarker>,
}

impl Layout {
    pub(in crate::editor) fn active_source_lines(
        &self,
        text: &Text,
        selection: &EditorSelection,
    ) -> Vec<Range<usize>> {
        let Some(gutter) = self
            .source_frame
            .as_ref()
            .and_then(|frame| frame.gutter.as_ref())
        else {
            return Vec::new();
        };
        let marks = highlight_active_line_for_blocks(text, selection, |head| {
            let line = gutter
                .source_line_at(head)
                .or_else(|| text.line_at(head).map(|line| line.range))?;
            Some(line.start..line.end.max(head))
        });
        marks
            .ok()
            .map(|marks| {
                marks
                    .iter(gutter.body.start)
                    .take_while(|mark| mark.from <= gutter.body.end)
                    .filter_map(|mark| gutter.source_line_at(mark.from))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn active_gutter_blocks(&self, selection: &EditorSelection) -> Vec<Range<usize>> {
        let Some(gutter) = self
            .source_frame
            .as_ref()
            .and_then(|frame| frame.gutter.as_ref())
        else {
            return Vec::new();
        };
        let blocks = selection.ranges().iter().filter_map(|range| {
            let start = gutter.source_line_at(range.head())?.start;
            let block = self.line_block(start)?;
            // A folded suffix can share a content block with a preceding line,
            // but its own logical start has no projected gutter block.
            (block.start == start && block.end <= gutter.body.end).then_some(block)
        });
        super::distinct_line_blocks(blocks)
    }

    pub(in crate::editor) fn prepare_fold_gutter(
        &mut self,
        editor: &mut Editor,
        viewport: Bounds<Pixels>,
    ) {
        let Some(gutter) = self
            .source_frame
            .as_mut()
            .and_then(|frame| frame.gutter.as_mut())
        else {
            return;
        };
        if gutter.folds.is_none() {
            return;
        }
        let Some(query) = editor.source_fold_query(&gutter.body) else {
            return;
        };
        let first = self
            .rows
            .partition_point(|row| self.origin.y + row.y + row.height <= viewport.top());
        let config = &gutter.folds.as_ref().unwrap().config;
        let markers = self.rows[first..]
            .iter()
            .take_while(|row| self.origin.y + row.y < viewport.bottom())
            .filter_map(|row| {
                let line = gutter.first_line_at(row)?;
                Some((row.start, query.marker(config, line)?))
            })
            .collect();
        gutter.folds.as_mut().unwrap().markers = markers;
    }
}

impl Editor {
    pub(in crate::editor) fn mouse_down_fold_gutter(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(layout) = &self.layout else {
            return false;
        };
        let Some(gutter) = layout
            .source_frame
            .as_ref()
            .and_then(|frame| frame.gutter.as_ref())
        else {
            return false;
        };
        let Some(slot) = &gutter.folds else {
            return false;
        };
        let row = layout.row_at(position);
        let point = position - layout.origin;
        if !gutter.fold_bounds(row).contains(&point)
            || point.y >= row.y + row.height
            || point.x >= SOURCE_PADDING + gutter.width
        {
            return false;
        }
        let Some(line) = gutter.first_line_at(row) else {
            return false;
        };
        let body = gutter.body.clone();
        let config = slot.config.clone();
        self.click_source_fold_gutter(&body, line, &config, cx);
        true
    }
}

impl SourceGutter {
    pub(super) fn new(
        editor: &EditorState,
        body: Range<usize>,
        lines: Option<&Text>,
        theme: Theme,
        window: &Window,
    ) -> Option<Self> {
        let state = EditorState::create_from_text(
            lines
                .cloned()
                .or_else(|| editor.text().slice_owned(body.clone()))?,
            None,
            line_numbers(Default::default()),
        )
        .ok()?;
        // Query one line to obtain the spacer without projecting the entire fence.
        let sample = gutter_slots(&state, 1..=1).ok()?.pop()?.spacer?.content?;
        let label = shape_label(window, sample, gutter_foreground(theme));
        let numbers_width = label.width.max(px(8.) * label.text.len() as f32) + LABEL_PAD * 2.;
        let folds = FoldGutterConfig::configured(editor);
        // Measure both states so a wider custom label does not move the text
        // or change wrapping when a fold opens or closes.
        let fold_width = folds.as_ref().map_or(px(0.), |config| {
            let width = [
                config.spacer(editor),
                config.render(editor, FoldGutterState::Open),
            ]
            .into_iter()
            .filter_map(|marker| marker.marker.content)
            .map(|label| {
                let label = shape_label(window, one_line(&label), gutter_foreground(theme));
                label.width.max(px(8.) * label.text.chars().count() as f32)
            })
            .fold(px(0.), Pixels::max);
            width + LABEL_PAD * 2.
        });
        Some(Self {
            body,
            width: numbers_width + fold_width,
            numbers_width,
            state,
            folds: folds.map(|config| FoldSlot {
                config,
                markers: BTreeMap::new(),
            }),
        })
    }

    fn first_line_at(&self, row: &Row) -> Option<Range<usize>> {
        let (number, first) = self.line_at_row(row)?;
        let end = self.state.text().line(number)?.range.end;
        first.then_some(row.start..self.body.start + end)
    }

    fn fold_bounds(&self, row: &Row) -> Bounds<Pixels> {
        Bounds::new(
            point(SOURCE_PADDING + self.numbers_width, row.y),
            size(self.width - self.numbers_width, row.height),
        )
    }

    fn source_line_at(&self, head: usize) -> Option<Range<usize>> {
        let line = self
            .state
            .text()
            .line_at(head.checked_sub(self.body.start)?)?;
        Some(self.body.start + line.range.start..self.body.start + line.range.end)
    }

    pub(super) fn line_at_row(&self, row: &Row) -> Option<(usize, bool)> {
        let from = row.start.checked_sub(self.body.start)?;
        let line = self.state.text().line_at(from)?;
        Some((line.number, from == line.range.start))
    }

    pub(super) fn label(&self, line: usize) -> Option<String> {
        gutter_slots(&self.state, line..=line)
            .ok()?
            .pop()?
            .rows
            .pop()?
            .markers
            .into_iter()
            .find_map(|marker| marker.content)
    }

    pub(super) fn row_bounds(&self, origin: Point<Pixels>, row: &Row) -> Option<Bounds<Pixels>> {
        // The layout already places row starts on source byte boundaries.
        (self.body.start..=self.body.end)
            .contains(&row.start)
            .then(|| {
                Bounds::new(
                    origin + point(SOURCE_PADDING, row.y),
                    size(self.width, row.height),
                )
            })
    }

    pub(super) fn paint_row(
        &self,
        origin: Point<Pixels>,
        row: &Row,
        theme: Theme,
        window: &mut Window,
        cx: &mut App,
        #[cfg(test)] painted_labels: &mut Vec<ShapedLine>,
    ) {
        let Some((number, first_row)) = self.line_at_row(row) else {
            return;
        };
        let top = origin + point(SOURCE_PADDING, row.y);
        if first_row && let Some(label) = self.label(number) {
            let label = shape_label(window, label, gutter_foreground(theme));
            let _ = label.paint(
                top + point(self.numbers_width - LABEL_PAD - label.width, px(0.)),
                row.height,
                window,
                cx,
            );
            #[cfg(test)]
            painted_labels.push(label);
        }
        if let Some(label) = self
            .folds
            .as_ref()
            .and_then(|slot| slot.markers.get(&row.start))
            .and_then(|marker| marker.marker.content.as_deref())
            .filter(|_| first_row)
        {
            let label = shape_label(window, one_line(label), gutter_foreground(theme));
            let bounds = self.fold_bounds(row);
            let top =
                origin + bounds.origin + point((bounds.size.width - label.width) / 2., px(0.));
            let _ = label.paint(top, row.height, window, cx);
        }
    }
}

fn gutter_foreground(theme: Theme) -> Rgba {
    theme
        .view
        .map_or(theme.muted, |style| rgba(style.gutters.foreground.rgba32()))
}

fn shape_label(window: &Window, label: String, color: Rgba) -> ShapedLine {
    let len = label.len();
    window.text_system().shape_line(
        label.into(),
        LABEL_SIZE,
        &[TextRun {
            len,
            font: window.text_style().font(),
            color: color.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    )
}
