//! Live text layout for opted-in syntax isolates in a code fence. Keep the
//! Markdown fence markers outside the TextKit source and bitmap.

use std::{cell::RefCell, collections::BTreeMap, ops::Range, rc::Rc, sync::Arc};

use gpui::{
    App, Bounds, ContentMask, Edges, EntityId, Pixels, Point, RenderImage, TextRun, Window, px,
};
use gpui_editor_codemirror::{
    Assoc, EditorSelection, EditorState, Precedence, SelectionRange, Text,
    language::{BidiIsolateOptions, StreamSettings, configured_bidi_isolates},
    language_data::{NativeLanguage, NativeSyntax},
    view::{
        Viewport,
        bidi::{BIDI_ISOLATED_RANGES, BidiIsolate, line_isolates},
    },
};
use unicode_segmentation::UnicodeSegmentation;

use super::{
    DEFAULT_LINE_HEIGHT, Editor, Item, ItemPaint, Layout, LayoutBuilder, Row, text_runs,
    textkit::{MAX_ROWS, MAX_SOURCE_BYTES, TextKitLayout, TextKitOptions, TextKitRow},
};

type IsolatedRows = Vec<Rc<IsolatedRow>>;
pub(in crate::editor) type BodyCache = Rc<RefCell<CachedBody>>;

#[derive(Default)]
pub(in crate::editor) struct CachedBody {
    shape: Option<(ShapeInput, Option<IsolatedRows>)>,
    painted: Option<Arc<RenderImage>>,
}

#[derive(PartialEq)]
struct ShapeInput {
    origin: usize,
    source: String,
    isolates: Vec<BidiIsolate>,
    runs: Vec<TextRun>,
    options: TextKitOptions,
}

impl ShapeInput {
    fn shape(&self) -> Option<IsolatedRows> {
        let layout = Rc::new(
            TextKitLayout::shape(&self.source, &self.isolates, &self.runs, &self.options).ok()?,
        );
        IsolatedRow::all(layout, self.origin)
    }
}

impl CachedBody {
    pub(super) fn for_window(editor: EntityId, window: &mut Window, cx: &mut App) -> BodyCache {
        let state = window.use_keyed_state(("source-isolation", editor), cx, |window, cx| {
            cx.on_release_in(window, |cache: &mut BodyCache, window, cx| {
                cache.borrow_mut().paint_image(None, window, cx);
            })
            .detach();
            BodyCache::default()
        });
        state.read(cx).clone()
    }

    fn shape(&mut self, input: ShapeInput) -> Option<IsolatedRows> {
        if let Some((previous, rows)) = &self.shape
            && *previous == input
        {
            return rows.clone();
        }
        let rows = input.shape();
        self.shape = Some((input, rows.clone()));
        rows
    }

    fn paint_image(&mut self, next: Option<Arc<RenderImage>>, window: &Window, cx: &mut App) {
        if let (Some(previous), Some(next)) = (&self.painted, &next)
            && Arc::ptr_eq(previous, next)
        {
            return;
        }
        if let Some(previous) = std::mem::replace(&mut self.painted, next) {
            // The current GPUI scene can still refer to the old sprite until
            // the new frame is installed. Retire it outside the paint phase.
            window.defer(cx, move |window, _| {
                let _ = window.drop_image(previous);
            });
        }
    }
}

impl Layout {
    pub(in crate::editor) fn prepare_isolated_paint(
        &self,
        cache: Option<&BodyCache>,
        window: &Window,
        cx: &mut App,
    ) {
        let Some(cache) = cache else {
            return;
        };
        let image = self
            .rows
            .iter()
            .find_map(Row::isolated_item)
            .map(|(_, native)| native.layout.image.clone());
        let mut cache = cache.borrow_mut();
        if image.is_none() && cache.shape.as_ref().is_some_and(|(_, rows)| rows.is_some()) {
            cache.shape = None;
        }
        cache.paint_image(image, window, cx);
    }
}

struct Stop {
    before: Pixels,
    after: Pixels,
    primary: Pixels,
}

struct Cluster {
    bytes: Range<usize>,
    start: Pixels,
    end: Pixels,
}

