//! Offscreen TextKit 1 layout for source with bidi isolates. The caller must
//! choose this backend for both paint and geometry, or fall back for both.

mod paint;
mod projection;

use std::{ops::Range, ptr, sync::Arc};

use gpui::{Bounds, Edges, Font, Pixels, Point, RenderImage, Rgba, Size, TextRun, point, px, size};
use gpui_editor_codemirror::view::bidi::BidiIsolate;
use objc2::{
    AnyThread,
    rc::{Retained, autoreleasepool},
};
use objc2_app_kit::{
    NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSLayoutManager,
    NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSTextContainer, NSTextStorage,
    NSWritingDirection,
};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};

use projection::Projection;

const MAX_EXTENT: f64 = 8192.;
pub(super) const MAX_SOURCE_BYTES: usize = 32 * 1024;
pub(super) const MAX_ROWS: usize = 256;
const MAX_RUNS: usize = 1024;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextKitOptions {
    /// Total available width, including both horizontal margins.
    pub(crate) width: Pixels,
    pub(crate) margins: Edges<Pixels>,
    pub(crate) font_size: Pixels,
    pub(crate) line_height: Pixels,
    pub(crate) scale_factor: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextKitError {
    InvalidOptions,
    InvalidIsolate,
    InvalidRuns,
    UnsupportedSource,
    UnsupportedStyle,
    Limit,
    NativeLayout,
    Bitmap,
}

/// One insertion position reported by TextKit. Alternate is a native label,
/// not CodeMirror association or bidi level. A byte at a wrap may be present
/// on both rows; its x coordinate is not sufficient to identify the row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TextKitCaret {
    pub(crate) byte: usize,
    pub(crate) x: Pixels,
    pub(crate) alternate: bool,
}

#[derive(Debug)]
pub(crate) struct TextKitRow {
    /// Source bytes covered by the fragment, including any line terminator.
    pub(crate) bytes: Range<usize>,
    pub(crate) bounds: Bounds<Pixels>,
    /// Native insertion positions in increasing x order. Equal x values can
    /// represent different source bytes at a bidi boundary.
    pub(crate) carets: Vec<TextKitCaret>,
    logical_stops: Vec<usize>,
}

pub(crate) struct TextKitLayout {
    // NSTextStorage owns its layout manager, and must outlive its queries.
    _storage: Retained<NSTextStorage>,
    manager: Retained<NSLayoutManager>,
    container: Retained<NSTextContainer>,
    projection: Projection,
    rows: Vec<TextKitRow>,
    offset: NSPoint,
    /// Logical size, including margins but not glyph overhangs.
    pub(crate) size: Size<Pixels>,
    /// Paint at these bounds relative to the layout origin. The bounds are
    /// pixel-aligned at the requested scale and can include ink overhangs.
    pub(crate) image_bounds: Bounds<Pixels>,
    pub(crate) image: Arc<RenderImage>,
}

impl TextKitLayout {
    /// Text, run lengths, and isolate endpoints use source UTF-8 bytes.
    /// Only regular system UI or Menlo with foreground colors are supported.
    /// TextKit supplies its own fallback fonts. Use one layout per bounded
    /// source piece; do not pass an entire document or share across threads.
    pub(crate) fn shape(
        source: &str,
        isolates: &[BidiIsolate],
        runs: &[TextRun],
        options: &TextKitOptions,
    ) -> Result<Self, TextKitError> {
        autoreleasepool(|_| Self::shape_inner(source, isolates, runs, options))
    }

