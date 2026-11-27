use std::{fs, io::ErrorKind, path::Path};

use gpui_editor_workspace::{DocumentFile, EntryKind, Workspace};
use tempfile::tempdir;

#[test]
fn lists_only_top_level_markdown_and_can_open_nested_documents() {
    let folder = tempdir().unwrap();
    let root = folder.path();
    fs::write(root.join("z.md"), "# Last\n").unwrap();
    fs::write(root.join("a.md"), "# First\n").unwrap();
    fs::write(root.join("plain.txt"), "not Markdown").unwrap();
    fs::create_dir(root.join("folder.md")).unwrap();
    fs::write(root.join("folder.md/nested.md"), "# 中文\n").unwrap();
    let workspace = Workspace::open(root).unwrap();
    assert_eq!(
        workspace.markdown_files().unwrap(),
        [workspace.root().join("a.md"), workspace.root().join("z.md")]
    );
    let nested = workspace
        .read_document(Path::new("folder.md/nested.md"))
        .unwrap();
    assert_eq!(nested.path(), workspace.root().join("folder.md/nested.md"));
    assert_eq!(nested.source(), "# 中文\n");
    let entries = workspace.directory_entries(workspace.root()).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| (
                entry.path.file_name().unwrap().to_str().unwrap(),
                entry.kind
            ))
            .collect::<Vec<_>>(),
        [
            ("folder.md", EntryKind::Folder),
            ("a.md", EntryKind::Markdown),
            ("plain.txt", EntryKind::File),
            ("z.md", EntryKind::Markdown),
        ]
    );
    assert_eq!(
        workspace.directory_entries(Path::new("folder.md")).unwrap()[0].path,
        nested.path()
    );
}

#[cfg(unix)]
#[test]
fn workspace_tree_omits_symlinks_and_does_not_list_paths_through_them() {
    let folder = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    fs::create_dir(workspace.root().join("real")).unwrap();
    for (name, destination) in [
        ("outside", outside.path()),
        ("inside", &workspace.root().join("real")),
    ] {
        std::os::unix::fs::symlink(destination, workspace.root().join(name)).unwrap();
        assert!(workspace.directory_entries(Path::new(name)).is_err());
    }
    let entries = workspace.directory_entries(workspace.root()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, workspace.root().join("real"));
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[test]
fn a_replaced_workspace_requires_reopening_before_listing_or_opening_documents() {
    let folder = tempdir().unwrap();
    let root = folder.path().join("workspace");
    let path = Path::new("document.md");
    fs::create_dir(&root).unwrap();
    fs::write(root.join(path), "original").unwrap();
    let workspace = Workspace::open(&root).unwrap();
    let identity = workspace.root_identity();
    let mut document = workspace.read_document(path).unwrap();

    fs::rename(&root, folder.path().join("moved")).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(root.join(path), "replacement").unwrap();
    for error in [
        workspace.markdown_files().unwrap_err(),
        workspace.directory_entries(workspace.root()).unwrap_err(),
        workspace.read_document(path).unwrap_err(),
        document.save("draft").unwrap_err(),
    ] {
        assert!(error.to_string().contains("folder has moved"), "{error}");
    }

    assert_eq!(workspace.root_identity(), identity);
    assert_eq!(document.source(), "original");
    assert_eq!(
        fs::read_to_string(folder.path().join("moved").join(path)).unwrap(),
        "original"
    );
    let reopened = Workspace::open(&root).unwrap();
    assert_ne!(reopened.root_identity(), identity);
    assert_eq!(
        reopened.markdown_files().unwrap(),
        [reopened.root().join(path)]
    );
    assert_eq!(
        reopened.read_document(path).unwrap().source(),
        "replacement"
    );
}

#[test]
fn missing_non_utf8_and_binary_documents_fail_to_open() {
    let folder = tempdir().unwrap();
    fs::write(folder.path().join("invalid.md"), [0xff]).unwrap();
    fs::write(folder.path().join("invalid.txt"), [0xff]).unwrap();
    fs::write(
        folder.path().join("binary"),
        b"valid UTF-8\0with a NUL byte",
    )
    .unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    for (path, kind) in [
        ("missing.md", ErrorKind::NotFound),
        ("invalid.md", ErrorKind::InvalidData),
        ("invalid.txt", ErrorKind::InvalidData),
        ("binary", ErrorKind::InvalidData),
    ] {
        assert_eq!(
            workspace.read_document(Path::new(path)).unwrap_err().kind(),
            kind
        );
    }
}

#[test]
fn opens_and_saves_existing_plain_text_without_changing_its_name_or_line_endings() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    for name in ["note.txt", "config.json", "Makefile", ".gitignore"] {
        let path = workspace.root().join(name);
        fs::write(&path, "# 原文\r\n\t**literal**").unwrap();
        let mut file = workspace.read_document(Path::new(name)).unwrap();
        assert_eq!(file.path(), path);
        assert_eq!(file.source(), "# 原文\r\n\t**literal**");
        let changed = "# 新文字\r\n\t**literal**";
        file.save(changed).unwrap();
        assert_eq!(fs::read(&path).unwrap(), changed.as_bytes());
        assert_eq!(workspace.read_document(&path).unwrap().source(), changed);
    }
}

