//! Windows publication with pinned directories and a recoverable ReplaceFileW backup.

use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Component, Path, PathBuf},
    ptr,
};

use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY,
        FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
        MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW,
    },
};

use super::{DocumentFile, Stamp, Workspace, external_change};

pub(super) fn create(
    workspace: &Workspace,
    path: &Path,
    source: &str,
) -> io::Result<(Stamp, (u64, u64))> {
    let parent = destination(workspace, path)?;
    let parent_identity = identity(parent.last().expect("the parent is pinned"))?;
    let stage = Stage::new(path.parent().expect("a document has a parent"))?;
    let draft = stage.write_draft(source)?;
    let draft_stamp = Stamp::read_file(&draft)?;
    // No MOVEFILE_REPLACE_EXISTING: even a late entry must not be overwritten.
    let from = wide(&stage.version())?;
    let to = wide(path)?;
    // SAFETY: Both paths are NUL-terminated and their directories remain pinned.
    status(unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) })?;
    let verified = check_published(path, &draft_stamp, source);
    verified
        .map(|stamp| (stamp, parent_identity))
        .map_err(|error| {
            io::Error::other(format!(
                "could not verify the new file at {} ({error}); the draft may be at the document path",
                path.display()
            ))
        })
}

pub(super) fn save(
    document: &DocumentFile,
    source: &str,
    before_exchange: impl FnOnce(),
) -> io::Result<Stamp> {
    let parent = destination(&document.directory, &document.path)?;
    if identity(parent.last().expect("the parent is pinned"))? != document.parent_identity {
        return Err(folder_moved());
    }
    // This open checks write access without creating or truncating the original.
    let mut original = open_file(&document.path, true)?;
    check_original(document, &original)?;
    let mut stage = Stage::new(document.path.parent().expect("a document has a parent"))?;
    let draft = stage.write_draft(source)?;
    let draft_stamp = Stamp::read_file(&draft)?;
    // ReplaceFileW requires exclusive access to the replacement file.
    drop(draft);
    if !same_bytes(&mut original, &document.source)? {
        return Err(external_change());
    }
    check_original(document, &open_file(&document.path, true)?)?;
    drop(original);
    let target = wide(&document.path)?;
    let replacement = wide(&stage.version())?;
    let backup = wide(&stage.displaced())?;
    before_exchange();
    // A failed ReplaceFileW can also leave a backup or move the replacement. Do not
    // remove either entry after this point until all versions have been verified.
    stage.keep = true;
    // SAFETY: All paths are NUL-terminated; every ancestor and the private stage are
    // pinned against rename/deletion. No IGNORE_MERGE_ERRORS flag: preserving the
    // original DACL, attributes and streams is required, not best effort.
    let replaced = status(unsafe {
        ReplaceFileW(
            target.as_ptr(),
            replacement.as_ptr(),
            backup.as_ptr(),
            0,
            ptr::null(),
            ptr::null(),
        )
    });
    let verified = replaced.and_then(|()| {
        let mut displaced = open_file(&stage.displaced(), false)?;
        let before = Stamp::read_file(&displaced)?;
        if before != document.stamp
            || !same_bytes(&mut displaced, &document.source)?
            || before != Stamp::read_file(&displaced)?
        {
            return Err(external_change());
        }
        check_published(&document.path, &draft_stamp, source)
    });
    match verified {
        Ok(stamp) => {
            stage.keep = false;
            Ok(stamp)
        }
        Err(error) => Err(io::Error::other(format!(
            "could not verify the replacement ({error}); the draft may be at the document path; \
             recovery entries were kept in {} (the displaced entry, if present, is {})",
            stage.path.display(),
            stage.displaced().display()
        ))),
    }
}

fn check_original(document: &DocumentFile, file: &File) -> io::Result<()> {
    if file.metadata()?.permissions().readonly() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the file is read-only",
        ));
    }
    if Stamp::read_file(file)? != document.stamp {
        return Err(external_change());
    }
    Ok(())
}

fn check_published(path: &Path, draft: &Stamp, source: &str) -> io::Result<Stamp> {
    let mut published = open_file(path, false)?;
    let stamp = Stamp::read_file(&published)?;
    if stamp != *draft || !same_bytes(&mut published, source)? || self::stamp(path)? != stamp {
        return Err(external_change());
    }
    Ok(stamp)
}

fn same_bytes(file: &mut File, expected: &str) -> io::Result<bool> {
    let mut bytes = Vec::new();
    file.take(expected.len() as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes == expected.as_bytes())
}

/// Hold every canonical ancestor without delete sharing. Unlike a check-then-write by
/// path, this prevents a directory rename or junction substitution during publication.
fn destination(workspace: &Workspace, path: &Path) -> io::Result<Vec<File>> {
    let parent = path.parent().ok_or_else(folder_moved)?;
    let name = path.file_name().ok_or_else(folder_moved)?;
    if name
        .encode_wide()
        .any(|unit| matches!(unit, 0 | 47 | 58 | 92))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid file name",
        ));
    }
    if !parent.starts_with(&workspace.root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the path is outside the workspace",
        ));
    }
    let mut current = PathBuf::new();
    let mut held = Vec::new();
    let mut found_root = false;
    for component in parent.components() {
        current.push(component.as_os_str());
        match component {
            Component::Prefix(_) => continue,
            Component::RootDir | Component::Normal(_) => {}
            _ => return Err(folder_moved()),
        }
        let directory = open_directory(&current)?;
        if current == workspace.root {
            if identity(&directory)? != workspace.identity {
                return Err(folder_moved());
            }
            found_root = true;
        }
        held.push(directory);
    }
    if !found_root {
        return Err(folder_moved());
    }
    Ok(held)
}