    fn shape_inner(
        source: &str,
        isolates: &[BidiIsolate],
        runs: &[TextRun],
        options: &TextKitOptions,
    ) -> Result<Self, TextKitError> {
        let width = f64::from(f32::from(options.width));
        let font_size = f64::from(f32::from(options.font_size));
        let line_height = f64::from(f32::from(options.line_height));
        let scale = f64::from(options.scale_factor);
        let Edges {
            top,
            right,
            bottom,
            left,
        } = options.margins;
        let [top, right, bottom, left] =
            [top, right, bottom, left].map(|v| f64::from(f32::from(v)));
        let text_width = width - left - right;
        if !(1. ..=MAX_EXTENT).contains(&text_width)
            || !(1. ..=128.).contains(&font_size)
            || !(font_size..=256.).contains(&line_height)
            || !(1. ..=4.).contains(&scale)
            || [top, right, bottom, left]
                .iter()
                .any(|v| !(0. ..=MAX_EXTENT).contains(v))
        {
            return Err(TextKitError::InvalidOptions);
        }
        if width * scale > MAX_EXTENT {
            return Err(TextKitError::Limit);
        }
        let offset = NSPoint::new(left, top);
        let projection = Projection::new(source, isolates)?;
        let manager = NSLayoutManager::new();
        let (storage, styles) = make_storage(&projection, runs, &manager, font_size, line_height)?;
        let container = NSTextContainer::initWithContainerSize(
            NSTextContainer::alloc(),
            NSSize::new(text_width, MAX_EXTENT / scale),
        );
        container.setLineFragmentPadding(0.);
        manager.addTextContainer(&container);
        storage.addLayoutManager(&manager);
        manager.ensureLayoutForTextContainer(&container);
        let glyphs = manager.glyphRangeForTextContainer(&container);
        if glyphs.location != 0 || glyphs.length != manager.numberOfGlyphs() {
            return Err(TextKitError::Limit);
        }
        if styles.len() > 1 {
            check_style_boundaries(&manager, &projection, &styles)?;
        }
        let rows = native_rows(&manager, &container, &projection, glyphs, offset)?;
        let height = rows
            .last()
            .ok_or(TextKitError::NativeLayout)?
            .bounds
            .bottom();
        let height = f64::from(f32::from(height)) + bottom;
        let ink = manager.boundingRectForGlyphRange_inTextContainer(glyphs, &container);
        let (image, image_bounds) =
            paint::render(&manager, glyphs, offset, ink, width, height, scale)?;
        Ok(Self {
            _storage: storage,
            manager,
            container,
            projection,
            rows,
            offset,
            size: size(px(width as f32), px(height as f32)),
            image_bounds,
            image,
        })
    }

    pub(crate) fn source(&self) -> &str {
        self.projection.source()
    }

    /// All display UTF-16 boundaries for a source UTF-8 byte boundary.
    #[cfg(test)]
    pub(crate) fn display_units_at(&self, byte: usize) -> Option<std::ops::RangeInclusive<usize>> {
        self.projection.units_at(byte)
    }

    pub(crate) fn source_byte_at(&self, unit: usize) -> Option<usize> {
        self.projection.byte_at(unit)
    }

    pub(crate) fn rows(&self) -> &[TextKitRow] {
        &self.rows
    }

    /// Snap to a native insertion position on the row under `at`. Use native
    /// selection geometry to choose the byte at a shared bidi edge, then
    /// TextKit's character hit if the selection geometry cannot distinguish it.
    /// Coordinates are relative to the same origin as `image_bounds`.
    #[cfg(test)]
    pub(crate) fn hit(&self, at: Point<Pixels>) -> Option<(usize, TextKitCaret)> {
        self.hit_where(at, |_| true)
    }

    /// Restrict hits to stops that the host can represent on a row.
    pub(super) fn hit_where(
        &self,
        at: Point<Pixels>,
        accept: impl Fn(&TextKitCaret) -> bool,
    ) -> Option<(usize, TextKitCaret)> {
        let x = f32::from(at.x);
        let y = f32::from(at.y);
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let index = self.rows.partition_point(|row| row.bounds.bottom() <= at.y);
        let row = self.rows.get(index)?;
        if at.y < row.bounds.origin.y {
            return None;
        }
        let carets = &row.carets;
        let right = carets.partition_point(|caret| caret.x < at.x);
        let nearest = match (
            carets[..right].iter().rev().find(|c| accept(c)),
            carets[right..].iter().find(|c| accept(c)),
        ) {
            (Some(left), Some(right)) if at.x - left.x < right.x - at.x => *left,
            (_, Some(right)) => *right,
            (Some(left), None) => *left,
            (None, None) => return None,
        };
        let first = carets.partition_point(|caret| caret.x < nearest.x);
        let last = carets.partition_point(|caret| caret.x <= nearest.x);
        let candidates = || carets[first..last].iter().copied().filter(&accept);
        if last - first == 1 {
            return Some((index, nearest));
        }
        autoreleasepool(|_| {
            // There are normally two candidates. Bound the extra native
            // queries when many zero-width source characters coincide.
            if last - first <= 8 && at.x != nearest.x {
                for caret in candidates() {
                    if self.hit_neighbor(row, caret, at) {
                        return Some((index, caret));
                    }
                }
            }
            // SAFETY: Optional fractional output is unused.
            let unit = unsafe {
                self.manager.characterIndexForPoint_inTextContainer_fractionOfDistanceBetweenInsertionPoints(
                    NSPoint::new(f64::from(x) - self.offset.x, f64::from(y) - self.offset.y),
                    &self.container,
                    ptr::null_mut(),
                )
            };
            let matching = self
                .source_byte_at(unit)
                .and_then(|byte| candidates().find(|caret| caret.byte == byte));
            Some((
                index,
                matching.unwrap_or_else(|| candidates().next().unwrap_or(nearest)),
            ))
        })
    }