#[test]
fn rejects_oversized_plain_text_and_does_not_publish_unsupported_edits() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let path = workspace.root().join("data.txt");
    let limit = 1024 * 1024;
    let source = "x".repeat(limit);
    fs::write(&path, &source).unwrap();
    let mut file = workspace.read_document(&path).unwrap();
    assert_eq!(file.source().len(), limit);
    for draft in ["invalid\0".to_owned(), "x".repeat(limit + 1)] {
        assert_eq!(
            file.save(&draft).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
        assert!(fs::read(&path).unwrap() == source.as_bytes());
    }
    fs::write(&path, "x".repeat(limit + 1)).unwrap();
    let error = workspace.read_document(&path).unwrap_err();
    assert!(error.to_string().contains("1 MiB"));
}

#[test]
fn create_refreshes_listing_and_repeated_saves_keep_a_valid_snapshot() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let draft = "# Draft\n\n|  中文  |\n";
    let mut file =
        DocumentFile::create(Some(&workspace), Path::new("notes"), draft.into()).unwrap();
    assert_eq!(file.path(), workspace.root().join("notes.md"));
    assert_eq!(file.source(), draft);
    assert_eq!(fs::read(file.path()).unwrap(), draft.as_bytes());
    assert_eq!(
        workspace.markdown_files().unwrap(),
        [file.path().to_owned()]
    );
    file.save("# Notes\n\n|  preserved  |\n").unwrap();
    file.save("# Final\n\n|  preserved  |\n").unwrap();
    assert_eq!(file.source(), "# Final\n\n|  preserved  |\n");
    assert_eq!(fs::read_to_string(file.path()).unwrap(), file.source());
    assert_eq!(fs::read_dir(workspace.root()).unwrap().count(), 1);
}

#[test]
fn document_can_be_created_without_a_workspace() {
    let folder = tempdir().unwrap();
    let path = folder.path().join("draft.md");
    let mut file = DocumentFile::create(None, &path, "start".into()).unwrap();
    file.save("finish").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "finish");
}

#[test]
fn new_document_never_overwrites_existing_path() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let path = workspace.root().join("exists.md");
    fs::write(&path, "keep me").unwrap();
    let error = DocumentFile::create(Some(&workspace), &path, "draft".into()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");
    assert_eq!(fs::read_dir(workspace.root()).unwrap().count(), 1);
}

#[test]
fn new_document_never_overwrites_an_existing_directory() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let path = workspace.root().join("exists.md");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("keep.txt"), "keep me").unwrap();

    assert!(DocumentFile::create(Some(&workspace), &path, "draft".into()).is_err());
    assert_eq!(
        fs::read_to_string(path.join("keep.txt")).unwrap(),
        "keep me"
    );
    assert_eq!(fs::read_dir(workspace.root()).unwrap().count(), 1);
}

