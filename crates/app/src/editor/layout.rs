#[cfg(target_os = "macos")]
use std::rc::Rc;
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::Arc,
};

use gpui::{
    App, Bounds, ContentMask, Context, Edges, FontStyle, FontWeight, PaintQuad, Pixels, Point,
    RenderImage, ShapedLine, SharedString, StrikethroughStyle, TextRun, UnderlineStyle, Window,
    fill, point, px, quad, rgba, size,
};
#[cfg(target_os = "macos")]
use gpui_editor_codemirror::view::bidi::BidiIsolate;
use gpui_editor_codemirror::{
    Assoc, EditorSelection, EditorState, SelectionRange, Text,
    language::{
        Color, HighlightContext, HighlightProperties, HighlightSpan, HighlightTag, StreamSettings,
        TextDecoration, highlighting_for_context,
    },
    language_data::SupportedDocument,
    theme_one_dark::highlight_configuration,
    view::{
        ScrollBlock, ViewHighlight,
        bidi::{Direction, move_visually},
        gutters::highlight_active_line_gutter_enabled,
        placeholder_projection,
    },
};
use unicode_segmentation::UnicodeSegmentation;

use super::{Editor, ImageEntry, PreviewDiagnostic, highlight};
use crate::{
    document::{Section, SectionKind},
    media::{Target, image_target, load_image},
    rendering::{Raster, render_math, render_mermaid},
    theme::Theme,
    workspace::Workspace,
};

#[cfg(test)]
mod active_line_tests;
mod bidi;
mod blocks;
mod gutter;
#[cfg(target_os = "macos")]
mod isolation;
#[cfg(test)]
mod source_frame_tests;
#[cfg(target_os = "macos")]
mod textkit;

use bidi::BidiRow;
use blocks::BlockLayout;
use gutter::{SourceGutter, StandaloneGutter};
#[cfg(target_os = "macos")]
use isolation::{BodyCache, CachedBody, IsolatedRow};
#[cfg(not(target_os = "macos"))]
type BodyCache = std::convert::Infallible;

pub(super) const CODE_FONT_FAMILY: &str = if cfg!(target_os = "windows") {
    "Consolas"
} else {
    "Menlo"
};

#[derive(Clone, Copy, Default)]
struct Face {
    heading: bool,
    bold: bool,
    italic: bool,
    muted: bool,
    code: bool,
}

#[derive(Clone)]
pub(super) struct Graphic {
    pub(super) width: Pixels,
    pub(super) height: Pixels,
    pub(super) image: Arc<RenderImage>,
    bytes: usize,
}

impl Graphic {
    fn new(raster: Raster, scale: f32) -> Option<Self> {
        let bytes = raster.bgra.len();
        let pixels = image::RgbaImage::from_raw(raster.width, raster.height, raster.bgra)?;
        let image = RenderImage::new(vec![image::Frame::new(pixels)]);
        Some(Self {
            width: px(raster.width as f32 / scale),
            height: px(raster.height as f32 / scale),
            image: Arc::new(image),
            bytes,
        })
    }
}

pub(super) const DEFAULT_LINE_HEIGHT: Pixels = px(26.);

const SOURCE_PADDING: Pixels = px(8.);
const SOURCE_HEADER: Pixels = px(24.);
const SOURCE_MARGIN: Pixels = px(4.);

pub(super) struct Layout {
    pub(super) theme: Theme,
    pub(super) origin: Point<Pixels>,
    pub(super) rows: Vec<Row>,
    pub(super) source_frame: Option<SourceFrame>,
    standalone_gutter: Option<StandaloneGutter>,
    // Paint only. This list does not take part in source geometry or hit testing.
    placeholders: Vec<Placeholder>,
    #[cfg(test)]
    pub(super) painted_drop_cursor: Option<gpui::PaintQuad>,
    #[cfg(test)]
    pub(super) painted_carets: Vec<gpui::PaintQuad>,
    #[cfg(test)]
    pub(super) painted_find_quads: Vec<gpui::PaintQuad>,
    #[cfg(test)]
    pub(super) painted_bracket_quads: Vec<gpui::PaintQuad>,
    #[cfg(test)]
    pub(super) painted_active_lines: PaintedActiveLines,
    #[cfg(test)]
    pub(super) painted_layer_order: PaintedLayerOrder,
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct PaintedActiveLines {
    pub(super) content: Vec<gpui::PaintQuad>,
    pub(super) gutter: Vec<gpui::PaintQuad>,
    pub(super) ordinary_gutter: Vec<gpui::PaintQuad>,
    pub(super) gutter_labels: Vec<ShapedLine>,
    pub(super) standalone_fold_labels: Vec<ShapedLine>,
    standalone_gutter_border: Option<gpui::PaintQuad>,
}

#[cfg(test)]
pub(super) enum ContentPaintLayer {
    SelectionAndFind,
    ActiveLine,
    Brackets,
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct PaintedLayerOrder {
    pub(super) quads: Vec<gpui::PaintQuad>,
    pub(super) standalone_text_after_quads: Vec<usize>,
    selection_and_find: usize,
    active_line: usize,
    brackets: usize,
}

#[cfg(test)]
impl PaintedLayerOrder {
    // Copy only new submissions from the painter that just ran.
    pub(super) fn record(&mut self, layer: ContentPaintLayer, quads: &[gpui::PaintQuad]) {
        let offset = match layer {
            ContentPaintLayer::SelectionAndFind => &mut self.selection_and_find,
            ContentPaintLayer::ActiveLine => &mut self.active_line,
            ContentPaintLayer::Brackets => &mut self.brackets,
        };
        self.quads.extend_from_slice(&quads[*offset..]);
        *offset = quads.len();
    }
}

#[derive(Default)]
pub(super) struct ActiveSourceHighlights {
    content: Vec<Range<usize>>,
    gutter: Vec<Range<usize>>,
    legacy: bool,
}

impl ActiveSourceHighlights {
    pub(super) fn paint_row(
        &self,
        frame: &SourceFrame,
        origin: Point<Pixels>,
        row: &Row,
        theme: Theme,
        window: &mut Window,
        #[cfg(test)] painted: &mut PaintedActiveLines,
    ) {
        let content = frame.active_line_bounds(origin, row, &self.content);
        if let Some(mut bounds) = content {
            if !self.legacy
                && let Some(gutter) = &frame.gutter
            {
                // Without a themed background, the gutter cannot cover a content fill.
                let left = origin.x + SOURCE_PADDING + gutter.width;
                bounds.size.width = (bounds.right() - left).max(px(0.));
                bounds.origin.x = left;
            }
            if bounds.size.width > px(0.) {
                // CodeMirror view 6.44.0 base light fill when there is no view style.
                let color = theme
                    .view
                    .map_or(0xcceeff44, |style| style.active_line.rgba32());
                let quad = fill(bounds, rgba(color));
                #[cfg(test)]
                painted.content.push(quad.clone());
                window.paint_quad(quad);
            }
        }
        if let Some(bounds) = frame
            .gutter
            .as_ref()
            .and_then(|gutter| gutter.row_bounds(origin, row))
        {
            let active = (self.legacy && content.is_some())
                || frame
                    .active_line_bounds(origin, row, &self.gutter)
                    .is_some();
            let color = if active {
                Some(
                    theme
                        .view
                        .map_or(0xe2f2ffff, |style| style.active_line_gutter.rgba32()),
                )
            } else {
                theme.view.map(|style| style.gutters.background.rgba32())
            };
            if let Some(color) = color {
                let quad = fill(bounds, rgba(color));
                #[cfg(test)]
                if active {
                    painted.gutter.push(quad.clone());
                } else {
                    painted.ordinary_gutter.push(quad.clone());
                }
                window.paint_quad(quad);
            }
        }
    }
}

struct Placeholder {
    line: ShapedLine,
    #[cfg(test)]
    painted_at: Option<Point<Pixels>>,
}

pub(super) struct SourceFrame {
    pub(super) bounds: Bounds<Pixels>,
    content: Range<usize>,
    editable: Option<Range<usize>>,
    label: ShapedLine,
    gutter: Option<SourceGutter>,
}

impl SourceFrame {
    pub(super) fn paint_gutter_row(
        &self,
        origin: Point<Pixels>,
        row: &Row,
        theme: Theme,
        window: &mut Window,
        cx: &mut App,
        #[cfg(test)] painted: &mut PaintedActiveLines,
    ) {
        if let Some(gutter) = &self.gutter {
            gutter.paint_row(
                origin,
                row,
                theme,
                window,
                cx,
                #[cfg(test)]
                &mut painted.gutter_labels,
            );
        }
    }

    pub(super) fn contains_row(&self, row: &Row) -> bool {
        (self.content.start..=self.content.end).contains(&row.start)
            && row.y >= self.bounds.top() + SOURCE_HEADER
            && row.y + row.height <= self.bounds.bottom()
    }

    pub(super) fn active_line_bounds(
        &self,
        origin: Point<Pixels>,
        row: &Row,
        lines: &[Range<usize>],
    ) -> Option<Bounds<Pixels>> {
        if !active_block_contains_row(lines, row) || !self.contains_row(row) {
            return None;
        }
        let side = px(1.);
        Some(Bounds::new(
            origin + point(self.bounds.left() + side, row.y),
            size((self.bounds.size.width - side * 2.).max(px(0.)), row.height),
        ))
    }

    fn background_quads(
        bounds: Bounds<Pixels>,
        theme: Theme,
    ) -> impl Iterator<Item = (Option<ContentMask<Pixels>>, PaintQuad)> {
        let chrome = quad(
            bounds,
            px(6.),
            theme.preview,
            Edges::all(px(1.)),
            theme.border,
            Default::default(),
        );
        let (header, body) = if theme.source_body_inherits_surface {
            let body_top = bounds.top() + SOURCE_HEADER;
            let mask = |top, bottom| {
                Some(ContentMask {
                    bounds: Bounds::from_corners(
                        point(bounds.left(), top),
                        point(bounds.right(), bottom),
                    ),
                })
            };
            // Keep the rounded border, but let the root paint the body only once.
            (
                mask(bounds.top(), body_top),
                Some((
                    mask(body_top, bounds.bottom()),
                    chrome.clone().background(rgba(0)),
                )),
            )
        } else {
            (None, None)
        };
        std::iter::once((header, chrome)).chain(body)
    }

    pub(super) fn paint(
        &self,
        origin: Point<Pixels>,
        theme: Theme,
        window: &mut Window,
        cx: &mut App,
    ) {
        let bounds = Bounds::new(origin + self.bounds.origin, self.bounds.size);
        if !window.content_mask().bounds.intersects(&bounds) {
            return;
        }
        for (mask, quad) in Self::background_quads(bounds, theme) {
            window.with_content_mask(mask, |window| window.paint_quad(quad));
        }
        let _ = self.label.paint(
            bounds.origin + point(SOURCE_PADDING, SOURCE_MARGIN),
            SOURCE_HEADER - SOURCE_MARGIN,
            window,
            cx,
        );
    }
}

pub(super) struct Row {
    pub(super) items: Vec<Item>,
    start: usize,
    end: usize,
    hidden_end: usize,
    soft_end: bool,
    pub(super) y: Pixels,
    pub(super) height: Pixels,
    pub(super) width: Pixels,
}

impl Row {
    fn position_at(&self, x: Pixels) -> (usize, bool) {
        #[cfg(target_os = "macos")]
        if let Some(hit) = self.isolated_hit(x, self.height / 2.) {
            return (hit.head(), self.soft_end && hit.head() == self.end);
        }
        let offset = self
            .items
            .iter()
            .find(|item| x < item.left + item.width)
            .map_or(self.end, |item| item.offset(x));
        (offset, self.soft_end && offset == self.end)
    }