    fn hit_neighbor(&self, row: &TextKitRow, caret: TextKitCaret, at: Point<Pixels>) -> bool {
        let Ok(i) = row.logical_stops.binary_search(&caret.byte) else {
            return false;
        };
        [i.checked_sub(1), Some(i + 1)]
            .into_iter()
            .flatten()
            .any(|i| {
                let Some(&other) = row.logical_stops.get(i) else {
                    return false;
                };
                let bytes = other.min(caret.byte)..other.max(caret.byte);
                self.selection_rects_inner(bytes).is_some_and(|rects| {
                    rects.iter().any(|r| {
                        r.origin.x <= at.x
                            && at.x <= r.right()
                            && r.origin.y <= at.y
                            && at.y < r.bottom()
                            && (r.origin.x == caret.x || r.right() == caret.x)
                    })
                })
            })
    }

    /// Native selection rectangles for a source range. TextKit can return
    /// disjoint rectangles, use the full line width for selected breaks, or
    /// snap endpoints inside a shaped cluster. `None` is an invalid range or
    /// failed native query, not an empty selection.
    pub(crate) fn selection_rects(&self, bytes: Range<usize>) -> Option<Vec<Bounds<Pixels>>> {
        autoreleasepool(|_| self.selection_rects_inner(bytes))
    }

    fn selection_rects_inner(&self, bytes: Range<usize>) -> Option<Vec<Bounds<Pixels>>> {
        if self.projection.is_crlf_middle(bytes.start) || self.projection.is_crlf_middle(bytes.end)
        {
            return None;
        }
        let units = self.projection.display_range(bytes)?;
        if units.is_empty() {
            return Some(Vec::new());
        }
        let range = NSRange::new(units.start, units.len());
        let mut count = 0;
        // SAFETY: The range is within storage and the out pointer is live.
        // Copy the rectangles before making another layout manager call.
        let rects = unsafe {
            let pointer = self
                .manager
                .rectArrayForCharacterRange_withinSelectedCharacterRange_inTextContainer_rectCount(
                    range,
                    range,
                    &self.container,
                    (&mut count).into(),
                );
            if count == 0 {
                return Some(Vec::new());
            }
            if pointer.is_null() || count > self.projection.units() + self.rows.len() {
                return None;
            }
            std::slice::from_raw_parts(pointer, count)
        };
        let mut result = Vec::with_capacity(count);
        for &rect in rects {
            if !valid_rect(rect) {
                return None;
            }
            if rect.size.width > 0. && rect.size.height > 0. {
                result.push(bounds(rect, self.offset));
            }
        }
        Some(result)
    }
}

fn make_storage(
    projection: &Projection,
    runs: &[TextRun],
    manager: &NSLayoutManager,
    font_size: f64,
    line_height: f64,
) -> Result<(Retained<NSTextStorage>, Vec<NSRange>), TextKitError> {
    if runs.len() > MAX_RUNS {
        return Err(TextKitError::Limit);
    }
    let first = runs.first().ok_or(TextKitError::InvalidRuns)?;
    let family = first.font.family.as_ref();
    if first.font != gpui::font(family.to_owned()) {
        return Err(TextKitError::UnsupportedStyle);
    }
    let font = match family {
        ".SystemUIFont" => NSFont::systemFontOfSize(font_size),
        "Menlo" => NSFont::fontWithName_size(&NSString::from_str("Menlo-Regular"), font_size)
            .ok_or(TextKitError::UnsupportedStyle)?,
        _ => return Err(TextKitError::UnsupportedStyle),
    };
    if line_height < manager.defaultLineHeightForFont(&font) {
        return Err(TextKitError::InvalidOptions);
    }
    let styles = source_styles(projection, runs, &first.font)?;
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setBaseWritingDirection(NSWritingDirection::LeftToRight);
    paragraph.setMinimumLineHeight(line_height);
    paragraph.setMaximumLineHeight(line_height);
    let storage = NSTextStorage::new();
    storage.beginEditing();
    let display = NSString::from_str(&projection.text);
    storage.replaceCharactersInRange_withString(NSRange::new(0, 0), &display);
    let full = NSRange::new(0, projection.units());
    // SAFETY: Ranges are validated display UTF-16 ranges. Each value has the
    // expected object type for its attribute key.
    unsafe {
        storage.addAttribute_value_range(NSFontAttributeName, &font, full);
        storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, full);
        for &(range, rgba) in &styles {
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(
                f64::from(rgba.r),
                f64::from(rgba.g),
                f64::from(rgba.b),
                f64::from(rgba.a),
            );
            storage.addAttribute_value_range(NSForegroundColorAttributeName, &color, range);
        }
    }
    storage.endEditing();
    Ok((
        storage,
        styles.into_iter().map(|(range, _)| range).collect(),
    ))
}