pub(in crate::editor) struct IsolatedRow {
    layout: Rc<TextKitLayout>,
    index: usize,
    origin: usize,
    visible: Range<usize>,
    stops: BTreeMap<usize, Stop>,
    soft_start: bool,
    soft_end: bool,
    visual: Vec<Cluster>,
}

impl Editor {
    pub(super) fn source_isolates(&mut self) -> Option<Vec<BidiIsolate>> {
        if !self.input_enabled || self.is_composing() || self.view.state().read_only() {
            return None;
        }
        let always_isolate = configured_bidi_isolates(self.view.state())?;
        let fence = self.source_fence()?;
        let parser = fence.parser?;
        let language = parser.language();
        if !matches!(
            language,
            NativeLanguage::Java
                | NativeLanguage::Json
                | NativeLanguage::Wast
                | NativeLanguage::Sql(_)
        ) {
            return None;
        }
        let body = fence.body;
        let active = self.active.as_ref()?;
        let source = self.source();
        let header_start = active.content.start;
        let leading = &source[..header_start];
        // Use complete, standalone fences. Do not give nested Markdown
        // prefixes, fence separators, or an unterminated draft to TextKit.
        if !leading.rsplit('\n').next()?.bytes().all(|b| b == b' ')
            || !source
                .get(body.end..active.content.end)?
                .starts_with(['\r', '\n'])
            || !supported_body(source.get(body.clone())?)
        {
            return None;
        }
        let settings = StreamSettings {
            tab_size: self.view.state().tab_size(),
            ..Default::default()
        };
        let syntax = self.source_document(&body, &parser, settings)?.syntax();
        let viewport = Viewport::complete(syntax.text());
        let options = BidiIsolateOptions {
            always_isolate,
            ..Default::default()
        };
        let (lines, marks) = match syntax {
            NativeSyntax::Java(doc) => (
                doc.syntax_lines(),
                doc.bidi_isolate_marks(&viewport, options).ok()?,
            ),
            NativeSyntax::Json(doc) => (
                doc.syntax_lines(),
                doc.bidi_isolate_marks(&viewport, options).ok()?,
            ),
            NativeSyntax::Wast(doc) => (
                doc.syntax_lines(),
                doc.bidi_isolate_marks(&viewport, options).ok()?,
            ),
            NativeSyntax::Sql(doc) => {
                (doc.text(), doc.bidi_isolate_marks(&viewport, options).ok()?)
            }
            _ => return None,
        };
        if marks.is_empty() {
            return None;
        }
        let state = EditorState::create_from_text(
            lines.clone(),
            None,
            BIDI_ISOLATED_RANGES
                .of(marks)
                .with_precedence(Precedence::Lowest),
        )
        .ok()?;
        let mut isolates = Vec::new();
        for number in 1..=lines.line_count() {
            let line = lines.line(number)?;
            for mut isolate in line_isolates(&state, number).ok()? {
                offset_isolate(&mut isolate, line.range.start);
                isolates.push(isolate);
            }
        }
        (!isolates.is_empty()).then_some(isolates)
    }
}

fn supported_body(source: &str) -> bool {
    !source.is_empty()
        && source.len() <= MAX_SOURCE_BYTES
        && source
            .bytes()
            .filter(|&byte| byte == b'\n')
            .take(MAX_ROWS)
            .count()
            < MAX_ROWS
        && !source.char_indices().any(|(byte, ch)| match ch {
            '\r' => source.as_bytes().get(byte + 1) != Some(&b'\n'),
            '\t'
            | '\u{0000}'
            | '\u{000b}'
            | '\u{000c}'
            | '\u{001c}'..='\u{001e}'
            | '\u{0085}'
            | '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}' => true,
            _ => false,
        })
}

fn offset_isolate(isolate: &mut BidiIsolate, offset: usize) {
    isolate.from += offset;
    isolate.to += offset;
    for child in &mut isolate.inner {
        offset_isolate(child, offset);
    }
}

