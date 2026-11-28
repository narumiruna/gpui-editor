use std::{collections::BTreeMap, ops::Range, sync::Arc};

use gpui::{
    App, Bounds, ContentMask, Context, Edges, PathBuilder, Pixels, Point, Rgba, ShapedLine,
    TextRun, Window, fill, point, px, quad, rgba, size,
};
use gpui_editor_codemirror::{
    EditorSelection, EditorState, Line, RangeSet, Text,
    language::fold::{FoldGutterConfig, FoldGutterMarker, FoldGutterState},
    lint::{LintGutterMarker, Severity, lint_gutter_markers},
    view::{
        gutters::{
            gutter_slots, highlight_active_line_gutter_enabled, line_numbers,
            standalone_line_numbers,
        },
        highlight_active_line_for_blocks,
    },
};

use super::{Layout, Row, SOURCE_PADDING, one_line};
use crate::{
    editor::{Editor, source_folds::StandaloneFoldMarker},
    theme::Theme,
};

#[cfg(test)]
mod tests;

const LABEL_SIZE: Pixels = px(12.);
const LABEL_PAD: Pixels = px(6.);
// The lint pin reserves 1.4 em for a 1 em icon.
const LINT_ICON_SIZE: Pixels = LABEL_SIZE;
const LINT_WIDTH: Pixels = px(16.8);

pub(super) struct StandaloneGutter {
    state: EditorState,
    width: Pixels,
    numbers_width: Option<Pixels>,
    folds: Option<StandaloneFoldSlot>,
    lint: Option<StandaloneLintSlot>,
    border: Option<(Pixels, Rgba)>,
    active: Vec<usize>,
}

struct StandaloneFoldSlot {
    width: Pixels,
    config: Arc<FoldGutterConfig>,
    markers: BTreeMap<usize, StandaloneFoldMarker>,
}

struct StandaloneLintSlot {
    markers: Arc<RangeSet<LintGutterMarker>>,
    viewport: Option<Bounds<Pixels>>,
    #[cfg(test)]
    painted: BTreeMap<usize, StandaloneLintPaint>,
}

#[cfg(test)]
struct StandaloneLintPaint {
    severity: Severity,
    icon: Bounds<Pixels>,
    hit: Bounds<Pixels>,
}

impl StandaloneGutter {
    pub(super) fn new(
        state: &EditorState,
        folds: Option<Arc<FoldGutterConfig>>,
        allow_activity: bool,
        theme: Theme,
        window: &Window,
    ) -> Option<Self> {
        let numbers_width = standalone_line_numbers(state, std::ops::RangeInclusive::new(1, 0))
            .ok()
            .flatten()
            .and_then(|slot| slot.spacer?.content)
            .map(|label| {
                shape_label(window, one_line(&label), gutter_foreground(theme)).width
                    + LABEL_PAD * 2.
            });
        let lint = lint_gutter_markers(state);
        if numbers_width.is_none() && folds.is_none() && lint.is_none() {
            return None;
        }
        let fold_width = folds.as_ref().map_or(px(0.), |config| {
            fold_slot_width(config, state, theme, window)
        });
        let lint_width = if lint.is_some() { LINT_WIDTH } else { px(0.) };
        let border = match theme.view {
            Some(style) => style
                .gutters
                .border
                .map(|stroke| (px(f32::from(stroke.width_px)), rgba(stroke.color.rgba32()))),
            None => Some((px(1.), theme.border)),
        };
        let mut active = Vec::new();
        if allow_activity && highlight_active_line_gutter_enabled(state) {
            active.extend(
                state
                    .selection()
                    .ranges()
                    .iter()
                    .filter_map(|range| Some(state.text().line_at(range.head())?.range.start)),
            );
            active.sort_unstable();
            active.dedup();
        }
        Some(Self {
            state: state.clone(),
            width: numbers_width.unwrap_or(px(0.)) + fold_width + lint_width,
            numbers_width,
            folds: folds.map(|config| StandaloneFoldSlot {
                width: fold_width,
                config,
                markers: BTreeMap::new(),
            }),
            lint: lint.map(|markers| StandaloneLintSlot {
                markers,
                viewport: None,
                #[cfg(test)]
                painted: BTreeMap::new(),
            }),
            border,
            active,
        })
    }

    pub(super) fn inset(&self) -> Pixels {
        self.width + self.border.map_or(px(0.), |(width, _)| width)
    }

    fn fold_bounds(&self, row: &Row) -> Bounds<Pixels> {
        let left = self.numbers_width.unwrap_or(px(0.));
        let width = self.folds.as_ref().map_or(px(0.), |slot| slot.width);
        Bounds::new(point(left, row.y), size(width, row.height))
    }