    #[cfg(target_os = "macos")]
    fn isolated_item(&self) -> Option<(&Item, &IsolatedRow)> {
        let [item] = self.items.as_slice() else {
            return None;
        };
        let ItemPaint::Isolated(native) = &item.paint else {
            return None;
        };
        Some((item, native))
    }

    #[cfg(target_os = "macos")]
    fn isolated_hit(&self, x: Pixels, y: Pixels) -> Option<SelectionRange> {
        let (item, native) = self.isolated_item()?;
        if x < item.left || !(px(0.)..self.height).contains(&y) {
            return None;
        }
        native.hit(point(x - item.left, y))
    }

    fn caret_at(&self, x: Pixels) -> SelectionRange {
        #[cfg(target_os = "macos")]
        if let Some(hit) = self.isolated_hit(x, self.height / 2.) {
            return hit;
        }
        if let [item] = self.items.as_slice()
            && let Some(bidi) = &item.bidi
            && (item.left..=item.left + item.width).contains(&x)
        {
            let hit = bidi.hit(x - item.left);
            return EditorSelection::cursor(
                item.source.start + hit.head(),
                hit.assoc(),
                hit.bidi_level(),
                None,
            );
        }
        let (head, upstream) = self.position_at(x);
        let assoc = if upstream {
            Assoc::Before
        } else {
            Assoc::After
        };
        EditorSelection::cursor(head, Some(assoc), None, None)
    }

    fn x_at(&self, offset: usize) -> Pixels {
        self.items
            .iter()
            .find(|item| offset <= item.source.end)
            .map_or(self.width, |item| item.x(offset))
    }
}

pub(super) struct Item {
    pub(super) source: Range<usize>,
    pub(super) target: Option<Section>,
    atomic: bool,
    bidi: Option<Box<BidiRow>>,
    pub(super) left: Pixels,
    pub(super) width: Pixels,
    pub(super) paint: ItemPaint,
}

pub(super) enum ItemPaint {
    Text(Box<ShapedLine>),
    #[cfg(target_os = "macos")]
    Isolated(Rc<IsolatedRow>),
    Fold(Box<ShapedLine>),
    Graphic(Graphic),
    Block(Box<BlockLayout>),
}

impl Item {
    pub(super) fn framed_graphic(&self) -> bool {
        self.target
            .as_ref()
            .is_some_and(|section| is_block_preview(section.kind))
    }

    pub(super) fn offset(&self, x: Pixels) -> usize {
        let x = x - self.left;
        if x <= px(0.) {
            return self.source.start;
        }
        if let Some(bidi) = &self.bidi {
            return self.source.start + bidi.hit(x).head();
        }
        match &self.paint {
            #[cfg(target_os = "macos")]
            ItemPaint::Isolated(native) => native
                .hit(point(x, native.height() / 2.))
                .map_or(self.source.start, SelectionRange::head),
            ItemPaint::Text(line) if !self.atomic => {
                let index = line.closest_index_for_x(x);
                self.source.start
                    + if index == line.text.len() {
                        index
                    } else {
                        line.text
                            .grapheme_indices(true)
                            .take_while(|(start, _)| *start <= index)
                            .last()
                            .map_or(0, |(start, _)| start)
                    }
            }
            _ => {
                if x < self.width / 2. {
                    self.source.start
                } else {
                    self.source.end
                }
            }
        }
    }

    fn x(&self, offset: usize) -> Pixels {
        if let Some(x) = offset
            .checked_sub(self.source.start)
            .and_then(|local| self.bidi.as_ref()?.x_at(local, None, None))
        {
            return self.left + x;
        }
        self.left
            + match &self.paint {
                #[cfg(target_os = "macos")]
                ItemPaint::Isolated(native) => native.x_at(offset, None),
                ItemPaint::Text(line) if !self.atomic => line.x_for_index(
                    offset.clamp(self.source.start, self.source.end) - self.source.start,
                ),
                _ => {
                    if offset <= self.source.start {
                        px(0.)
                    } else {
                        self.width
                    }
                }
            }
    }

    pub(super) fn is_text(&self) -> bool {
        #[cfg(target_os = "macos")]
        if matches!(self.paint, ItemPaint::Isolated(_)) {
            return true;
        }
        matches!(self.paint, ItemPaint::Text(_))
    }

    fn uses_bidi_carets(&self) -> bool {
        #[cfg(target_os = "macos")]
        if matches!(self.paint, ItemPaint::Isolated(_)) {
            return true;
        }
        self.bidi.is_some()
    }

    pub(super) fn selection_source(&self) -> Range<usize> {
        #[cfg(target_os = "macos")]
        if let ItemPaint::Isolated(native) = &self.paint {
            return native.selection_source();
        }
        self.source.clone()
    }

    /// Horizontal intervals of a selected source range. Bidi runs can put
    /// parts of a logical range at nonadjacent positions on the same row.
    pub(super) fn x_spans(
        &self,
        source: Range<usize>,
    ) -> impl Iterator<Item = Range<Pixels>> + use<> {
        let start = source.start.max(self.source.start);
        let end = source.end.min(self.source.end);
        let (plain, bidi) = match (&self.paint, &self.bidi) {
            #[cfg(target_os = "macos")]
            (ItemPaint::Isolated(native), _) => (None, native.x_spans(source)),
            _ if start >= end => (None, Vec::new()),
            (_, Some(bidi)) => (
                None,
                bidi.selection_spans(start - self.source.start..end - self.source.start),
            ),
            _ => {
                let x = self.x(start);
                let y = self.x(end);
                (Some(x.min(y)..x.max(y)), Vec::new())
            }
        };
        let left = self.left;
        plain.into_iter().chain(
            bidi.into_iter()
                .map(move |span| left + span.start..left + span.end),
        )
    }

    fn stops(&self) -> Vec<usize> {
        match &self.paint {
            #[cfg(target_os = "macos")]
            ItemPaint::Isolated(native) => native.logical_stops(),
            ItemPaint::Text(line) if !self.atomic => line
                .text
                .grapheme_indices(true)
                .map(|(byte, _)| self.source.start + byte)
                .chain([self.source.end])
                .collect(),
            _ => vec![self.source.start, self.source.end],
        }
    }
}

impl Layout {
    pub(super) fn paint_placeholders(&mut self, window: &mut Window, cx: &mut App) {
        // The canvas paints both noninteractive adornments after the text rows.
        self.paint_standalone_gutter(window, cx);
        if self.placeholders.is_empty() {
            return;
        }
        let row = &self.rows[0];
        let mut origin = self.origin + point(row.width + px(1.), row.y);
        let clip = window.content_mask().bounds;
        for placeholder in &mut self.placeholders {
            let bounds = Bounds::new(origin, size(placeholder.line.width, row.height));
            if clip.intersects(&bounds) {
                let result = placeholder.line.paint(origin, row.height, window, cx);
                #[cfg(test)]
                if result.is_ok() {
                    placeholder.painted_at = Some(origin);
                }
                let _ = result;
            }
            origin.x += bounds.size.width;
        }
    }

    #[cfg(test)]
    pub(super) fn painted_placeholders(&self) -> impl Iterator<Item = (&str, Point<Pixels>)> {
        self.placeholders.iter().filter_map(|placeholder| {
            Some((placeholder.line.text.as_ref(), placeholder.painted_at?))
        })
    }

    pub(super) fn row_at(&self, position: Point<Pixels>) -> &Row {
        let y = position.y - self.origin.y;
        let index = self.rows.partition_point(|row| row.y + row.height <= y);
        &self.rows[index.min(self.rows.len() - 1)]
    }

    pub(super) fn scroll_block_at_height(&self, height: f64) -> ScrollBlock {
        let height = Pixels::from(height);
        let index = self.rows.partition_point(|row| row.y + row.height < height);
        let row = &self.rows[index.min(self.rows.len() - 1)];
        ScrollBlock {
            from: row.start,
            top: row.y.into(),
        }
    }

    pub(super) fn scroll_block_top(&self, from: usize) -> f64 {
        let index = self.rows.partition_point(|row| row.start < from);
        let row = self
            .rows
            .get(index)
            .filter(|row| row.start == from)
            .unwrap_or(&self.rows[index.saturating_sub(1)]);
        row.y.into()
    }

    pub(super) fn source_frame_contains(&self, position: Point<Pixels>) -> bool {
        self.source_frame
            .as_ref()
            .is_some_and(|frame| frame.bounds.contains(&(position - self.origin)))
    }

    pub(super) fn item_at(&self, position: Point<Pixels>) -> Option<&Item> {
        let row = self.row_at(position);
        let x = position.x - self.origin.x;
        let y = position.y - self.origin.y;
        if y < row.y || y >= row.y + row.height {
            return None;
        }
        row.items
            .iter()
            .find(|item| x >= item.left && x < item.left + item.width)
    }

    pub(super) fn index_at(&self, position: Point<Pixels>) -> usize {
        self.position_at(position).0
    }

    pub(super) fn position_at(&self, position: Point<Pixels>) -> (usize, bool) {
        let local = position - self.origin;
        let frame = self
            .source_frame
            .as_ref()
            .filter(|frame| frame.bounds.contains(&local));
        if let Some(frame) = frame {
            if local.y < frame.bounds.top() + SOURCE_HEADER {
                return (frame.content.start, false);
            }
            if local.y >= frame.bounds.bottom() - SOURCE_PADDING {
                return (frame.content.end, false);
            }
        }
        let row = self.row_at(position);
        #[cfg(not(target_os = "macos"))]
        let (offset, upstream) = row.position_at(local.x);
        #[cfg(target_os = "macos")]
        let (offset, upstream) = row.isolated_hit(local.x, local.y - row.y).map_or_else(
            || row.position_at(local.x),
            |hit| (hit.head(), row.soft_end && hit.head() == row.end),
        );
        (
            frame.map_or(offset, |frame| {
                offset.clamp(frame.content.start, frame.content.end)
            }),
            upstream,
        )
    }

    // GPUI's test platform has no text shaper. Tests can install the glyph
    // positions of a shaped row without substituting the movement algorithm.
    #[cfg(test)]
    pub(super) fn set_bidi_test_geometry(
        &mut self,
        start: usize,
        width: Pixels,
        glyphs: impl IntoIterator<Item = (usize, Pixels)>,
    ) {
        let index = self.row_index(start, false).unwrap();
        let row = &mut self.rows[index];
        let [item] = row.items.as_mut_slice() else {
            panic!("one item for a source row");
        };
        let ItemPaint::Text(line) = &item.paint else {
            panic!("a shaped source row");
        };
        item.bidi = Some(Box::new(
            BidiRow::measure(&line.text, width, glyphs).unwrap(),
        ));
        item.width = width;
        row.width = item.left + width;
    }

    pub(super) fn bidi_hit_at(&self, position: Point<Pixels>) -> Option<SelectionRange> {
        #[cfg(target_os = "macos")]
        if self.source_frame_contains(position) {
            let row = self.row_at(position);
            let local = position - self.origin;
            if let Some(hit) = row.isolated_hit(local.x, local.y - row.y) {
                return Some(hit);
            }
        }
        let item = self.item_at(position)?;
        let hit = item
            .bidi
            .as_ref()?
            .hit(position.x - self.origin.x - item.left);
        Some(EditorSelection::cursor(
            item.source.start + hit.head(),
            hit.assoc(),
            hit.bidi_level(),
            None,
        ))
    }