impl LayoutBuilder<'_> {
    pub(super) fn add_isolated(
        &mut self,
        source: &str,
        body: Range<usize>,
        isolates: &[BidiIsolate],
    ) -> Option<()> {
        if self.row.start != body.start || !self.row.items.is_empty() {
            return None;
        }
        let options = TextKitOptions {
            width: self.wrap_width - self.inset,
            margins: Edges::default(),
            font_size: px(16.),
            line_height: DEFAULT_LINE_HEIGHT,
            scale_factor: self.window.scale_factor(),
        };
        let template = TextRun {
            len: body.len(),
            font: self.window.text_style().font(),
            color: self.theme.text.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = text_runs(&template, body.clone(), None, self.highlights);
        let input = ShapeInput {
            origin: body.start,
            source: source[body].into(),
            isolates: isolates.to_vec(),
            runs,
            options,
        };
        // Validate every row before replacing any of the GPUI body rows.
        let rows = if let Some(cache) = &self.isolation_cache {
            cache.borrow_mut().shape(input)
        } else {
            input.shape()
        }?;
        let mut rows = rows.into_iter().peekable();
        while let Some(native) = rows.next() {
            let bounds = native.bounds();
            self.row.height = bounds.size.height;
            self.row.soft_end = native.soft_end;
            self.row.end = native.origin + native.visible.end;
            if native.selection_source().end == self.row.end + 2 {
                self.row.hidden_end = self.row.end + 1;
            }
            let width = native.width();
            self.row.width = self.inset + width;
            self.row.items.push(Item {
                source: native.origin + native.visible.start..self.row.end,
                target: None,
                atomic: false,
                bidi: None,
                left: self.inset,
                width,
                paint: ItemPaint::Isolated(native),
            });
            if let Some(next) = rows.peek() {
                self.next_row(next.origin + next.visible.start);
            }
        }
        Some(())
    }
}

