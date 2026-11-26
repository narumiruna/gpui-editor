use std::{cell::Cell, fs, io::ErrorKind};

use gpui_editor_workspace::{SearchHit, SearchLocation, SearchResults, Workspace};
use tempfile::tempdir;

#[test]
fn finds_saved_filenames_and_first_literals_per_line_in_path_order() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let root = workspace.root();
    for name in ["z", "a"] {
        fs::create_dir(root.join(name)).unwrap();
        fs::write(
            root.join(name).join("needle.md"),
            "Needle\r\n中文 needle needle\r\n\r\nneedle\n",
        )
        .unwrap();
    }
    fs::write(root.join("needle.txt"), "needle").unwrap();
    fs::write(root.join("needle.MD"), "needle").unwrap();
    fs::write(root.join("a/unrelated.md"), "no matches").unwrap();

    let results = workspace.search_markdown("needle", || false).unwrap();
    let mut expected = Vec::new();
    for name in ["a", "z"] {
        for location in [
            None,
            Some(SearchLocation {
                line: 2,
                text: "中文 needle needle".into(),
                column: 7,
            }),
            Some(SearchLocation {
                line: 4,
                text: "needle".into(),
                column: 0,
            }),
        ] {
            expected.push(SearchHit {
                path: root.join(name).join("needle.md"),
                location,
            });
        }
    }
    assert_eq!(results.hits, expected);
    assert!(!results.skipped);
    assert!(!results.truncated);
    assert_eq!(
        workspace
            .search_markdown("中文", || false)
            .unwrap()
            .hits
            .len(),
        2
    );
    for query in ["absent", "", "needle\nneedle"] {
        assert_eq!(
            workspace.search_markdown(query, || false).unwrap(),
            SearchResults::default()
        );
    }
}

#[test]
fn validates_a_location_before_returning_its_byte_offset() {
    let source = "one\r\n中文 needle\r\nlast";
    let mut location = SearchLocation {
        line: 2,
        text: "中文 needle".into(),
        column: 7,
    };
    assert_eq!(location.offset_in(source), source.find("needle"));
    assert_eq!(location.offset_in("longer\n中文 needle"), Some(14));
    assert_eq!(location.offset_in("one\r\n中文 needle edited"), None);
    assert_eq!(location.offset_in("中文 needle\none"), None);
    for column in [1, location.text.len(), usize::MAX] {
        location.column = column;
        assert_eq!(location.offset_in(source), None);
    }
    location.column = 7;
    location.line = 0;
    assert_eq!(location.offset_in(source), None);
}

#[test]
fn skips_invalid_and_oversized_contents_but_can_match_their_names() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let root = workspace.root();
    fs::write(root.join("invalid-needle.md"), b"needle\n\xff").unwrap();
    fs::write(root.join("large-needle.md"), b"needle\n").unwrap();
    fs::File::options()
        .write(true)
        .open(root.join("large-needle.md"))
        .unwrap()
        .set_len(1024 * 1024 + 1)
        .unwrap();
    let mut exact = b"needle\n".to_vec();
    exact.resize(1024 * 1024, b'x');
    fs::write(root.join("valid.md"), exact).unwrap();

    let results = workspace.search_markdown("needle", || false).unwrap();
    assert!(results.skipped);
    assert!(!results.truncated);
    assert_eq!(results.hits.len(), 3);
    assert_eq!(results.hits[0].path, root.join("invalid-needle.md"));
    assert_eq!(results.hits[1].path, root.join("large-needle.md"));
    assert_eq!(results.hits[2].path, root.join("valid.md"));
    assert!(results.hits[0].location.is_none());
    assert!(results.hits[1].location.is_none());
    assert_eq!(results.hits[2].location.as_ref().unwrap().line, 1);
}