    pub(super) fn uses_bidi_carets(&self, head: usize) -> bool {
        self.row_index(head, false).is_some_and(|row| {
            self.rows[row].items.iter().any(|item| {
                item.uses_bidi_carets() && (item.source.start..=item.source.end).contains(&head)
            })
        })
    }

    pub(super) fn bidi_step<'a>(
        &self,
        text: &'a Text,
        start: SelectionRange,
        forward: bool,
    ) -> Option<(SelectionRange, &'a str)> {
        let row = self.rows.get(self.row_index(start.head(), false)?)?;
        let [item] = row.items.as_slice() else {
            return None;
        };
        #[cfg(target_os = "macos")]
        if let ItemPaint::Isolated(native) = &item.paint
            && !native.has_visual_motion()
        {
            // On wrapped or visually ambiguous rows, keep logical motion but
            // do not discard the native association of the current selection.
            let start = start.head();
            let head = self.step(start, forward);
            let crossed = text.slice(start.min(head)..start.max(head))?;
            let assoc = if forward { Assoc::Before } else { Assoc::After };
            return (head != start).then(|| {
                (
                    EditorSelection::cursor(head, Some(assoc), None, None),
                    if crossed == "\r\n" { "\n" } else { crossed },
                )
            });
        }
        let line = text.line_at(start.head())?;
        if line.range != item.source {
            return None;
        }
        #[cfg(target_os = "macos")]
        let use_gpui = if let ItemPaint::Isolated(native) = &item.paint {
            if let Some(step) = native.visual_step(text, start, forward) {
                return Some(step);
            }
            false
        } else {
            true
        };
        #[cfg(not(target_os = "macos"))]
        let use_gpui = true;
        if use_gpui {
            let bidi = item.bidi.as_ref()?;
            bidi.x_at(
                start.head() - line.range.start,
                start.assoc(),
                start.bidi_level(),
            )?;
            if let Some(moved) = move_visually(&line, bidi.spans(), Direction::Ltr, start, forward)
            {
                let next = moved.selection;
                bidi.x_at(
                    next.head() - line.range.start,
                    next.assoc(),
                    next.bidi_level(),
                )?;
                return Some((next, moved.moved_over));
            }
        }
        let adjacent = text.line(if forward {
            line.number + 1
        } else {
            line.number - 1
        })?;
        let next = if forward {
            EditorSelection::cursor(adjacent.range.start, Some(Assoc::Before), None, None)
        } else {
            EditorSelection::cursor(adjacent.range.end, Some(Assoc::After), None, None)
        };
        self.bounds_at_selection(next, false)?;
        Some((next, "\n"))
    }

    pub(super) fn fold_hides(&self, offset: usize) -> bool {
        self.row_index(offset, false).is_some_and(|index| {
            self.rows[index].items.iter().any(|item| {
                matches!(item.paint, ItemPaint::Fold(_))
                    && item.source.start < offset
                    && offset < item.source.end
            })
        })
    }

    pub(super) fn bounds_at(&self, offset: usize, upstream: bool) -> Option<Bounds<Pixels>> {
        let row = &self.rows[self.row_index(offset, upstream)?];
        Some(self.bounds_in_row(row, offset))
    }

    pub(super) fn bounds_at_selection(
        &self,
        range: SelectionRange,
        upstream: bool,
    ) -> Option<Bounds<Pixels>> {
        let index = self.row_index(range.head(), upstream)?;
        #[cfg(target_os = "macos")]
        let index = if let Some(assoc) = range.assoc()
            && (assoc == Assoc::Before) != upstream
            && let Some(other) = self.row_index(range.head(), !upstream)
            && index != other
            && self.rows[index.min(other)].isolated_item().is_some()
        {
            other
        } else {
            index
        };
        let row = &self.rows[index];
        #[cfg(target_os = "macos")]
        if let Some((item, native)) = row.isolated_item() {
            return Some(Bounds::new(
                self.origin + point(item.left + native.x_at(range.head(), range.assoc()), row.y),
                size(px(1.), row.height),
            ));
        }
        if let [item] = row.items.as_slice()
            && let Some(local) = range.head().checked_sub(item.source.start)
            && let Some(x) = item
                .bidi
                .as_ref()
                .and_then(|bidi| bidi.x_at(local, range.assoc(), range.bidi_level()))
        {
            return Some(Bounds::new(
                self.origin + point(item.left + x, row.y),
                size(px(1.), row.height),
            ));
        }
        Some(self.bounds_in_row(row, range.head()))
    }

    pub(super) fn drop_cursor_bounds(&self, offset: usize) -> Option<Bounds<Pixels>> {
        let row = &self.rows[self.row_index(offset, false)?];
        // Selection geometry can snap through source gaps or use atomic preview
        // edges. Require a real projected text stop instead.
        let stop = if row.items.is_empty() {
            offset == row.start && offset == row.end
        } else {
            row.items.iter().any(|item| {
                !item.atomic
                    && item.is_text()
                    && (item.source.start..=item.source.end).contains(&offset)
                    && item.stops().contains(&offset)
            })
        };
        if !stop {
            return None;
        }
        let caret = EditorSelection::cursor(offset, Some(Assoc::After), None, None);
        self.bounds_at_selection(caret, false)
    }

    fn bounds_in_row(&self, row: &Row, offset: usize) -> Bounds<Pixels> {
        if let Some(item) = row.items.iter().find(|item| {
            matches!(item.paint, ItemPaint::Block(_))
                && (offset == item.source.start || offset == item.source.end)
        }) {
            let height = px(26.);
            let y = if offset == item.source.end {
                row.y + row.height - height
            } else {
                row.y
            };
            return Bounds::new(self.origin + point(item.left, y), size(px(1.), height));
        }
        Bounds::new(
            self.origin + point(row.x_at(offset), row.y),
            size(px(1.), row.height),
        )
    }

    pub(super) fn source_bounds(&self, offset: usize) -> Option<Bounds<Pixels>> {
        let index = self.rows.partition_point(|row| {
            let end = row.end.max(row.hidden_end);
            end < offset || (row.soft_end && end == offset)
        });
        let row = self.rows.get(index)?;
        // A preview has no caret stops inside its source range. Reveal its leading edge.
        if let Some(item) = row
            .items
            .iter()
            .find(|item| item.atomic && item.source.contains(&offset))
        {
            return Some(Bounds::new(
                self.origin + point(item.left, row.y),
                size(px(1.), row.height.min(px(26.))),
            ));
        }
        Some(self.bounds_in_row(row, offset.min(row.end)))
    }

    fn row_index(&self, offset: usize, upstream: bool) -> Option<usize> {
        let upstream = upstream
            && self
                .source_frame
                .as_ref()
                .is_none_or(|frame| offset != frame.content.start);
        let index = self.rows.partition_point(|row| {
            row.end < offset || (!upstream && row.soft_end && row.end == offset)
        });
        // A programmatic selection can address the middle of CRLF even
        // though no pointer or keyboard stop can. Keep it on the break's row.
        #[cfg(target_os = "macos")]
        if let Some(previous) = index.checked_sub(1) {
            let row = &self.rows[previous];
            if row.end < offset && offset <= row.hidden_end && row.isolated_item().is_some() {
                return Some(previous);
            }
        }
        (index < self.rows.len()).then_some(index)
    }

    pub(super) fn line_block(&self, head: usize) -> Option<Range<usize>> {
        let row = self.row_index(head, false)?;
        let first = self.rows[..row]
            .iter()
            .rposition(|row| !row.soft_end)
            .map_or(0, |previous| previous + 1);
        let last = self.rows[row..].iter().find(|row| !row.soft_end)?;
        Some(self.rows[first].start..last.end)
    }

    pub(super) fn active_source_highlights(
        &self,
        state: &EditorState,
        focused: bool,
        input_enabled: bool,
    ) -> ActiveSourceHighlights {
        // The workspace freezes the source canvas during operations. The
        // explicit extensions themselves are still installed in that state.
        if !input_enabled || self.source_frame.is_none() {
            return ActiveSourceHighlights::default();
        }
        let line = ViewHighlight::ActiveLine.enabled(state);
        let gutter = highlight_active_line_gutter_enabled(state);
        if line || gutter {
            return ActiveSourceHighlights {
                content: if line {
                    self.source_frame
                        .as_ref()
                        .and_then(|frame| frame.editable.as_ref())
                        .map_or_else(Vec::new, |scope| self.active_content_blocks(state, scope))
                } else {
                    Vec::new()
                },
                gutter: if gutter {
                    self.active_gutter_blocks(state.selection())
                } else {
                    Vec::new()
                },
                legacy: false,
            };
        }
        // Customizing either channel replaces the shared GPUI default. The
        // original focused, themed highlight returns when neither is installed.
        if focused && self.theme.view.is_some() {
            return ActiveSourceHighlights {
                content: self.active_source_lines(state.text(), state.selection()),
                legacy: true,
                ..Default::default()
            };
        }
        ActiveSourceHighlights::default()
    }

    pub(super) fn standalone_active_lines(&self, state: &EditorState) -> Vec<Range<usize>> {
        // Unlike the framed source, a standalone file has no legacy active line.
        if self.theme.view.is_none() || !ViewHighlight::ActiveLine.enabled(state) {
            return Vec::new();
        }
        self.active_content_blocks(state, &(0..state.text().len()))
    }

    pub(super) fn standalone_active_line_quad(
        &self,
        row: &Row,
        width: Pixels,
        lines: &[Range<usize>],
    ) -> Option<PaintQuad> {
        if !active_block_contains_row(lines, row) {
            return None;
        }
        let color = self.theme.view?.active_line.rgba32();
        let inset = self
            .standalone_gutter
            .as_ref()
            .map_or(px(0.), StandaloneGutter::inset);
        let bounds = Bounds::new(
            self.origin + point(inset, row.y),
            size((width - inset).max(px(0.)), row.height),
        );
        Some(fill(bounds, rgba(color)))
    }

    fn active_content_blocks(
        &self,
        state: &EditorState,
        scope: &Range<usize>,
    ) -> Vec<Range<usize>> {
        let text = state.text().as_str().as_bytes();
        let blocks = state.selection().ranges().iter().filter_map(|range| {
            let head = range.head();
            if !(scope.start..=scope.end).contains(&head) {
                return None;
            }
            let block = self
                .line_block(head)
                .filter(|block| (block.start..=block.end).contains(&head))
                // A programmatic head can be between CR and LF. Use the
                // preceding measured block, but never cross a fence separator.
                .or_else(|| {
                    (head > scope.start && text.get(head - 1..=head) == Some(b"\r\n"))
                        .then(|| self.line_block(head - 1))
                        .flatten()
                })?;
            (scope.start <= block.start && block.end <= scope.end).then_some(block)
        });
        distinct_line_blocks(blocks)
    }

    pub(super) fn row_edge(
        &self,
        offset: usize,
        upstream: bool,
        forward: bool,
    ) -> Option<(usize, bool)> {
        let row = &self.rows[self.row_index(offset, upstream)?];
        Some(if forward {
            (row.end, row.soft_end)
        } else {
            (row.start, false)
        })
    }

    pub(super) fn vertical(
        &self,
        offset: usize,
        upstream: bool,
        preferred_x: Option<Pixels>,
        down: bool,
    ) -> Option<((usize, bool), Pixels)> {
        let row = self.row_index(offset, upstream)?;
        let next = if down { row + 1 } else { row.checked_sub(1)? };
        let next = self.rows.get(next)?;
        let x = preferred_x.unwrap_or_else(|| self.rows[row].x_at(offset));
        Some((next.position_at(x), x))
    }