    fn lint_left(&self) -> Pixels {
        self.numbers_width.unwrap_or(px(0.)) + self.folds.as_ref().map_or(px(0.), |slot| slot.width)
    }

    fn lint_marker(&self, origin: Point<Pixels>, row: &Row) -> Option<(Severity, Bounds<Pixels>)> {
        let mark = self.lint.as_ref()?.markers.iter(row.start).next()?;
        // Match the actual standalone LF/CRLF row start. The core lint model
        // can split lone CR, but the standalone canvas does not.
        if mark.from != row.start || self.first_line_at(row).is_none() {
            return None;
        }
        let severity = mark.value.severity();
        if severity == Severity::Hint {
            // The pin gives hints no visible icon.
            return None;
        }
        let icon = Bounds::new(
            origin
                + point(
                    self.lint_left() + (LINT_WIDTH - LINT_ICON_SIZE) / 2.,
                    row.y + (row.height - LINT_ICON_SIZE) / 2.,
                ),
            size(LINT_ICON_SIZE, LINT_ICON_SIZE),
        );
        Some((severity, icon))
    }

    fn paint_lint_marker(&mut self, origin: Point<Pixels>, row: &Row, window: &mut Window) {
        let Some((severity, icon)) = self.lint_marker(origin, row) else {
            return;
        };
        let clip = window.content_mask().bounds;
        if clip.intersects(&icon) {
            paint_lint_icon(icon, severity, window);
            #[cfg(test)]
            self.lint.as_mut().unwrap().painted.insert(
                row.start,
                StandaloneLintPaint {
                    severity,
                    icon,
                    hit: clip_lint_icon(icon, clip),
                },
            );
        }
    }

    fn first_line_at(&self, row: &Row) -> Option<Line<'_>> {
        let line = self.state.text().line_at(row.start)?;
        (row.start == line.range.start).then_some(line)
    }

    fn labels(&self, lines: impl Iterator<Item = usize>) -> BTreeMap<usize, String> {
        let mut labels = BTreeMap::new();
        if self.numbers_width.is_none() {
            return labels;
        }
        let mut lines = lines.peekable();
        // Batch consecutive visible lines, but do not ask for folded-away labels.
        while let Some(first) = lines.next() {
            let mut last = first;
            while lines.next_if_eq(&(last + 1)).is_some() {
                last += 1;
            }
            if let Ok(Some(slot)) = standalone_line_numbers(&self.state, first..=last) {
                labels.extend(slot.rows.into_iter().filter_map(|row| {
                    let label = row.markers.into_iter().find_map(|marker| marker.content)?;
                    Some((row.from, label))
                }));
            }
        }
        labels
    }
}