#[cfg(unix)]
#[test]
fn reports_unreadable_files_and_distinguishes_an_unreadable_root() {
    use std::os::unix::fs::PermissionsExt as _;

    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let root = workspace.root();
    let file = root.join("unreadable.md");
    fs::write(&file, "needle").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    // Privileged test runners can bypass the permission bits.
    if fs::File::open(&file).is_err() {
        let results = workspace.search_markdown("needle", || false).unwrap();
        assert!(results.skipped);
        assert!(results.hits.is_empty());
    }
    let permissions = fs::metadata(root).unwrap().permissions();
    fs::set_permissions(root, fs::Permissions::from_mode(0o000)).unwrap();
    let inaccessible = fs::read_dir(root).is_err();
    let results = workspace.search_markdown("needle", || false);
    fs::set_permissions(root, permissions).unwrap();
    if inaccessible {
        assert_eq!(results.unwrap_err().kind(), ErrorKind::PermissionDenied);
    }
}

#[test]
fn does_not_enter_private_recovery_folders() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let root = workspace.root();
    fs::create_dir(root.join(".gpui-save-recovery")).unwrap();
    fs::write(root.join(".gpui-save-recovery/needle.md"), "needle").unwrap();
    fs::write(root.join("inside.md"), "needle").unwrap();

    let results = workspace.search_markdown("needle", || false).unwrap();
    assert!(!results.skipped);
    assert!(!results.truncated);
    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].path, root.join("inside.md"));
}

#[cfg(unix)]
#[test]
fn does_not_follow_symlinks() {
    use std::os::unix::fs::symlink;

    let folder = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let root = workspace.root();
    fs::write(root.join("inside.md"), "needle").unwrap();
    fs::write(outside.path().join("needle.md"), "needle").unwrap();
    symlink(root.join("inside.md"), root.join("needle-link.md")).unwrap();
    symlink(outside.path().join("needle.md"), root.join("outside.md")).unwrap();
    symlink(outside.path(), root.join("outside-folder")).unwrap();
    symlink(root, root.join("loop")).unwrap();

    let results = workspace.search_markdown("needle", || false).unwrap();
    assert!(!results.skipped);
    assert!(!results.truncated);
    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].path, root.join("inside.md"));
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
#[test]
fn rejects_a_replaced_workspace_root() {
    let folder = tempdir().unwrap();
    let root = folder.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let workspace = Workspace::open(&root).unwrap();
    fs::rename(&root, folder.path().join("original")).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(root.join("needle.md"), "needle").unwrap();

    assert!(workspace.search_markdown("needle", || false).is_err());
}

#[test]
fn retains_only_the_first_200_hits_and_reports_additional_matches() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let file = workspace.root().join("a.md");
    fs::write(&file, "needle\n".repeat(200)).unwrap();
    let results = workspace.search_markdown("needle", || false).unwrap();
    assert_eq!(results.hits.len(), 200);
    assert!(!results.truncated);

    fs::write(workspace.root().join("z-needle.md"), "needle").unwrap();
    let results = workspace.search_markdown("needle", || false).unwrap();
    assert!(results.truncated);
    assert!(!results.skipped);
    assert_eq!(results.hits.len(), 200);
    for (index, hit) in results.hits.iter().enumerate() {
        assert_eq!(hit.path, file);
        assert_eq!(hit.location.as_ref().unwrap().line, index + 1);
    }
}

#[test]
fn limits_the_total_saved_text_of_matching_lines() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let line = "needle".to_owned() + &"x".repeat(1024 * 1024 - 6);
    for index in 0..9 {
        fs::write(workspace.root().join(format!("{index}.md")), &line).unwrap();
    }
    let results = workspace.search_markdown("needle", || false).unwrap();
    assert!(results.truncated);
    assert!(!results.skipped);
    assert_eq!(results.hits.len(), 8);
}

#[test]
fn counts_even_non_markdown_entries_against_the_traversal_limit() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    for index in 0..10_001 {
        fs::write(workspace.root().join(format!("{index}.txt")), "").unwrap();
    }
    let results = workspace.search_markdown("needle", || false).unwrap();
    assert!(results.truncated);
    assert!(!results.skipped);
    assert!(results.hits.is_empty());
}

#[test]
fn cancellation_interrupts_traversal() {
    let folder = tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    for index in 0..20 {
        fs::write(workspace.root().join(format!("{index}.md")), "needle").unwrap();
    }
    let checks = Cell::new(0);
    let error = workspace
        .search_markdown("needle", || {
            checks.set(checks.get() + 1);
            checks.get() == 10
        })
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Interrupted);
}
