//! Store only the workspace and saved tab paths, never document contents.

use std::{
    collections::HashMap,
    io::{self, Write as _},
    path::{Component, Path, PathBuf},
};

#[cfg(unix)]
use std::{ffi::OsStr, os::unix::ffi::OsStrExt as _};
#[cfg(windows)]
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt as _, OsStringExt as _},
};

use serde::{Deserialize, Serialize};

use super::{
    sidebar::{Snapshot, snapshot},
    state_file::{self, write_atomically},
};
use crate::workspace::{DocumentFile, Workspace};

const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Session {
    version: u32,
    workspace: Option<SavedWorkspace>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedWorkspace {
    // Store native path bytes: Unix bytes on macOS and UTF-16LE on Windows.
    root: Vec<u8>,
    identity: (u64, u64),
    tabs: Vec<Vec<u8>>,
    selected: Option<Vec<u8>>,
}

#[derive(Debug)]
pub(super) struct Restored {
    pub(super) workspace: Workspace,
    pub(super) tree: Snapshot,
    pub(super) documents: Vec<DocumentFile>,
    pub(super) selected: Option<usize>,
}

impl Session {
    /// Capture paths already known to the app without consulting the filesystem.
    pub(super) fn capture<'a>(
        workspace: Option<&Workspace>,
        paths: impl Iterator<Item = &'a Path>,
        selected: Option<&Path>,
    ) -> Self {
        Self {
            version: VERSION,
            workspace: workspace.map(|workspace| {
                let relative = |path: &Path| {
                    let path = path.strip_prefix(workspace.root()).ok()?;
                    is_relative_document_path(path).then(|| path_bytes(path))
                };
                let tabs: Vec<_> = paths.filter_map(relative).collect();
                let selected = selected
                    .and_then(relative)
                    .filter(|path| tabs.contains(path));
                SavedWorkspace {
                    root: path_bytes(workspace.root()),
                    identity: workspace.root_identity(),
                    tabs,
                    selected,
                }
            }),
        }
    }

    /// Publish the new session atomically. The destination must have a dedicated directory.
    pub(super) fn write(&self, path: &Path) -> io::Result<()> {
        write_atomically(path, |file| {
            let mut file = io::BufWriter::new(file);
            serde_json::to_writer(&mut file, self)?;
            file.flush()
        })
    }
}