pub(super) fn open_search_file(workspace: &Workspace, path: &Path) -> io::Result<File> {
    let _parent = destination(workspace, path)?;
    open_file(path, false)
}

pub(super) fn directory_identity(path: &Path) -> io::Result<(u64, u64)> {
    identity(&open_directory(path)?)
}

fn open_directory(path: &Path) -> io::Result<File> {
    // ReplaceFileW requires directory write sharing. These handles prevent renames,
    // not in-place reparse-data changes by an actor with directory write access.
    let file = OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() || is_reparse(&metadata) {
        return Err(folder_moved());
    }
    Ok(file)
}

fn open_file(path: &Path, write: bool) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(write)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err(external_change());
    }
    Ok(file)
}

pub(super) fn is_reparse(metadata: &Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

pub(super) fn folder_moved() -> io::Error {
    io::Error::other("the workspace or destination folder has moved")
}

pub(super) fn stamp(path: &Path) -> io::Result<Stamp> {
    Stamp::read_file(&open_file(path, false)?)
}

fn identity(file: &File) -> io::Result<(u64, u64)> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: The file handle is live and info points to a correctly sized output value.
    status(unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) })?;
    Ok((
        u64::from(info.dwVolumeSerialNumber),
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    ))
}

impl Stamp {
    pub(super) fn read_file(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            modified: metadata.modified()?,
            len: metadata.len(),
            permissions: metadata.permissions(),
            identity: identity(file)?,
        })
    }
}

struct Stage {
    path: PathBuf,
    directory: Option<File>,
    keep: bool,
}

impl Stage {
    fn new(parent: &Path) -> io::Result<Self> {
        // tempfile only generates a unique basename. Cleanup must never recurse by path.
        tempfile::Builder::new()
            .prefix(".gpui-save-")
            .rand_bytes(12)
            .disable_cleanup(true)
            .make_in(parent, |path| {
                private_directory(path)?;
                let directory = open_directory(path)?;
                Ok(Self {
                    path: path.to_owned(),
                    directory: Some(directory),
                    keep: false,
                })
            })
            .map(tempfile::NamedTempFile::into_file)
    }

    fn version(&self) -> PathBuf {
        self.path.join("version")
    }

    fn displaced(&self) -> PathBuf {
        self.path.join("displaced")
    }

    fn write_draft(&self, source: &str) -> io::Result<File> {
        let mut draft = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(self.version())?;
        draft.write_all(source.as_bytes())?;
        draft.sync_all()?;
        Ok(draft)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        let _ = fs::remove_file(self.version());
        let _ = fs::remove_file(self.displaced());
        // All ancestor handles remain held by the caller. Remove only this empty folder,
        // never a recursive tree, after releasing its sharing restriction.
        drop(self.directory.take());
        let _ = fs::remove_dir(&self.path);
    }
}

fn private_directory(path: &Path) -> io::Result<()> {
    // Protected DACL: only the owner has access. Inherit the same rule onto draft files.
    let path = wide(path)?;
    let descriptor: Vec<u16> = "D:P(A;OICI;FA;;;OW)\0".encode_utf16().collect();
    let mut security = ptr::null_mut();
    // SAFETY: The descriptor is NUL-terminated and security is a valid output pointer.
    status(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor.as_ptr(),
            SDDL_REVISION_1,
            &mut security,
            ptr::null_mut(),
        )
    })?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security,
        bInheritHandle: 0,
    };
    // SAFETY: Both the path and security descriptor are valid for this call.
    let result = status(unsafe { CreateDirectoryW(path.as_ptr(), &attributes) });
    // SAFETY: The conversion allocated security with LocalAlloc; this is its sole owner.
    unsafe { LocalFree(security) };
    result
}

fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut units: Vec<u16> = path.as_os_str().encode_wide().collect();
    if units.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"));
    }
    units.push(0);
    Ok(units)
}

fn status(result: i32) -> io::Result<()> {
    if result != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_external_saves_keep_the_displaced_version() {
        for replace in [false, true] {
            let folder = tempfile::tempdir().unwrap();
            let workspace = Workspace::open(folder.path()).unwrap();
            let path = workspace.root().join("document.md");
            fs::write(&path, "original").unwrap();
            let document = workspace.read_document(&path).unwrap();
            let error = save(&document, "editor draft", || {
                if replace {
                    let competing = workspace.root().join("external.tmp");
                    fs::write(&competing, "competing version").unwrap();
                    fs::rename(competing, &path).unwrap();
                } else {
                    fs::write(&path, "competing version").unwrap();
                }
            })
            .unwrap_err();
            let recovery = fs::read_dir(workspace.root())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".gpui-save-")
                })
                .unwrap()
                .join("displaced");
            assert!(
                error.to_string().contains(&recovery.display().to_string()),
                "{error}"
            );
            assert_eq!(
                fs::read_to_string(&recovery)
                    .unwrap_or_else(|read_error| panic!("{error}; {read_error}")),
                "competing version"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), "editor draft");
            assert_eq!(document.source(), "original");
        }
    }

    #[test]
    fn pinned_ancestors_cannot_be_renamed_during_publication() {
        let folder = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(folder.path()).unwrap();
        let parent = workspace.root().join("nested");
        fs::create_dir(&parent).unwrap();
        let held = destination(&workspace, &parent.join("document.md")).unwrap();
        assert!(fs::rename(&parent, workspace.root().join("moved")).is_err());
        assert!(fs::rename(workspace.root(), folder.path().with_extension("moved")).is_err());
        drop(held);
        fs::rename(&parent, workspace.root().join("moved")).unwrap();
    }
}
