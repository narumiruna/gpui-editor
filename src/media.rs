//! Resolve Markdown destinations and load bounded, still-image previews.

use std::{
    fs::{self, OpenOptions},
    io::{Cursor, Read},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use url::Url;

use crate::rendering::Raster;

const MAX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SIDE: u32 = 4096;
const MAX_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;

/// A destination that the editor can open. Local paths from the target functions
/// are absolute and canonical (symlinks are resolved).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Target {
    Web(Url),
    Local(PathBuf),
}

/// Accept explicit HTTP(S) URLs or existing `.md` files within the workspace.
/// Relative paths start at the document's directory, or at the workspace for
/// a new document. Local heading links and query strings are not supported.
pub fn link_target(
    destination: &str,
    document: Option<&Path>,
    workspace: Option<&Path>,
) -> Result<Target, String> {
    classify(destination, document, workspace, false)
}

/// Accept explicit HTTPS URLs or regular files within the workspace. Relative
/// paths start at the document's directory, or the workspace for a new document.
/// With no workspace, the saved document's directory is also the boundary.
pub fn image_target(
    destination: &str,
    document: Option<&Path>,
    workspace: Option<&Path>,
) -> Result<Target, String> {
    classify(destination, document, workspace, true)
}

fn classify(
    destination: &str,
    document: Option<&Path>,
    workspace: Option<&Path>,
    image: bool,
) -> Result<Target, String> {
    if destination.trim().is_empty() || destination.contains(char::is_control) {
        return Err("Empty or invalid destination".into());
    }
    if let Some((scheme, rest)) = destination.split_once(':')
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
    {
        let supported =
            scheme.eq_ignore_ascii_case("https") || (!image && scheme.eq_ignore_ascii_case("http"));
        if !supported {
            return Err(if image {
                "Remote images require HTTPS"
            } else {
                "Web links require HTTP or HTTPS"
            }
            .into());
        }
        let authority = rest
            .strip_prefix("//")
            .and_then(|rest| rest.split(['/', '?', '#']).next())
            .filter(|authority| !authority.is_empty() && !authority.contains('@'));
        if authority.is_none() || destination.contains('\\') {
            return Err("Expected an explicit URL without credentials".into());
        }
        let url = Url::parse(destination).map_err(|error| format!("Invalid URL: {error}"))?;
        check_web_url(&url, image)?;
        return Ok(Target::Web(url));
    }

    if !image && workspace.is_none() {
        return Err("Open a workspace to follow local links".into());
    }
    let path = local_path(destination)?;
    if !image && !is_markdown(&path) {
        return Err("Local links must point to a .md file".into());
    }
    let workspace = workspace.map(canonical_directory).transpose()?;
    let document_dir = match document {
        Some(path) => {
            let parent = path.parent().ok_or("The document has no directory")?;
            if path.is_absolute() {
                Some(parent.to_owned())
            } else {
                Some(
                    workspace
                        .as_ref()
                        .ok_or("Expected an absolute document path")?
                        .join(parent),
                )
            }
        }
        None => None,
    };
    let root = match workspace {
        Some(workspace) => workspace,
        None => canonical_directory(
            document_dir
                .as_deref()
                .ok_or("Save the document or open a workspace first")?,
        )?,
    };
    let path = document_dir.as_deref().unwrap_or(&root).join(path);
    let path = path
        .canonicalize()
        .map_err(|error| format!("Local target: {error}"))?;
    if !path.starts_with(&root) {
        return Err("Local target is outside the allowed folder".into());
    }
    if !image && !is_markdown(&path) {
        return Err("Local links must point to a .md file".into());
    }
    regular_file(&path)?;
    Ok(Target::Local(path))
}

fn check_web_url(url: &Url, image: bool) -> Result<(), String> {
    if !(url.scheme() == "https" || (!image && url.scheme() == "http")) || !url.has_host() {
        return Err("Expected a supported HTTP(S) URL with a host".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URL credentials are not allowed".into());
    }
    Ok(())
}

fn local_path(destination: &str) -> Result<PathBuf, String> {
    if destination.contains(['?', '#']) {
        return Err("Local targets do not support query strings or fragments".into());
    }
    let mut bytes = destination.bytes();
    let mut decoded = Vec::with_capacity(destination.len());
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let mut hex = || bytes.next().and_then(|b| char::from(b).to_digit(16));
            let high = hex().ok_or("Invalid escape in local path")?;
            let low = hex().ok_or("Invalid escape in local path")?;
            (high * 16 + low) as u8
        } else {
            byte
        });
    }
    let path = String::from_utf8(decoded).map_err(|_| "Invalid UTF-8 in local path")?;
    if path.starts_with("//") || path.contains('\\') || path.contains(char::is_control) {
        return Err("Invalid local path".into());
    }
    Ok(path.into())
}

fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "md")
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("Allowed folder: {error}"))?;
    if !path.is_dir() {
        return Err("Expected a folder".into());
    }
    Ok(path)
}

fn regular_file(path: &Path) -> Result<fs::Metadata, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| format!("Local target: {error}"))?;
    if !metadata.is_file() {
        return Err("Local target must be a regular file".into());
    }
    Ok(metadata)
}

/// Load a target from [`image_target`] into straight-alpha BGRA pixels. Run this
/// blocking operation on a background worker. Accepts at most 8 MiB of encoded
/// data, 4096 pixels per side, and 4,194,304 pixels in total.
/// HTTPS requests do not follow redirects. Animated files yield a still image.
pub fn load_image(target: &Target) -> Result<Raster, String> {
    let bytes = match target {
        Target::Local(path) => read_local_image(path)?,
        Target::Web(url) => fetch_image(url)?,
    };
    decode_image(&bytes)
}

fn read_local_image(path: &Path) -> Result<Vec<u8>, String> {
    // Darwin <sys/fcntl.h>. Do not follow symlinks introduced after resolution,
    // even in parent directories, or block opening a FIFO swapped into place.
    const O_NOFOLLOW_ANY: i32 = 0x2000_0000;
    const O_NONBLOCK: i32 = 0x0004;
    if !path.is_absolute() {
        return Err("Expected a resolved local image path".into());
    }
    check_byte_count(regular_file(path)?.len())?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW_ANY | O_NONBLOCK)
        .open(path)
        .map_err(|error| format!("Open image: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Local image: {error}"))?;
    if !metadata.is_file() {
        return Err("Local image must be a regular file".into());
    }
    check_byte_count(metadata.len())?;
    read_bounded(file)
}

fn fetch_image(url: &Url) -> Result<Vec<u8>, String> {
    check_web_url(url, true)?;
    let mut url = url.clone();
    url.set_fragment(None);
    let agent = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .proxy(None)
        .timeout_global(Some(Duration::from_secs(8)))
        .timeout_connect(Some(Duration::from_secs(3)))
        .build()
        .new_agent();
    let mut response = agent
        .get(url.as_str())
        .call()
        .map_err(|error| format!("Fetch image: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("Image server returned {}", response.status()));
    }
    if let Some(length) = response.body().content_length() {
        check_byte_count(length)?;
    }
    read_bounded(response.body_mut().as_reader())
}

fn read_bounded(reader: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Read image: {error}"))?;
    check_byte_count(bytes.len() as u64)?;
    Ok(bytes)
}

fn check_byte_count(len: u64) -> Result<(), String> {
    if len > MAX_BYTES {
        return Err("Image exceeds the 8 MiB input limit".into());
    }
    Ok(())
}

fn check_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err("Image exceeds the dimension limits or is empty".into());
    }
    Ok(())
}

fn decode_image(bytes: &[u8]) -> Result<Raster, String> {
    let format = match image::guess_format(bytes) {
        Ok(
            format @ (ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP),
        ) => format,
        _ => return Err("Expected a PNG, JPEG, GIF, or WebP image".into()),
    };
    if format == ImageFormat::WebP {
        // An extended WebP can claim a small canvas but contain a large VP8
        // frame. The codec decodes that frame before checking the mismatch.
        check_webp_chunks(bytes.get(12..).ok_or("Invalid WebP header")?, true)?;
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits.clone());
    let decode_error = |error| format!("Decode image: {error}");
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    check_dimensions(width, height)?;
    limits
        .reserve(decoder.total_bytes())
        .map_err(decode_error)?;
    decoder.set_limits(limits).map_err(decode_error)?;
    let mut bgra = DynamicImage::from_decoder(decoder)
        .map_err(decode_error)?
        .into_rgba8()
        .into_raw();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(Raster {
        width,
        height,
        bgra,
    })
}

fn check_webp_chunks(mut chunks: &[u8], top_level: bool) -> Result<(), String> {
    const INVALID: &str = "Invalid WebP chunk";
    while !chunks.is_empty() {
        let (header, contents) = chunks.split_at_checked(8).ok_or(INVALID)?;
        let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let (chunk, rest) = contents.split_at_checked(len + (len % 2)).ok_or(INVALID)?;
        let chunk = &chunk[..len];
        match &header[..4] {
            b"ANMF" if top_level => check_webp_chunks(chunk.get(16..).ok_or(INVALID)?, false)?,
            b"VP8 " => {
                let dimensions = chunk.get(6..10).ok_or(INVALID)?;
                let width = u16::from_le_bytes([dimensions[0], dimensions[1]]) & 0x3fff;
                let height = u16::from_le_bytes([dimensions[2], dimensions[3]]) & 0x3fff;
                check_dimensions(width.into(), height.into())?;
            }
            _ => {}
        }
        chunks = rest;
    }
    Ok(())
}