impl IsolatedRow {
    fn all(layout: Rc<TextKitLayout>, origin: usize) -> Option<IsolatedRows> {
        let rows = layout.rows();
        let mut previous_byte = 0;
        let mut y = px(0.);
        let mut soft_start = false;
        let boundaries: Vec<_> = layout
            .source()
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .chain([layout.source().len()])
            .collect();
        let mut result = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            if row.bytes.start != previous_byte
                || row.bounds.top() != y
                || row.bounds.left() != px(0.)
                || row.bounds.size.width != layout.size.width
            {
                return None;
            }
            let piece = layout.source().get(row.bytes.clone())?;
            let (visible_text, hard_end) =
                piece.strip_suffix('\n').map_or((piece, false), |text| {
                    (text.strip_suffix('\r').unwrap_or(text), true)
                });
            if visible_text.contains(['\r', '\n']) {
                return None;
            }
            let visible = row.bytes.start..row.bytes.start + visible_text.len();
            let soft_end = !hard_end && index + 1 < rows.len();
            let stops = stops(&layout, row, &visible, &boundaries, soft_start, soft_end)?;
            let visual = if !soft_start && !soft_end {
                visual_clusters(&stops)
            } else {
                Vec::new()
            };
            result.push(Rc::new(Self {
                layout: layout.clone(),
                index,
                origin,
                visible,
                soft_start,
                soft_end,
                stops,
                visual,
            }));
            previous_byte = row.bytes.end;
            y = row.bounds.bottom();
            soft_start = soft_end;
        }
        (previous_byte == layout.source().len() && y == layout.size.height && !result.is_empty())
            .then_some(result)
    }

    fn bounds(&self) -> Bounds<Pixels> {
        self.layout.rows()[self.index].bounds
    }

    pub(super) fn height(&self) -> Pixels {
        self.bounds().size.height
    }

    fn width(&self) -> Pixels {
        self.stops
            .values()
            .map(|stop| stop.before.max(stop.after))
            .fold(px(0.), Pixels::max)
    }

    pub(super) fn selection_source(&self) -> Range<usize> {
        let bytes = &self.layout.rows()[self.index].bytes;
        self.origin + bytes.start..self.origin + bytes.end
    }

    pub(in crate::editor) fn paint(&self, top: Point<Pixels>, window: &mut Window) {
        let row = self.bounds();
        let origin = top - row.origin;
        let clip = Bounds::new(top, row.size);
        let image = self.layout.image_bounds;
        window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
            let _ = window.paint_image(
                Bounds::new(origin + image.origin, image.size),
                Default::default(),
                self.layout.image.clone(),
                0,
                false,
            );
        });
    }

    pub(super) fn x_at(&self, offset: usize, assoc: Option<Assoc>) -> Pixels {
        let byte = offset
            .saturating_sub(self.origin)
            .clamp(self.visible.start, self.visible.end);
        let stop = if assoc == Some(Assoc::After) {
            self.stops.range(byte..).next()
        } else {
            self.stops.range(..=byte).next_back()
        }
        .expect("validated row endpoints")
        .1;
        match assoc {
            Some(Assoc::Before) => stop.before,
            Some(Assoc::After) => stop.after,
            None => stop.primary,
        }
    }

    pub(super) fn logical_stops(&self) -> Vec<usize> {
        self.stops.keys().map(|b| self.origin + b).collect()
    }

    pub(super) fn hit(&self, at: Point<Pixels>) -> Option<SelectionRange> {
        let row = self.bounds();
        let at = at + row.origin;
        let (_, native) = self.layout.hit_where(at, |caret| {
            self.stops
                .get(&caret.byte)
                .is_some_and(|stop| caret.x == stop.before || caret.x == stop.after)
        })?;
        let byte = native.byte;
        let stop = &self.stops[&byte];
        let assoc = if self.soft_start && byte == self.visible.start {
            Assoc::After
        } else if (self.soft_end && byte == self.visible.end)
            || (stop.before != stop.after && native.x == stop.before)
        {
            Assoc::Before
        } else {
            Assoc::After
        };
        Some(EditorSelection::cursor(
            self.origin + byte,
            Some(assoc),
            None,
            None,
        ))
    }

    pub(super) fn x_spans(&self, range: Range<usize>) -> Vec<Range<Pixels>> {
        let bytes = self.selection_source();
        let start = range.start.max(bytes.start);
        let end = range.end.min(bytes.end);
        if start >= end {
            return Vec::new();
        }
        let row = self.bounds();
        let source = self.layout.source().as_bytes();
        let start = range.start.saturating_sub(self.origin);
        let end = (range.end - self.origin).min(source.len());
        let middle_of_crlf =
            |byte: usize| byte > 0 && source.get(byte - 1..byte + 1) == Some(b"\r\n");
        let start = start - usize::from(middle_of_crlf(start));
        let end = end + usize::from(middle_of_crlf(end));
        // Keep the whole selection during the query. Native selection can
        // extend to the line margin when the range crosses a wrap or break.
        self.layout
            .selection_rects(start..end)
            .into_iter()
            .flatten()
            .filter_map(|rect| {
                if row.top() >= rect.bottom() || rect.top() >= row.bottom() {
                    return None;
                }
                let x = rect.left().max(row.left())..rect.right().min(row.right());
                (!x.is_empty()).then_some(x)
            })
            .collect()
    }

    pub(super) fn has_visual_motion(&self) -> bool {
        !self.visual.is_empty()
    }

    pub(super) fn visual_step<'a>(
        &self,
        text: &'a Text,
        range: SelectionRange,
        forward: bool,
    ) -> Option<(SelectionRange, &'a str)> {
        let x = self.x_at(range.head(), range.assoc());
        let cluster = self.visual.iter().find(|cluster| {
            if forward {
                x == cluster.start.min(cluster.end)
            } else {
                x == cluster.start.max(cluster.end)
            }
        })?;
        let (byte, assoc) = if forward == (cluster.start < cluster.end) {
            (cluster.bytes.end, Assoc::Before)
        } else {
            (cluster.bytes.start, Assoc::After)
        };
        let bytes = self.origin + cluster.bytes.start..self.origin + cluster.bytes.end;
        Some((
            EditorSelection::cursor(self.origin + byte, Some(assoc), None, None),
            text.slice(bytes)?,
        ))
    }
}

