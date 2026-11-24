#![cfg(target_os = "macos")]

use std::{
    fs::{self, File},
    os::unix::{fs::symlink, net::UnixListener},
};

use gpui_editor::media::{Target, image_target, link_target, load_image};
use image::{GrayImage, ImageFormat, RgbImage, Rgba, RgbaImage};
use tempfile::tempdir;
use url::Url;

#[test]
fn resolve_local_paths_from_the_document_directory() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    let document = root.join("notes/current.md");
    let guide = root.join("guide.md");
    let image = root.join("notes/images/photo #1.png");
    fs::create_dir_all(image.parent().unwrap()).unwrap();
    for file in [&document, &guide, &image, &root.join("other.txt")] {
        fs::write(file, []).unwrap();
    }

    assert_eq!(
        link_target("../guide.md", Some(&document), Some(root)).unwrap(),
        Target::Local(guide.canonicalize().unwrap()),
    );
    assert_eq!(
        link_target("guide.md", None, Some(root)).unwrap(),
        Target::Local(guide.canonicalize().unwrap()),
    );
    let local_image = Target::Local(image.canonicalize().unwrap());
    assert_eq!(
        image_target("images/photo%20%231.png", Some(&document), Some(root)).unwrap(),
        local_image,
    );
    assert_eq!(
        image_target("images/photo%20%231.png", Some(&document), None).unwrap(),
        local_image,
    );
    assert!(link_target("../guide.md", Some(&document), None).is_err());
    assert!(image_target("../guide.md", Some(&document), None).is_err());
    assert!(image_target("guide.md", None, None).is_err());
    for destination in ["missing.md", "other.txt", "notes", "guide.md#heading"] {
        assert!(
            link_target(destination, None, Some(root)).is_err(),
            "{destination}"
        );
    }
}

#[test]
fn reject_workspace_escapes_and_nonregular_files() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("workspace");
    let sibling = temp.path().join("workspace-other");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&sibling).unwrap();
    let outside = sibling.join("outside.md");
    fs::write(&outside, []).unwrap();
    fs::write(root.join("inside.txt"), []).unwrap();
    symlink(&outside, root.join("escape.md")).unwrap();
    symlink(&sibling, root.join("escape-dir")).unwrap();
    symlink("inside.txt", root.join("not-markdown.md")).unwrap();
    let socket = root.join("socket.md");
    let _listener = UnixListener::bind(&socket).unwrap();

    for destination in [
        "../workspace-other/outside.md",
        "%2e%2e/workspace-other/outside.md",
        "escape.md",
        "escape-dir/outside.md",
        outside.to_str().unwrap(),
        "socket.md",
    ] {
        assert!(
            link_target(destination, None, Some(&root)).is_err(),
            "{destination}"
        );
        assert!(
            image_target(destination, None, Some(&root)).is_err(),
            "{destination}"
        );
    }
    assert!(link_target("not-markdown.md", None, Some(&root)).is_err());
    assert!(load_image(&Target::Local(socket.canonicalize().unwrap())).is_err());
}

#[test]
fn reject_symlinks_inserted_after_resolution() {
    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let inner = root.path().join("images");
    fs::create_dir(&inner).unwrap();
    let file = inner.join("preview.png");
    let replacement = outside.path().join("preview.png");
    let fixture = RgbaImage::new(1, 1);
    fixture.save(&file).unwrap();
    fixture.save(&replacement).unwrap();
    let target = image_target("images/preview.png", None, Some(root.path())).unwrap();
    assert!(load_image(&target).is_ok());

    fs::remove_file(&file).unwrap();
    symlink(&replacement, &file).unwrap();
    assert!(load_image(&target).is_err());

    fs::remove_file(&file).unwrap();
    fs::remove_dir(&inner).unwrap();
    symlink(outside.path(), &inner).unwrap();
    assert!(load_image(&target).is_err());
}

