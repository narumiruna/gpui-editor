use std::{ptr, sync::Arc};

use gpui::{Bounds, Pixels, RenderImage, point, px, size};
use objc2_app_kit::{NSGraphicsContext, NSLayoutManager};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextGetData, CGColorSpace, CGContext, CGImageAlphaInfo,
    CGImageByteOrderInfo, kCGColorSpaceSRGB,
};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize};

use super::{MAX_EXTENT, TextKitError, valid_rect};

const MAX_BITMAP_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn render(
    manager: &NSLayoutManager,
    glyphs: NSRange,
    offset: NSPoint,
    ink: NSRect,
    width: f64,
    height: f64,
    scale: f64,
) -> Result<(Arc<RenderImage>, Bounds<Pixels>), TextKitError> {
    if !valid_rect(ink) {
        return Err(TextKitError::NativeLayout);
    }
    // Include native glyph overhangs plus one device pixel for antialiasing.
    // Keep the image origin and extent aligned to the requested scale.
    let left = ((ink.origin.x + offset.x).min(0.) * scale).floor() - 1.;
    let top = ((ink.origin.y + offset.y).min(0.) * scale).floor() - 1.;
    let right = ((ink.origin.x + offset.x + ink.size.width).max(width) * scale).ceil() + 1.;
    let bottom = ((ink.origin.y + offset.y + ink.size.height).max(height) * scale).ceil() + 1.;
    let (w, h) = (right - left, bottom - top);
    if !(1. ..=MAX_EXTENT).contains(&w) || !(1. ..=MAX_EXTENT).contains(&h) {
        return Err(TextKitError::Limit);
    }
    let (w, h) = (w as usize, h as usize);
    let stride = w * 4;
    let length = stride * h;
    if length > MAX_BITMAP_BYTES {
        return Err(TextKitError::Limit);
    }
    // SAFETY: kCGColorSpaceSRGB is an immutable framework constant.
    let color_space =
        CGColorSpace::with_name(unsafe { Some(kCGColorSpaceSRGB) }).ok_or(TextKitError::Bitmap)?;
    // SAFETY: Null requests CGContext-owned storage. It stays live while we
    // draw and copy its pixels, including any AppKit-owned context references.
    let context = unsafe {
        CGBitmapContextCreate(
            ptr::null_mut(),
            w,
            h,
            8,
            stride,
            Some(&color_space),
            CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0,
        )
    }
    .ok_or(TextKitError::Bitmap)?;
    CGContext::clear_rect(
        Some(&context),
        NSRect::new(NSPoint::ZERO, NSSize::new(w as f64, h as f64)),
    );
    CGContext::translate_ctm(Some(&context), -left, h as f64 + top);
    CGContext::scale_ctm(Some(&context), scale, -scale);
    CGContext::set_allows_font_smoothing(Some(&context), false);
    {
        let graphics = NSGraphicsContext::graphicsContextWithCGContext_flipped(&context, true);
        NSGraphicsContext::saveGraphicsState_class();
        let _restore = RestoreGraphicsState;
        NSGraphicsContext::setCurrentContext(Some(&graphics));
        manager.drawGlyphsForGlyphRange_atPoint(glyphs, offset);
    }
    CGContext::flush(Some(&context));
    let data = CGBitmapContextGetData(Some(&context)).cast::<u8>();
    if data.is_null() {
        return Err(TextKitError::Bitmap);
    }
    // SAFETY: The live context owns a cleared, non-planar bitmap with the
    // requested stride and height; drawing and flushing are complete.
    let mut pixels = unsafe { std::slice::from_raw_parts(data, length) }.to_vec();
    // GPUI expects BGRA with straight alpha, not Core Graphics premultiplied BGRA.
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    let pixels =
        image::RgbaImage::from_raw(w as u32, h as u32, pixels).ok_or(TextKitError::Bitmap)?;
    let image = Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]));
    let bounds = Bounds::new(
        point(px((left / scale) as f32), px((top / scale) as f32)),
        size(px((w as f64 / scale) as f32), px((h as f64 / scale) as f32)),
    );
    Ok((image, bounds))
}

struct RestoreGraphicsState;

impl Drop for RestoreGraphicsState {
    fn drop(&mut self) {
        NSGraphicsContext::restoreGraphicsState_class();
    }
}
