//! Window screenshots and clipboard images for Developer Mode attachments.

#[cfg(target_os = "macos")]
use std::{
    fs::{File, Permissions},
    io::BufReader,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use std::{
    io::{BufRead, Cursor, Seek},
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui::{App, ClipboardEntry, Image, ImageFormat, RenderImage, Window};
use image::{
    DynamicImage, Frame, ImageDecoder, ImageEncoder, ImageFormat as CodecFormat, ImageReader,
    Limits, RgbaImage, codecs::png::PngEncoder, metadata::Orientation,
};
#[cfg(target_os = "macos")]
use objc2::{MainThreadMarker, msg_send, runtime::AnyObject};
#[cfg(target_os = "macos")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_INPUT_EDGE: u32 = 8192;
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EDGE: u32 = 1600;
const SMALL_EDGE: u32 = 800;
const MAX_BASE64_BYTES: usize = 4 * 1024 * 1024;
// Padded base64 represents each three input bytes as four ASCII bytes.
const MAX_PNG_BYTES: usize = MAX_BASE64_BYTES / 4 * 3;
#[cfg(target_os = "macos")]
const CAPTURE_FAILED: &str = "Could not capture the editor window. Keep it open and check Screen & System Audio Recording permissions in System Settings, or paste an image instead.";
const DECODE_LIMIT: &str = "The image exceeds the decoding limits.";
#[cfg(not(target_os = "macos"))]
const CAPTURE_UNSUPPORTED: &str =
    "Window capture is not supported on this platform. Paste an image instead.";

pub struct Picture {
    /// Standard padded base64 of a single PNG, with no data URL prefix.
    pub data: String,
    pub preview: Arc<RenderImage>,
}

/// Get the native ID of the editor window on the UI thread.
#[cfg(target_os = "macos")]
pub fn window_id(window: &Window) -> Result<u32, String> {
    let _main = MainThreadMarker::new().ok_or("Window capture requires the macOS main thread.")?;
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|_| "The editor window has no native handle.")?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("The editor window is not an AppKit window.".into());
    };

    // SAFETY: GPUI supplies a live NSView in the borrowed handle. On the main
    // thread its NSWindow can be queried without retaining it or sending it across threads.
    let number: isize = unsafe {
        let view = handle.ns_view.cast::<AnyObject>();
        let native_window: *mut AnyObject = msg_send![view.as_ptr(), window];
        if native_window.is_null() {
            return Err("The editor view is not attached to a native window.".into());
        }
        msg_send![native_window, windowNumber]
    };
    u32::try_from(number)
        .ok()
        .filter(|&id| id != 0)
        .ok_or_else(|| "The editor window has no valid capture ID.".into())
}

#[cfg(not(target_os = "macos"))]
pub fn window_id(_window: &Window) -> Result<u32, String> {
    Err(CAPTURE_UNSUPPORTED.into())
}

/// On a background thread, capture only the window identified by `window_id`.
#[cfg(target_os = "macos")]
pub fn capture(id: u32) -> Result<Picture, String> {
    if id == 0 {
        return Err("The editor window has no valid capture ID.".into());
    }
    let temp = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o700))
        .tempdir()
        .map_err(|_| "Could not create private storage for the screenshot.")?;
    let path = temp.path().join("window.png");

    // -l selects one window; -a excludes attached windows. Do not retry with a
    // different capture scope or surface command output (it may contain paths).
    let mut child = Command::new("/usr/sbin/screencapture")
        .args(["-x", "-l"])
        .arg(id.to_string())
        .args(["-t", "png", "-a", "-o"])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not start macOS window capture.")?;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return Err(CAPTURE_FAILED.into()),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(30)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CAPTURE_FAILED.into());
            }
        }
    }
    let file = File::open(&path).map_err(|_| CAPTURE_FAILED)?;
    let len = file.metadata().map_err(|_| CAPTURE_FAILED)?.len();
    if len == 0 {
        return Err(CAPTURE_FAILED.into());
    }
    check_input_size(len)?;
    let pixels = decode(BufReader::new(file), CodecFormat::Png)?;
    drop(temp);
    picture(pixels)
}

#[cfg(not(target_os = "macos"))]
pub fn capture(_id: u32) -> Result<Picture, String> {
    Err(CAPTURE_UNSUPPORTED.into())
}

/// Read one image from GPUI's clipboard on the UI thread, without decoding it.
pub fn clipboard(cx: &App) -> Result<Arc<Image>, String> {
    let image = cx
        .read_from_clipboard()
        .and_then(|item| {
            item.into_entries().find_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(image),
                ClipboardEntry::String(_) => None,
            })
        })
        .ok_or("The clipboard does not contain an image that GPUI can read.")?;
    // GPUI has already copied the native bytes. Move them without a second copy.
    check_input_size(image.bytes.len() as u64)?;
    Ok(Arc::new(image))
}

/// On a background thread, decode one raster image and make a static PNG attachment.
pub fn from_clipboard(image: Arc<Image>) -> Result<Picture, String> {
    let format = match image.format {
        ImageFormat::Png => CodecFormat::Png,
        ImageFormat::Jpeg => CodecFormat::Jpeg,
        ImageFormat::Tiff => CodecFormat::Tiff,
        _ => return Err("Paste a PNG, JPEG, or TIFF image.".into()),
    };
    check_input_size(image.bytes.len() as u64)?;
    let pixels = decode(Cursor::new(&image.bytes), format)?;
    drop(image);
    picture(pixels)
}