fn check_style_boundaries(
    manager: &NSLayoutManager,
    projection: &Projection,
    styles: &[NSRange],
) -> Result<(), TextKitError> {
    for &range in styles {
        let mut actual = NSRange::new(0, 0);
        // SAFETY: The input is a valid storage range, and the out pointer is live.
        unsafe { manager.glyphRangeForCharacterRange_actualCharacterRange(range, &mut actual) };
        let end = actual
            .location
            .checked_add(actual.length)
            .ok_or(TextKitError::NativeLayout)?;
        // TextKit expands a partial ligature or composed character to a whole
        // glyph range. Do not silently color any source bytes outside a run.
        if projection.byte_at(actual.location) != projection.byte_at(range.location)
            || projection.byte_at(end) != projection.byte_at(range.location + range.length)
        {
            return Err(TextKitError::UnsupportedStyle);
        }
    }
    Ok(())
}

fn source_styles(
    projection: &Projection,
    runs: &[TextRun],
    font: &Font,
) -> Result<Vec<(NSRange, Rgba)>, TextKitError> {
    let mut byte = 0usize;
    let mut styles: Vec<(NSRange, Rgba)> = Vec::with_capacity(runs.len());
    for run in runs {
        if &run.font != font
            || run.background_color.is_some()
            || run.underline.is_some()
            || run.strikethrough.is_some()
        {
            return Err(TextKitError::UnsupportedStyle);
        }
        let color: Rgba = run.color.into();
        if [color.r, color.g, color.b, color.a]
            .iter()
            .any(|c| !(0. ..=1.).contains(c))
        {
            return Err(TextKitError::UnsupportedStyle);
        }
        let end = byte.checked_add(run.len).ok_or(TextKitError::InvalidRuns)?;
        if end == byte || projection.is_crlf_middle(end) {
            return Err(TextKitError::InvalidRuns);
        }
        let range = projection
            .display_range(byte..end)
            .ok_or(TextKitError::InvalidRuns)?;
        if let Some((last, _)) = styles.last_mut().filter(|(_, previous)| *previous == color) {
            last.length = range.end - last.location;
        } else {
            styles.push((NSRange::new(range.start, range.len()), color));
        }
        byte = end;
    }
    if byte != projection.source().len() {
        return Err(TextKitError::InvalidRuns);
    }
    Ok(styles)
}