    pub(super) fn vertical_caret(
        &self,
        caret: SelectionRange,
        preferred_x: Option<Pixels>,
        page: Option<Pixels>,
        down: bool,
    ) -> Option<(SelectionRange, Pixels)> {
        let upstream = caret.assoc() == Some(Assoc::Before);
        let index = self.row_index(caret.head(), upstream)?;
        let row = &self.rows[index];
        let x = preferred_x.or_else(|| {
            self.bounds_at_selection(caret, upstream)
                .map(|bounds| bounds.left() - self.origin.x)
        })?;
        let next = if let Some(distance) = page {
            let distance = distance.max(row.height);
            let y = if down {
                row.y + row.height + distance
            } else {
                row.y - distance
            };
            self.rows
                .partition_point(|row| row.y + row.height <= y)
                .min(self.rows.len() - 1)
        } else if down {
            index + 1
        } else {
            index.checked_sub(1)?
        };
        Some((self.rows.get(next)?.caret_at(x), x))
    }

    pub(super) fn step(&self, offset: usize, right: bool) -> usize {
        fn in_row(row: &Row, offset: usize, right: bool) -> Option<usize> {
            let stops = row
                .items
                .iter()
                .flat_map(Item::stops)
                .chain([row.start, row.end]);
            if right {
                stops.filter(|&stop| stop > offset).min()
            } else {
                stops.filter(|&stop| stop < offset).max()
            }
        }
        if right {
            let first = self.rows.partition_point(|row| row.end <= offset);
            self.rows[first..]
                .iter()
                .find_map(|row| in_row(row, offset, true))
        } else {
            let end = self.rows.partition_point(|row| row.start < offset);
            self.rows[..end]
                .iter()
                .rev()
                .find_map(|row| in_row(row, offset, false))
        }
        .unwrap_or(offset)
    }
}

struct LayoutBuilder<'a> {
    pub(super) rows: Vec<Row>,
    row: Row,
    window: &'a Window,
    theme: Theme,
    highlights: &'a [(Range<usize>, HighlightProperties)],
    wrap_width: Pixels,
    inset: Pixels,
    source_frame: Option<SourceFrame>,
    standalone_gutter: Option<StandaloneGutter>,
    marked: Option<Range<usize>>,
    #[cfg(target_os = "macos")]
    isolation_cache: Option<BodyCache>,
}

impl<'a> LayoutBuilder<'a> {
    fn new(
        window: &'a Window,
        theme: Theme,
        marked: Option<Range<usize>>,
        wrap_width: Pixels,
    ) -> Self {
        Self {
            rows: Vec::new(),
            row: Self::row(0, px(0.)),
            window,
            theme,
            highlights: &[],
            wrap_width,
            inset: px(0.),
            source_frame: None,
            standalone_gutter: None,
            marked,
            #[cfg(target_os = "macos")]
            isolation_cache: None,
        }
    }

    fn row(start: usize, y: Pixels) -> Row {
        Row {
            items: Vec::new(),
            start,
            end: start,
            hidden_end: 0,
            soft_end: false,
            y,
            height: DEFAULT_LINE_HEIGHT,
            width: px(0.),
        }
    }

    fn next_row(&mut self, start: usize) {
        let mut y = self.row.y + self.row.height;
        if let Some(frame) = &self.source_frame {
            y = y.max(frame.bounds.bottom() + SOURCE_MARGIN);
        }
        let mut next = Self::row(start, y);
        next.width = self.inset;
        self.rows.push(std::mem::replace(&mut self.row, next));
    }

    fn resume_after_source_frame(&mut self, start: usize) {
        if self
            .source_frame
            .as_ref()
            .is_some_and(|frame| self.row.y < frame.bounds.bottom())
        {
            self.next_row(start);
        }
    }

    fn wrap_row(&mut self, start: usize) {
        self.row.soft_end = self.row.end == start;
        self.next_row(start);
    }

    fn add_text(&mut self, source: &str, range: Range<usize>, face: Face) {
        let start = range.start;
        self.add_text_at(&source[range], start, face);
    }

    fn add_text_at(&mut self, source: &str, mut start: usize, face: Face) {
        for text in source.split_inclusive('\n') {
            let newline = text.ends_with('\n');
            let visible = if newline {
                text.trim_end_matches('\n')
                    .strip_suffix('\r')
                    .unwrap_or(text.trim_end_matches('\n'))
            } else {
                text
            };
            if !visible.is_empty() {
                self.text(visible, start..start + visible.len(), face, None, false);
            }
            start += text.len();
            if newline {
                self.next_row(start);
            }
        }
    }