pub(super) fn restore(path: &Path) -> io::Result<Option<Restored>> {
    let Some(file) = state_file::open(path)? else {
        return Ok(None);
    };
    let session: Session = serde_json::from_reader(io::BufReader::new(file))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if session.version != VERSION {
        return Err(invalid_data("unsupported session version"));
    }
    let Some(saved) = session.workspace else {
        return Ok(None);
    };
    let root = path_from_bytes(&saved.root)?;
    if !root.is_absolute() {
        return Err(invalid_data("the saved workspace path is not absolute"));
    }
    let workspace = Workspace::open(&root)?;
    if workspace.root() != root || workspace.root_identity() != saved.identity {
        return Err(io::Error::other(
            "the saved workspace folder has moved or changed",
        ));
    }
    let tree = snapshot(&workspace, Vec::new())?;
    let mut documents = Vec::new();
    let mut indices = HashMap::new();
    let mut selected = None;
    for relative in &saved.tabs {
        let Ok(path) = path_from_bytes(relative) else {
            continue;
        };
        if !is_relative_document_path(&path) {
            continue;
        }
        if let Ok(document) = workspace.read_document(&path) {
            let index = *indices
                .entry(document.path().to_owned())
                .or_insert_with(|| {
                    let index = documents.len();
                    documents.push(document);
                    index
                });
            if saved.selected.as_ref() == Some(relative) {
                selected = Some(index);
            }
        }
    }
    workspace.check_root()?;
    let selected = selected.or_else(|| (!documents.is_empty()).then_some(0));
    Ok(Some(Restored {
        workspace,
        tree,
        documents,
        selected,
    }))
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> io::Result<PathBuf> {
    Ok(PathBuf::from(OsStr::from_bytes(bytes)))
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(windows)]
fn path_from_bytes(bytes: &[u8]) -> io::Result<PathBuf> {
    let (units, remaining) = bytes.as_chunks::<2>();
    if !remaining.is_empty() {
        return Err(invalid_data("invalid saved path encoding"));
    }
    let wide: Vec<_> = units.iter().copied().map(u16::from_le_bytes).collect();
    Ok(PathBuf::from(OsString::from_wide(&wide)))
}

fn is_relative_document_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    use serde_json::json;

    use super::*;

    struct Fixture {
        temp: tempfile::TempDir,
        workspace: Workspace,
        session_path: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("notes");
            fs::create_dir(&root).unwrap();
            let workspace = Workspace::open(&root).unwrap();
            let session_path = temp
                .path()
                .join("Application Support/GPUI Editor/session.json");
            Self {
                temp,
                workspace,
                session_path,
            }
        }

        fn document(&self, name: &str, source: &str) -> PathBuf {
            let path = self.workspace.root().join(name);
            fs::write(&path, source).unwrap();
            path
        }
    }

    #[test]
    fn stores_only_paths_in_order_and_reloads_from_disk() {
        let fixture = Fixture::new();
        let first = fixture.document("z.md", "original z contents");
        let second = fixture.document("a.md", "original a contents");
        let outside = fixture.temp.path().join("outside.md");
        let session = Session::capture(
            Some(&fixture.workspace),
            [&first, &outside, &second]
                .into_iter()
                .map(PathBuf::as_path),
            Some(&second),
        );
        session.write(&fixture.session_path).unwrap();
        let bytes = fs::read(&fixture.session_path).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            json!({
                "version": 1,
                "workspace": {
                    "root": path_bytes(fixture.workspace.root()),
                    "identity": fixture.workspace.root_identity(),
                    "tabs": [path_bytes(Path::new("z.md")), path_bytes(Path::new("a.md"))],
                    "selected": path_bytes(Path::new("a.md")),
                },
            }),
        );
        #[cfg(unix)]
        {
            assert_eq!(
                fs::metadata(fixture.session_path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                0o700,
            );
            assert_eq!(
                fs::metadata(&fixture.session_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                0o600
            );
        }

        fs::write(&second, "updated on disk").unwrap();
        let restored = restore(&fixture.session_path).unwrap().unwrap();
        assert_eq!(restored.workspace, fixture.workspace);
        assert_eq!(
            restored.tree[restored.workspace.root()]
                .iter()
                .map(|entry| &entry.path)
                .collect::<Vec<_>>(),
            [&second, &first]
        );
        assert_eq!(restored.documents[0].path(), first);
        assert_eq!(restored.documents[1].path(), second);
        assert_eq!(restored.documents[1].source(), "updated on disk");
        assert_eq!(restored.selected, Some(1));
    }

    #[test]
    fn keeps_a_workspace_with_no_tabs_but_ignores_tabs_without_a_workspace() {
        let fixture = Fixture::new();
        let file = fixture.document("a.md", "note");
        assert!(restore(&fixture.session_path).unwrap().is_none());
        Session::capture(Some(&fixture.workspace), std::iter::empty(), None)
            .write(&fixture.session_path)
            .unwrap();
        let restored = restore(&fixture.session_path).unwrap().unwrap();
        assert_eq!(restored.workspace, fixture.workspace);
        assert_eq!(
            restored.tree[restored.workspace.root()]
                .iter()
                .map(|entry| &entry.path)
                .collect::<Vec<_>>(),
            [&file]
        );
        assert!(restored.documents.is_empty());
        assert_eq!(restored.selected, None);

        Session::capture(None, std::iter::once(file.as_path()), Some(&file))
            .write(&fixture.session_path)
            .unwrap();
        assert!(restore(&fixture.session_path).unwrap().is_none());
    }

    #[test]
    fn skips_missing_invalid_external_and_duplicate_tabs_and_falls_back() {
        let fixture = Fixture::new();
        let first = fixture.document("first.md", "first");
        let last = fixture.document("last.md", "last");
        let missing = fixture.document("missing.md", "missing");
        let bad = fixture.workspace.root().join("bad.md");
        fs::write(&bad, [0xff]).unwrap();
        let folder = fixture.workspace.root().join("folder.md");
        fs::create_dir(&folder).unwrap();
        let outside = fixture.temp.path().join("outside.md");
        fs::write(&outside, "outside").unwrap();
        let mut session = Session::capture(
            Some(&fixture.workspace),
            [&missing, &first, &first, &bad, &folder, &last]
                .into_iter()
                .map(PathBuf::as_path),
            Some(&missing),
        );
        let tabs = &mut session.workspace.as_mut().unwrap().tabs;
        tabs.insert(0, path_bytes(Path::new("../outside.md")));
        tabs.insert(0, path_bytes(&outside));
        fs::remove_file(&missing).unwrap();
        session.write(&fixture.session_path).unwrap();

        let restored = restore(&fixture.session_path).unwrap().unwrap();
        let paths: Vec<_> = restored.documents.iter().map(DocumentFile::path).collect();
        assert_eq!(paths, [first.as_path(), last.as_path()]);
        assert_eq!(restored.selected, Some(0));

        // The normalized path can identify an open tab even if its contents cannot be read.
        assert_eq!(
            fixture
                .workspace
                .resolve_document(Path::new("bad.md"))
                .unwrap(),
            bad
        );
        assert!(
            fixture
                .workspace
                .resolve_document(Path::new("folder.md"))
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn skips_external_symlinks_and_deduplicates_internal_symlinks() {
        let fixture = Fixture::new();
        let first = fixture.document("first.md", "first");
        let outside = fixture.temp.path().join("outside.md");
        fs::write(&outside, "outside").unwrap();
        let external_link = fixture.workspace.root().join("external.md");
        symlink(&outside, &external_link).unwrap();
        let duplicate_link = fixture.workspace.root().join("alias.md");
        symlink(&first, &duplicate_link).unwrap();
        Session::capture(
            Some(&fixture.workspace),
            [&first, &external_link, &duplicate_link]
                .into_iter()
                .map(PathBuf::as_path),
            Some(&duplicate_link),
        )
        .write(&fixture.session_path)
        .unwrap();
        let restored = restore(&fixture.session_path).unwrap().unwrap();
        assert_eq!(restored.documents.len(), 1);
        assert_eq!(restored.documents[0].path(), first);
        assert_eq!(restored.selected, Some(0));
        assert_eq!(
            fixture
                .workspace
                .resolve_document(Path::new("alias.md"))
                .unwrap(),
            first
        );
        assert!(
            fixture
                .workspace
                .resolve_document(Path::new("external.md"))
                .is_err()
        );
    }

    #[test]
    fn rejects_a_missing_or_replaced_root_even_when_capture_happens_after_replacement() {
        let fixture = Fixture::new();
        let root = fixture.workspace.root();
        let file = fixture.document("a.md", "old note");
        let capture = || {
            Session::capture(
                Some(&fixture.workspace),
                std::iter::once(file.as_path()),
                Some(&file),
            )
        };
        capture().write(&fixture.session_path).unwrap();
        fs::rename(root, root.with_extension("moved")).unwrap();
        assert_eq!(
            restore(&fixture.session_path).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );

        fs::create_dir(root).unwrap();
        fs::write(&file, "different note").unwrap();
        capture().write(&fixture.session_path).unwrap();
        assert!(restore(&fixture.session_path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn round_trips_non_utf8_path_bytes_without_requiring_a_filesystem_that_accepts_them() {
        let fixture = Fixture::new();
        let relative = b"one\xff/two\xfe.md";
        let file = fixture
            .workspace
            .root()
            .join(path_from_bytes(relative).unwrap());
        let session = Session::capture(
            Some(&fixture.workspace),
            std::iter::once(file.as_path()),
            Some(&file),
        );
        session.write(&fixture.session_path).unwrap();
        let saved: Session =
            serde_json::from_slice(&fs::read(&fixture.session_path).unwrap()).unwrap();
        let workspace = saved.workspace.unwrap();
        assert_eq!(
            path_from_bytes(&workspace.tabs[0]).unwrap(),
            path_from_bytes(relative).unwrap()
        );
        assert_eq!(workspace.selected.as_deref(), Some(relative.as_slice()));
    }

    #[cfg(windows)]
    #[test]
    fn round_trips_native_unicode_workspace_and_document_paths() {
        let mut fixture = Fixture::new();
        let root = fixture.workspace.root().join("資料-é-📝");
        fs::create_dir(&root).unwrap();
        fixture.workspace = Workspace::open(&root).unwrap();
        let file = fixture.document("日本語-é-📝.md", "note");
        Session::capture(
            Some(&fixture.workspace),
            std::iter::once(file.as_path()),
            Some(&file),
        )
        .write(&fixture.session_path)
        .unwrap();
        let restored = restore(&fixture.session_path).unwrap().unwrap();
        assert_eq!(restored.workspace, fixture.workspace);
        assert_eq!(restored.documents[0].path(), file);
        assert_eq!(restored.selected, Some(0));
    }

    #[cfg(windows)]
    #[test]
    fn round_trips_unpaired_utf16_surrogates_as_path_bytes() {
        let fixture = Fixture::new();
        let relative = PathBuf::from(OsString::from_wide(&[
            0xd800, 0x5c, 0xdc00, 0x2d, 0xd83d, 0xdcc4, 0x2e, 0x6d, 0x64,
        ]));
        let file = fixture.workspace.root().join(&relative);
        Session::capture(
            Some(&fixture.workspace),
            std::iter::once(file.as_path()),
            Some(&file),
        )
        .write(&fixture.session_path)
        .unwrap();
        let saved: Session =
            serde_json::from_slice(&fs::read(&fixture.session_path).unwrap()).unwrap();
        let workspace = saved.workspace.unwrap();
        assert_eq!(path_from_bytes(&workspace.tabs[0]).unwrap(), relative);
        assert_eq!(&workspace.tabs[0][..2], &0xd800_u16.to_le_bytes());
        assert_eq!(workspace.selected.as_ref(), Some(&workspace.tabs[0]));
        assert_eq!(
            path_from_bytes(&workspace.root).unwrap(),
            fixture.workspace.root()
        );
    }

    #[cfg(windows)]
    #[test]
    fn rejects_invalid_windows_path_encoding() {
        let fixture = Fixture::new();
        let file = fixture.document("a.md", "note");
        let mut session = Session::capture(
            Some(&fixture.workspace),
            std::iter::once(file.as_path()),
            Some(&file),
        );
        session
            .workspace
            .as_mut()
            .unwrap()
            .tabs
            .insert(0, vec![0xff]);
        session.write(&fixture.session_path).unwrap();
        let restored = restore(&fixture.session_path).unwrap().unwrap();
        assert_eq!(restored.documents.len(), 1);
        assert_eq!(restored.documents[0].path(), file);

        session.workspace.as_mut().unwrap().root = b"\xff".to_vec();
        session.write(&fixture.session_path).unwrap();
        assert_eq!(
            restore(&fixture.session_path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn repeated_writes_replace_the_record_without_leaving_staging_files() {
        let fixture = Fixture::new();
        let first = fixture.document("first.md", "first");
        let second = fixture.document("second.md", "second");
        for selected in [&first, &second, &first] {
            Session::capture(
                Some(&fixture.workspace),
                std::iter::once(selected.as_path()),
                Some(selected),
            )
            .write(&fixture.session_path)
            .unwrap();
            let restored = restore(&fixture.session_path).unwrap().unwrap();
            assert_eq!(restored.documents.len(), 1);
            assert_eq!(restored.documents[0].path(), selected);
            assert_eq!(restored.selected, Some(0));
            assert_eq!(
                fs::read_dir(fixture.session_path.parent().unwrap())
                    .unwrap()
                    .count(),
                1
            );
        }
    }

    #[test]
    fn corrupt_and_unsupported_sessions_return_invalid_data() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.session_path.parent().unwrap()).unwrap();
        for data in ["{", r#"{"version":2,"workspace":null}"#] {
            fs::write(&fixture.session_path, data).unwrap();
            assert_eq!(
                restore(&fixture.session_path).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn a_failed_write_preserves_the_old_record_and_removes_the_staging_file() {
        let fixture = Fixture::new();
        let session = Session::capture(Some(&fixture.workspace), std::iter::empty(), None);
        session.write(&fixture.session_path).unwrap();
        let original = fs::read(&fixture.session_path).unwrap();
        let result = write_atomically(&fixture.session_path, |file| {
            file.write_all(b"part of the new record")?;
            Err(io::Error::other("simulated write failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&fixture.session_path).unwrap(), original);
        assert_eq!(
            fs::read_dir(fixture.session_path.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            restore(&fixture.session_path).unwrap().unwrap().workspace,
            fixture.workspace
        );
    }
}