fn native_rows(
    manager: &NSLayoutManager,
    container: &NSTextContainer,
    projection: &Projection,
    glyphs: NSRange,
    offset: NSPoint,
) -> Result<Vec<TextKitRow>, TextKitError> {
    let mut rows = Vec::new();
    let mut glyph = glyphs.location;
    let mut expected_unit = 0;
    while glyph < glyphs.length {
        if rows.len() == MAX_ROWS {
            return Err(TextKitError::Limit);
        }
        let mut fragment = NSRange::new(0, 0);
        // SAFETY: Glyph is within the laid out glyph range; the out pointer is live.
        let rect =
            unsafe { manager.lineFragmentRectForGlyphAtIndex_effectiveRange(glyph, &mut fragment) };
        if fragment.location != glyph
            || fragment.length == 0
            || fragment.length > glyphs.length - glyph
            || !valid_rect(rect)
            || rect.size.height <= 0.
        {
            return Err(TextKitError::NativeLayout);
        }
        // SAFETY: The fragment is within the laid out range. Optional output is unused.
        let chars = unsafe {
            manager.characterRangeForGlyphRange_actualGlyphRange(fragment, ptr::null_mut())
        };
        if chars.location != expected_unit
            || chars.length == 0
            || chars.length > projection.units() - expected_unit
        {
            return Err(TextKitError::NativeLayout);
        }
        expected_unit += chars.length;
        let byte_start = projection
            .byte_at(chars.location)
            .ok_or(TextKitError::NativeLayout)?;
        let byte_end = projection
            .byte_at(expected_unit)
            .ok_or(TextKitError::NativeLayout)?;
        let carets = native_carets(manager, chars, projection, rect.origin.x + offset.x)?;
        rows.push(make_row(byte_start..byte_end, bounds(rect, offset), carets));
        glyph += fragment.length;
    }
    if expected_unit != projection.units() {
        return Err(TextKitError::Limit);
    }
    let extra = manager.extraLineFragmentRect();
    if extra.size.height > 0. {
        if rows.len() == MAX_ROWS {
            return Err(TextKitError::Limit);
        }
        if !valid_rect(extra)
            || manager.extraLineFragmentTextContainer().as_deref() != Some(container)
        {
            return Err(TextKitError::NativeLayout);
        }
        let used = manager.extraLineFragmentUsedRect();
        if !valid_rect(used) {
            return Err(TextKitError::NativeLayout);
        }
        let byte = projection.source().len();
        rows.push(make_row(
            byte..byte,
            bounds(extra, offset),
            vec![TextKitCaret {
                byte,
                x: px((used.origin.x + offset.x) as f32),
                alternate: false,
            }],
        ));
    }
    if rows
        .windows(2)
        .any(|pair| pair[0].bounds.bottom() > pair[1].bounds.origin.y)
    {
        return Err(TextKitError::NativeLayout);
    }
    Ok(rows)
}

fn native_carets(
    manager: &NSLayoutManager,
    chars: NSRange,
    projection: &Projection,
    origin_x: f64,
) -> Result<Vec<TextKitCaret>, TextKitError> {
    let capacity = chars.length + 1;
    let mut xs = vec![0.; capacity];
    let mut indices = vec![0; capacity];
    let mut carets = Vec::new();
    for alternate in [false, true] {
        // SAFETY: The character is within the fragment. The SDK guarantees
        // at most one insertion point per UTF-16 unit plus the final point
        // for each query. Both arrays have that capacity.
        let count = unsafe {
            manager.getLineFragmentInsertionPointsForCharacterAtIndex_alternatePositions_inDisplayOrder_positions_characterIndexes(
                chars.location, alternate, true, xs.as_mut_ptr(), indices.as_mut_ptr(),
            )
        };
        if count > capacity {
            return Err(TextKitError::NativeLayout);
        }
        for (&unit, &x) in indices[..count].iter().zip(&xs[..count]) {
            if !(chars.location..=chars.location + chars.length).contains(&unit)
                || !(-MAX_EXTENT..=MAX_EXTENT).contains(&x)
            {
                return Err(TextKitError::NativeLayout);
            }
            let byte = projection.byte_at(unit).ok_or(TextKitError::NativeLayout)?;
            if !projection.is_crlf_middle(byte) {
                carets.push(TextKitCaret {
                    byte,
                    x: px((x + origin_x) as f32),
                    alternate,
                });
            }
        }
    }
    if carets.is_empty() {
        return Err(TextKitError::NativeLayout);
    }
    carets.sort_by(|a, b| {
        f32::from(a.x)
            .total_cmp(&f32::from(b.x))
            .then_with(|| (a.byte, a.alternate).cmp(&(b.byte, b.alternate)))
    });
    carets.dedup_by_key(|caret| (caret.byte, caret.x));
    Ok(carets)
}

fn make_row(bytes: Range<usize>, bounds: Bounds<Pixels>, carets: Vec<TextKitCaret>) -> TextKitRow {
    let mut logical_stops: Vec<_> = carets.iter().map(|caret| caret.byte).collect();
    logical_stops.sort_unstable();
    logical_stops.dedup();
    TextKitRow {
        bytes,
        bounds,
        carets,
        logical_stops,
    }
}

fn valid_rect(rect: NSRect) -> bool {
    [rect.origin.x, rect.origin.y]
        .iter()
        .all(|v| (-MAX_EXTENT..=MAX_EXTENT).contains(v))
        && [rect.size.width, rect.size.height]
            .iter()
            .all(|v| (0. ..=MAX_EXTENT).contains(v))
}

fn bounds(rect: NSRect, offset: NSPoint) -> Bounds<Pixels> {
    Bounds::new(
        point(
            px((rect.origin.x + offset.x) as f32),
            px((rect.origin.y + offset.y) as f32),
        ),
        size(px(rect.size.width as f32), px(rect.size.height as f32)),
    )
}

#[cfg(test)]
mod tests;