    fn add_source(&mut self, section: &Section, projection: &Projection<'_>) {
        let state = projection.state;
        let content = section.content.clone();
        if !self.row.items.is_empty() {
            self.wrap_row(content.start);
        }
        let first = self.rows.len();
        let top = self.row.y + SOURCE_MARGIN;
        self.row = Self::row(content.start, top + SOURCE_HEADER);
        self.row.width = SOURCE_PADDING;
        self.inset = SOURCE_PADDING;
        let width = self.wrap_width;
        let source_width = (width - SOURCE_PADDING).max(SOURCE_PADDING + px(1.));
        self.wrap_width = source_width;
        let body_lines = projection.body_lines.as_ref();
        let gutter = projection.gutter_body.as_ref().and_then(|body| {
            SourceGutter::new(state, body.clone(), body_lines, self.theme, self.window)
        });
        let source = state.text().as_str();
        let body = gutter.as_ref().map_or(&content, |gutter| &gutter.body);
        self.add_text(source, content.start..body.start, Face::default());
        if let Some(gutter) = &gutter {
            self.inset = SOURCE_PADDING + gutter.width;
            self.row.width = self.inset;
            self.wrap_width = source_width.max(self.inset + px(1.));
        }
        let body_text = body_lines
            .filter(|_| gutter.is_some())
            .map_or(&source[body.clone()], Text::as_str);
        #[cfg(not(target_os = "macos"))]
        let isolated = false;
        #[cfg(target_os = "macos")]
        let isolated = gutter.is_some()
            && projection.folds.is_empty()
            && self.marked.is_none()
            && projection
                .isolates
                .as_deref()
                .is_some_and(|marks| self.add_isolated(source, body.clone(), marks).is_some());
        if !isolated {
            self.add_folded_text(
                state,
                body_text,
                body.start,
                &projection.folds,
                Face::default(),
            );
        }
        self.inset = SOURCE_PADDING;
        self.wrap_width = source_width;
        self.add_text(source, body.end..content.end, Face::default());
        if section.kind == SectionKind::DraftCode && self.marked.is_none() {
            for row in self.rows[first..].iter_mut().chain([&mut self.row]) {
                let [item] = row.items.as_mut_slice() else {
                    continue;
                };
                let suffix = &source[row.end..];
                if !row.soft_end
                    && item.source == (row.start..row.end)
                    && (row.start == 0 || source.as_bytes()[row.start - 1] == b'\n')
                    && (suffix.is_empty() || suffix.starts_with('\n') || suffix.starts_with("\r\n"))
                    && let ItemPaint::Text(line) = &item.paint
                {
                    item.bidi = BidiRow::new(line).map(Box::new);
                }
            }
        }
        self.wrap_width = width;
        self.inset = px(0.);

        let text = match section.kind {
            SectionKind::FrontMatter => "<front-matter>",
            SectionKind::Mermaid => "<mermaid>",
            SectionKind::InlineMath => "<inline-math>",
            SectionKind::DisplayMath => "<display-math>",
            SectionKind::Image => "<image>",
            SectionKind::Link => "<link>",
            SectionKind::DraftCode => "<code>",
            _ => unreachable!("only editable items enter source mode"),
        };
        let mut font = self.window.text_style().font();
        font.weight = FontWeight::SEMIBOLD;
        let label = self.window.text_system().shape_line(
            text.into(),
            px(12.),
            &[TextRun {
                len: text.len(),
                font,
                color: self.theme.muted.into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        let right = self.rows[first..]
            .iter()
            .chain([&self.row])
            .map(|row| row.width + SOURCE_PADDING)
            .fold(width.max(label.width + SOURCE_PADDING * 2.), Pixels::max);
        let bottom = self.row.y + self.row.height + SOURCE_PADDING;
        let editable = if section.kind == SectionKind::DraftCode {
            projection.editable_fence_body.clone()
        } else {
            Some(content.clone())
        };
        self.source_frame = Some(SourceFrame {
            bounds: Bounds::new(point(px(0.), top), size(right, bottom - top)),
            content,
            editable,
            label,
            gutter,
        });
    }

    fn add_folded_text(
        &mut self,
        state: &EditorState,
        text: &str,
        origin: usize,
        folds: &[Range<usize>],
        face: Face,
    ) {
        let mut start = origin;
        for fold in folds {
            self.add_text_at(&text[start - origin..fold.start - origin], start, face);
            self.fold_placeholder(state, fold.clone());
            start = fold.end;
        }
        self.add_text_at(&text[start - origin..], start, face);
    }

    fn fold_placeholder(&mut self, state: &EditorState, source: Range<usize>) {
        use gpui_editor_codemirror::language::fold::fold_placeholder;

        let style = self.window.text_style();
        let text = fold_placeholder(state, &source);
        let label = text.as_ref().map_or("…", |widget| widget.text.as_ref());
        let label = format!(" {} ", one_line(label));
        let length = label.len();
        let color = self.theme.view.map_or(self.theme.accent, |style| {
            rgba(style.fold_placeholder.foreground.rgba32())
        });
        let line = self.window.text_system().shape_line(
            label.into(),
            px(16.),
            &[TextRun {
                len: length,
                font: style.font(),
                color: color.into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        self.item(
            source,
            None,
            true,
            line.width,
            DEFAULT_LINE_HEIGHT,
            ItemPaint::Fold(Box::new(line)),
        );
    }

    fn text(
        &mut self,
        text: &str,
        source: Range<usize>,
        face: Face,
        target: Option<Section>,
        atomic: bool,
    ) {
        self.resume_after_source_frame(source.start);
        let style = self.window.text_style();
        let mut font = style.font();
        if face.code {
            font.family = CODE_FONT_FAMILY.into();
        }
        if face.heading || face.bold {
            font.weight = FontWeight::BOLD;
        }
        if face.italic {
            font.style = FontStyle::Italic;
        }
        let link = target
            .as_ref()
            .is_some_and(|section| section.kind == SectionKind::Link);
        let color = if link {
            self.theme.accent.into()
        } else if face.muted {
            self.theme.muted.into()
        } else {
            self.theme.text.into()
        };
        let template = TextRun {
            len: text.len(),
            font,
            color,
            background_color: None,
            underline: link.then_some(UnderlineStyle {
                color: Some(color),
                thickness: px(1.),
                wavy: false,
            }),
            strikethrough: None,
        };
        let font_size = px(if face.heading { 28. } else { 16. });
        let height = if face.heading {
            px(42.)
        } else {
            DEFAULT_LINE_HEIGHT
        };
        let window = self.window;
        let marked = self.marked.clone().filter(|_| !atomic);
        let highlights = if atomic {
            &[]
        } else {
            let start = self
                .highlights
                .partition_point(|(range, _)| range.end <= source.start);
            let end = self
                .highlights
                .partition_point(|(range, _)| range.start < source.end);
            &self.highlights[start..end]
        };
        let shape = |text: &str, source: Range<usize>| {
            let source = if atomic { 0..text.len() } else { source };
            let runs = text_runs(&template, source, marked.as_ref(), highlights);
            #[cfg(test)]
            super::one_dark_tests::record_fonts(false, text, &runs);
            window.text_system().shape_line(
                SharedString::from(text.to_owned()),
                font_size,
                &runs,
                None,
            )
        };
        let line = shape(text, source.clone());
        if atomic || line.width <= self.wrap_width - self.row.width {
            self.item(
                source,
                target,
                atomic,
                line.width,
                height,
                ItemPaint::Text(Box::new(line)),
            );
            return;
        }

        // Measure the whole span once. Shape each wrapped piece separately so
        // its source offsets and IME underline are local to that visual row.
        let mut glyphs: Vec<_> = line.runs.iter().flat_map(|run| &run.glyphs).collect();
        glyphs.sort_by_key(|glyph| glyph.index);
        let left_to_right = glyphs
            .windows(2)
            .all(|pair| pair[0].position.x <= pair[1].position.x);
        let mut glyphs = glyphs.into_iter().peekable();
        let mut words = text.split_word_bound_indices().map(|(ix, _)| ix).peekable();
        let mut x = px(0.);
        let mut boundaries: Vec<_> = text
            .grapheme_indices(true)
            .map(|(ix, grapheme)| {
                while glyphs.peek().is_some_and(|glyph| glyph.index < ix) {
                    glyphs.next();
                }
                while words.peek().is_some_and(|word| *word < ix) {
                    words.next();
                }
                let word_break =
                    words.peek() == Some(&ix) && !grapheme.starts_with(char::is_whitespace);
                let position = if left_to_right {
                    glyphs.peek().map_or(line.width, |glyph| glyph.position.x)
                } else {
                    let position = x;
                    let start = source.start + ix;
                    x += shape(grapheme, start..start + grapheme.len()).width;
                    position
                };
                (ix, position, word_break)
            })
            .collect();
        boundaries.push((text.len(), if left_to_right { line.width } else { x }, true));

        let last = boundaries.len() - 1;
        let mut start = 0;
        while start < last {
            let available = self.wrap_width - self.row.width;
            let (start_byte, x, _) = boundaries[start];
            let mut fit = start;
            let mut word_break = None;
            for (i, &(_, next_x, word)) in boundaries.iter().enumerate().skip(start + 1) {
                if next_x - x > available {
                    break;
                }
                fit = i;
                if word {
                    word_break = Some(i);
                }
            }
            if word_break.is_none() && !self.row.items.is_empty() {
                self.wrap_row(source.start + start_byte);
                continue;
            }
            let mut end = word_break.unwrap_or(fit).max(start + 1);
            let (range, line) = loop {
                let end_byte = boundaries[end].0;
                let range = source.start + start_byte..source.start + end_byte;
                let line = shape(&text[start_byte..end_byte], range.clone());
                if line.width <= available || end == start + 1 {
                    break (range, line);
                }
                end -= 1;
            };
            if line.width > available && !self.row.items.is_empty() {
                self.wrap_row(range.start);
                continue;
            }
            self.item(
                range,
                None,
                false,
                line.width,
                height,
                ItemPaint::Text(Box::new(line)),
            );
            start = end;
            if start < last {
                self.wrap_row(source.start + boundaries[start].0);
            }
        }
    }

    fn item(
        &mut self,
        source: Range<usize>,
        target: Option<Section>,
        atomic: bool,
        width: Pixels,
        height: Pixels,
        paint: ItemPaint,
    ) {
        self.resume_after_source_frame(source.start);
        if atomic && !self.row.items.is_empty() && self.row.width + width > self.wrap_width {
            self.wrap_row(source.start);
        }
        self.row.end = source.end;
        self.row.items.push(Item {
            source,
            target,
            atomic,
            bidi: None,
            left: self.row.width,
            width,
            paint,
        });
        self.row.width += width;
        self.row.height = self.row.height.max(height);
    }

    fn shape_placeholders(&self, state: &EditorState) -> Vec<Placeholder> {
        // Use the visual projection only. GPUI does not expose an accessibility
        // placeholder attribute on the editor's input surface.
        let Ok(projection) = placeholder_projection(state) else {
            return Vec::new();
        };
        projection
            .widgets
            .iter()
            .flat_map(|set| set.iter(0))
            .filter_map(|range| range.value.widget_value())
            .map(|label| {
                // Keep hard breaks on the single empty document row.
                let label = label.replace("\r\n", " ").replace(
                    ['\r', '\n', '\x0b', '\x0c', '\u{85}', '\u{2028}', '\u{2029}'],
                    " ",
                );
                let run = TextRun {
                    len: label.len(),
                    font: self.window.text_style().font(),
                    color: self.theme.muted.into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                Placeholder {
                    line: self
                        .window
                        .text_system()
                        .shape_line(label.into(), px(16.), &[run], None),
                    #[cfg(test)]
                    painted_at: None,
                }
            })
            .collect()
    }

    fn build(mut self, placeholders: Vec<Placeholder>) -> Layout {
        self.rows.push(self.row);
        Layout {
            theme: self.theme,
            origin: point(px(0.), px(0.)),
            rows: self.rows,
            source_frame: self.source_frame,
            standalone_gutter: self.standalone_gutter,
            placeholders,
            #[cfg(test)]
            painted_drop_cursor: None,
            #[cfg(test)]
            painted_carets: Vec::new(),
            #[cfg(test)]
            painted_find_quads: Vec::new(),
            #[cfg(test)]
            painted_bracket_quads: Vec::new(),
            #[cfg(test)]
            painted_active_lines: PaintedActiveLines::default(),
            #[cfg(test)]
            painted_layer_order: PaintedLayerOrder::default(),
        }
    }
}

fn active_block_contains_row(lines: &[Range<usize>], row: &Row) -> bool {
    let next = lines.partition_point(|line| line.start <= row.start);
    next.checked_sub(1)
        .is_some_and(|index| row.start <= lines[index].end)
}

fn distinct_line_blocks(blocks: impl Iterator<Item = Range<usize>>) -> Vec<Range<usize>> {
    let mut blocks: Vec<_> = blocks.collect();
    blocks.sort_unstable_by_key(|block| block.start);
    blocks.dedup_by_key(|block| block.start);
    blocks
}

fn code_highlights<'a>(
    document: &'a SupportedDocument,
    spans: impl IntoIterator<Item = HighlightSpan<'a>>,
    mut resolve: impl FnMut(&[HighlightTag], Option<HighlightContext>) -> HighlightProperties,
) -> Vec<(Range<usize>, HighlightProperties)> {
    let mut highlights = Vec::new();
    let mut push = |range, tags: &[HighlightTag]| {
        let scope = document.highlight_context(&range);
        // Prove scope for the whole gap before either renderer clips it to a row.
        if tags.is_empty() && scope.is_none() {
            return;
        }
        let style = resolve(tags, scope);
        if !tags.is_empty() || style != HighlightProperties::default() {
            highlights.push((range, style));
        }
    };
    let mut end = 0;
    for span in spans.into_iter().filter(|span| !span.range.is_empty()) {
        if end < span.range.start {
            push(end..span.range.start, &[]);
        }
        end = end.max(span.range.end);
        push(span.range, span.tags);
    }
    if end < document.text().len() {
        push(end..document.text().len(), &[]);
    }
    highlights
}

pub(super) fn apply_highlight(run: &mut TextRun, style: HighlightProperties) {
    if let Some(color) = style.foreground {
        run.color = rgba(color.rgba32()).into();
    }
    if style.bold {
        run.font.weight = FontWeight::BOLD;
    }
    if style.italic {
        run.font.style = FontStyle::Italic;
    }
    match style.text_decoration {
        Some(TextDecoration::Underline) => {
            run.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: false,
            })
        }
        Some(TextDecoration::LineThrough) => {
            run.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            })
        }
        None => {}
    }
}

fn plain_text_highlights(
    spans: &[highlight::Span],
    theme: Theme,
) -> Vec<(Range<usize>, HighlightProperties)> {
    spans
        .iter()
        .map(|span| {
            let foreground = Color::from_rgba32(span.kind.color(theme).into());
            (
                span.range.clone(),
                HighlightProperties {
                    foreground: Some(foreground),
                    ..Default::default()
                },
            )
        })
        .collect()
}

fn text_runs(
    template: &TextRun,
    source: Range<usize>,
    marked: Option<&Range<usize>>,
    highlights: &[(Range<usize>, HighlightProperties)],
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut push = |range: Range<usize>, style| {
        if range.is_empty() {
            return;
        }
        let mut run = template.clone();
        apply_highlight(&mut run, style);
        if let Some(marked) = marked.filter(|m| m.start < range.end && range.start < m.end) {
            let start = marked.start.max(range.start);
            let end = marked.end.min(range.end);
            if range.start < start {
                runs.push(TextRun {
                    len: start - range.start,
                    ..run.clone()
                });
            }
            runs.push(TextRun {
                len: end - start,
                underline: Some(UnderlineStyle {
                    color: Some(run.color),
                    thickness: px(1.),
                    wavy: false,
                }),
                ..run.clone()
            });
            if end < range.end {
                runs.push(TextRun {
                    len: range.end - end,
                    ..run
                });
            }
        } else {
            runs.push(TextRun {
                len: range.len(),
                ..run
            });
        }
    };
    let mut start = source.start;
    for (range, style) in highlights {
        let from = start.max(range.start);
        let to = source.end.min(range.end);
        if from < to {
            push(start..from, HighlightProperties::default());
            push(from..to, *style);
            start = to;
        }
    }
    push(start..source.end, HighlightProperties::default());
    runs
}

impl Editor {
    pub(crate) fn has_blocked_images(&self) -> bool {
        !self.allow_remote_images
            && self
                .preview_sections(&self.projected_sections())
                .any(|section| {
                    section.kind == SectionKind::Image
                        && matches!(
                            image_target(
                                section.destination.as_deref().unwrap_or_default(),
                                None,
                                None
                            ),
                            Ok(Target::Web(_))
                        )
                })
    }

    fn projected_sections(&self) -> Vec<Section> {
        if !self.is_markdown() {
            return Vec::new();
        }
        let mut sections = self.markdown().sections();
        if let Some(active) = &self.active {
            sections.retain(|section| section.source != active.source);
            sections.push(active.clone());
            sections.sort_by_key(|section| {
                (section.source.start, std::cmp::Reverse(section.source.end))
            });
        }

        sections
    }

    fn preview_sections<'a>(
        &self,
        sections: &'a [Section],
    ) -> impl Iterator<Item = &'a Section> + use<'a> {
        let active = self.active.as_ref().map(|section| section.source.clone());
        let mut skip_to = 0;
        sections.iter().filter(move |section| {
            if section.source.start < skip_to {
                return false;
            }
            let is_active = active.as_ref() == Some(&section.source);
            if is_active
                || !matches!(
                    section.kind,
                    SectionKind::Heading
                        | SectionKind::Bullet
                        | SectionKind::Strong
                        | SectionKind::Emphasis
                )
            {
                skip_to = section.source.end;
            }
            !is_active
                && matches!(
                    section.kind,
                    SectionKind::Image
                        | SectionKind::InlineMath
                        | SectionKind::DisplayMath
                        | SectionKind::Mermaid
                )
        })
    }

    #[cfg(test)]
    pub(super) fn shape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Layout {
        self.shape_for_width(
            window,
            cx,
            super::wrap_width(self.scroll.bounds().size.width),
        )
    }

    #[cfg(test)]
    pub(super) fn shape_for_width(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        width: Pixels,
    ) -> Layout {
        self.shape_layout(window, cx, width, false).0
    }

    pub(super) fn shape_for_paint(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        width: Pixels,
    ) -> (Layout, Option<BodyCache>) {
        self.shape_layout(window, cx, width, true)
    }

    fn shape_layout(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        width: Pixels,
        for_paint: bool,
    ) -> (Layout, Option<BodyCache>) {
        if !self.is_markdown() {
            let (layers, appearance) = highlight_configuration(self.view.state());
            let theme = Theme::for_editor(window, self.view.state());
            let bracket_syntax = for_paint
                && theme.view.is_some()
                && self.focus.is_focused(window)
                && self.standalone_bracket_options().is_some()
                && self.has_standalone_bracket_syntax();
            if bracket_syntax || !layers.is_empty() {
                self.request_native_highlighting(cx);
            }
            let native = if layers.is_empty() {
                None
            } else {
                self.current_native_document().and_then(|document| {
                    let spans = highlight::verified_native_spans(document)?;
                    let mut styles = code_highlights(document, spans, |tags, scope| {
                        highlighting_for_context(&layers, tags, scope, appearance)
                    });
                    styles.retain(|(_, style)| *style != HighlightProperties::default());
                    (!styles.is_empty()).then_some(styles)
                })
            };
            // Select the fallback for the whole file, never for individual native gaps.
            let highlights = native.unwrap_or_else(|| {
                self.request_highlighting(cx);
                plain_text_highlights(self.highlighting.spans(), theme)
            });
            let state = self.view.state();
            let source = state.text().as_str();
            let standalone = self.standalone_folds();
            let folds = standalone
                .as_ref()
                .map_or_else(Vec::new, |folds| folds.ranges());
            let mut builder = LayoutBuilder::new(window, theme, self.marked.clone(), width);
            builder.standalone_gutter = StandaloneGutter::new(
                state,
                standalone.map(|folds| folds.config),
                self.input_enabled && !self.is_composing(),
                theme,
                window,
            );
            if let Some(gutter) = &builder.standalone_gutter {
                builder.inset = gutter.inset();
                builder.row.width = builder.inset;
            }
            builder.highlights = &highlights;
            builder.add_folded_text(
                state,
                source,
                0,
                &folds,
                Face {
                    code: true,
                    ..Face::default()
                },
            );
            let placeholders = builder.shape_placeholders(state);
            return (builder.build(placeholders), None);
        }
        let scale = window.scale_factor();
        let highlights = self.source_highlights();
        let body_lines = self.source_body_line_projection();
        let folds = self.source_fold_ranges();
        #[cfg(target_os = "macos")]
        let isolates = if folds.is_empty() {
            self.source_isolates()
        } else {
            None
        };
        #[cfg(target_os = "macos")]
        let isolation_cache = (for_paint && isolates.is_some())
            .then(|| CachedBody::for_window(cx.entity_id(), window, cx));
        #[cfg(not(target_os = "macos"))]
        let isolation_cache = {
            let _ = for_paint;
            None
        };
        let state = self.view.state();
        let source = state.text().as_str();
        let theme = Theme::for_editor(window, state);
        let mut builder = LayoutBuilder::new(window, theme, self.marked.clone(), width);
        builder.highlights = &highlights;
        #[cfg(target_os = "macos")]
        {
            builder.isolation_cache = isolation_cache.clone();
        }
        let sections = self.projected_sections();
        let visible_previews = self.preview_sections(&sections);
        let mut previews = std::collections::HashMap::new();
        let present = sections
            .iter()
            .filter(|section| {
                matches!(
                    section.kind,
                    SectionKind::InlineMath | SectionKind::DisplayMath | SectionKind::Mermaid
                )
            })
            .map(|section| (&source[section.content.clone()], section.kind))
            .collect::<std::collections::HashSet<_>>();
        self.graphics.retain(|(body, kind, dpi), _| {
            *dpi == scale.to_bits() && present.contains(&(body.as_str(), *kind))
        });
        let mut graphic_bytes: usize = self
            .graphics
            .values()
            .filter_map(|entry| entry.as_ref().ok())
            .map(|graphic| graphic.bytes)
            .sum();
        let image_destinations: std::collections::HashSet<_> = sections
            .iter()
            .filter(|section| section.kind == SectionKind::Image)
            .filter_map(|section| section.destination.as_deref())
            .collect();
        retain_images(&mut self.images, &image_destinations);
        for section in visible_previews {
            let body = &source[section.content.clone()];
            let kind = section.kind;
            let preview = match kind {
                SectionKind::InlineMath | SectionKind::DisplayMath | SectionKind::Mermaid => {
                    let key = (body.to_owned(), kind, scale.to_bits());
                    if self.graphics.len() >= 128 && !self.graphics.contains_key(&key) {
                        Err("Preview limit reached")
                    } else {
                        let graphic = self.graphics.entry(key).or_insert_with(|| {
                            let raster = match kind {
                                SectionKind::Mermaid => render_mermaid(body, scale),
                                _ => render_math(body, kind == SectionKind::DisplayMath, scale),
                            }
                            .inspect_err(|_| {
                                cx.emit(PreviewDiagnostic(if kind == SectionKind::Mermaid {
                                    "Mermaid preview failed"
                                } else {
                                    "Math preview failed"
                                }));
                            })?;
                            if graphic_bytes + raster.bgra.len() > 64 * 1024 * 1024 {
                                return Err("Preview memory limit reached".into());
                            }
                            graphic_bytes += raster.bgra.len();
                            Graphic::new(raster, scale)
                                .ok_or_else(|| "Could not decode preview".into())
                        });
                        graphic
                            .as_ref()
                            .cloned()
                            .map_err(|error| preview_error(kind, error))
                    }
                }
                SectionKind::Image => {
                    let destination = section.destination.clone().unwrap_or_default();
                    if let Some(image) = self.images.get(&destination) {
                        image_preview(image)
                    } else {
                        match image_target(
                            &destination,
                            self.path.as_deref(),
                            self.workspace.as_ref().map(Workspace::root),
                        ) {
                            Ok(Target::Web(_)) if !self.allow_remote_images => {
                                Err("Remote image blocked")
                            }
                            _ if self.images.len() >= 32 => Err("Image limit reached"),
                            _ if self.image_requests >= 3 => Err("Waiting for image"),
                            Err(error) => {
                                cx.emit(PreviewDiagnostic("Image destination rejected"));
                                self.images
                                    .insert(destination, ImageEntry::Ready(Err(error)));
                                Err("Image not loaded")
                            }
                            Ok(target) => {
                                self.image_requests += 1;
                                self.next_image_request += 1;
                                let request = self.next_image_request;
                                self.images
                                    .insert(destination.clone(), ImageEntry::Loading(request));
                                let workspace = self.workspace.clone();
                                let task = cx.background_executor().spawn(async move {
                                    load_preview_image(&target, workspace.as_ref())
                                });
                                cx.spawn(async move |editor, cx| {
                                    let result = task.await.and_then(|raster| {
                                        Graphic::new(raster, 1.)
                                            .ok_or_else(|| "Could not decode image".into())
                                    });
                                    let _ = editor.update(cx, |editor, cx| {
                                        if result.is_err() {
                                            cx.emit(PreviewDiagnostic("Image preview failed"));
                                        }
                                        editor.complete_image_request(destination, request, result);
                                        cx.notify();
                                    });
                                })
                                .detach();
                                Err("Loading image")
                            }
                        }
                    }
                }
                _ => continue,
            };
            previews.insert(section.source.start, preview);
        }
        let gutter_body = self.source_fence().map(|fence| fence.body);
        let editable_fence_body = gutter_body.clone().or_else(|| {
            self.source_fence_with_unlabeled(true)
                .map(|fence| fence.body)
        });
        let projection = Projection {
            state,
            active: self.active.as_ref(),
            gutter_body,
            editable_fence_body,
            body_lines,
            #[cfg(target_os = "macos")]
            isolates,
            folds,
            inline_caret: (self.focus.is_focused(window)
                && (self.selection().is_empty() || self.is_composing()))
            .then(|| self.cursor()),
            previews,
        };
        projection.add(&mut builder, 0..source.len(), &sections, Face::default(), 0);
        let placeholders = builder.shape_placeholders(state);
        (builder.build(placeholders), isolation_cache)
    }

    fn source_highlights(&mut self) -> Vec<(Range<usize>, HighlightProperties)> {
        let Some(fence) = self.source_fence() else {
            return Vec::new();
        };
        let Some(parser) = fence.parser else {
            return Vec::new();
        };
        let body = fence.body;
        let state = self.view.state().clone();
        let settings = StreamSettings {
            tab_size: state.tab_size(),
            ..Default::default()
        };
        let Some(document) = self.source_document(&body, &parser, settings) else {
            return Vec::new();
        };
        let Some(spans) = document.highlights(0..body.len()) else {
            return Vec::new();
        };
        let (layers, appearance) = highlight_configuration(&state);
        let mut highlights = code_highlights(document, spans, |tags, scope| {
            highlighting_for_context(&layers, tags, scope, appearance)
        });
        highlights.retain_mut(|(range, style)| {
            range.start += body.start;
            range.end += body.start;
            *style != HighlightProperties::default()
        });
        highlights
    }

    pub(super) fn complete_image_request(
        &mut self,
        destination: String,
        request: u64,
        result: Result<Graphic, String>,
    ) {
        self.image_requests -= 1;
        if !matches!(self.images.get(&destination), Some(ImageEntry::Loading(current)) if *current == request)
        {
            return;
        }
        let bytes: usize = self
            .images
            .values()
            .filter_map(|entry| match entry {
                ImageEntry::Ready(Ok(graphic)) => Some(graphic.bytes),
                _ => None,
            })
            .sum();
        let entry = match &result {
            Ok(graphic) if bytes + graphic.bytes > 64 * 1024 * 1024 => ImageEntry::BudgetExceeded,
            _ => ImageEntry::Ready(result),
        };
        self.images.insert(destination, entry);
    }
}

fn load_preview_image(target: &Target, workspace: Option<&Workspace>) -> Result<Raster, String> {
    if let Some(workspace) = workspace.filter(|_| matches!(target, Target::Local(_))) {
        workspace.check_root().map_err(|error| error.to_string())?;
        let raster = load_image(target)?;
        workspace.check_root().map_err(|error| error.to_string())?;
        Ok(raster)
    } else {
        load_image(target)
    }
}

fn retain_images(images: &mut HashMap<String, ImageEntry>, destinations: &HashSet<&str>) {
    let mut freed_budget = false;
    images.retain(|destination, entry| {
        let keep = destinations.contains(destination.as_str());
        freed_budget |= !keep && matches!(entry, ImageEntry::Ready(Ok(_)));
        keep
    });
    if freed_budget {
        images.retain(|_, entry| !matches!(entry, ImageEntry::BudgetExceeded));
    }
}

fn image_preview(entry: &ImageEntry) -> Result<Graphic, &'static str> {
    match entry {
        ImageEntry::Loading(_) => Err("Loading image"),
        ImageEntry::BudgetExceeded => Err("Image memory limit reached"),
        ImageEntry::Ready(result) => result
            .as_ref()
            .cloned()
            .map_err(|error| preview_error(SectionKind::Image, error)),
    }
}