#[test]
fn external_changes_leave_file_and_saved_snapshot_intact() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let path = workspace.root().join("document.md");
    fs::write(&path, "old").unwrap();
    let mut file = workspace.read_document(&path).unwrap();
    fs::write(&path, "external change").unwrap();
    assert!(file.save("unsaved edit").is_err());
    assert_eq!(file.source(), "old");
    assert_eq!(fs::read_to_string(&path).unwrap(), "external change");
    fs::remove_file(&path).unwrap();
    assert!(file.save("unsaved edit").is_err());
    assert_eq!(file.source(), "old");
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn internal_symlinks_resolve_but_external_links_and_paths_do_not() {
    use std::os::unix::fs::symlink;

    let base = tempdir().unwrap();
    let root = base.path().join("workspace");
    let outside = base.path().join("workspace-other");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(root.join("inside.md"), "inside").unwrap();
    fs::write(outside.join("outside.md"), "outside").unwrap();
    symlink(root.join("inside.md"), root.join("internal-link.md")).unwrap();
    symlink(outside.join("outside.md"), root.join("external-link.md")).unwrap();
    symlink(&outside, root.join("external-folder")).unwrap();
    let workspace = Workspace::open(&root).unwrap();
    let inside = workspace.root().join("inside.md");
    assert_eq!(
        workspace.markdown_files().unwrap(),
        std::slice::from_ref(&inside)
    );
    let mut file = workspace
        .read_document(Path::new("internal-link.md"))
        .unwrap();
    assert_eq!(file.path(), inside);
    for path in [
        "external-link.md",
        "external-folder/outside.md",
        "../workspace-other/outside.md",
    ] {
        assert_eq!(
            workspace.read_document(Path::new(path)).unwrap_err().kind(),
            ErrorKind::PermissionDenied
        );
    }
    assert_eq!(
        workspace
            .read_document(&outside.join("outside.md"))
            .unwrap_err()
            .kind(),
        ErrorKind::PermissionDenied
    );
    let error = DocumentFile::create(
        Some(&workspace),
        Path::new("external-folder/new.md"),
        "draft".into(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    assert!(!outside.join("new.md").exists());

    fs::remove_file(&inside).unwrap();
    symlink(outside.join("outside.md"), &inside).unwrap();
    assert!(file.save("unsafe write").is_err());
    assert_eq!(file.source(), "inside");
    assert_eq!(
        fs::read_to_string(outside.join("outside.md")).unwrap(),
        "outside"
    );
    assert!(fs::symlink_metadata(&inside).unwrap().is_symlink());
}

#[cfg(unix)]
#[test]
fn read_only_file_is_not_replaced() {
    use std::os::unix::fs::PermissionsExt;

    let folder = tempdir().unwrap();
    let path = folder.path().join("read-only.md");
    fs::write(&path, "old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let mut file = workspace.read_document(&path).unwrap();
    assert_eq!(
        file.save("unsaved edit").unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(file.source(), "old");
    assert_eq!(fs::read_to_string(&path).unwrap(), "old");
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[test]
fn saves_reject_a_replaced_parent_for_opened_and_created_documents() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let nested = workspace.root().join("nested");
    let moved = workspace.root().join("moved");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("opened.md"), "original").unwrap();
    let opened = workspace.read_document(&nested.join("opened.md")).unwrap();
    let created =
        DocumentFile::create(Some(&workspace), &nested.join("created.md"), "new".into()).unwrap();

    fs::rename(&nested, &moved).unwrap();
    fs::create_dir(&nested).unwrap();
    for (mut file, original) in [(opened, "original"), (created, "new")] {
        // Names can match even though this is a different folder.
        fs::write(file.path(), "replacement").unwrap();
        let error = file.save("draft").unwrap_err();
        assert!(error.to_string().contains("folder has moved"), "{error}");
        assert_eq!(file.source(), original);
        let name = file.path().file_name().unwrap();
        assert_eq!(fs::read_to_string(moved.join(name)).unwrap(), original);
        assert_eq!(fs::read_to_string(file.path()).unwrap(), "replacement");
    }
    assert_eq!(fs::read_dir(&nested).unwrap().count(), 2);
}

#[cfg(target_os = "macos")]
fn add_acl(path: &Path, entry: &str) {
    let result = std::process::Command::new("/bin/chmod")
        .args(["+a", entry])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn acl_write_denial_overrides_writable_mode_bits() {
    use std::os::unix::fs::PermissionsExt;

    let folder = tempdir().unwrap();
    let path = folder.path().join("denied.md");
    fs::write(&path, "old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    add_acl(&path, "everyone deny write,append");
    let workspace = Workspace::open(folder.path()).unwrap();
    let mut file = workspace.read_document(&path).unwrap();
    assert_eq!(
        file.save("unsaved edit").unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(file.source(), "old");
    assert_eq!(fs::read_to_string(&path).unwrap(), "old");
    assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 1);
}

#[cfg(target_os = "macos")]
#[test]
fn save_preserves_explicit_file_acl_and_mode() {
    use std::{os::unix::fs::PermissionsExt, process::Command};

    fn acl(path: &Path) -> String {
        let output = Command::new("/bin/ls")
            .arg("-le")
            .arg(path)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .skip(1)
            .collect::<Vec<_>>()
            .join("\n")
    }

    let folder = tempdir().unwrap();
    let path = folder.path().join("protected.md");
    fs::write(&path, "old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    add_acl(&path, "everyone deny execute");
    let before = acl(&path);
    assert!(before.contains("everyone deny execute"), "{before}");
    let workspace = Workspace::open(folder.path()).unwrap();
    let mut file = workspace.read_document(&path).unwrap();
    file.save("saved").unwrap();
    assert_eq!(acl(&path), before);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "saved");
}