fn stops(
    layout: &TextKitLayout,
    row: &TextKitRow,
    visible: &Range<usize>,
    boundaries: &[usize],
    soft_start: bool,
    soft_end: bool,
) -> Option<BTreeMap<usize, Stop>> {
    let mut carets: Vec<_> = row
        .carets
        .iter()
        .copied()
        .filter(|caret| {
            (visible.start..=visible.end).contains(&caret.byte)
                && boundaries.binary_search(&caret.byte).is_ok()
        })
        .collect();
    carets.sort_by_key(|caret| caret.byte);
    if carets.first()?.byte != visible.start
        || carets.last()?.byte != visible.end
        || carets
            .iter()
            .any(|caret| !(row.bounds.left()..=row.bounds.right()).contains(&caret.x))
    {
        return None;
    }
    let first = boundaries.partition_point(|&byte| byte < visible.start);
    let last = boundaries.partition_point(|&byte| byte <= visible.end);
    let groups = carets.chunk_by(|a, b| a.byte == b.byte);
    if !groups
        .clone()
        .map(|group| group[0].byte)
        .eq(boundaries[first..last].iter().copied())
    {
        return None;
    }
    let mut logical = groups.peekable();
    let mut previous = None;
    let mut result = BTreeMap::new();
    while let Some(carets) = logical.next() {
        let byte = carets[0].byte;
        let primary = carets
            .iter()
            .find(|caret| !caret.alternate)
            .unwrap_or(&carets[0])
            .x;
        let (before, after) = match carets {
            [only] => (only.x, only.x),
            [a, b] => {
                // The SDK's alternate flag is not an association. Match the
                // native stops to the actual edges of the adjacent selection.
                let edge = |start, end| {
                    let rects = layout.selection_rects(start..end)?;
                    let matches = |x| {
                        rects.iter().any(|rect| {
                            rect.top() <= row.bounds.center().y
                                && rect.bottom() > row.bounds.center().y
                                && (near(rect.left(), x) || near(rect.right(), x))
                        })
                    };
                    match (matches(a.x), matches(b.x)) {
                        (true, false) => Some(a.x),
                        (false, true) => Some(b.x),
                        _ => None,
                    }
                };
                let other = |x| if x == a.x { b.x } else { a.x };
                let before = previous.and_then(|start| edge(start, byte));
                let after = logical.peek().and_then(|next| edge(byte, next[0].byte));
                match (before, after) {
                    (Some(before), Some(after)) if before != after => (before, after),
                    // At a wrap the other association belongs to the adjacent
                    // row. Keep only the stop on this row's side of the wrap.
                    (None, Some(after)) if byte == visible.start => {
                        (if soft_start { after } else { other(after) }, after)
                    }
                    (Some(before), None) if byte == visible.end => {
                        (before, if soft_end { before } else { other(before) })
                    }
                    _ => return None,
                }
            }
            _ => return None,
        };
        let primary = if before == after { before } else { primary };
        result.insert(
            byte,
            Stop {
                before,
                after,
                primary,
            },
        );
        previous = Some(byte);
    }
    Some(result)
}

fn near(a: Pixels, b: Pixels) -> bool {
    f32::from(a - b).abs() < 0.01
}

// Only use visual keyboard motion on a complete line whose stops define
// nonoverlapping, adjacent clusters. Otherwise retain logical motion.
fn visual_clusters(stops: &BTreeMap<usize, Stop>) -> Vec<Cluster> {
    let mut clusters: Vec<_> = stops
        .iter()
        .zip(stops.iter().skip(1))
        .map(|((&from, a), (&to, b))| Cluster {
            bytes: from..to,
            start: a.after,
            end: b.before,
        })
        .collect();
    clusters
        .sort_by(|a, b| f32::from(a.start.min(a.end)).total_cmp(&f32::from(b.start.min(b.end))));
    if !clusters.iter().any(|c| c.start > c.end)
        || clusters.iter().any(|c| c.start == c.end)
        || clusters
            .windows(2)
            .any(|pair| pair[0].start.max(pair[0].end) != pair[1].start.min(pair[1].end))
    {
        return Vec::new();
    }
    clusters
}

#[cfg(test)]
mod tests;