fn preview_error(kind: SectionKind, error: &str) -> &'static str {
    let error = error.to_ascii_lowercase();
    if error.contains("limit") || error.starts_with("raster side") {
        "Preview limit reached"
    } else {
        match kind {
            SectionKind::Image => "Image not loaded",
            SectionKind::Mermaid => "Invalid Mermaid — click to edit",
            _ => "Invalid formula — click to edit",
        }
    }
}

struct Projection<'a> {
    state: &'a EditorState,
    active: Option<&'a Section>,
    gutter_body: Option<Range<usize>>,
    editable_fence_body: Option<Range<usize>>,
    body_lines: Option<Text>,
    #[cfg(target_os = "macos")]
    isolates: Option<Vec<BidiIsolate>>,
    folds: Vec<Range<usize>>,
    inline_caret: Option<usize>,
    previews: std::collections::HashMap<usize, Result<Graphic, &'static str>>,
}

impl Projection<'_> {
    fn add(
        &self,
        builder: &mut LayoutBuilder<'_>,
        range: Range<usize>,
        sections: &[Section],
        face: Face,
        depth: usize,
    ) {
        use SectionKind as K;
        let text = self.state.text().as_str();
        if depth >= 32 {
            builder.add_text(text, range, face);
            return;
        }
        let mut start = range.start;
        for (index, section) in sections.iter().enumerate() {
            if section.source.start < start || section.source.end > range.end {
                continue;
            }
            builder.add_text(text, start..section.source.start, face);

            let source = &text[section.source.clone()];
            let body = &text[section.content.clone()];
            let face_muted = Face {
                muted: true,
                ..face
            };
            if let Some(active) = self.active.filter(|active| active.source == section.source) {
                builder.add_source(active, self);
                self.finish_line(builder, active.content.end, section);
            } else {
                match section.kind {
                    K::Heading | K::Bullet | K::Strong | K::Emphasis => {
                        let mut next = face;
                        match section.kind {
                            K::Heading => {
                                next.heading = true;
                                if builder.row.items.is_empty() {
                                    builder.row.start = section.content.start;
                                    builder.row.end = section.content.start;
                                }
                            }
                            K::Bullet => {
                                builder.text(
                                    "•  ",
                                    section.source.start..section.content.start,
                                    face,
                                    None,
                                    true,
                                );
                            }
                            K::Strong => next.bold = true,
                            K::Emphasis => next.italic = true,
                            _ => unreachable!(),
                        }
                        self.add(
                            builder,
                            section.content.clone(),
                            &sections[index + 1..],
                            next,
                            depth + 1,
                        );
                        if matches!(section.kind, K::Strong | K::Emphasis) {
                            builder.row.end = section.source.end;
                        } else if section.kind == K::Heading {
                            let suffix = &text[section.content.end..section.source.end];
                            let same_line =
                                suffix.split('\n').next().unwrap().trim_end_matches('\r');
                            builder.row.end =
                                builder.row.end.max(section.content.end + same_line.len());
                        }
                        self.finish_line(builder, section.content.end, section);
                    }
                    K::InlineCode => {
                        let face = Face { code: true, ..face };
                        if self.inline_caret.is_some_and(|caret| {
                            section.source.start < caret && caret < section.source.end
                        }) {
                            builder.add_text(text, section.source.clone(), face);
                        } else {
                            if let Some(code) = &section.inline_code {
                                builder.text(code, section.content.clone(), face, None, true);
                            } else {
                                builder.add_text(text, section.content.clone(), face);
                            }
                            builder.row.end = section.source.end;
                        }
                    }
                    K::DraftCode => builder.add_text(text, section.source.clone(), face),
                    K::ReadOnly => {
                        if let Some(preview) = &section.preview {
                            let block = BlockLayout::new(
                                preview,
                                &section.source,
                                self.state,
                                builder.theme,
                                builder.window,
                            );
                            builder.item(
                                section.source.clone(),
                                None,
                                true,
                                block.width,
                                block.height,
                                ItemPaint::Block(Box::new(block)),
                            );
                            self.finish_line(builder, section.source.start, section);
                        } else {
                            builder.add_text(text, section.source.clone(), face_muted);
                        }
                    }
                    K::Link => {
                        let text = if body.is_empty() {
                            section.destination.as_deref().unwrap_or("Link")
                        } else {
                            body
                        };
                        builder.text(
                            &one_line(text),
                            section.source.clone(),
                            face,
                            Some(section.clone()),
                            true,
                        );
                    }
                    K::FrontMatter => {
                        let label = front_matter(body)
                            .unwrap_or_else(|| "Invalid front matter — click to edit".into());
                        builder.text(
                            &label,
                            section.source.clone(),
                            face_muted,
                            Some(section.clone()),
                            true,
                        );
                        self.finish_line(builder, section.content.end, section);
                    }
                    _ => {
                        let preview = self.previews.get(&section.source.start);
                        if let Some(Ok(graphic)) = preview {
                            let ratio = if section.kind == K::Image {
                                (px(640.) / graphic.width).min(1.)
                            } else {
                                1.
                            };
                            let mut graphic = graphic.clone();
                            graphic.width *= ratio;
                            graphic.height *= ratio;
                            let framed = is_block_preview(section.kind);
                            builder.item(
                                section.source.clone(),
                                Some(section.clone()),
                                true,
                                graphic.width + px(if framed { 24. } else { 0. }),
                                graphic.height + px(if framed { 24. } else { 8. }),
                                ItemPaint::Graphic(graphic),
                            );
                        } else {
                            let reason = match preview {
                                Some(Err(reason)) => *reason,
                                _ => "Preview unavailable",
                            };
                            let label = if section.kind == K::Image && !body.is_empty() {
                                format!("[{reason}: {}]", one_line(body))
                            } else {
                                reason.into()
                            };
                            builder.text(
                                &label,
                                section.source.clone(),
                                face_muted,
                                Some(section.clone()),
                                true,
                            );
                        }
                        if source.ends_with('\n') {
                            self.finish_line(builder, section.content.end, section);
                        }
                    }
                }
            }
            start = section.source.end;
        }
        builder.add_text(text, start..range.end, face);
    }

    fn finish_line(&self, builder: &mut LayoutBuilder<'_>, content_end: usize, section: &Section) {
        let end = section.source.end;
        let ends_line =
            end > content_end && self.state.text().as_str().as_bytes().get(end - 1) == Some(&b'\n');
        if section.kind == SectionKind::Heading {
            // Source navigation includes the hidden Setext underline, but caret motion does not.
            builder.row.hidden_end = end - usize::from(ends_line);
        }
        if ends_line {
            builder.next_row(end);
        }
    }
}