fn clip_lint_icon(icon: Bounds<Pixels>, viewport: Bounds<Pixels>) -> Bounds<Pixels> {
    if icon.is_contained_within(&viewport) {
        icon
    } else {
        viewport.intersect(&icon)
    }
}

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
    pub(in crate::editor) fn set_standalone_lint_viewport(&mut self, viewport: Bounds<Pixels>) {
        if let Some(slot) = self
            .standalone_gutter
            .as_mut()
            .and_then(|g| g.lint.as_mut())
        {
            slot.viewport = Some(viewport);
        }
    }

    pub(in crate::editor) fn standalone_lint_viewport(&self) -> Option<Bounds<Pixels>> {
        self.standalone_gutter.as_ref()?.lint.as_ref()?.viewport
    }

    pub(in crate::editor) fn standalone_lint_lane_contains(&self, pointer: Point<Pixels>) -> bool {
        let Some(gutter) = &self.standalone_gutter else {
            return false;
        };
        let lint_left = gutter.lint_left();
        self.standalone_gutter_contains(pointer)
            && (lint_left..lint_left + LINT_WIDTH).contains(&(pointer.x - self.origin.x))
            && self
                .standalone_lint_viewport()
                .is_some_and(|viewport| viewport.contains(&pointer))
    }

    pub(in crate::editor) fn standalone_lint_hit(
        &self,
        state: &EditorState,
        pointer: Point<Pixels>,
    ) -> Option<(usize, Bounds<Pixels>)> {
        if !self.standalone_lint_lane_contains(pointer) {
            return None;
        }
        let gutter = self.standalone_gutter.as_ref()?;
        let slot = gutter.lint.as_ref()?;
        let viewport = slot.viewport?;
        let y = pointer.y - self.origin.y;
        let index = self.rows.partition_point(|row| row.y + row.height <= y);
        let row = self.rows.get(index)?;
        if index > 0 && self.rows[index - 1].soft_end {
            return None;
        }
        let (_, icon) = gutter.lint_marker(self.origin, row)?;
        if !viewport.intersects(&icon) || !icon.contains(&pointer) {
            return None;
        }
        let live_markers = lint_gutter_markers(state)?;
        Arc::ptr_eq(&slot.markers, &live_markers)
            .then(|| (row.start, clip_lint_icon(icon, viewport)))
    }

    pub(super) fn paint_standalone_gutter(&mut self, window: &mut Window, cx: &mut App) {
        let Some(gutter) = &mut self.standalone_gutter else {
            return;
        };
        #[cfg(test)]
        if let Some(slot) = &mut gutter.lint {
            slot.painted.clear();
        }
        let clip = window.content_mask().bounds;
        let top = clip.top() - self.origin.y;
        let bottom = clip.bottom() - self.origin.y;
        let first = self.rows.partition_point(|row| row.y + row.height <= top);
        let last = self.rows.partition_point(|row| row.y < bottom);
        if first >= last {
            return;
        }
        let first_block = self.rows[..first]
            .iter()
            .rposition(|row| !row.soft_end)
            .map_or(0, |previous| previous + 1);
        let mut labels = gutter.labels((first..last).filter_map(|index| {
            if index > 0 && self.rows[index - 1].soft_end {
                return None;
            }
            Some(gutter.first_line_at(&self.rows[index])?.number)
        }));
        let last_row = self.rows.last().unwrap();
        let height = last_row.y + last_row.height;
        let column = Bounds::new(self.origin, size(gutter.width, height));
        let base = self.theme.view.map_or(self.theme.preview, |style| {
            rgba(style.gutters.background.rgba32())
        });
        let active = self.theme.view.map_or(rgba(0xe2f2ffff), |style| {
            rgba(style.active_line_gutter.rgba32())
        });
        let quad = fill(column, base);
        #[cfg(test)]
        self.painted_active_lines.ordinary_gutter.push(quad.clone());
        window.paint_quad(quad);
        if let Some((width, color)) = gutter.border {
            let border = Bounds::new(
                self.origin + point(gutter.width, px(0.)),
                size(width, height),
            );
            let quad = fill(border, color);
            #[cfg(test)]
            {
                self.painted_active_lines.standalone_gutter_border = Some(quad.clone());
            }
            window.paint_quad(quad);
        }

        window.with_content_mask(Some(ContentMask { bounds: column }), |window| {
            let mut start = first_block;
            while start < last {
                let row = &self.rows[start];
                let tail = start
                    + self.rows[start..]
                        .iter()
                        .position(|row| !row.soft_end)
                        .unwrap();
                let end = &self.rows[tail];
                let origin = self.origin + point(px(0.), row.y);
                if gutter.active.binary_search(&row.start).is_ok() {
                    let quad = fill(
                        Bounds::new(origin, size(gutter.width, end.y + end.height - row.y)),
                        active,
                    );
                    #[cfg(test)]
                    self.painted_active_lines.gutter.push(quad.clone());
                    window.paint_quad(quad);
                }
                if let Some(label) = labels.remove(&row.start)
                    && let Some(width) = gutter.numbers_width
                {
                    let label =
                        shape_label(window, one_line(&label), gutter_foreground(self.theme));
                    let origin = origin + point(width - LABEL_PAD - label.width, px(0.));
                    let _ = label.paint(origin, row.height, window, cx);
                    #[cfg(test)]
                    self.painted_active_lines.gutter_labels.push(label);
                }
                if start >= first
                    && let Some(marker) = gutter
                        .folds
                        .as_ref()
                        .and_then(|slot| slot.markers.get(&row.start))
                {
                    let label = paint_fold_label(
                        self.origin,
                        gutter.fold_bounds(row),
                        &marker.text,
                        self.theme,
                        window,
                        cx,
                    );
                    #[cfg(test)]
                    self.painted_active_lines.standalone_fold_labels.push(label);
                    #[cfg(not(test))]
                    let _ = label;
                }
                // `start` is the first row of its displayed block. A wrap or
                // a folded suffix cannot contribute a separate lint marker.
                if start >= first {
                    gutter.paint_lint_marker(self.origin, row, window);
                }
                start = tail + 1;
            }
        });
    }

    fn standalone_gutter_contains(&self, position: Point<Pixels>) -> bool {
        let Some(gutter) = &self.standalone_gutter else {
            return false;
        };
        let last = self.rows.last().unwrap();
        let local = position - self.origin;
        (px(0.)..gutter.inset()).contains(&local.x)
            && (px(0.)..last.y + last.height).contains(&local.y)
    }

    pub(in crate::editor) fn standalone_fold_hit_is_current(&self, state: &EditorState) -> bool {
        self.standalone_gutter.as_ref().is_some_and(|gutter| {
            gutter.state.text() == state.text()
                && gutter
                    .folds
                    .as_ref()
                    .zip(FoldGutterConfig::configured(state))
                    .is_some_and(|(painted, current)| Arc::ptr_eq(&painted.config, &current))
        })
    }

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
        if self.standalone_gutter.is_some() {
            self.prepare_standalone_fold_gutter(editor, viewport);
            return;
        }
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

    fn prepare_standalone_fold_gutter(&mut self, editor: &Editor, viewport: Bounds<Pixels>) {
        let gutter = self.standalone_gutter.as_mut().unwrap();
        let Some(slot) = &mut gutter.folds else {
            return;
        };
        slot.markers.clear();
        if !editor.standalone_fold_input_enabled() {
            return;
        }
        let Some(folds) = editor.standalone_folds() else {
            return;
        };
        let ranges = folds.ranges();
        let query = folds.query(&ranges);
        let first = self
            .rows
            .partition_point(|row| self.origin.y + row.y + row.height <= viewport.top());
        let last = self
            .rows
            .partition_point(|row| self.origin.y + row.y < viewport.bottom());
        let markers = (first..last)
            .filter_map(|index| {
                let row = &self.rows[index];
                if index > 0 && self.rows[index - 1].soft_end {
                    return None;
                }
                let line = gutter.first_line_at(row)?;
                Some((
                    row.start,
                    query.standalone_marker(&folds.config, line.range)?,
                ))
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
        // This hook runs before the editor moves the selection or takes focus.
        if layout.standalone_gutter_contains(position) {
            let gutter = layout.standalone_gutter.as_ref().unwrap();
            // Consume the passive lane, including its shared edge with folds.
            if gutter.lint.is_some() && position.x >= layout.origin.x + gutter.lint_left() {
                return true;
            }
            let row = layout.row_at(position);
            if layout.standalone_fold_hit_is_current(self.view.state())
                && self.scroll.bounds().contains(&position)
                && gutter
                    .fold_bounds(row)
                    .contains(&(position - layout.origin))
                && position.y < layout.origin.y + row.y + row.height
                && let Some(slot) = &gutter.folds
                && let Some(marker) = slot.markers.get(&row.start)
                && let Some(line) = gutter.first_line_at(row)
            {
                let config = slot.config.clone();
                let marker = marker.clone();
                self.click_standalone_fold_gutter(line.range, &config, &marker, cx);
            }
            return true;
        }
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
        let fold_width = folds.as_ref().map_or(px(0.), |config| {
            fold_slot_width(config, editor, theme, window)
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
            paint_fold_label(origin, self.fold_bounds(row), label, theme, window, cx);
        }
    }
}

fn paint_lint_icon(icon: Bounds<Pixels>, severity: Severity, window: &mut Window) {
    // Native geometry for the three visible icons in @codemirror/lint 6.9.8.
    let (background, border, radius) = match severity {
        Severity::Error => (0xff8877ff, 0xff4433ff, LINT_ICON_SIZE / 2.),
        Severity::Info => (0xaaaaffff, 0x7777eeff, px(1.)),
        Severity::Warning => {
            let at = |x, y| icon.origin + point(px(x), px(y));
            for (points, color) in [
                ([at(6., 0.5), at(12., 11.5), at(0., 11.5)], 0xffdd77ff),
                ([at(6., 2.5), at(10.5, 10.5), at(1.5, 10.5)], 0xffee88ff),
            ] {
                let mut path = PathBuilder::fill();
                path.add_polygon(&points, true);
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgba(color));
                }
            }
            return;
        }
        Severity::Hint => return,
    };
    let pad = px(0.6);
    let side = LINT_ICON_SIZE - 2. * pad;
    window.paint_quad(quad(
        Bounds::new(icon.origin + point(pad, pad), size(side, side)),
        radius,
        rgba(background),
        Edges::all(px(1.8)),
        rgba(border),
        Default::default(),
    ));
}

fn fold_slot_width(
    config: &FoldGutterConfig,
    state: &EditorState,
    theme: Theme,
    window: &Window,
) -> Pixels {
    // Measure both states so opening and closing do not change wrapping.
    [
        config.spacer(state),
        config.render(state, FoldGutterState::Open),
    ]
    .into_iter()
    .filter_map(|marker| marker.marker.content)
    .map(|text| {
        let label = shape_label(window, one_line(&text), gutter_foreground(theme));
        label.width.max(px(8.) * label.text.chars().count() as f32)
    })
    .fold(px(0.), Pixels::max)
        + LABEL_PAD * 2.
}

fn paint_fold_label(
    origin: Point<Pixels>,
    bounds: Bounds<Pixels>,
    text: &str,
    theme: Theme,
    window: &mut Window,
    cx: &mut App,
) -> ShapedLine {
    let label = shape_label(window, one_line(text), gutter_foreground(theme));
    let top = origin + bounds.origin + point((bounds.size.width - label.width) / 2., px(0.));
    let _ = label.paint(top, bounds.size.height, window, cx);
    label
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