fn check_input_size(len: u64) -> Result<(), String> {
    match len {
        0 => Err("The image is empty.".into()),
        1..=MAX_INPUT_BYTES => Ok(()),
        _ => Err("The image exceeds the 32 MiB input limit.".into()),
    }
}

fn decode(reader: impl BufRead + Seek, format: CodecFormat) -> Result<RgbaImage, String> {
    let mut reader = ImageReader::with_format(reader, format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_INPUT_EDGE);
    limits.max_image_height = Some(MAX_INPUT_EDGE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits.clone());

    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err("The image is empty.".into());
    }
    // Count the output buffer as part of the budget, even for codecs that
    // do not enforce their own allocation limit.
    limits
        .reserve(decoder.total_bytes())
        .map_err(decode_error)?;
    decoder.set_limits(limits).map_err(decode_error)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut pixels = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    if width > MAX_EDGE || height > MAX_EDGE {
        pixels = pixels.thumbnail(MAX_EDGE, MAX_EDGE);
    }
    pixels.apply_orientation(orientation);
    Ok(pixels.into_rgba8())
}

fn decode_error(error: image::ImageError) -> String {
    match error {
        image::ImageError::Limits(_) => DECODE_LIMIT.into(),
        _ => "Could not decode the image.".into(),
    }
}

fn picture(mut pixels: RgbaImage) -> Result<Picture, String> {
    let mut png = encode_png(&pixels)?;
    if png.len() > MAX_PNG_BYTES {
        drop(png);
        // An 800 × 800 RGBA image has about 2.5 MiB of raw pixels, leaving
        // room for PNG overhead even if the pixels do not compress well.
        pixels = DynamicImage::ImageRgba8(pixels)
            .thumbnail(SMALL_EDGE, SMALL_EDGE)
            .into_rgba8();
        png = encode_png(&pixels)?;
        if png.len() > MAX_PNG_BYTES {
            return Err("The PNG exceeds the 4 MiB base64 attachment limit.".into());
        }
    }
    let data = STANDARD.encode(png);

    // GPUI RenderImage uses BGRA. Build it from the exact pixels sent in the PNG.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let preview = Arc::new(RenderImage::new([Frame::new(pixels)]));
    Ok(Picture { data, preview })
}

fn encode_png(pixels: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            pixels.as_raw(),
            pixels.width(),
            pixels.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| "Could not encode the image as PNG.")?;
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn clipboard_png(pixels: &RgbaImage) -> Arc<Image> {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            encode_png(pixels).unwrap(),
        ))
    }

    fn attachment_pixels(picture: &Picture) -> RgbaImage {
        assert!(picture.data.len() <= MAX_BASE64_BYTES);
        let png = STANDARD.decode(&picture.data).unwrap();
        image::load_from_memory_with_format(&png, CodecFormat::Png)
            .unwrap()
            .into_rgba8()
    }

    #[test]
    fn tiff_becomes_png_and_preview_has_matching_pixels() {
        let mut tiff = Cursor::new(Vec::new());
        image::codecs::tiff::TiffEncoder::new(&mut tiff)
            .write_image(&[11, 29, 177, 231], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let input = Arc::new(Image::from_bytes(ImageFormat::Tiff, tiff.into_inner()));
        let picture = from_clipboard(input).unwrap();
        assert_eq!(attachment_pixels(&picture).as_raw(), &[11, 29, 177, 231]);
        assert_eq!(picture.preview.frame_count(), 1);
        assert_eq!(picture.preview.as_bytes(0).unwrap(), &[177, 29, 11, 231]);
    }

    #[test]
    fn large_image_is_scaled_without_changing_its_aspect_ratio() {
        let input = RgbaImage::from_pixel(3200, 200, Rgba([17, 33, 65, 255]));
        let picture = from_clipboard(clipboard_png(&input)).unwrap();
        let decoded = attachment_pixels(&picture);
        assert_eq!(decoded.dimensions(), (1600, 100));
        assert_eq!(decoded.get_pixel(0, 0), input.get_pixel(0, 0));
    }

    #[test]
    fn noisy_image_shrinks_to_fit_the_wire_budget() {
        // Deterministic, poorly compressible RGBA noise.
        let mut state = 1u32;
        let input = RgbaImage::from_fn(1600, 800, |_, _| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            Rgba(state.to_le_bytes())
        });
        let picture = from_clipboard(clipboard_png(&input)).unwrap();
        let decoded = attachment_pixels(&picture);
        assert_eq!(decoded.dimensions(), (800, 400));
        assert_eq!(picture.preview.as_bytes(0).unwrap().len(), decoded.len());
    }

    #[test]
    fn rejects_empty_unsupported_invalid_and_over_limit_inputs() {
        for (format, bytes) in [
            (ImageFormat::Png, Vec::new()),
            (ImageFormat::Png, b"not a PNG".to_vec()),
            (ImageFormat::Svg, b"<svg/>".to_vec()),
            (ImageFormat::Webp, b"RIFF".to_vec()),
        ] {
            assert!(from_clipboard(Arc::new(Image::from_bytes(format, bytes))).is_err());
        }
        assert!(check_input_size(MAX_INPUT_BYTES + 1).is_err());
        let too_wide = RgbaImage::new(MAX_INPUT_EDGE + 1, 1);
        assert!(from_clipboard(clipboard_png(&too_wide)).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rejects_zero_window_id() {
        assert!(capture(0).is_err());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn window_capture_is_explicitly_unsupported() {
        for id in [0, 1, u32::MAX] {
            let error = capture(id).err().expect("capture must be unsupported");
            assert_eq!(error, CAPTURE_UNSUPPORTED);
        }
    }
}
