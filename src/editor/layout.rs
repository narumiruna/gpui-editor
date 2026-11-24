use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::Arc,
};

use gpui::{
    Bounds, Context, FontStyle, FontWeight, Pixels, Point, RenderImage, ShapedLine, SharedString,
    TextRun, UnderlineStyle, Window, point, px, rgb, size,
};
use unicode_segmentation::UnicodeSegmentation;

use super::{Editor, ImageEntry};
use crate::{
    document::{Section, SectionKind},
    media::{Target, image_target, load_image},
    rendering::{Raster, render_math, render_mermaid},
    workspace::Workspace,
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
        let row = &self.rows[self.row_index(offset)?];
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

    fn row_index(&self, offset: usize) -> Option<usize> {
        self.rows.iter().position(|row| offset <= row.end)
    }

    pub(super) fn vertical(
        &self,
        offset: usize,
        preferred_x: Option<Pixels>,
        down: bool,
    ) -> Option<(usize, Pixels)> {
        let row = self.row_index(offset)?;
        let next = if down { row + 1 } else { row.checked_sub(1)? };
        let next = self.rows.get(next)?;
        let x = preferred_x.unwrap_or(self.bounds_at(offset)?.origin.x - self.origin.x);
        let position = next
            .items
            .iter()
            .find(|item| x < item.left + item.width)
            .map_or(next.end, |item| item.offset(x));
        Some((position, x))
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
        let image_destinations: std::collections::HashSet<_> = sections
            .iter()
            .filter(|section| section.kind == SectionKind::Image)
            .filter_map(|section| section.destination.as_deref())
            .collect();
        retain_images(&mut self.images, &image_destinations);
        self.has_blocked_images = false;
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
                            }?;
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
                    } else if self.images.len() >= 32 {
                        Err("Image limit reached")
                    } else if self.image_requests >= 3 {
                        Err("Waiting for image")
                    } else {
                        match image_target(
                            &destination,
                            self.path.as_deref(),
                            self.workspace.as_ref().map(Workspace::root),
                        ) {
                            Err(error) => {
                                self.images
                                    .insert(destination, ImageEntry::Ready(Err(error)));
                                Err("Image not loaded")
                            }
                            Ok(Target::Web(_)) if !self.allow_remote_images => {
                                self.has_blocked_images = true;
                                Err("Remote image blocked")
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
                if active.content.is_empty() {
                    if builder.row.items.is_empty() {
                        builder.row.start = active.content.start;
                    }
                    builder.row.end = active.content.end;
                } else {
                    builder.add_text(self.source, active.content.clone(), face);
                }
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
                            builder.item(
                                section.source.clone(),
                                Some(section.clone()),
                                true,
                                graphic.width,
                                graphic.height + px(8.),
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
