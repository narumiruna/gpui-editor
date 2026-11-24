use std::{fs, io::ErrorKind, path::Path};

use gpui_editor::workspace::{DocumentFile, Workspace};
use tempfile::tempdir;

#[test]
fn sidebar_lists_only_top_level_markdown_and_can_open_nested_documents() {
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
}

#[test]
fn missing_non_utf8_and_non_markdown_documents_fail_to_open() {
    let folder = tempdir().unwrap();
    fs::write(folder.path().join("invalid.md"), [0xff]).unwrap();
    fs::write(folder.path().join("plain.txt"), "valid UTF-8").unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    for (path, kind) in [
        ("missing.md", ErrorKind::NotFound),
        ("invalid.md", ErrorKind::InvalidData),
        ("plain.txt", ErrorKind::InvalidInput),
    ] {
        assert_eq!(
            workspace.read_document(Path::new(path)).unwrap_err().kind(),
            kind
        );
    }
}

#[test]
fn create_refreshes_listing_and_repeated_saves_keep_a_valid_snapshot() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let mut file =
        DocumentFile::create(Some(&workspace), Path::new("notes"), "draft\n".into()).unwrap();
    assert_eq!(file.path(), workspace.root().join("notes.md"));
    assert_eq!(
        workspace.markdown_files().unwrap(),
        [file.path().to_owned()]
    );
    file.save("# Notes\n\n|  preserved  |\n").unwrap();
    file.save("# Final\n\n|  preserved  |\n").unwrap();
    assert_eq!(file.source(), "# Final\n\n|  preserved  |\n");
    assert_eq!(fs::read_to_string(file.path()).unwrap(), file.source());
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
