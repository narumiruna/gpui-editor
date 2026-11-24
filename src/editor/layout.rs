use std::{ops::Range, sync::Arc};

use gpui::{
    Bounds, Context, FontStyle, FontWeight, Pixels, Point, RenderImage, ShapedLine, SharedString,
    TextRun, UnderlineStyle, Window, point, px, rgb, size,
};
use unicode_segmentation::UnicodeSegmentation;

use super::Editor;
use crate::{
    document::{Section, SectionKind},
    media::{image_target, load_image},
    rendering::{Raster, render_math, render_mermaid},
};

#[derive(Clone, Copy, Default)]
struct Face {
    heading: bool,
    bold: bool,
    italic: bool,
    muted: bool,
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

pub(super) struct Layout {
    pub(super) origin: Point<Pixels>,
    pub(super) rows: Vec<Row>,
}

pub(super) struct Row {
    pub(super) items: Vec<Item>,
    start: usize,
    end: usize,
    pub(super) y: Pixels,
    pub(super) height: Pixels,
    pub(super) width: Pixels,
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
}

impl Item {
    pub(super) fn offset(&self, x: Pixels) -> usize {
        let x = x - self.left;
        match &self.paint {
            ItemPaint::Text(line) if !self.atomic => {
                self.source.start + line.closest_index_for_x(x)
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
        self.rows
            .iter()
            .find(|row| y < row.y + row.height)
            .unwrap_or_else(|| self.rows.last().unwrap())
    }

    pub(super) fn contains_row(&self, position: Point<Pixels>) -> bool {
        let y = position.y - self.origin.y;
        self.rows
            .last()
            .is_some_and(|row| y >= px(0.) && y < row.y + row.height)
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
        let row = self.row_at(position);
        let x = position.x - self.origin.x;
        row.items
            .iter()
            .find(|item| x < item.left + item.width)
            .map_or(row.end, |item| item.offset(x))
    }

    pub(super) fn bounds_at(&self, offset: usize) -> Option<Bounds<Pixels>> {
        let row = self.rows.iter().find(|row| offset <= row.end)?;
        let x = row
            .items
            .iter()
            .find(|item| offset <= item.source.end)
            .map_or(row.width, |item| item.x(offset));
        Some(Bounds::new(
            self.origin + point(x, row.y),
            size(px(1.), row.height),
        ))
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
    marked: Option<Range<usize>>,
}

impl<'a> LayoutBuilder<'a> {
    fn new(window: &'a Window, marked: Option<Range<usize>>) -> Self {
        Self {
            rows: Vec::new(),
            row: Self::row(0, px(0.)),
            window,
            marked,
        }
    }

    fn row(start: usize, y: Pixels) -> Row {
        Row {
            items: Vec::new(),
            start,
            end: start,
            y,
            height: px(26.),
            width: px(0.),
        }
    }

    fn next_row(&mut self, start: usize) {
        let next = Self::row(start, self.row.y + self.row.height);
        self.rows.push(std::mem::replace(&mut self.row, next));
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

    fn text(
        &mut self,
        text: &str,
        source: Range<usize>,
        face: Face,
        target: Option<Section>,
        atomic: bool,
    ) {
        let style = self.window.text_style();
        let mut font = style.font();
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
            rgb(0x145aac).into()
        } else if face.muted {
            rgb(0x656b77).into()
        } else {
            style.color
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
        let runs = if let Some(marked) = self
            .marked
            .as_ref()
            .filter(|m| m.start < source.end && m.end > source.start && !atomic)
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
        let line = self.window.text_system().shape_line(
            SharedString::from(text.to_owned()),
            px(if face.heading { 28. } else { 16. }),
            &runs,
            None,
        );
        let width = line.width;
        self.item(
            source,
            target,
            atomic,
            width,
            px(if face.heading { 42. } else { 26. }),
            ItemPaint::Text(Box::new(line)),
        );
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
        }
    }
}

impl Editor {
    pub(super) fn shape(&mut self, window: &Window, cx: &mut Context<Self>) -> Layout {
        let scale = window.scale_factor();
        let source = self.document.source();
        let mut builder = LayoutBuilder::new(window, self.marked.clone());
        let mut sections = self.document.sections();
        if let Some(active) = &self.active {
            sections.retain(|section| section.source != active.source);
            sections.push(active.clone());
            sections.sort_by_key(|section| {
                (section.source.start, std::cmp::Reverse(section.source.end))
            });
        }

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
        let mut pending_images = self.images.values().filter(|entry| entry.is_none()).count();
        let mut skip_to = 0;
        for section in &sections {
            if section.source.start < skip_to {
                continue;
            }
            if section.kind == SectionKind::ReadOnly
                || self
                    .active
                    .as_ref()
                    .is_some_and(|active| active.source == section.source)
            {
                skip_to = section.source.end;
                continue;
            }
            let body = &source[section.content.clone()];
            let kind = section.kind;
            match kind {
                SectionKind::InlineMath | SectionKind::DisplayMath | SectionKind::Mermaid => {
                    let key = (body.to_owned(), kind, scale.to_bits());
                    if self.graphics.len() >= 128 && !self.graphics.contains_key(&key) {
                        continue;
                    }
                    let graphic = self.graphics.entry(key).or_insert_with(|| {
                        let raster = match kind {
                            SectionKind::Mermaid => render_mermaid(body, scale),
                            _ => render_math(body, kind == SectionKind::DisplayMath, scale),
                        }?;
                        if graphic_bytes + raster.bgra.len() > 64 * 1024 * 1024 {
                            return Err("Preview memory limit reached".into());
                        }
                        graphic_bytes += raster.bgra.len();
                        Graphic::new(raster, scale).ok_or_else(|| "Could not decode preview".into())
                    });
                    previews.insert(section.source.start, graphic.as_ref().ok().cloned());
                }
                SectionKind::Image => {
                    let destination = section.destination.clone().unwrap_or_default();
                    if (self.images.len() >= 32 || pending_images >= 3)
                        && !self.images.contains_key(&destination)
                    {
                        continue;
                    }
                    let entry =
                        self.images.entry(destination.clone()).or_insert_with(
                            || match image_target(
                                &destination,
                                self.path.as_deref(),
                                self.workspace.as_deref(),
                            ) {
                                Err(error) => Some(Err(error)),
                                Ok(target) => {
                                    pending_images += 1;
                                    let task = cx
                                        .background_executor()
                                        .spawn(async move { load_image(&target) });
                                    cx.spawn(async move |editor, cx| {
                                        let result = task.await.and_then(|raster| {
                                            Graphic::new(raster, 1.)
                                                .ok_or_else(|| "Could not decode image".into())
                                        });
                                        let _ = editor.update(cx, |editor, cx| {
                                            let image_bytes: usize = editor
                                                .images
                                                .values()
                                                .filter_map(|entry| entry.as_ref()?.as_ref().ok())
                                                .map(|graphic| graphic.bytes)
                                                .sum();
                                            if let Some(entry) = editor.images.get_mut(&destination)
                                            {
                                                *entry = Some(result.and_then(|graphic| {
                                                    (image_bytes + graphic.bytes
                                                        <= 64 * 1024 * 1024)
                                                        .then_some(graphic)
                                                        .ok_or_else(|| {
                                                            "Image memory limit reached".into()
                                                        })
                                                }));
                                                cx.notify();
                                            }
                                        });
                                    })
                                    .detach();
                                    None
                                }
                            },
                        );
                    previews.insert(
                        section.source.start,
                        entry
                            .as_ref()
                            .and_then(|result| result.as_ref().ok())
                            .cloned(),
                    );
                }
                _ => {}
            }
        }
        let projection = Projection {
            source,
            active: self.active.as_ref(),
            previews,
        };
        projection.add(&mut builder, 0..source.len(), &sections, Face::default(), 0);
        builder.build()
    }
}

struct Projection<'a> {
    source: &'a str,
    active: Option<&'a Section>,
    previews: std::collections::HashMap<usize, Option<Graphic>>,
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
                builder.add_text(self.source, active.content.clone(), face);
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
                        self.finish_line(builder, section.content.end, section);
                    }
                    K::ReadOnly => {
                        builder.add_text(self.source, section.source.clone(), face_muted)
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
                        if let Some(graphic) = self
                            .previews
                            .get(&section.source.start)
                            .and_then(Option::as_ref)
                        {
                            let ratio = if section.kind == K::Image {
                                (px(640.) / graphic.width).min(1.)
                            } else {
                                1.
                            };
                            let mut graphic = graphic.clone();
                            graphic.width *= ratio;
                            graphic.height *= ratio;
                            builder.item(
                                section.source.clone(),
                                Some(section.clone()),
                                true,
                                graphic.width,
                                graphic.height + px(8.),
                                ItemPaint::Graphic(graphic),
                            );
                        } else {
                            let label = match section.kind {
                                K::Image => format!("[Image not loaded: {}]", one_line(body)),
                                K::Mermaid => "Invalid Mermaid — click to edit".into(),
                                _ => "Invalid formula — click to edit".into(),
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
        if end > content_end && self.source.as_bytes().get(end - 1) == Some(&b'\n') {
            builder.next_row(end);
        }
    }
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