fn is_block_preview(kind: SectionKind) -> bool {
    matches!(kind, SectionKind::Mermaid | SectionKind::DisplayMath)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn front_matter(text: &str) -> Option<String> {
    if text.len() > 16 * 1024 {
        return None;
    }
    let fields: serde_yaml_ng::Mapping = serde_yaml_ng::from_str(text).ok()?;
    let mut line = String::from("Front matter");
    for (name, value) in fields {
        let name = serde_yaml_ng::to_string(&name).ok()?;
        let value = serde_yaml_ng::to_string(&value).ok()?;
        line.push_str(&format!("    {}: {}", one_line(&name), one_line(&value)));
        if line.len() > 2048 {
            return None;
        }
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::document::BlockPreview;

    #[test]
    fn code_gap_scope_requires_the_whole_html_mount_interval() {
        use gpui_editor_codemirror::{
            language::{HighlightLayer, HighlightStyle, HighlightTheme},
            language_data::NativeLanguage,
        };

        let source = "<style> a  { } </style>";
        let document = NativeLanguage::Html
            .support()
            .parse(Text::new(source))
            .unwrap();
        let name = source.find("a ").unwrap();
        let brace = source.find('{').unwrap();
        let tags = [HighlightTag::new("content")];
        // The gap between the two spans is in CSS. The outer gaps cross the mount edges.
        let spans =
            [name..name + 1, brace..brace + 1].map(|range| HighlightSpan { range, tags: &tags });
        let underline = HighlightProperties {
            text_decoration: Some(TextDecoration::Underline),
            ..Default::default()
        };
        let layers = [HighlightLayer::Active(
            HighlightStyle::define(&[])
                .with_native_scope(NativeLanguage::Css)
                .with_all(underline),
        )];
        let highlights = code_highlights(&document, spans, |tags, scope| {
            highlighting_for_context(&layers, tags, scope, HighlightTheme::Light)
        });
        assert_eq!(
            highlights,
            [
                (name..name + 1, underline),
                (name + 1..brace, underline),
                (brace..brace + 1, underline),
            ]
        );
    }

    #[test]
    fn local_preview_checks_the_selected_workspace_identity() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let workspace = Workspace::open(&root).unwrap();
        let target = Target::Local(workspace.root().join("picture.png"));
        std::fs::rename(&root, directory.path().join("moved")).unwrap();
        std::fs::create_dir(&root).unwrap();
        let error = load_preview_image(&target, Some(&workspace)).unwrap_err();
        assert!(error.contains("workspace folder has moved"));
    }

    #[gpui::test]
    fn prose_prefers_a_word_boundary_to_a_grapheme_boundary(cx: &mut TestAppContext) {
        let (_editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.update(|window, _| {
            let prefix = "First word ";
            let text = "First word continuation";
            let theme = Theme::for_editor(window, &EditorState::new(""));
            let mut probe = LayoutBuilder::new(window, theme, None, px(300.));
            probe.add_text(text, 0..prefix.len(), Face::default());
            let width = probe.row.width + px(14.);
            let mut builder = LayoutBuilder::new(window, theme, None, width);
            builder.add_text(text, 0..text.len(), Face::default());
            let layout = builder.build(Vec::new());
            assert_eq!(layout.rows[0].end, prefix.len());
            assert_eq!(layout.rows[1].start, prefix.len());
            assert_eq!(layout.step(prefix.len(), false), prefix.len() - 1);
        });
    }

    #[gpui::test]
    fn syntax_colors_clip_to_a_row_and_keep_ime_underline(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.update(|window, _| {
            let theme = Theme::for_window(window);
            let template = TextRun {
                len: 7,
                color: theme.text.into(),
                font: window.text_style().font(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            // Width is measured on one set of runs; a wrap can clip that run around `字`.
            let source = "ab字cd";
            let highlights = plain_text_highlights(
                &[highlight::Span {
                    range: 1..6,
                    kind: highlight::Kind::String,
                }],
                theme,
            );
            let runs = text_runs(&template, 2..source.len(), Some(&(5..7)), &highlights);
            let string = highlight::Kind::String.color(theme).into();
            let actual: Vec<_> = runs
                .iter()
                .map(|run| (run.len, run.color, run.underline.is_some()))
                .collect();
            assert_eq!(
                actual,
                [
                    (3, string, false),
                    (1, string, true),
                    (1, theme.text.into(), true)
                ]
            );
        });
    }

    #[gpui::test]
    fn plain_text_shapes_markdown_syntax_as_literal_text(cx: &mut TestAppContext) {
        let source = "# **Title**\r\n> `[raw]`\r\n";
        let (editor, cx) = cx.add_window_view(|window, cx| {
            let mut editor = Editor::new(source.into(), window, cx);
            editor.set_path(Some("Makefile".into()), None);
            editor
        });
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                let layout = e.shape_for_width(window, cx, px(400.));
                assert_eq!(layout.rows.len(), 3);
                for (row, expected) in layout.rows.iter().zip(["# **Title**", "> `[raw]`"]) {
                    let [item] = row.items.as_slice() else { panic!("expected one text item"); };
                    assert!(matches!(&item.paint, ItemPaint::Text(line) if line.text.as_ref() == expected));
                    assert_eq!(&source[item.source.clone()], expected);
                }
            });
        });
    }

    #[gpui::test]
    fn multiline_inline_code_uses_the_same_row_and_monospace_font(cx: &mut TestAppContext) {
        let source = "**Before `` a\nb `` after**";
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                let layout = e.shape_for_width(window, cx, px(400.));
                assert_eq!(layout.rows.len(), 1);
                let code = layout.rows[0]
                    .items
                    .iter()
                    .find_map(|item| match &item.paint {
                        ItemPaint::Text(line) if line.text.as_ref() == "a b" => Some(line),
                        _ => None,
                    })
                    .unwrap();
                let mut font = window.text_style().font();
                font.family = CODE_FONT_FAMILY.into();
                font.weight = FontWeight::BOLD;
                let expected = window.text_system().shape_line(
                    "a b".into(),
                    px(16.),
                    &[TextRun {
                        len: 3,
                        font,
                        color: Theme::for_window(window).text.into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                );
                assert_eq!(code.runs[0].font_id, expected.runs[0].font_id);
                assert_eq!(e.source(), source);
            });
        });
    }

    #[gpui::test]
    fn editable_items_have_a_label_and_frame_only_in_source_mode(cx: &mut TestAppContext) {
        use SectionKind as K;
        for (kind, label, source) in [
            (
                K::FrontMatter,
                "<front-matter>",
                "---\ntitle: Test\n---\nAfter",
            ),
            (
                K::Mermaid,
                "<mermaid>",
                "```mermaid\nflowchart LR\nA --> B\n```\nAfter",
            ),
            (K::InlineMath, "<inline-math>", "Before $x^2$ after"),
            (K::DisplayMath, "<display-math>", "$$\nx^2\n$$\nAfter"),
            (K::Image, "<image>", "Before ![Alt](picture.png) after"),
            (
                K::Link,
                "<link>",
                "Before [test](https://example.com/long-path) after",
            ),
        ] {
            let (editor, cx) =
                cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
            cx.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    let width = px(112.);
                    assert!(
                        editor
                            .shape_for_width(window, cx, width)
                            .source_frame
                            .is_none()
                    );
                    let section = editor
                        .markdown()
                        .sections()
                        .into_iter()
                        .find(|section| section.kind == kind)
                        .unwrap();
                    editor.activate(section, true, cx);
                    let layout = editor.shape_for_width(window, cx, width);
                    let frame = layout.source_frame.as_ref().expect(label);
                    let content = &editor.active.as_ref().unwrap().content;
                    assert_eq!(frame.label.text.as_ref(), label);
                    assert_eq!(&frame.content, content);
                    assert_eq!(editor.selection(), *content);
                    for offset in [content.start, content.end] {
                        let caret = layout.bounds_at(offset, false).unwrap();
                        assert!(
                            frame.bounds.contains(&caret.center()),
                            "{label} at {offset}"
                        );
                        assert_eq!(layout.index_at(caret.origin), offset);
                    }
                    let inside = layout
                        .rows
                        .iter()
                        .filter(|row| row.y > frame.bounds.top() && row.y < frame.bounds.bottom());
                    for item in inside.flat_map(|row| &row.items) {
                        assert!(item.left >= SOURCE_PADDING);
                        assert!(item.left + item.width <= frame.bounds.right() - SOURCE_PADDING);
                        assert!(
                            content.start <= item.source.start && item.source.end <= content.end
                        );
                    }
                    let suffix = source.rfind("fter").unwrap();
                    assert!(layout.bounds_at(suffix, false).unwrap().top() > frame.bounds.bottom());
                    assert_eq!(editor.source(), source);
                    editor.finish_active();
                    assert!(
                        editor
                            .shape_for_width(window, cx, width)
                            .source_frame
                            .is_none()
                    );
                });
            });
        }
    }

    #[gpui::test]
    fn active_source_line_covers_wrapped_rows_but_not_other_lines(cx: &mut TestAppContext) {
        let source = "Before\r\n\r\n```js\r\n// first line with enough words to wrap at least once\r\nconst other = 1;\r\n```\r\n\r\nAfter";
        let start = source.find("// first").unwrap();
        let end = start + source[start..].find('\r').unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                let section = editor
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                    .unwrap();
                let scope = section.source;
                editor.activate(
                    Section::new(SectionKind::DraftCode, scope.clone(), scope),
                    false,
                    cx,
                );
                let layout = editor.shape_for_width(window, cx, px(130.));
                let frame = layout.source_frame.as_ref().unwrap();
                let gutter = frame.gutter.as_ref().unwrap();
                assert_eq!(gutter.body.start, start);
                let mut line_numbers = Vec::new();
                let mut painted = 0;
                for row in &layout.rows {
                    if let Some((number, first_row)) = gutter.line_at_row(row) {
                        let item = row.items.first().unwrap();
                        assert_eq!(item.left, SOURCE_PADDING + gutter.width);
                        assert_eq!(
                            layout.index_at(point(SOURCE_PADDING, row.y + px(5.))),
                            row.start
                        );
                        if first_row {
                            line_numbers.push(gutter.label(number).unwrap());
                        }
                    } else if let Some(item) = row.items.first()
                        && item.source.start >= frame.content.start
                        && item.source.end <= frame.content.end
                    {
                        assert_eq!(item.left, SOURCE_PADDING);
                    }
                    let bounds = frame.active_line_bounds(
                        layout.origin,
                        row,
                        std::slice::from_ref(&(start..end)),
                    );
                    if (start..=end).contains(&row.start) {
                        let bounds = bounds.unwrap();
                        assert_eq!(bounds.top(), layout.origin.y + row.y);
                        assert_eq!(bounds.size.height, row.height);
                        painted += 1;
                    } else {
                        assert!(bounds.is_none());
                    }
                }
                assert!(painted > 1, "expected multiple visual rows");
                assert_eq!(line_numbers, ["1", "2"]);
                assert_eq!(editor.source(), source);
            });
        });
    }

    #[gpui::test]
    fn source_line_numbers_and_caret_stops_use_mixed_editor_breaks(cx: &mut TestAppContext) {
        let prefix = "Intro\r\n\r\n```json\r\n";
        let body = "{\r  \"word\": \"字\",\r\n\r  \"other\": false\r}";
        let source = format!("{prefix}{body}\r\n```\r\n\r\nAfter");
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.clone(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                let block = editor
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                    .unwrap();
                let scope = block.source;
                editor.activate(
                    Section::new(SectionKind::DraftCode, scope.clone(), scope),
                    false,
                    cx,
                );
                let layout = editor.shape_for_width(window, cx, px(450.));
                let frame = layout.source_frame.as_ref().unwrap();
                let gutter = frame.gutter.as_ref().unwrap();
                let actual: Vec<_> = layout
                    .rows
                    .iter()
                    .filter_map(|row| {
                        let (number, first) = gutter.line_at_row(row)?;
                        first.then(|| (gutter.label(number).unwrap(), row.start..row.end))
                    })
                    .collect();
                let mut starts = [0; 5];
                let mut ends = [0; 5];
                let mut cursor = 0;
                for (index, (line, separator)) in [
                    ("{", "\r"),
                    ("  \"word\": \"字\",", "\r\n"),
                    ("", "\r"),
                    ("  \"other\": false", "\r"),
                    ("}", ""),
                ]
                .into_iter()
                .enumerate()
                {
                    starts[index] = prefix.len() + cursor;
                    cursor += line.len();
                    ends[index] = prefix.len() + cursor;
                    cursor += separator.len();
                }
                let expected: Vec<_> = (0..5)
                    .map(|index| ((index + 1).to_string(), starts[index]..ends[index]))
                    .collect();
                assert_eq!(actual, expected);
                let selection = EditorSelection::single(starts[3], starts[3]);
                let lines = layout.active_source_lines(editor.view.state().text(), &selection);
                assert_eq!(
                    lines,
                    std::iter::once(starts[3]..ends[3]).collect::<Vec<_>>()
                );
                let painted: Vec<_> = layout
                    .rows
                    .iter()
                    .filter(|row| {
                        frame
                            .active_line_bounds(layout.origin, row, &lines)
                            .is_some()
                    })
                    .map(|row| row.start)
                    .collect();
                assert_eq!(painted, [starts[3]]);
                for (end, next) in ends[..4].iter().copied().zip(starts[1..].iter().copied()) {
                    assert_eq!(layout.step(end, true), next);
                    assert_eq!(layout.step(next, false), end);
                }
                assert_eq!(editor.source(), source);
                assert_eq!(editor.view.revision(), 0);
                assert!(!editor.view.can_undo());
            });
        });
    }

    #[gpui::test]
    fn collapsed_source_rows_keep_the_original_line_numbers(cx: &mut TestAppContext) {
        let source =
            "```js\r\nconst data = {\r\n  a: '漢',\r\n  b: 2,\r\n};\r\n\r\nconst next = 0;\r\n```";
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(source.into(), window, cx));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                let section = editor
                    .markdown()
                    .sections()
                    .into_iter()
                    .find(|section| matches!(section.preview, Some(BlockPreview::Code { .. })))
                    .unwrap();
                let layout = editor.shape_for_width(window, cx, px(600.));
                assert!(layout.source_frame.is_none());
                let scope = section.source;
                editor.activate(
                    Section::new(SectionKind::DraftCode, scope.clone(), scope),
                    false,
                    cx,
                );
                let numbers = |layout: &Layout| {
                    let gutter = layout
                        .source_frame
                        .as_ref()
                        .unwrap()
                        .gutter
                        .as_ref()
                        .unwrap();
                    layout
                        .rows
                        .iter()
                        .filter_map(|row| {
                            let (number, first) = gutter.line_at_row(row)?;
                            first.then(|| gutter.label(number).unwrap())
                        })
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    numbers(&editor.shape_for_width(window, cx, px(600.))),
                    ["1", "2", "3", "4", "5", "6"]
                );
                let offset = source.find('{').unwrap() + 1;
                editor.set_selection(offset, offset);
                assert!(editor.toggle_source_fold(cx));
                assert_eq!(
                    numbers(&editor.shape_for_width(window, cx, px(600.))),
                    ["1", "5", "6"]
                );
                assert_eq!(editor.source(), source);
                assert!(!editor.view.can_undo());
            });
        });
    }

    #[gpui::test]
    fn image_budget_rejections_retry_only_after_freeing_space(cx: &mut TestAppContext) {
        let (editor, cx) = cx.add_window_view(|window, cx| Editor::new(String::new(), window, cx));
        cx.update(|_, cx| {
            editor.update(cx, |e, _| {
                let graphic = |mib: usize| {
                    let mut graphic = Graphic::new(
                        Raster {
                            width: 1,
                            height: 1,
                            bgra: vec![0; 4],
                        },
                        1.,
                    )
                    .unwrap();
                    graphic.bytes = mib * 1024 * 1024;
                    graphic
                };
                e.images
                    .insert("large.png".into(), ImageEntry::Ready(Ok(graphic(63))));
                e.images.insert(
                    "broken.png".into(),
                    ImageEntry::Ready(Err("invalid image".into())),
                );
                e.images.insert("other.png".into(), ImageEntry::Loading(1));
                e.image_requests = 1;
                e.complete_image_request("other.png".into(), 1, Ok(graphic(3)));
                assert!(matches!(
                    e.images.get("other.png"),
                    Some(ImageEntry::BudgetExceeded)
                ));

                retain_images(
                    &mut e.images,
                    &HashSet::from(["large.png", "other.png", "broken.png"]),
                );
                assert!(matches!(
                    e.images.get("other.png"),
                    Some(ImageEntry::BudgetExceeded)
                ));
                retain_images(&mut e.images, &HashSet::from(["other.png", "broken.png"]));
                assert!(!e.images.contains_key("other.png"));
                assert!(matches!(
                    e.images.get("broken.png"),
                    Some(ImageEntry::Ready(Err(_)))
                ));

                e.images.insert("other.png".into(), ImageEntry::Loading(2));
                e.image_requests = 1;
                e.complete_image_request("other.png".into(), 2, Ok(graphic(3)));
                assert!(matches!(
                    e.images.get("other.png"),
                    Some(ImageEntry::Ready(Ok(_)))
                ));
                assert_eq!(e.image_requests, 0);
            })
        });
    }
}
