use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::Arc,
};

use gpui::{
    App, Bounds, Context, Edges, FontStyle, FontWeight, Pixels, Point, RenderImage, ShapedLine,
    SharedString, TextRun, UnderlineStyle, Window, point, px, quad, size,
};
use unicode_segmentation::UnicodeSegmentation;

use super::{Editor, ImageEntry, PreviewDiagnostic};
use crate::{
    document::{Section, SectionKind},
    media::{Target, image_target, load_image},
    rendering::{Raster, render_math, render_mermaid},
    theme::Theme,
    workspace::Workspace,
};

mod blocks;

use blocks::BlockLayout;

const CODE_FONT_FAMILY: &str = if cfg!(target_os = "windows") {
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

const SOURCE_PADDING: Pixels = px(8.);
const SOURCE_HEADER: Pixels = px(24.);
const SOURCE_MARGIN: Pixels = px(4.);

pub(super) struct Layout {
    pub(super) origin: Point<Pixels>,
    pub(super) rows: Vec<Row>,
    pub(super) source_frame: Option<SourceFrame>,
}

pub(super) struct SourceFrame {
    pub(super) bounds: Bounds<Pixels>,
    content: Range<usize>,
    label: ShapedLine,
}

impl SourceFrame {
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
        window.paint_quad(quad(
            bounds,
            px(6.),
            theme.preview,
            Edges::all(px(1.)),
            theme.border,
            Default::default(),
        ));
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
        let offset = self
            .items
            .iter()
            .find(|item| x < item.left + item.width)
            .map_or(self.end, |item| item.offset(x));
        (offset, self.soft_end && offset == self.end)
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
    pub(super) left: Pixels,
    pub(super) width: Pixels,
    pub(super) paint: ItemPaint,
}

pub(super) enum ItemPaint {
    Text(Box<ShapedLine>),
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
        match &self.paint {
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

    pub(super) fn x(&self, offset: usize) -> Pixels {
        self.left
            + match &self.paint {
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

    fn stops(&self) -> Vec<usize> {
        match &self.paint {
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
    pub(super) fn row_at(&self, position: Point<Pixels>) -> &Row {
        let y = position.y - self.origin.y;
        let index = self.rows.partition_point(|row| row.y + row.height <= y);
        &self.rows[index.min(self.rows.len() - 1)]
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
        let (offset, upstream) = self.row_at(position).position_at(local.x);
        (
            frame.map_or(offset, |frame| {
                offset.clamp(frame.content.start, frame.content.end)
            }),
            upstream,
        )
    }

    pub(super) fn bounds_at(&self, offset: usize, upstream: bool) -> Option<Bounds<Pixels>> {
        let row = &self.rows[self.row_index(offset, upstream)?];
        Some(self.bounds_in_row(row, offset))
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
        (index < self.rows.len()).then_some(index)
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

    pub(super) fn step(&self, offset: usize, right: bool) -> usize {
        let stops = self.rows.iter().flat_map(|row| {
            row.items
                .iter()
                .flat_map(Item::stops)
                .chain([row.start, row.end])
        });
        if right {
            stops.filter(|stop| *stop > offset).min()
        } else {
            stops.filter(|stop| *stop < offset).max()
        }
        .unwrap_or(offset)
    }
}

struct LayoutBuilder<'a> {
    pub(super) rows: Vec<Row>,
    row: Row,
    window: &'a Window,
    theme: Theme,
    wrap_width: Pixels,
    inset: Pixels,
    source_frame: Option<SourceFrame>,
    marked: Option<Range<usize>>,
}

impl<'a> LayoutBuilder<'a> {
    fn new(window: &'a Window, marked: Option<Range<usize>>, wrap_width: Pixels) -> Self {
        Self {
            rows: Vec::new(),
            row: Self::row(0, px(0.)),
            window,
            theme: Theme::for_window(window),
            wrap_width,
            inset: px(0.),
            source_frame: None,
            marked,
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
            height: px(26.),
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
        let mut start = range.start;
        for text in source[range].split_inclusive('\n') {
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

    fn add_source(&mut self, source: &str, section: &Section) {
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
        self.wrap_width = (width - SOURCE_PADDING).max(SOURCE_PADDING + px(1.));
        self.add_text(source, content.clone(), Face::default());
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
        self.source_frame = Some(SourceFrame {
            bounds: Bounds::new(point(px(0.), top), size(right, bottom - top)),
            content,
            label,
        });
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
        let height = px(if face.heading { 42. } else { 26. });
        let window = self.window;
        let marked = self.marked.clone();
        let shape = |text: &str, source: Range<usize>| {
            Self::shape_text(
                window,
                font_size,
                &template,
                marked.as_ref(),
                text,
                source,
                atomic,
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

    fn shape_text(
        window: &Window,
        font_size: Pixels,
        template: &TextRun,
        marked: Option<&Range<usize>>,
        text: &str,
        source: Range<usize>,
        atomic: bool,
    ) -> ShapedLine {
        let mut template = template.clone();
        template.len = text.len();
        let color = template.color;
        let runs = if let Some(marked) =
            marked.filter(|m| m.start < source.end && m.end > source.start && !atomic)
        {
            let left = marked.start.saturating_sub(source.start);
            let right = marked.end.min(source.end) - source.start;
            vec![
                TextRun {
                    len: left,
                    ..template.clone()
                },
                TextRun {
                    len: right - left,
                    underline: Some(UnderlineStyle {
                        color: Some(color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..template.clone()
                },
                TextRun {
                    len: text.len() - right,
                    ..template
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![template]
        };
        window
            .text_system()
            .shape_line(SharedString::from(text.to_owned()), font_size, &runs, None)
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
            left: self.row.width,
            width,
            paint,
        });
        self.row.width += width;
        self.row.height = self.row.height.max(height);
    }

    fn build(mut self) -> Layout {
        self.rows.push(self.row);
        Layout {
            origin: point(px(0.), px(0.)),
            rows: self.rows,
            source_frame: self.source_frame,
        }
    }
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
    pub(super) fn shape(&mut self, window: &Window, cx: &mut Context<Self>) -> Layout {
        self.shape_for_width(
            window,
            cx,
            super::wrap_width(self.scroll.bounds().size.width),
        )
    }

    pub(super) fn shape_for_width(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
        width: Pixels,
    ) -> Layout {
        let scale = window.scale_factor();
        let source = self.history.state().text().as_str();
        let mut builder = LayoutBuilder::new(window, self.marked.clone(), width);
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
        let projection = Projection {
            source,
            active: self.active.as_ref(),
            inline_caret: (self.focus.is_focused(window)
                && (self.selection().is_empty() || self.is_composing()))
            .then(|| self.cursor()),
            previews,
        };
        projection.add(&mut builder, 0..source.len(), &sections, Face::default(), 0);
        builder.build()
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
    source: &'a str,
    active: Option<&'a Section>,
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
        if depth >= 32 {
            builder.add_text(self.source, range, face);
            return;
        }
        let mut start = range.start;
        for (index, section) in sections.iter().enumerate() {
            if section.source.start < start || section.source.end > range.end {
                continue;
            }
            builder.add_text(self.source, start..section.source.start, face);

            let source = &self.source[section.source.clone()];
            let body = &self.source[section.content.clone()];
            let face_muted = Face {
                muted: true,
                ..face
            };
            if let Some(active) = self.active.filter(|active| active.source == section.source) {
                builder.add_source(self.source, active);
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
                            let suffix = &self.source[section.content.end..section.source.end];
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
                            builder.add_text(self.source, section.source.clone(), face);
                        } else {
                            if let Some(code) = &section.inline_code {
                                builder.text(code, section.content.clone(), face, None, true);
                            } else {
                                builder.add_text(self.source, section.content.clone(), face);
                            }
                            builder.row.end = section.source.end;
                        }
                    }
                    K::DraftCode => builder.add_text(self.source, section.source.clone(), face),
                    K::ReadOnly => {
                        if let Some(preview) = &section.preview {
                            let block = BlockLayout::new(preview, builder.window);
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
                            builder.add_text(self.source, section.source.clone(), face_muted);
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
        builder.add_text(self.source, start..range.end, face);
    }

    fn finish_line(&self, builder: &mut LayoutBuilder<'_>, content_end: usize, section: &Section) {
        let end = section.source.end;
        let ends_line = end > content_end && self.source.as_bytes().get(end - 1) == Some(&b'\n');
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
            let mut probe = LayoutBuilder::new(window, None, px(300.));
            probe.add_text(text, 0..prefix.len(), Face::default());
            let width = probe.row.width + px(14.);
            let mut builder = LayoutBuilder::new(window, None, width);
            builder.add_text(text, 0..text.len(), Face::default());
            let layout = builder.build();
            assert_eq!(layout.rows[0].end, prefix.len());
            assert_eq!(layout.rows[1].start, prefix.len());
            assert_eq!(layout.step(prefix.len(), false), prefix.len() - 1);
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