#[test]
fn classify_web_destinations_without_fetching() {
    let destination = "http://example.test/docs?q=test#intro";
    let link = Target::Web(Url::parse(destination).unwrap());
    assert_eq!(link_target(destination, None, None).unwrap(), link);
    assert!(image_target(destination, None, None).is_err());
    assert!(load_image(&link).is_err());

    let destination = "HTTPS://example.test/image.png?size=2#fragment";
    let image = Target::Web(Url::parse(destination).unwrap());
    assert_eq!(link_target(destination, None, None).unwrap(), image);
    assert_eq!(image_target(destination, None, None).unwrap(), image);

    let root = tempdir().unwrap();
    fs::write(root.path().join("mailto:example.md"), []).unwrap();
    for destination in [
        "mailto:example.md",
        "file:///tmp/guide.md",
        "data:image/png;base64,eA==",
        "javascript:alert(1)",
        "//example.test/guide.md",
        "https:/example.test/guide.md",
        "https:///example.test/guide.md",
        "https://user:pass@example.test/guide.md",
        "https://@example.test/guide.md",
        "https:\\/example.test/guide.md",
        "#heading",
        "",
    ] {
        assert!(
            link_target(destination, None, Some(root.path())).is_err(),
            "{destination}"
        );
        assert!(
            image_target(destination, None, Some(root.path())).is_err(),
            "{destination}"
        );
    }
    let credentials = Url::parse("https://user:pass@example.test/image.png").unwrap();
    assert!(load_image(&Target::Web(credentials)).is_err());
}

#[test]
fn decode_allowed_raster_formats_by_content() {
    let root = tempdir().unwrap();
    let file = root.path().join("image.bin");
    let target = || image_target("image.bin", None, Some(root.path())).unwrap();
    let rgba = RgbaImage::from_pixel(2, 1, Rgba([19, 43, 71, 101]));
    for format in [ImageFormat::Png, ImageFormat::WebP, ImageFormat::Gif] {
        rgba.save_with_format(&file, format).unwrap();
        let raster = load_image(&target()).unwrap();
        assert_eq!((raster.width, raster.height), (2, 1));
        assert_eq!(raster.bgra.len(), 8);
        if format != ImageFormat::Gif {
            assert_eq!(raster.bgra, [71, 43, 19, 101, 71, 43, 19, 101]);
        }
    }
    RgbImage::new(2, 1)
        .save_with_format(&file, ImageFormat::Jpeg)
        .unwrap();
    let raster = load_image(&target()).unwrap();
    assert_eq!((raster.width, raster.height), (2, 1));
    assert_eq!(raster.bgra, [0, 0, 0, 255, 0, 0, 0, 255]);
}

#[test]
fn reject_unsupported_corrupt_and_oversize_images() {
    let root = tempdir().unwrap();
    let file = root.path().join("image.png");
    let target = || image_target("image.png", None, Some(root.path())).unwrap();
    for bytes in [
        b"<svg xmlns='http://www.w3.org/2000/svg'/>".as_slice(),
        b"\x89PNG\r\n\x1a\n",
        b"RIFF\0\0\0\0WEBP",
    ] {
        fs::write(&file, bytes).unwrap();
        assert!(load_image(&target()).is_err());
    }
    RgbImage::new(1, 1)
        .save_with_format(&file, ImageFormat::Bmp)
        .unwrap();
    assert!(load_image(&target()).is_err());

    for (width, height) in [(4097, 1), (2049, 2048)] {
        GrayImage::new(width, height).save(&file).unwrap();
        assert!(load_image(&target()).is_err());
    }

    RgbaImage::new(1, 1).save(&file).unwrap();
    File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();
    assert!(load_image(&target()).unwrap_err().contains("input limit"));
}

#[test]
fn check_webp_frame_dimensions_not_only_canvas() {
    let root = tempdir().unwrap();
    // Extended WebP declares a 1×1 canvas but a 4097×1 lossy frame.
    let header = b"RIFF\x28\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0\0\0\0\0\0\0\
        VP8 \x0a\0\0\0\x10\0\0\x9d\x01\x2a\x01\x10\x01\0";
    fs::write(root.path().join("image.webp"), header).unwrap();
    let target = image_target("image.webp", None, Some(root.path())).unwrap();
    assert!(load_image(&target).unwrap_err().contains("dimension"));
}
